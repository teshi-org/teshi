"""Compare the Python and Rust broker state machines on shared fixtures."""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

RESOURCES = Path(__file__).resolve().parents[1]
REPO_ROOT = RESOURCES.parent
FIXTURE_PATH = RESOURCES / "browser_contract_fixtures.json"
sys.path.insert(0, str(RESOURCES))

from browser_agent_broker import BrokerError, BrowserSessionBroker  # noqa: E402
from resources.tests.test_browser_agent_broker import heartbeat  # noqa: E402


def target(instance_id: str, window_id: int, tab_id: int) -> dict[str, object]:
    return {
        "extension_instance_id": instance_id,
        "window_id": window_id,
        "tab_id": tab_id,
    }


async def prepare_python_pending(
    broker: BrowserSessionBroker,
    *,
    instance_id: str,
    request_id: str,
    project: str,
    caller: str,
    lease: dict[str, object] | None = None,
) -> tuple[object, asyncio.Future[dict], dict[str, object]]:
    record = broker.sessions.get(instance_id)
    if record is None:
        record = broker.register_heartbeat(heartbeat(instance_id))
    if lease is None:
        lease = broker.acquire_lease(
            instance_id,
            caller,
            30,
            project_root=project,
            caller_label=caller,
        )
    command = {
        "cmd": "get_page_snapshot",
        "request_id": request_id,
        "target": target(instance_id, 7, 42),
        "lease_token": lease["lease_token"],
    }
    broker.authorize_command(
        {
            **command,
            "project_root": project,
            "caller_label": caller,
        }
    )
    future = asyncio.get_running_loop().create_future()
    broker.queue_command(record, command["target"], command, future)
    queued = broker.heartbeat_response(record)
    assert queued["cmd"]["request_id"] == request_id
    return record, future, lease


async def python_lease_scope_oracle(
    scenario: dict[str, object],
) -> dict[str, object]:
    owner = scenario["owner"]
    assert isinstance(owner, dict)
    broker = BrowserSessionBroker()
    broker.register_heartbeat(heartbeat("profile-a"))
    broker.register_heartbeat(heartbeat("profile-b"))
    lease = broker.acquire_lease(
        owner["extension_instance_id"],
        owner["caller_label"],
        30,
        project_root=owner["project_root"],
        caller_label=owner["caller_label"],
    )
    errors = []
    for mismatch in scenario["mismatches"]:
        assert isinstance(mismatch, dict)
        command = {
            "cmd": "get_page_snapshot",
            "request_id": f"lease-scope-{mismatch['name']}",
            "target": target(mismatch["extension_instance_id"], 7, 42),
            "lease_token": lease["lease_token"],
            "project_root": mismatch["project_root"],
            "caller_label": mismatch["caller_label"],
        }
        try:
            broker.authorize_command(command)
        except BrokerError as error:
            errors.append({"name": mismatch["name"], "error": error.code})
        else:
            raise AssertionError(f"lease mismatch unexpectedly authorized: {mismatch}")
    return {
        "errors": errors,
        "queued_commands": sum(
            len(record.command_queue) for record in broker.sessions.values()
        ),
        "session_count": len(broker.sessions),
    }


async def python_cancel_response_oracle(
    scenario: dict[str, object],
) -> dict[str, object]:
    instance_id = scenario["extension_instance_id"]
    project = scenario["project_root"]
    caller = scenario["caller_label"]
    response_first = BrowserSessionBroker()
    await prepare_python_pending(
        response_first,
        instance_id=instance_id,
        request_id=scenario["response_first_request_id"],
        project=project,
        caller=caller,
    )
    request_id = scenario["response_first_request_id"]
    response_first.accept_response(
        {
            "type": "response",
            "request_id": request_id,
            "extension_instance_id": instance_id,
            "target": target(instance_id, 7, 42),
            "ok": True,
            "url": scenario["response_url"],
        }
    )
    cancel_after_response = (
        scenario["cancel_after_response_error"]
        if request_id not in response_first.pending
        else "unexpected_pending_request"
    )

    cancel_first = BrowserSessionBroker()
    _, future, _ = await prepare_python_pending(
        cancel_first,
        instance_id=instance_id,
        request_id=scenario["cancel_first_request_id"],
        project=project,
        caller=caller,
    )
    request_id = scenario["cancel_first_request_id"]
    cancel_first.cancel_request(
        request_id,
        BrokerError(scenario["cancel_error"], "fixture cancellation"),
    )
    cancelled_result = await future
    try:
        cancel_first.accept_response(
            {
                "type": "response",
                "request_id": request_id,
                "extension_instance_id": instance_id,
                "target": target(instance_id, 7, 42),
                "ok": True,
                "url": scenario["response_url"],
            }
        )
    except BrokerError as error:
        late_response = error.code
    else:
        raise AssertionError("cancelled request accepted a late response")
    return {
        "response_first": {
            "operation": "ok",
            "cancel": cancel_after_response,
        },
        "cancel_first": {
            "operation": cancelled_result["code"],
            "cancel": "cancelled",
            "late_response": late_response,
        },
    }


async def python_timeout_oracle(scenario: dict[str, object]) -> dict[str, object]:
    broker = BrowserSessionBroker()
    instance_id = scenario["extension_instance_id"]
    _, future, _ = await prepare_python_pending(
        broker,
        instance_id=instance_id,
        request_id=scenario["request_id"],
        project=scenario["project_root"],
        caller=scenario["caller_label"],
    )
    broker.cancel_request(
        scenario["request_id"],
        BrokerError(scenario["timeout_error"], "fixture timeout"),
    )
    timeout_result = await future
    try:
        broker.accept_response(
            {
                "type": "response",
                "request_id": scenario["request_id"],
                "extension_instance_id": instance_id,
                "target": target(instance_id, 7, 42),
                "ok": True,
                "url": "https://late.example.test/",
            }
        )
    except BrokerError as error:
        late_response = error.code
    else:
        raise AssertionError("timed out request accepted a late response")
    return {
        "timeout": timeout_result["code"],
        "late_response": late_response,
        "queued_commands": len(broker.sessions[instance_id].command_queue),
    }


async def python_generation_oracle(
    scenario: dict[str, object],
) -> dict[str, object]:
    broker = BrowserSessionBroker(heartbeat_ttl=0.01)
    instance_id = scenario["extension_instance_id"]
    record, future, lease = await prepare_python_pending(
        broker,
        instance_id=instance_id,
        request_id=scenario["request_id"],
        project=scenario["project_root"],
        caller=scenario["caller_label"],
    )
    broker.expire_stale(record.last_heartbeat + 1)
    disconnected = (await future)["code"]
    try:
        broker.accept_response(
            {
                "type": "response",
                "request_id": scenario["request_id"],
                "extension_instance_id": instance_id,
                "target": target(instance_id, 7, 42),
                "ok": True,
                "url": "https://old-generation.example.test/",
            }
        )
    except BrokerError as error:
        old_generation_response = error.code
    else:
        raise AssertionError("disconnected request accepted an old response")

    broker.register_heartbeat(heartbeat(instance_id))
    new_lease = broker.acquire_lease(
        instance_id,
        scenario["caller_label"],
        30,
        project_root=scenario["project_root"],
        caller_label=scenario["caller_label"],
    )
    assert new_lease["lease_token"] != lease["lease_token"]
    reused = {
        "cmd": "get_page_snapshot",
        "request_id": scenario["request_id"],
        "target": target(instance_id, 7, 42),
        "lease_token": new_lease["lease_token"],
        "project_root": scenario["project_root"],
        "caller_label": scenario["caller_label"],
    }
    try:
        broker.authorize_command(reused)
        _, _, _ = await prepare_python_pending(
            broker,
            instance_id=instance_id,
            request_id=scenario["request_id"],
            project=scenario["project_root"],
            caller=scenario["caller_label"],
            lease=new_lease,
        )
    except BrokerError as error:
        reused_request = error.code
    else:
        raise AssertionError("retired request ID was reused")
    return {
        "disconnect": disconnected,
        "old_generation_response": old_generation_response,
        "reused_request": reused_request,
        "reconnected": True,
        "session_count": len(broker.sessions),
    }


async def python_queue_fairness_oracle(
    scenario: dict[str, object],
) -> dict[str, object]:
    broker = BrowserSessionBroker()
    blocked = broker.register_heartbeat(heartbeat(scenario["blocked_profile"]))
    healthy = broker.register_heartbeat(heartbeat(scenario["healthy_profile"]))
    blocked_lease = broker.acquire_lease(
        scenario["blocked_profile"],
        scenario["caller_label"],
        30,
        project_root=scenario["project_root"],
        caller_label=scenario["caller_label"],
    )
    healthy_lease = broker.acquire_lease(
        scenario["healthy_profile"],
        "caller-b",
        30,
        project_root="project-b",
        caller_label="caller-b",
    )
    filler_index = 0
    while True:
        try:
            filler = asyncio.get_running_loop().create_future()
            broker.queue_command(
                blocked,
                target(scenario["blocked_profile"], 7, 42),
                {
                    "cmd": "get_page_snapshot",
                    "request_id": f"queue-filler-{filler_index}",
                },
                filler,
            )
            filler_index += 1
        except BrokerError as error:
            blocked_error = error.code
            break

    blocked_command = {
        "cmd": "get_page_snapshot",
        "request_id": scenario["blocked_request_id"],
        "target": target(scenario["blocked_profile"], 7, 42),
        "lease_token": blocked_lease["lease_token"],
        "project_root": scenario["project_root"],
        "caller_label": scenario["caller_label"],
    }
    broker.authorize_command(blocked_command)
    try:
        broker.queue_command(
            blocked,
            blocked_command["target"],
            blocked_command,
            asyncio.get_running_loop().create_future(),
        )
    except BrokerError as error:
        blocked_operation = error.code
    else:
        raise AssertionError("full profile queue accepted another command")

    healthy_command = {
        "cmd": "get_page_snapshot",
        "request_id": scenario["healthy_request_id"],
        "target": target(scenario["healthy_profile"], 7, 42),
        "lease_token": healthy_lease["lease_token"],
        "project_root": "project-b",
        "caller_label": "caller-b",
    }
    broker.authorize_command(healthy_command)
    healthy_future = asyncio.get_running_loop().create_future()
    broker.queue_command(
        healthy,
        healthy_command["target"],
        healthy_command,
        healthy_future,
    )
    assert broker.heartbeat_response(healthy)["cmd"]["request_id"] == scenario[
        "healthy_request_id"
    ]
    broker.accept_response(
        {
            "type": "response",
            "request_id": scenario["healthy_request_id"],
            "extension_instance_id": scenario["healthy_profile"],
            "target": target(scenario["healthy_profile"], 7, 42),
            "ok": True,
            "url": scenario["healthy_url"],
        }
    )
    healthy_result = await healthy_future
    return {
        "blocked_error": blocked_operation,
        "healthy_dispatched": healthy_result["url"] == scenario["healthy_url"],
        "healthy_queue_empty": not healthy.command_queue,
    }


async def python_oracle(fixture: dict[str, object]) -> dict[str, object]:
    stateful = fixture["stateful"]
    assert isinstance(stateful, dict)

    race = stateful["profile_response_race"]
    assert isinstance(race, dict)
    broker = BrowserSessionBroker()
    pending: dict[str, asyncio.Future[dict]] = {}
    records = {}
    requests = race["requests"]
    assert isinstance(requests, list)

    for item in requests:
        assert isinstance(item, dict)
        instance_id = item["extension_instance_id"]
        caller = f"agent-{instance_id}"
        project = f"project-{instance_id}"
        record = broker.register_heartbeat(heartbeat(instance_id))
        records[instance_id] = record
        lease = broker.acquire_lease(
            instance_id,
            caller,
            30,
            project_root=project,
            caller_label=caller,
        )
        command = {
            "cmd": "get_page_snapshot",
            "request_id": item["request_id"],
            "target": target(instance_id, item["window_id"], item["tab_id"]),
            "lease_token": lease["lease_token"],
            "project_root": project,
            "caller_label": caller,
        }
        broker.authorize_command(command)
        future = asyncio.get_running_loop().create_future()
        broker.queue_command(record, command["target"], command, future)
        pending[item["request_id"]] = future
        # Model the extension consuming the heartbeat fallback before a response.
        self_response = broker.heartbeat_response(record)
        assert self_response["cmd"]["request_id"] == item["request_id"]

    request_by_id = {item["request_id"]: item for item in requests}
    for request_id in race["response_order"]:
        item = request_by_id[request_id]
        broker.accept_response(
            {
                "type": "response",
                "request_id": request_id,
                "extension_instance_id": item["extension_instance_id"],
                "target": target(
                    item["extension_instance_id"],
                    item["window_id"],
                    item["tab_id"],
                ),
                "ok": True,
                "url": item["result_url"],
            }
        )

    completed = []
    for item in requests:
        result = await pending[item["request_id"]]
        completed.append(
            {
                "request_id": item["request_id"],
                "extension_instance_id": result["extension_instance_id"],
                "url": result["url"],
            }
        )
    completed.sort(key=lambda item: item["request_id"])

    ambiguous = stateful["ambiguous_implicit_target"]
    assert isinstance(ambiguous, dict)
    ambiguous_broker = BrowserSessionBroker()
    for item in ambiguous["targets"]:
        ambiguous_broker.register_heartbeat(heartbeat(item["extension_instance_id"]))
    try:
        ambiguous_broker.resolve_target(None)
    except BrokerError as error:
        ambiguous_error = error.code
    else:
        raise AssertionError("implicit target unexpectedly resolved")

    lease_scope = await python_lease_scope_oracle(stateful["lease_scope_isolation"])
    cancel_response = await python_cancel_response_oracle(
        stateful["cancel_response_race"]
    )
    timeout = await python_timeout_oracle(stateful["timeout_late_response"])
    generation = await python_generation_oracle(stateful["generation_reconnect"])
    queue_fairness = await python_queue_fairness_oracle(
        stateful["queue_fairness"]
    )
    return {
        "profile_response_race": {
            "completed": completed,
            "queued_commands": {
                instance_id: len(record.command_queue)
                for instance_id, record in sorted(records.items())
            },
            "session_count": len(broker.sessions),
        },
        "ambiguous_implicit_target": {
            "error": ambiguous_error,
            "queued_commands": sum(
                len(record.command_queue)
                for record in ambiguous_broker.sessions.values()
            ),
            "session_count": len(ambiguous_broker.sessions),
        },
        "lease_scope_isolation": lease_scope,
        "cancel_response_race": cancel_response,
        "timeout_late_response": timeout,
        "generation_reconnect": generation,
        "queue_fairness": queue_fairness,
    }


class BrowserContractDifferentialTests(unittest.TestCase):
    def test_python_and_rust_state_oracles_match_shared_fixture(self) -> None:
        fixture = json.loads(FIXTURE_PATH.read_text(encoding="utf-8"))
        python_result = asyncio.run(python_oracle(fixture))
        scenario = fixture["stateful"]
        self.assertEqual(
            python_result["ambiguous_implicit_target"]["error"],
            scenario["ambiguous_implicit_target"]["expected_error"],
        )
        self.assertEqual(
            python_result["ambiguous_implicit_target"]["queued_commands"],
            scenario["ambiguous_implicit_target"]["expected_dispatched_commands"],
        )

        child_env = os.environ.copy()
        child_env["CARGO_TERM_COLOR"] = "never"
        completed = subprocess.run(
            [
                "cargo",
                "run",
                "-p",
                "teshi-browser-broker",
                "--example",
                "browser_contract_oracle",
                "--features",
                "contract-oracle",
                "--locked",
                "--",
                str(FIXTURE_PATH),
            ],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            timeout=180,
            check=False,
            env=child_env,
        )
        if completed.returncode != 0:
            self.fail(
                "Rust differential oracle failed "
                f"(exit {completed.returncode})\n"
                f"stdout:\n{completed.stdout}\n"
                f"stderr:\n{completed.stderr}"
            )
        rust_lines = [
            line.strip() for line in completed.stdout.splitlines() if line.strip()
        ]
        self.assertTrue(rust_lines, completed.stderr)
        rust_result = json.loads(rust_lines[-1])
        self.assertEqual(rust_result, python_result)


if __name__ == "__main__":
    unittest.main()
