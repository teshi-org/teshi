"""End-to-end tests for ChromeBridge registration and locator orchestration."""

from __future__ import annotations

import asyncio
import sys
import tempfile
import unittest
from pathlib import Path

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

from browser_service import ChromeBridge  # noqa: E402

from test_browser_agent_broker import heartbeat, target  # noqa: E402


class ChromeBridgeAgentFlowTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-chrome-bridge-test-")
        self.project_root = Path(self.temp.name).resolve()
        self.bridge = ChromeBridge(
            self.project_root,
            "ws://127.0.0.1:20254",
            17373,
            "ws://127.0.0.1:20254/extension/frames",
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def profile_heartbeat(self, instance_id: str) -> dict:
        payload = heartbeat(instance_id)
        payload["project_root"] = str(self.project_root)
        return payload

    async def register(self, instance_id: str) -> dict:
        result = await self.bridge.handle_heartbeat(
            self.profile_heartbeat(instance_id)
        )
        self.assertTrue(result["ok"])
        self.assertTrue(result["compatible"])
        return result

    async def next_command(self, instance_id: str, expected: str) -> dict:
        for _attempt in range(100):
            heartbeat_result = await self.bridge.handle_heartbeat(
                self.profile_heartbeat(instance_id)
            )
            command = heartbeat_result.get("cmd")
            if command is not None:
                self.assertEqual(command["cmd"], expected)
                self.assertEqual(command["extension_instance_id"], instance_id)
                self.assertEqual(command["target"], target(instance_id))
                return command
            await asyncio.sleep(0.001)
        self.fail(f"no {expected} command queued for {instance_id}")

    async def acquire(self, instance_id: str) -> str:
        result = await self.bridge.forward_command(
            {
                "cmd": "acquire_browser_lease",
                "request_id": f"lease-{instance_id}",
                "extension_instance_id": instance_id,
                "owner_label": f"Agent {instance_id}",
                "ttl_secs": 30,
            }
        )
        self.assertTrue(result["ok"])
        return str(result["lease"]["lease_token"])

    def locator_task(self, instance_id: str, lease_token: str, name: str):
        return asyncio.create_task(
            self.bridge.forward_command(
                {
                    "cmd": "resolve_playwright_locator",
                    "request_id": f"locator-{instance_id}",
                    "target": target(instance_id),
                    "lease_token": lease_token,
                    "intent": {"role": "button", "text": name},
                    "test_id_attributes": ["data-testid"],
                }
            )
        )

    async def respond_to_snapshot(
        self, instance_id: str, command: dict, name: str
    ) -> None:
        await self.bridge.handle_extension_response(
            {
                "type": "response",
                "cmd": command["cmd"],
                "request_id": command["request_id"],
                "extension_instance_id": instance_id,
                "target": command["target"],
                "ok": True,
                "url": f"https://{instance_id}.example.test/",
                "title": f"Page {instance_id}",
                "page_context_revision": f"revision-{instance_id}",
                "interactive_elements": [
                    {
                        "element_ref": f"save-{instance_id}",
                        "tag": "button",
                        "role": "button",
                        "accessible_name": name,
                        "text": name,
                        "attributes": {"data-testid": f"save-{instance_id}"},
                        "visible": True,
                    }
                ],
            }
        )

    async def respond_to_verification(
        self, instance_id: str, command: dict
    ) -> None:
        verification = []
        for index, candidate in enumerate(command["candidates"]):
            verification.append(
                {
                    "expression": candidate["expression"],
                    "match_count": 1 if index == 0 else 0,
                    "visible": index == 0,
                    "enabled": index == 0,
                }
            )
        await self.bridge.handle_extension_response(
            {
                "type": "response",
                "cmd": command["cmd"],
                "request_id": command["request_id"],
                "extension_instance_id": instance_id,
                "target": command["target"],
                "ok": True,
                "page_context_revision": f"revision-{instance_id}",
                "verification": verification,
            }
        )

    async def test_single_extension_registration_reaches_verified_locator(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")
        task = self.locator_task("profile-a", lease, "Save A")
        snapshot = await self.next_command("profile-a", "get_page_snapshot")
        await self.respond_to_snapshot("profile-a", snapshot, "Save A")
        verify = await self.next_command("profile-a", "verify_playwright_locators")
        await self.respond_to_verification("profile-a", verify)
        result = await task
        self.assertTrue(result["ok"])
        self.assertEqual(result["target"], target("profile-a"))
        self.assertEqual(result["page_context_revision"], "revision-profile-a")
        self.assertEqual(result["recommended"]["verification"], "verified")
        self.assertIn("Save A", result["recommended"]["expression"])

    async def test_two_extensions_complete_in_reverse_order_without_crossing(self) -> None:
        await self.register("profile-a")
        await self.register("profile-b")
        lease_a, lease_b = await asyncio.gather(
            self.acquire("profile-a"), self.acquire("profile-b")
        )
        task_a = self.locator_task("profile-a", lease_a, "Save A")
        task_b = self.locator_task("profile-b", lease_b, "Save B")
        snapshot_a, snapshot_b = await asyncio.gather(
            self.next_command("profile-a", "get_page_snapshot"),
            self.next_command("profile-b", "get_page_snapshot"),
        )
        await self.respond_to_snapshot("profile-b", snapshot_b, "Save B")
        await self.respond_to_snapshot("profile-a", snapshot_a, "Save A")
        verify_b, verify_a = await asyncio.gather(
            self.next_command("profile-b", "verify_playwright_locators"),
            self.next_command("profile-a", "verify_playwright_locators"),
        )
        await self.respond_to_verification("profile-b", verify_b)
        await self.respond_to_verification("profile-a", verify_a)
        result_b, result_a = await asyncio.gather(task_b, task_a)
        self.assertEqual(result_a["target"], target("profile-a"))
        self.assertEqual(result_b["target"], target("profile-b"))
        self.assertIn("Save A", result_a["recommended"]["expression"])
        self.assertNotIn("Save B", result_a["recommended"]["expression"])
        self.assertIn("Save B", result_b["recommended"]["expression"])
        self.assertNotIn("Save A", result_b["recommended"]["expression"])
        self.assertEqual(result_a["url"], "https://profile-a.example.test/")
        self.assertEqual(result_b["url"], "https://profile-b.example.test/")


if __name__ == "__main__":
    unittest.main()
