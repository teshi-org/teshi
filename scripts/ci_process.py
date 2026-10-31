"""Owned process trees and deadline-bound JSON-line requests for CI fixtures."""

from collections import deque
import ctypes
from ctypes import wintypes
from contextlib import suppress
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import threading
import time


class WindowsJob:
    """Keep descendants contained even if their parent exits before cleanup."""

    def __init__(self):
        class Limits(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                        ("flags", wintypes.DWORD), ("min_ws", ctypes.c_size_t),
                        ("max_ws", ctypes.c_size_t), ("processes", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [("basic", Limits), ("io", ctypes.c_uint64 * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process_memory", ctypes.c_size_t), ("peak_job_memory", ctypes.c_size_t)]

        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.api.CreateJobObjectW.restype = wintypes.HANDLE
        self.api.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.api.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.api.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.api.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        self.api.CloseHandle.argtypes = [wintypes.HANDLE]
        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.api.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.WinError(ctypes.get_last_error())
            self.close()
            raise error

    def attach(self, process):
        if not self.api.AssignProcessToJobObject(self.handle, wintypes.HANDLE(int(process._handle))):
            raise ctypes.WinError(ctypes.get_last_error())

    def close(self):
        if self.handle:
            if not self.api.CloseHandle(self.handle):
                raise ctypes.WinError(ctypes.get_last_error())
            self.handle = None

    def terminate(self, timeout):
        class Accounting(ctypes.Structure):
            _fields_ = [("user_time", ctypes.c_int64), ("kernel_time", ctypes.c_int64),
                        ("period_user_time", ctypes.c_int64), ("period_kernel_time", ctypes.c_int64),
                        ("page_faults", wintypes.DWORD), ("total", wintypes.DWORD),
                        ("active", wintypes.DWORD), ("terminated", wintypes.DWORD)]

        try:
            if not self.api.TerminateJobObject(self.handle, 1):
                raise ctypes.WinError(ctypes.get_last_error())
            deadline = time.monotonic() + timeout
            while True:
                accounting = Accounting()
                if not self.api.QueryInformationJobObject(self.handle, 1, ctypes.byref(accounting), ctypes.sizeof(accounting), None):
                    raise ctypes.WinError(ctypes.get_last_error())
                if accounting.active == 0:
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"owned Job still contains {accounting.active} processes")
                time.sleep(.01)
        finally:
            self.close()


class OwnedProcess:
    """Own a fresh POSIX session or Windows Job, never a process-name match."""

    def __init__(self, command, **kwargs):
        self.job = WindowsJob() if os.name == "nt" else None
        try:
            self.process = subprocess.Popen(command, start_new_session=os.name != "nt", **kwargs)
            if self.job:
                self.job.attach(self.process)
        except BaseException:
            if hasattr(self, "process"):
                self.process.kill()
                self.process.wait(timeout=5)
            if self.job:
                self.job.close()
            raise
        self.closed = False

    def stop(self, timeout=5):
        if self.closed:
            return
        # Killing the container also collects descendants after a normal root exit.
        if self.job:
            self.job.terminate(timeout)
        else:
            # PTYs and nested supervisors may create their own sessions. Capture
            # the live owned subtree before killing its parent; process-group
            # cleanup alone would miss these descendants on Linux.
            descendants = self._linux_descendants()
            for pid, birth in reversed(descendants):
                current = self._linux_process(pid)
                if current and current[1] == birth[1]:
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.process.wait(timeout=timeout)
        self.closed = True

    @staticmethod
    def _linux_process(pid):
        try:
            fields = Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()
            return int(fields[1]), int(fields[19])  # parent PID and start ticks
        except (FileNotFoundError, ProcessLookupError):
            return None

    def _linux_descendants(self):
        if not sys.platform.startswith("linux"):
            return []
        processes = {}
        for directory in Path("/proc").iterdir():
            if directory.name.isdigit():
                identity = self._linux_process(int(directory.name))
                if identity:
                    processes[int(directory.name)] = identity
        owned = {self.process.pid}
        descendants = []
        while True:
            new = [(pid, identity) for pid, identity in processes.items()
                   if pid not in owned and identity[0] in owned]
            if not new:
                return descendants
            descendants.extend(new)
            owned.update(pid for pid, _ in new)


class HostClient:
    """A single I/O worker bounds writes, flushes and reads on Windows pipes too.

    A timed-out stream is poisoned: late replies cannot become the next reply,
    and failed mutations are never retried. The caller owns final tree cleanup.
    """

    def __init__(self, command, evidence, env=None):
        self.evidence = Path(evidence)
        self.evidence.mkdir(parents=True, exist_ok=True)
        self.stderr = (self.evidence / "terminal-host.stderr.log").open("w", encoding="utf-8")
        try:
            self.owned = OwnedProcess(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=self.stderr, text=True, encoding="utf-8", env=env)
        except BaseException:
            self.stderr.close()
            raise
        self.process = self.owned.process
        self.requests = queue.Queue()
        self.poisoned = False
        self.last_screen = ""
        self.last_capture = None
        self.current = None
        self.history = deque(maxlen=20)
        self.worker = threading.Thread(target=self._serve, daemon=True)
        self.worker.start()

    def _serve(self):
        while True:
            item = self.requests.get()
            if item is None:
                return
            payload, response = item
            try:
                self.process.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
                self.process.stdin.flush()
                line = self.process.stdout.readline()
                if not line:
                    raise EOFError("terminal-host closed stdout")
                response.put((json.loads(line), None))
            except Exception as error:
                response.put((None, error))

    def request(self, payload, deadline, context=None):
        if self.poisoned:
            raise RuntimeError("terminal-host stream is unusable after an earlier failure")
        started = time.monotonic()
        if deadline <= started:
            raise TimeoutError("terminal-host request budget exhausted before dispatch")
        self.current = dict(request=payload, context=context, deadline=deadline)
        response = queue.Queue(maxsize=1)
        self.requests.put((payload, response))
        status = "failed"
        try:
            try:
                reply, error = response.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty:
                raise TimeoutError(f"terminal-host {payload['op']} exceeded its request deadline") from None
            if error:
                raise error
            if not isinstance(reply, dict) or not isinstance(reply.get("ok"), bool):
                raise ValueError("invalid terminal-host reply")
            if not reply["ok"]:
                # A valid error reply keeps framing intact, unlike timeout/EOF.
                status = "rejected"
                raise AssertionError(reply.get("error", "terminal-host rejected request"))
            value = reply.get("value")
            if payload["op"] == "screen":
                if not isinstance(value, str):
                    raise ValueError("terminal-host screen reply must be text")
                self.last_screen = value
            elif payload["op"] == "capture":
                self.last_capture = dict(directory=payload.get("directory"), files=value)
            status = "passed"
            return value
        except Exception as error:
            self.poisoned = status != "rejected"
            self.diagnose(error)
            raise
        finally:
            record = dict(op=payload["op"], status=status,
                          elapsed_seconds=time.monotonic() - started, context=context)
            self.history.append(record)
            with (self.evidence / "requests.ndjson").open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(record, ensure_ascii=False) + "\n")

    def diagnose(self, error):
        # Never issue another request on the failed pipe to collect diagnostics.
        diagnostic = dict(error=str(error), error_type=type(error).__name__, current=self.current,
                          pid=self.process.pid, returncode=self.process.poll(), poisoned=self.poisoned,
                          worker_alive=self.worker.is_alive(), recent_requests=list(self.history),
                          last_successful_capture=self.last_capture)
        with (self.evidence / "failures.ndjson").open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(diagnostic, ensure_ascii=False) + "\n")
        primary = self.evidence / "timeout-diagnostic.json"
        if not primary.exists():
            primary.write_text(json.dumps(diagnostic, ensure_ascii=False, indent=2), encoding="utf-8")
            (self.evidence / "last-successful-screen.txt").write_text(self.last_screen, encoding="utf-8")

    def close(self):
        self.poisoned = True
        self.owned.stop()
        self.requests.put(None)
        self.worker.join(timeout=1)
        # Closing a TextIOWrapper from another thread can itself block on its lock.
        if not self.worker.is_alive():
            with suppress(BrokenPipeError):
                self.process.stdin.close()
            self.process.stdout.close()
        self.stderr.close()
