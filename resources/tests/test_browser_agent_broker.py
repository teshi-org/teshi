"""Contract, isolation, lease, and locator tests for the browser agent broker."""

from __future__ import annotations

import asyncio
import json
import sys
import time
import unittest
from pathlib import Path

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

from browser_agent_broker import (  # noqa: E402
    BrokerError,
    BrowserSessionBroker,
    apply_verification_results,
    generate_playwright_candidates,
)


def heartbeat(instance_id: str, tab_id: int = 42, window_id: int = 7) -> dict:
    """Build one protocol-v1 fake extension heartbeat."""
    return {
        "extension_instance_id": instance_id,
        "profile_label": f"Profile {instance_id}",
        "extension_version": "0.7.9",
        "protocol_version": 1,
        "browser": {"name": "Chromium", "version": "140", "platform": "Linux"},
        "active_window_id": window_id,
        "active_tab_id": tab_id,
        "url": f"https://{instance_id}.example.test/",
        "title": instance_id,
        "windows": [
            {
                "id": window_id,
                "focused": True,
                "tabs": [
                    {
                        "id": tab_id,
                        "window_id": window_id,
                        "url": f"https://{instance_id}.example.test/",
                        "title": instance_id,
                        "active": True,
                        "debuggable": True,
                    }
                ],
            }
        ],
    }


def target(instance_id: str, tab_id: int = 42, window_id: int = 7) -> dict:
    """Build the canonical composite target used by tests."""
    return {
        "extension_instance_id": instance_id,
        "window_id": window_id,
        "tab_id": tab_id,
    }


class BrowserSessionBrokerTests(unittest.IsolatedAsyncioTestCase):
    async def test_multiple_profiles_with_colliding_tab_ids_stay_isolated(self) -> None:
        broker = BrowserSessionBroker()
        profile_a = broker.register_heartbeat(heartbeat("profile-a"))
        profile_b = broker.register_heartbeat(heartbeat("profile-b"))
        self.assertEqual(len(broker.list_sessions()), 2)

        lease_a = broker.acquire_lease("profile-a", "Agent A", 30)
        lease_b = broker.acquire_lease("profile-b", "Agent B", 30)
        loop = asyncio.get_running_loop()
        future_a = loop.create_future()
        future_b = loop.create_future()
        command_a = {
            "cmd": "get_page_snapshot",
            "request_id": "request-a",
            "target": target("profile-a"),
            "lease_token": lease_a["lease_token"],
        }
        command_b = {
            "cmd": "get_page_snapshot",
            "request_id": "request-b",
            "target": target("profile-b"),
            "lease_token": lease_b["lease_token"],
        }
        authorized_a = broker.authorize_command(command_a)
        authorized_b = broker.authorize_command(command_b)
        broker.queue_command(profile_a, target("profile-a"), command_a, future_a)
        broker.queue_command(profile_b, target("profile-b"), command_b, future_b)
        self.assertEqual(authorized_a[1]["extension_instance_id"], "profile-a")
        self.assertEqual(authorized_b[1]["extension_instance_id"], "profile-b")
        self.assertEqual(
            broker.heartbeat_response(profile_a)["cmd"]["request_id"], "request-a"
        )
        self.assertEqual(
            broker.heartbeat_response(profile_b)["cmd"]["request_id"], "request-b"
        )

        broker.accept_response(
            {
                "type": "response",
                "request_id": "request-b",
                "extension_instance_id": "profile-b",
                "target": target("profile-b"),
                "ok": True,
                "url": "https://profile-b.example.test/",
            }
        )
        broker.accept_response(
            {
                "type": "response",
                "request_id": "request-a",
                "extension_instance_id": "profile-a",
                "target": target("profile-a"),
                "ok": True,
                "url": "https://profile-a.example.test/",
            }
        )
        self.assertEqual((await future_a)["extension_instance_id"], "profile-a")
        self.assertEqual((await future_b)["extension_instance_id"], "profile-b")

    async def test_mismatched_response_is_quarantined_not_delivered(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        lease = broker.acquire_lease("profile-a", "Agent A")
        command = {
            "cmd": "get_page_snapshot",
            "request_id": "request-a",
            "target": target("profile-a"),
            "lease_token": lease["lease_token"],
        }
        loop = asyncio.get_running_loop()
        future = loop.create_future()
        broker.authorize_command(command)
        broker.queue_command(record, target("profile-a"), command, future)
        with self.assertRaisesRegex(BrokerError, "target does not match") as caught:
            broker.accept_response(
                {
                    "request_id": "request-a",
                    "extension_instance_id": "profile-b",
                    "target": target("profile-b"),
                    "ok": True,
                }
            )
        self.assertEqual(caught.exception.code, "mismatched_browser_response")
        self.assertFalse(future.done())
        self.assertEqual(broker.quarantined_responses[-1]["reason"], "target_mismatch")

    async def test_implicit_target_fails_closed_when_profiles_are_ambiguous(self) -> None:
        broker = BrowserSessionBroker()
        broker.register_heartbeat(heartbeat("profile-a"))
        broker.register_heartbeat(heartbeat("profile-b"))
        with self.assertRaises(BrokerError) as caught:
            broker.resolve_target(None)
        self.assertEqual(caught.exception.code, "ambiguous_browser_target")
        self.assertEqual(len(caught.exception.recovery["candidates"]), 2)

    async def test_distinct_agents_lease_distinct_profiles_concurrently(self) -> None:
        broker = BrowserSessionBroker()
        broker.register_heartbeat(heartbeat("profile-a"))
        broker.register_heartbeat(heartbeat("profile-b"))
        lease_a, lease_b = await asyncio.gather(
            asyncio.to_thread(broker.acquire_lease, "profile-a", "Agent A", 30),
            asyncio.to_thread(broker.acquire_lease, "profile-b", "Agent B", 30),
        )
        self.assertNotEqual(lease_a["lease_token"], lease_b["lease_token"])
        with self.assertRaises(BrokerError) as caught:
            broker.acquire_lease("profile-a", "Agent C", 30)
        self.assertEqual(caught.exception.code, "browser_session_busy")

    async def test_expired_lease_is_recoverable(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        first = broker.acquire_lease("profile-a", "Agent A", 5)
        self.assertTrue(first["lease_token"])
        self.assertIsNotNone(record.lease)
        record.lease.expires_monotonic = time.monotonic() - 1
        second = broker.acquire_lease("profile-a", "Agent B", 5)
        self.assertNotEqual(first["lease_token"], second["lease_token"])

    async def test_lease_tokens_are_cli_safe_and_never_start_with_hyphen(self) -> None:
        broker = BrowserSessionBroker()
        broker.register_heartbeat(heartbeat("profile-a"))
        lease = broker.acquire_lease("profile-a", "Agent A")
        self.assertTrue(lease["lease_token"].startswith("lease_"))

    async def test_disconnect_fails_pending_and_releases_lease(self) -> None:
        broker = BrowserSessionBroker(heartbeat_ttl=0.01)
        record = broker.register_heartbeat(heartbeat("profile-a"))
        lease = broker.acquire_lease("profile-a", "Agent A")
        command = {
            "cmd": "get_page_snapshot",
            "request_id": "request-a",
            "target": target("profile-a"),
            "lease_token": lease["lease_token"],
        }
        loop = asyncio.get_running_loop()
        future = loop.create_future()
        broker.authorize_command(command)
        broker.queue_command(record, target("profile-a"), command, future)
        broker.expire_stale(record.last_heartbeat + 1)
        result = await future
        self.assertEqual(result["code"], "browser_session_disconnected")
        self.assertIsNone(record.lease)
        self.assertFalse(record.command_queue)

    async def test_legacy_single_session_command_gets_bounded_compatibility_lease(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(
            {
                "active_tab_id": 42,
                "tabs": [
                    {
                        "id": 42,
                        "active": True,
                        "debuggable": True,
                        "url": "https://legacy.example.test/",
                    }
                ],
            }
        )
        command = {"cmd": "get_page_snapshot", "request_id": "legacy-request"}
        resolved, resolved_target, ephemeral = broker.authorize_command(command)
        self.assertEqual(resolved, record)
        self.assertEqual(resolved_target["tab_id"], 42)
        self.assertIsNotNone(ephemeral)
        future = asyncio.get_running_loop().create_future()
        broker.queue_command(
            record,
            resolved_target,
            command,
            future,
            ephemeral_lease_token=ephemeral,
        )
        broker.accept_response(
            {"request_id": "legacy-request", "ok": True, "tab_id": 42}
        )
        await future
        self.assertIsNone(record.lease)


class BrowserAgentEndToEndTests(unittest.IsolatedAsyncioTestCase):
    async def locator_round_trip(
        self,
        broker: BrowserSessionBroker,
        instance_id: str,
        accessible_name: str,
    ) -> dict:
        record = broker.register_heartbeat(heartbeat(instance_id))
        lease = broker.acquire_lease(instance_id, f"Agent {instance_id}", 30)
        request_id = f"snapshot-{instance_id}"
        command = {
            "cmd": "get_page_snapshot",
            "request_id": request_id,
            "target": target(instance_id),
            "lease_token": lease["lease_token"],
        }
        authorized_record, authorized_target, _ephemeral = broker.authorize_command(
            command
        )
        future = asyncio.get_running_loop().create_future()
        broker.queue_command(
            authorized_record,
            authorized_target,
            command,
            future,
        )
        queued = broker.heartbeat_response(record)["cmd"]
        self.assertEqual(queued["extension_instance_id"], instance_id)
        self.assertEqual(queued["target"], target(instance_id))
        await asyncio.sleep(0)
        snapshot = {
            "page_context_revision": f"revision-{instance_id}",
            "interactive_elements": [
                {
                    "element_ref": f"save-{instance_id}",
                    "tag": "button",
                    "role": "button",
                    "accessible_name": accessible_name,
                    "text": accessible_name,
                    "attributes": {"data-testid": f"save-{instance_id}"},
                    "visible": True,
                }
            ],
        }
        broker.accept_response(
            {
                "type": "response",
                "request_id": request_id,
                "extension_instance_id": instance_id,
                "target": target(instance_id),
                "ok": True,
                "url": f"https://{instance_id}.example.test/",
                **snapshot,
            }
        )
        response = await future
        _element, candidates = generate_playwright_candidates(
            response,
            {"role": "button", "text": accessible_name},
        )
        verified = apply_verification_results(
            candidates,
            [
                {
                    "expression": candidates[0]["expression"],
                    "match_count": 1,
                    "visible": True,
                    "enabled": True,
                }
            ],
        )
        recommended = next(
            candidate
            for candidate in verified
            if candidate["verification"] == "verified"
        )
        broker.release_lease(instance_id, lease["lease_token"])
        return {
            "target": response["target"],
            "url": response["url"],
            "page_context_revision": response["page_context_revision"],
            "recommended": recommended,
        }

    async def test_single_profile_registers_and_returns_verified_locator(self) -> None:
        broker = BrowserSessionBroker()
        result = await self.locator_round_trip(broker, "profile-a", "Save A")
        self.assertEqual(result["target"], target("profile-a"))
        self.assertEqual(result["page_context_revision"], "revision-profile-a")
        self.assertEqual(result["recommended"]["verification"], "verified")
        self.assertEqual(result["recommended"]["match_count"], 1)
        self.assertIn("Save A", result["recommended"]["expression"])

    async def test_two_profiles_and_agents_never_cross_locator_or_frame_data(self) -> None:
        broker = BrowserSessionBroker()
        result_a, result_b = await asyncio.gather(
            self.locator_round_trip(broker, "profile-a", "Save A"),
            self.locator_round_trip(broker, "profile-b", "Save B"),
        )
        self.assertEqual(result_a["target"], target("profile-a"))
        self.assertEqual(result_b["target"], target("profile-b"))
        self.assertNotEqual(result_a["url"], result_b["url"])
        self.assertIn("Save A", result_a["recommended"]["expression"])
        self.assertNotIn("Save B", result_a["recommended"]["expression"])
        self.assertIn("Save B", result_b["recommended"]["expression"])
        self.assertNotIn("Save A", result_b["recommended"]["expression"])

        broker.update_frame(
            "profile-a",
            target("profile-a"),
            {"extension_instance_id": "profile-a", "url": result_a["url"]},
        )
        broker.update_frame(
            "profile-b",
            target("profile-b"),
            {"extension_instance_id": "profile-b", "url": result_b["url"]},
        )
        self.assertEqual(
            broker.sessions["profile-a"].latest_frame["url"], result_a["url"]
        )
        self.assertEqual(
            broker.sessions["profile-b"].latest_frame["url"], result_b["url"]
        )


class PlaywrightLocatorCandidateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.snapshot = {
            "interactive_elements": [
                {
                    "element_ref": "save-button",
                    "tag": "button",
                    "role": "button",
                    "accessible_name": "Save",
                    "text": "Save",
                    "attributes": {"data-testid": "save", "class": "css-123"},
                    "shortSelector": "button.css-123:nth-of-type(2)",
                    "visible": True,
                },
                {
                    "element_ref": "email-input",
                    "tag": "input",
                    "label": "Email",
                    "placeholder": "name@example.test",
                    "attributes": {"data-qa": "email-field", "name": "email"},
                    "context": {"frame": "checkout-frame", "shadow_root": None},
                    "visible": True,
                },
                {
                    "element_ref": "shadow-submit",
                    "tag": "button",
                    "role": "button",
                    "accessible_name": "Submit",
                    "text": "Submit",
                    "attributes": {},
                    "context": {"frame": None, "shadow_root": "checkout-widget"},
                    "visible": True,
                },
            ]
        }

    def test_role_is_ranked_before_test_id_and_fragile_css(self) -> None:
        _element, candidates = generate_playwright_candidates(
            self.snapshot, {"element_ref": "save-button"}
        )
        self.assertEqual(candidates[0]["kind"], "role")
        self.assertEqual(candidates[0]["expression"], 'page.getByRole("button", { name: "Save", exact: true })')
        css = next(candidate for candidate in candidates if candidate["kind"] == "css")
        self.assertIn("generated_class", css["warnings"])
        self.assertIn("positional_selector", css["warnings"])

    def test_label_placeholder_and_custom_test_id_are_generated(self) -> None:
        _element, candidates = generate_playwright_candidates(
            self.snapshot,
            {"element_ref": "email-input"},
            ["data-qa"],
        )
        self.assertEqual(
            [candidate["kind"] for candidate in candidates[:4]],
            ["role", "label", "placeholder", "attribute"],
        )
        self.assertEqual(candidates[0]["context"]["frame"], "checkout-frame")

    def test_shadow_context_survives_candidate_generation(self) -> None:
        _element, candidates = generate_playwright_candidates(
            self.snapshot, {"text": "Submit", "role": "button"}
        )
        self.assertEqual(candidates[0]["context"]["shadow_root"], "checkout-widget")

    def test_structured_text_mismatch_does_not_fall_back_to_role_only(self) -> None:
        with self.assertRaises(BrokerError) as caught:
            generate_playwright_candidates(
                self.snapshot,
                {"role": "button", "text": "More information"},
            )
        self.assertEqual(caught.exception.code, "browser_target_not_found")

    def test_structured_role_mismatch_is_rejected(self) -> None:
        with self.assertRaises(BrokerError) as caught:
            generate_playwright_candidates(
                self.snapshot,
                {"role": "link", "text": "Save"},
            )
        self.assertEqual(caught.exception.code, "browser_target_not_found")

    def test_duplicate_name_is_marked_ambiguous_not_recommended(self) -> None:
        _element, candidates = generate_playwright_candidates(
            self.snapshot, {"element_ref": "save-button"}
        )
        verified = apply_verification_results(
            candidates,
            [
                {
                    "expression": candidates[0]["expression"],
                    "match_count": 2,
                    "visible": True,
                    "enabled": True,
                }
            ],
        )
        role = next(candidate for candidate in verified if candidate["kind"] == "role")
        self.assertEqual(role["verification"], "ambiguous")

    def test_dynamic_document_replacement_is_reported_stale(self) -> None:
        _element, candidates = generate_playwright_candidates(
            self.snapshot, {"element_ref": "save-button"}
        )
        verified = apply_verification_results(
            candidates,
            [
                {
                    "expression": candidates[0]["expression"],
                    "stale_page_context": True,
                }
            ],
        )
        role = next(candidate for candidate in verified if candidate["kind"] == "role")
        self.assertEqual(role["verification"], "stale_page_context")


class LegacyFixtureTests(unittest.TestCase):
    def test_single_session_fixture_captures_all_legacy_boundaries(self) -> None:
        fixture_path = RESOURCES / "browser_contract_fixtures.json"
        payload = json.loads(fixture_path.read_text(encoding="utf-8"))
        legacy = payload["legacy"]
        self.assertEqual(
            set(legacy),
            {"heartbeat", "command", "response", "frame_meta", "cdp_endpoint"},
        )
        self.assertEqual(legacy["command"]["request_id"], legacy["response"]["request_id"])


if __name__ == "__main__":
    unittest.main()
