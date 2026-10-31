"""Real owned-process fault injection without touching any native application."""
import asyncio
import json
import io
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import winapp_service as service

WORKER = '''import sys,json,time
for line in sys.stdin:
 p=json.loads(line)
 if p.get("cmd")=="mutate_then_hang":
  with open(p["counter"],"a") as f: f.write("effect\\n")
  time.sleep(60)
 if p.get("cmd")=="hang": time.sleep(60)
 if p.get("cmd")=="preview": time.sleep(0.2)
 if p.get("cmd")=="raw":
  print(p["raw"],flush=True)
  continue
 if p.get("cmd")=="bad_response":
  time.sleep(0.2)
  print("123",flush=True)
  continue
 print(json.dumps({"ok":True,"echo":p.get("cmd"),"_target_identity":p.get("identity")}),flush=True)
'''


class SupervisorTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.supervisor = service.WinAppSupervisor(self.root, [sys.executable, "-u", "-c", WORKER])
        self.addAsyncCleanup(self.supervisor.stop_worker)

    async def test_blocked_worker_does_not_block_health_and_recovers(self):
        self.assertTrue((await self.supervisor.execute({"cmd": "warmup"}))["ok"])
        started = time.monotonic()
        task = asyncio.create_task(self.supervisor.execute({"cmd": "hang", "timeout_ms": 300, "request_id": "stuck"}))
        await asyncio.sleep(0.05)
        worker = self.supervisor.worker
        health = await asyncio.wait_for(self.supervisor.execute({"cmd": "health"}), 0.1)
        self.assertTrue(health["busy"])
        self.assertTrue(health["active_request"].startswith("stuck:"))
        outcome = await task
        self.assertEqual(outcome["code"], "command_timeout")
        self.assertFalse(outcome["retry_safe"])
        self.assertLess(time.monotonic() - started, 1.5)
        self.assertIsNotNone(worker.returncode)
        self.assertTrue((await self.supervisor.execute({"cmd": "recovered"}))["ok"])

    async def test_cancel_is_identity_scoped_and_next_request_recovers(self):
        await self.supervisor.execute({"cmd": "warmup"})
        task = asyncio.create_task(self.supervisor.execute({"cmd": "hang", "request_id": "one"}))
        await asyncio.sleep(0.05)
        mismatch = await self.supervisor.execute({"cmd": "cancel", "cancel_request_id": "other"})
        self.assertEqual(mismatch["code"], "request_not_active")
        health = await self.supervisor.execute({"cmd": "health"})
        cancelled = await self.supervisor.execute({"cmd": "cancel", "cancel_request_id": health["active_request"]})
        self.assertTrue(cancelled["ok"])
        self.assertEqual((await task)["code"], "command_cancelled")
        self.assertTrue((await self.supervisor.execute({"cmd": "next"}))["ok"])

    async def test_target_identity_survives_restart_and_rejects_reused_window(self):
        identity = {"hwnd": 10, "pid": 20, "created": 30}
        await self.supervisor.execute({"cmd": "attach_window", "identity": identity})
        restarted = service.WinAppSupervisor(self.root)
        self.assertEqual(restarted.target_identity, identity)
        with patch.object(service, "native_target_identity", return_value={**identity, "created": 31}):
            self.assertFalse((await restarted.execute({"cmd": "health"}))["target_valid"])
            result = await self.supervisor.execute({"cmd": "execute_locator"})
            self.assertEqual(result["code"], "target_invalid")

    async def test_busy_request_is_not_dispatched_and_timeout_is_validated(self):
        invalid = await self.supervisor.execute({"cmd": "hang", "timeout_ms": -1})
        self.assertEqual(invalid["code"], "invalid_timeout")
        task = asyncio.create_task(self.supervisor.execute({"cmd": "hang", "timeout_ms": 300}))
        await asyncio.sleep(0.05)
        busy = await self.supervisor.execute({"cmd": "execute_locator"})
        self.assertEqual(busy["code"], "executor_busy")
        await task

    async def test_non_object_response_is_structured_and_worker_recovers(self):
        result = await self.supervisor.execute({"cmd": "bad_response"})
        self.assertEqual(result["code"], "executor_failed")
        self.assertFalse(result["retry_safe"])
        self.assertTrue((await self.supervisor.execute({"cmd": "next"}))["ok"])

    async def test_business_waits_for_preview_and_new_preview_yields(self):
        preview = asyncio.create_task(self.supervisor.execute({"cmd": "preview"}, background=True))
        await asyncio.sleep(0.05)
        business = asyncio.create_task(self.supervisor.execute({"cmd": "click"}))
        await asyncio.sleep(0)
        skipped = await self.supervisor.execute({"cmd": "preview"}, background=True)
        self.assertEqual(skipped["code"], "preview_skipped")
        self.assertTrue((await preview)["ok"])
        self.assertEqual((await business)["echo"], "click")

    async def test_window_lifecycle_checks_process_instead_of_destroyed_hwnd(self):
        identity = {"hwnd": 10, "pid": 20, "created": 30}
        await self.supervisor.execute({"cmd": "attach_window", "identity": identity})
        with patch.object(service, "native_target_identity", return_value=None), patch.object(
                service, "native_process_created", return_value=30):
            result = await self.supervisor.execute({"cmd": "wait_window", "pid": 20, "state": "hidden"})
            self.assertTrue(result["ok"])
            self.assertEqual(self.supervisor.target_identity, identity)
        with patch.object(service, "native_process_created", return_value=31):
            result = await self.supervisor.execute({"cmd": "wait_window", "pid": 20})
            self.assertEqual(result["code"], "target_invalid")

    async def test_wait_window_hidden_and_recreated_window(self):
        session = service.WinAppSession(self.root)
        session.hwnd, session.target_pid = 10, 20
        hidden = {"visible": False, "hwnd": None, "title": ""}
        visible = {"visible": True, "hwnd": 11, "title": "rebuilt"}
        payload = {"pid": 20, "_expected_process_created": 30}
        with patch.object(service, "native_process_created", return_value=30), patch.object(
                service, "process_window_state", return_value=hidden):
            self.assertTrue(session.wait_window({**payload, "state": "hidden"})["ok"])
        with patch.object(service, "native_process_created", return_value=30), patch.object(
                service, "process_window_state", side_effect=[hidden, visible]), patch.object(
                service.time, "sleep"), patch.object(service, "find_window", return_value=visible), patch.object(
                session, "_attach_target") as attach:
            self.assertEqual(session.wait_window(payload)["hwnd"], 11)
            attach.assert_called_once_with(visible)
        with patch.object(service, "native_process_created", return_value=31), patch.object(
                service, "process_window_state") as query:
            self.assertEqual(session.wait_window(payload)["code"], "target_invalid")
            query.assert_not_called()

    async def test_launch_paths_isolate_application_stdio(self):
        session = service.WinAppSession(self.root)
        window = {"hwnd": 10, "pid": 20, "title": "fixture"}
        with patch.object(service.subprocess, "Popen") as spawn, patch.object(
                service, "find_window", return_value=window), patch.object(session, "_attach_target"), patch.object(
                session, "target_info", return_value={}), patch.object(service, "_process_is_running", return_value=False):
            self.assertTrue((await session.launch({"path": "fixture.exe"}))["ok"])
            self.assertEqual(spawn.call_args.kwargs["stdout"], service.subprocess.DEVNULL)
            self.assertEqual(spawn.call_args.kwargs["stdin"], service.subprocess.DEVNULL)
            self.assertTrue(session._handle_exec("launch", "fixture.exe")["ok"])
            self.assertEqual(spawn.call_args.kwargs["stdout"], service.subprocess.DEVNULL)

    async def wait_until_active(self, request=None):
        # Liveness wait for the in-flight request, not a timing assertion: a
        # loaded runner can take seconds to start the worker process. A request
        # that already finished can never become active again, so stop there.
        async with asyncio.timeout(10):
            while self.supervisor.active_request is None:
                if request is not None and request.done():
                    return
                await asyncio.sleep(0.001)

    async def test_malformed_response_matrix_and_followup_alignment(self):
        for raw in ['null', '[]', '"log"', '{}', '{"ok":1}', '{"ok":"true"}',
                    'not json', '{"ok":true,"_target_identity":[]}']:
            with self.subTest(raw=raw):
                result = await self.supervisor.execute({"cmd": "raw", "raw": raw})
                self.assertEqual(result["code"], "executor_failed")
                self.assertIsNone(self.supervisor.worker)
                self.assertFalse(self.supervisor.lock.locked())
                followup = await self.supervisor.execute({"cmd": "followup"})
                self.assertEqual(followup["echo"], "followup")

    async def test_queue_deadline_does_not_cancel_preview(self):
        await self.supervisor.execute({"cmd": "warmup"})
        preview = asyncio.create_task(self.supervisor.execute({"cmd": "preview"}, background=True))
        await self.wait_until_active()
        worker = self.supervisor.worker
        result = await self.supervisor.execute({"cmd": "must_not_run", "timeout_ms": 20})
        self.assertEqual(result["code"], "command_timeout")
        self.assertEqual(result["stage"], "queue")
        self.assertTrue(result["retry_safe"])
        self.assertIs(self.supervisor.worker, worker)
        self.assertTrue((await preview)["ok"])
        self.assertEqual(self.supervisor.business_pending, 0)

    async def test_multiple_business_requests_queued_behind_preview(self):
        preview = asyncio.create_task(self.supervisor.execute({"cmd": "preview"}, background=True))
        await self.wait_until_active()
        tasks = [asyncio.create_task(self.supervisor.execute({"cmd": str(i)})) for i in range(3)]
        await asyncio.sleep(0)
        await preview
        results = await asyncio.gather(*tasks)
        self.assertEqual([r.get("echo") for r in results], ["0", "1", "2"])
        self.assertFalse(self.supervisor.lock.locked())
        self.assertEqual(self.supervisor.business_pending, 0)
        self.assertFalse(self.supervisor.preview_active)

    async def test_cancelled_queue_waiter_leaves_preview_running(self):
        preview = asyncio.create_task(self.supervisor.execute({"cmd": "preview"}, background=True))
        await self.wait_until_active()
        waiter = asyncio.create_task(self.supervisor.execute({"cmd": "cancelled"}))
        await asyncio.sleep(0)
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertTrue((await preview)["ok"])
        self.assertEqual(self.supervisor.business_pending, 0)
        self.assertTrue((await self.supervisor.execute({"cmd": "next"}))["ok"])

    async def test_preview_timeout_and_crash_release_waiting_business(self):
        for command in ({"cmd": "hang", "timeout_ms": 100}, {"cmd": "bad_response"}):
            with self.subTest(command=command):
                preview = asyncio.create_task(self.supervisor.execute(command, background=True))
                await self.wait_until_active(preview)
                business = asyncio.create_task(self.supervisor.execute({"cmd": "next"}))
                self.assertFalse((await preview)["ok"])
                self.assertEqual((await business)["echo"], "next")
                self.assertFalse(self.supervisor.preview_active)
                self.assertEqual(self.supervisor.business_pending, 0)

    async def test_process_reuse_during_window_poll_is_rejected(self):
        session = service.WinAppSession(self.root)
        with patch.object(service, "native_process_created", side_effect=[30, 31]), patch.object(
                service, "process_window_state", return_value={"visible": False}), patch.object(service.time, "sleep"):
            result = session.wait_window({"pid": 20, "_expected_process_created": 30})
            self.assertEqual(result["code"], "target_invalid")

    async def test_real_launched_app_output_cannot_shift_protocol_responses(self):
        child = self.root / "noisy.py"
        child.write_text('import sys\nprint("123",flush=True)\nprint("application log",file=sys.stderr,flush=True)\n')
        worker_code = r'''import asyncio,json,sys
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0, sys.argv[1])
import winapp_service as service
session = service.WinAppSession(None)
window = {"hwnd": 10, "pid": 20, "title": "fixture"}
for line in sys.stdin:
 p=json.loads(line)
 if p["cmd"] == "launch":
  with patch.object(service,"find_window",return_value=window), patch.object(session,"_attach_target"), patch.object(session,"target_info",return_value={}):
   result=asyncio.run(session.launch({"path":sys.executable,"args":[sys.argv[2]]}))
  session.process.wait(timeout=2)
 elif p["cmd"] == "exec_launch":
  real_spawn=service.subprocess.Popen
  children=[]
  def spawn(args, **kwargs):
   child=real_spawn([sys.executable,sys.argv[2]], **kwargs)
   children.append(child)
   return child
  with patch.object(service,"_process_is_running",return_value=False), patch.object(service.subprocess,"Popen",side_effect=spawn):
   result=session._handle_exec("launch","fixture.exe")
  children[0].wait(timeout=2)
 else:
  result={"ok":True}
 result["echo"]=p["cmd"]
 print(json.dumps(result),flush=True)
'''
        supervisor = service.WinAppSupervisor(self.root, [sys.executable, "-u", "-c", worker_code,
            str(Path(service.__file__).parent), str(child)])
        self.addAsyncCleanup(supervisor.stop_worker)
        for command in ("launch", "after_launch", "exec_launch", "after_exec"):
            response = await supervisor.execute({"cmd": command})
            self.assertTrue(response["ok"], response)
            self.assertEqual(response["echo"], command)

    async def test_queue_and_execution_share_one_deadline(self):
        await self.supervisor.execute({"cmd": "warmup"})
        preview = asyncio.create_task(self.supervisor.execute({"cmd": "preview"}, background=True))
        await self.wait_until_active()
        # Each operation takes 200ms; a fresh budget after queueing would incorrectly pass.
        result = await self.supervisor.execute({"cmd": "preview", "timeout_ms": 300})
        self.assertTrue((await preview)["ok"])
        self.assertEqual(result["code"], "command_timeout")
        self.assertEqual(result["stage"], "native_execution")
        self.assertFalse(self.supervisor.lock.locked())
        self.assertTrue((await self.supervisor.execute({"cmd": "next"}))["ok"])

    async def test_rebuilt_window_identity_is_persisted_and_reusable(self):
        old = {"hwnd": 10, "pid": 20, "created": 30}
        rebuilt = {**old, "hwnd": 11}
        await self.supervisor.execute({"cmd": "attach_window", "identity": old})
        with patch.object(service, "native_process_created", return_value=30):
            result = await self.supervisor.execute({"cmd": "wait_window", "pid": 20, "identity": rebuilt})
            self.assertTrue(result["ok"])
        restarted = service.WinAppSupervisor(self.root)
        self.assertEqual(restarted.target_identity, rebuilt)
        with patch.object(service, "native_target_identity", return_value=rebuilt):
            self.assertTrue((await self.supervisor.execute({"cmd": "get_ui_snapshot"}))["ok"])

    async def test_preview_restore_failure_preserves_lifecycle_recovery_identity(self):
        identity = {"hwnd": 10, "pid": 20, "created": 30}
        worker = r'''import sys,json
for line in sys.stdin:
 p=json.loads(line)
 if "_restore_target" in p:
  ok=p.get("_lifecycle",False)
  result={"ok":ok,"code":"restored" if ok else "target_invalid",
          "_target_identity":p["_restore_target"] if ok else None}
 else:
  result={"ok":True,"_target_identity":None}
 print(json.dumps(result),flush=True)
'''
        supervisor = service.WinAppSupervisor(self.root, [sys.executable, "-u", "-c", worker])
        self.addAsyncCleanup(supervisor.stop_worker)
        supervisor.target_identity = identity
        result = await supervisor.execute({"cmd": "screenshot"}, background=True)
        self.assertEqual(result["code"], "target_invalid")
        self.assertEqual(supervisor.target_identity, identity)
        self.assertIsNone(supervisor.worker)
        with patch.object(service, "native_process_created", return_value=30):
            result = await supervisor.execute({"cmd": "wait_window", "pid": 20, "state": "hidden"})
        self.assertTrue(result["ok"])
        self.assertEqual(supervisor.target_identity, identity)

    async def test_uncertain_mutation_is_never_replayed_on_recovery(self):
        counter = self.root / "effects.txt"
        result = await self.supervisor.execute({"cmd": "mutate_then_hang", "counter": str(counter), "timeout_ms": 500})
        self.assertEqual(result["code"], "command_timeout")
        self.assertFalse(result["retry_safe"])
        await self.supervisor.execute({"cmd": "recovered"})
        self.assertEqual(counter.read_text().splitlines(), ["effect"])


class ExecutorLifecycleTests(unittest.TestCase):
    def test_restore_destroyed_window_then_rebuild_and_execute(self):
        identity = {"hwnd": 10, "pid": 20, "created": 30}
        rebuilt = {**identity, "hwnd": 11}
        session = service.WinAppSession(None)
        commands = [{"_restore_target": identity, "_lifecycle": True},
                    {"cmd": "wait_window", "pid": 20, "state": "hidden", "_expected_process_created": 30},
                    {"cmd": "wait_window", "pid": 20, "_expected_process_created": 30},
                    {"cmd": "get_target"}]
        output = io.StringIO()
        def attach(window):
            session.hwnd, session.target_pid = window["hwnd"], window["pid"]
        with patch.object(service.threading, "Thread"), patch.object(service, "WinAppSession", return_value=session), patch.object(
                service.sys, "stdin", io.StringIO(''.join(json.dumps(p) + '\n' for p in commands))), patch.object(
                service.sys, "stdout", output), patch.object(service, "native_process_created", return_value=30), patch.object(
                service, "native_target_identity", side_effect=lambda hwnd: rebuilt if hwnd == 11 else None), patch.object(
                service, "process_window_state", side_effect=[{"visible": False, "hwnd": None, "title": ""},
                    {"visible": True, "hwnd": 11, "title": "rebuilt"}]), patch.object(
                service, "find_window", return_value={"hwnd": 11, "pid": 20, "title": "rebuilt"}), patch.object(
                session, "_attach_target", side_effect=attach), patch.object(session, "target_info", side_effect=lambda: {"hwnd": session.hwnd}):
            service.run_executor(None, 1)
        results = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertTrue(all(result["ok"] for result in results))
        self.assertEqual(results[0]["_target_identity"], identity)
        self.assertEqual(results[2]["_target_identity"], rebuilt)
        self.assertEqual(results[3]["target"]["hwnd"], 11)

    def test_lifecycle_restore_rejects_reused_process(self):
        identity = {"hwnd": 10, "pid": 20, "created": 30}
        output = io.StringIO()
        with patch.object(service.threading, "Thread"), patch.object(service.sys, "stdin", io.StringIO(
                json.dumps({"_restore_target": identity, "_lifecycle": True}) + '\n')), patch.object(
                service.sys, "stdout", output), patch.object(service, "native_process_created", return_value=31), patch.object(
                service, "native_target_identity", return_value=None):
            service.run_executor(None, 1)
        self.assertEqual(json.loads(output.getvalue())["code"], "target_invalid")


if __name__ == "__main__":
    unittest.main()
