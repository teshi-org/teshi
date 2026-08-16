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
        "features": [
            {"feature": "p0.control", "available": True},
            {"feature": "p1.observability_artifacts", "available": True},
        ],
        "supported_operations": [
            "capture_browser_screenshot",
            "generate_browser_pdf",
            "start_console_capture",
            "list_console_events",
            "clear_console_capture",
            "stop_console_capture",
            "start_network_capture",
            "list_network_requests",
            "get_network_request_detail",
            "clear_network_capture",
            "stop_network_capture",
        ],
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
    async def test_privileged_grants_bind_every_scope_and_never_leak_in_discovery(self) -> None:
        broker = BrowserSessionBroker(broker_instance_id="broker-a", local_user="user-a")
        record = broker.register_heartbeat(heartbeat("profile-a"))
        lease = broker.acquire_lease("profile-a", "agent-a", 60)
        issued = broker.create_capability_grant(
            extension_instance_id="profile-a",
            lease_token=lease["lease_token"],
            capability="javascript",
            project_root="project-a",
            caller_label="agent-a",
            interactive_confirmed=True,
        )
        token = issued["grant_token"]
        broker.validate_capability_grant(
            token=token,
            capability="javascript",
            extension_instance_id="profile-a",
            project_root="project-a",
            caller_label="agent-a",
        )
        discovery = json.dumps(broker.list_sessions())
        self.assertNotIn(token, discovery)
        self.assertNotIn("grant_token", json.dumps(broker.list_capability_grants(project_root="project-a")))
        for field, value in (
            ("capability", "raw-cdp"),
            ("extension_instance_id", "profile-b"),
            ("project_root", "project-b"),
            ("caller_label", "agent-b"),
        ):
            kwargs = {
                "token": token,
                "capability": "javascript",
                "extension_instance_id": "profile-a",
                "project_root": "project-a",
                "caller_label": "agent-a",
            }
            kwargs[field] = value
            with self.assertRaises(BrokerError) as mismatch:
                broker.validate_capability_grant(**kwargs)
            self.assertEqual(mismatch.exception.code, "browser_capability_denied")

        other = BrowserSessionBroker(broker_instance_id="broker-b", local_user="user-a")
        other.capability_grants = dict(broker.capability_grants)
        with self.assertRaises(BrokerError) as wrong_broker:
            other.validate_capability_grant(
                token=token,
                capability="javascript",
                extension_instance_id="profile-a",
                project_root="project-a",
                caller_label="agent-a",
            )
        self.assertEqual(wrong_broker.exception.code, "browser_capability_denied")

        broker.revoke_capability_grant(issued["grant_id"], project_root="project-a")
        with self.assertRaises(BrokerError) as revoked:
            broker.validate_capability_grant(
                token=token,
                capability="javascript",
                extension_instance_id="profile-a",
                project_root="project-a",
                caller_label="agent-a",
            )
        self.assertEqual(revoked.exception.code, "browser_capability_denied")
        self.assertEqual(record.extension_instance_id, "profile-a")

    async def test_privileged_grant_expiry_policy_and_permission_fail_closed(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        lease = broker.acquire_lease("profile-a", "agent-a", 60)
        with self.assertRaises(BrokerError) as denied:
            broker.create_capability_grant(
                extension_instance_id="profile-a",
                lease_token=lease["lease_token"],
                capability="raw-cdp",
                project_root="project-a",
                caller_label="agent-a",
                non_interactive=True,
                acknowledged_capability="raw-cdp",
                policy_capabilities=set(),
            )
        self.assertEqual(denied.exception.code, "browser_capability_denied")
        issued = broker.create_capability_grant(
            extension_instance_id="profile-a",
            lease_token=lease["lease_token"],
            capability="raw-cdp",
            project_root="project-a",
            caller_label="agent-a",
            non_interactive=True,
            acknowledged_capability="raw-cdp",
            policy_capabilities={"raw-cdp"},
        )
        grant = broker.capability_grants[issued["grant_id"]]
        grant.expires_monotonic = time.monotonic() - 1
        with self.assertRaises(BrokerError) as expired:
            broker.validate_capability_grant(
                token=issued["grant_token"],
                capability="raw-cdp",
                extension_instance_id="profile-a",
                project_root="project-a",
                caller_label="agent-a",
            )
        self.assertEqual(expired.exception.code, "browser_capability_denied")
        with self.assertRaises(BrokerError) as unavailable:
            broker.require_optional_permission(record, "cookies")
        self.assertEqual(unavailable.exception.code, "browser_capability_unavailable")

    async def test_privileged_audit_is_bounded_and_redacts_secret_fields(self) -> None:
        broker = BrowserSessionBroker()
        for index in range(1005):
            broker.append_privileged_audit(
                capability="raw-cdp",
                caller_label="agent-a",
                target=target("profile-a"),
                request_id=f"request-{index}",
                outcome="denied",
                arguments={"authorization": "Bearer secret", "method": "Runtime.evaluate"},
            )
        self.assertEqual(len(broker.privileged_audit), 1000)
        serialized = json.dumps(broker.list_privileged_audit(1))
        self.assertNotIn("Bearer secret", serialized)
        self.assertIn("[REDACTED]", serialized)

    async def test_lookup_and_unique_profile_labels_handle_colliding_tab_ids(self) -> None:
        broker = BrowserSessionBroker()
        broker.register_heartbeat(heartbeat("profile-a"))
        broker.register_heartbeat(heartbeat("profile-b"))
        self.assertEqual(len(broker.lookup_sessions(tab_id=42)), 2)
        broker.set_profile_label("profile-a", "checkout")
        self.assertEqual(
            broker.lookup_sessions(profile_label="checkout")[0]["identity"][
                "extension_instance_id"
            ],
            "profile-a",
        )
        with self.assertRaises(BrokerError) as duplicate:
            broker.set_profile_label("profile-b", "CHECKOUT")
        self.assertEqual(duplicate.exception.code, "ambiguous_browser_target")
        broker.clear_profile_label("profile-a")
        self.assertEqual(broker.lookup_sessions(profile_label="checkout"), [])

    async def test_disconnected_profile_does_not_reserve_managed_label(self) -> None:
        broker = BrowserSessionBroker(heartbeat_ttl=0.01)
        broker.register_heartbeat(heartbeat("profile-a"))
        broker.set_profile_label("profile-a", "checkout")
        await asyncio.sleep(0.02)
        broker.register_heartbeat(heartbeat("profile-b"))
        self.assertEqual(broker.set_profile_label("profile-b", "CHECKOUT"), "CHECKOUT")

    async def test_revision_bound_references_are_isolated_and_preserve_context(self) -> None:
        broker = BrowserSessionBroker()
        profile_a = broker.register_heartbeat(heartbeat("profile-a"))
        profile_b = broker.register_heartbeat(heartbeat("profile-b"))
        snapshot_a = {
            "request_id": "snapshot-a",
            "page_context_revision": "revision-a",
            "interactive_elements": [
                {
                    "element_ref": "opaque-a",
                    "shortSelector": "#save-a",
                    "context": {
                        "frame": "checkout-frame",
                        "shadow_root": "#widget-host",
                    },
                }
            ],
        }
        snapshot_b = {
            "request_id": "snapshot-b",
            "page_context_revision": "revision-b",
            "interactive_elements": [
                {"element_ref": "opaque-b", "shortSelector": "#save-b"}
            ],
        }
        broker.cache_snapshot_references(profile_a, target("profile-a"), snapshot_a)
        broker.cache_snapshot_references(profile_b, target("profile-b"), snapshot_b)
        self.assertEqual(snapshot_a["interactive_elements"][0]["ref"], "@e1")
        self.assertEqual(snapshot_b["interactive_elements"][0]["ref"], "@e1")
        resolved_a = broker.resolve_element_reference(
            "profile-a",
            target("profile-a"),
            "@e1",
            page_context_revision="revision-a",
            snapshot_id="snapshot-a",
        )
        resolved_b = broker.resolve_element_reference(
            "profile-b",
            target("profile-b"),
            "@e1",
            page_context_revision="revision-b",
            snapshot_id="snapshot-b",
        )
        self.assertEqual(resolved_a.element["element_ref"], "opaque-a")
        self.assertEqual(resolved_b.element["element_ref"], "opaque-b")
        self.assertEqual(resolved_a.context["frame"], "checkout-frame")
        self.assertEqual(resolved_a.context["shadow_root"], "#widget-host")
        with self.assertRaises(BrokerError) as wrong_profile:
            broker.resolve_element_reference(
                "profile-a", target("profile-b"), "@e1"
            )
        self.assertEqual(wrong_profile.exception.code, "stale_element_reference")

    async def test_new_snapshot_navigation_and_eviction_make_old_reference_stale(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        first = {
            "request_id": "snapshot-old",
            "page_context_revision": "revision-old",
            "interactive_elements": [
                {"element_ref": "opaque-old", "shortSelector": "#old"}
            ],
        }
        broker.cache_snapshot_references(record, target("profile-a"), first)
        second = {
            "request_id": "snapshot-new",
            "page_context_revision": "revision-new",
            "interactive_elements": [
                {"element_ref": "opaque-new", "shortSelector": "#new"}
            ],
        }
        broker.cache_snapshot_references(record, target("profile-a"), second)
        with self.assertRaises(BrokerError):
            broker.resolve_element_reference(
                "profile-a",
                target("profile-a"),
                "@e1",
                snapshot_id="snapshot-old",
            )

        navigated = heartbeat("profile-a")
        navigated["windows"][0]["tabs"][0]["url"] = "https://new.example.test/"
        broker.register_heartbeat(navigated)
        with self.assertRaises(BrokerError) as stale:
            broker.resolve_element_reference(
                "profile-a", target("profile-a"), "@e1"
            )
        self.assertEqual(stale.exception.code, "stale_element_reference")

    async def test_direct_transport_claims_once_and_restores_heartbeat_fallback(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        future = asyncio.get_running_loop().create_future()
        command = {
            "cmd": "get_page_snapshot",
            "request_id": "direct-once",
            "target": target("profile-a"),
        }
        broker.queue_command(record, target("profile-a"), command, future)

        claimed = broker.take_queued_command("profile-a", "direct-once")
        self.assertIsNotNone(claimed)
        self.assertIsNone(broker.heartbeat_response(record)["cmd"])
        self.assertIsNone(
            broker.take_queued_command("profile-a", "direct-once")
        )

        broker.restore_queued_command("profile-a", claimed)
        broker.restore_queued_command("profile-a", claimed)
        fallback = broker.heartbeat_response(record)["cmd"]
        self.assertEqual(fallback["request_id"], "direct-once")
        self.assertIsNone(broker.heartbeat_response(record)["cmd"])

    async def test_duplicate_action_request_is_never_dispatched_twice(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        first = asyncio.get_running_loop().create_future()
        second = asyncio.get_running_loop().create_future()
        command = {
            "cmd": "execute_browser_action",
            "request_id": "mutation-once",
            "target": target("profile-a"),
        }
        broker.queue_command(record, target("profile-a"), command, first)
        with self.assertRaises(BrokerError) as duplicate:
            broker.queue_command(record, target("profile-a"), command, second)
        self.assertEqual(duplicate.exception.code, "duplicate_browser_mutation")
        self.assertEqual(len(record.command_queue), 1)

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

    def test_phased_fixtures_cover_compatibility_boundaries(self) -> None:
        fixture_path = RESOURCES / "browser_contract_fixtures.json"
        phased = json.loads(fixture_path.read_text(encoding="utf-8"))["phased"]
        self.assertEqual(
            set(phased),
            {
                "p0_only_heartbeat",
                "p0_p1_heartbeat",
                "p2_optional_permissions_heartbeat",
                "incompatible_feature_request",
            },
        )
        self.assertTrue(phased["p0_only_heartbeat"]["features"][0]["available"])
        self.assertFalse(
            phased["p2_optional_permissions_heartbeat"]["optional_permissions"][
                "cookies"
            ]
        )


class PhasedCapabilityTests(unittest.IsolatedAsyncioTestCase):
    def fixture(self, name: str) -> dict:
        payload = json.loads(
            (RESOURCES / "browser_contract_fixtures.json").read_text(encoding="utf-8")
        )["phased"][name]
        result = {**heartbeat(payload["extension_instance_id"]), **payload}
        if "supported_operations" not in payload:
            result.pop("supported_operations", None)
        return result

    async def test_discovery_sanitizes_capabilities_and_never_exposes_tokens(self) -> None:
        broker = BrowserSessionBroker()
        payload = self.fixture("p0_p1_heartbeat")
        payload["features"].append(
            {"feature": "unknown.secret", "available": True, "reason": "ignore"}
        )
        payload["supported_actions"].append("arbitrary_secret_action")
        payload["supported_operations"].append("read_every_secret")
        record = broker.register_heartbeat(payload)
        lease = broker.acquire_lease(record.extension_instance_id, "Agent A")
        discovery = broker.list_sessions()[0]
        serialized = json.dumps(discovery)
        self.assertNotIn(lease["lease_token"], serialized)
        self.assertNotIn("unknown.secret", serialized)
        self.assertNotIn("arbitrary_secret_action", serialized)
        self.assertNotIn("read_every_secret", serialized)
        self.assertEqual(
            [item["feature"] for item in discovery["capabilities"]["features"]],
            ["p0.control", "p1.observability_artifacts"],
        )
        self.assertIn(
            "list_console_events", discovery["capabilities"]["supported_operations"]
        )

    async def test_required_feature_fails_before_dispatch(self) -> None:
        broker = BrowserSessionBroker()
        payload = self.fixture("p0_only_heartbeat")
        record = broker.register_heartbeat(payload)
        lease = broker.acquire_lease(record.extension_instance_id, "Agent A")
        command = {
            **json.loads(
                (RESOURCES / "browser_contract_fixtures.json").read_text(
                    encoding="utf-8"
                )
            )["phased"]["incompatible_feature_request"],
            "target": target(record.extension_instance_id),
            "lease_token": lease["lease_token"],
        }
        with self.assertRaises(BrokerError) as caught:
            broker.authorize_command(command)
        self.assertEqual(caught.exception.code, "browser_capability_unavailable")
        self.assertEqual(len(record.command_queue), 0)

    async def test_previous_extension_without_optional_permissions_keeps_p2_disabled(self) -> None:
        broker = BrowserSessionBroker()
        payload = heartbeat("previous-extension")
        payload.pop("optional_permissions", None)
        record = broker.register_heartbeat(payload)
        self.assertEqual(record.optional_permissions, {})
        lease = broker.acquire_lease(record.extension_instance_id, "Agent A")
        command = {
            "cmd": "list_browser_cookies",
            "request_id": "old-extension-cookie",
            "target": target(record.extension_instance_id),
            "lease_token": lease["lease_token"],
        }
        with self.assertRaises(BrokerError) as caught:
            broker.authorize_command(command, legacy_compatibility=False)
        self.assertEqual(caught.exception.code, "browser_capability_unavailable")
        self.assertEqual(len(record.command_queue), 0)

    async def test_console_capture_is_bounded_filtered_and_profile_isolated(self) -> None:
        broker = BrowserSessionBroker()
        record_a = broker.register_heartbeat(heartbeat("profile-a"))
        record_b = broker.register_heartbeat(heartbeat("profile-b"))
        broker.start_console_capture(
            record_a,
            target("profile-a"),
            levels=["info", "error"],
            max_entries=2,
            max_bytes=4096,
            max_age_ms=60_000,
        )
        broker.start_console_capture(record_b, target("profile-b"))

        self.assertFalse(
            broker.record_console_event(
                "profile-a", target("profile-a"), {"level": "debug", "text": "skip"}
            )
        )
        for index in range(3):
            self.assertTrue(
                broker.record_console_event(
                    "profile-a",
                    target("profile-a"),
                    {"level": "info", "text": f"event-{index}"},
                )
            )
        self.assertTrue(
            broker.record_console_event(
                "profile-b",
                target("profile-b"),
                {"level": "error", "text": "profile-b-only"},
            )
        )

        listed = broker.list_console_events(record_a, target("profile-a"))
        self.assertEqual([event["text"] for event in listed["events"]], ["event-1", "event-2"])
        self.assertNotIn("profile-b-only", json.dumps(listed))
        errors_only = broker.list_console_events(
            record_a, target("profile-a"), levels=["error"]
        )
        self.assertEqual(errors_only["events"], [])
        broker.start_console_capture(
            record_a,
            target("profile-a"),
            sensitive_fields=["account-id"],
        )
        broker.record_console_event(
            "profile-a",
            target("profile-a"),
            {
                "level": "error",
                "text": "token=private, account-id: customer-42, safe=visible",
            },
        )
        redacted = broker.list_console_events(record_a, target("profile-a"))
        self.assertNotIn("private", redacted["events"][0]["text"])
        self.assertNotIn("customer-42", redacted["events"][0]["text"])
        self.assertIn("safe=visible", redacted["events"][0]["text"])

    async def test_console_capture_clear_stop_age_and_lease_contract(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        lease = broker.acquire_lease("profile-a", "Agent A")
        command = {
            "cmd": "list_console_events",
            "target": target("profile-a"),
            "lease_token": lease["lease_token"],
        }
        authorized, resolved, _ephemeral = broker.authorize_command(
            command, legacy_compatibility=False
        )
        self.assertIs(authorized, record)
        self.assertEqual(resolved, target("profile-a"))
        with self.assertRaises(BrokerError) as missing_lease:
            broker.authorize_command(
                {"cmd": "list_console_events", "target": target("profile-a")},
                legacy_compatibility=False,
            )
        self.assertEqual(missing_lease.exception.code, "invalid_browser_lease")

        broker.start_console_capture(record, target("profile-a"), max_age_ms=1_000)
        broker.record_console_event(
            "profile-a", target("profile-a"), {"level": "warn", "text": "old"}
        )
        state = next(iter(record.console_captures.values()))
        state.events[0].created_at -= 2
        self.assertEqual(
            broker.list_console_events(record, target("profile-a"))["events"], []
        )
        broker.record_console_event(
            "profile-a", target("profile-a"), {"level": "error", "text": "new"}
        )
        cleared = broker.clear_console_capture(record, target("profile-a"))
        self.assertEqual(cleared["removed_entries"], 1)
        stopped = broker.stop_console_capture(record, target("profile-a"))
        self.assertFalse(stopped["active"])
        self.assertFalse(
            broker.record_console_event(
                "profile-a", target("profile-a"), {"level": "error", "text": "late"}
            )
        )

    async def test_network_capture_redacts_metadata_and_bounds_explicit_body(self) -> None:
        broker = BrowserSessionBroker()
        record = broker.register_heartbeat(heartbeat("profile-a"))
        broker.start_network_capture(
            record,
            target("profile-a"),
            max_entries=2,
            max_bytes=4096,
            max_body_bytes=1_024,
            sensitive_fields=["x-private"],
        )
        self.assertTrue(
            broker.record_network_event(
                "profile-a",
                target("profile-a"),
                {
                    "event_type": "request",
                    "request_id": "request-1",
                    "url": "https://example.test/api?token=private&safe=visible",
                    "method": "POST",
                    "resource_type": "Fetch",
                    "headers": {
                        "Authorization": "Bearer private",
                        "Cookie": "session=private",
                        "X-Private": "private",
                        "Accept": "application/json",
                    },
                },
            )
        )
        broker.record_network_event(
            "profile-a",
            target("profile-a"),
            {
                "event_type": "response",
                "request_id": "request-1",
                "status": 200,
                "headers": {"Set-Cookie": "secret=1", "Content-Type": "application/json"},
            },
        )
        detail = broker.get_network_request_detail(
            record, target("profile-a"), "request-1"
        )
        serialized = json.dumps(detail)
        self.assertNotIn("Bearer private", serialized)
        self.assertNotIn("session=private", serialized)
        self.assertNotIn("token=private", serialized)
        self.assertIn("safe=visible", serialized)
        self.assertNotIn("secret=1", serialized)
        self.assertNotIn('"X-Private": "private"', serialized)
        self.assertIn("[REDACTED]", serialized)
        self.assertNotIn("request_headers", json.dumps(broker.list_network_requests(record, target("profile-a"))))

        bounded = broker.bound_network_body(
            record,
            target("profile-a"),
            "request-1",
            "a" * 2_000,
            False,
        )
        self.assertEqual(bounded["body"], "a" * 1_024)
        self.assertTrue(bounded["truncated"])
        self.assertEqual(bounded["original_size"], 2_000)

    async def test_network_capture_is_profile_isolated_and_cleared_on_disconnect(self) -> None:
        broker = BrowserSessionBroker(heartbeat_ttl=0.01)
        record_a = broker.register_heartbeat(heartbeat("profile-a"))
        record_b = broker.register_heartbeat(heartbeat("profile-b"))
        broker.start_network_capture(record_a, target("profile-a"))
        broker.start_network_capture(record_b, target("profile-b"))
        broker.record_network_event(
            "profile-a",
            target("profile-a"),
            {"event_type": "request", "request_id": "only-a", "url": "https://a.test"},
        )
        self.assertEqual(
            broker.list_network_requests(record_b, target("profile-b"))["requests"], []
        )
        record_a.last_heartbeat -= 1
        broker.expire_stale()
        self.assertEqual(record_a.network_captures, {})
        self.assertEqual(record_a.console_captures, {})

    async def test_error_recovery_removes_nested_secrets(self) -> None:
        response = BrokerError(
            "browser_capability_denied",
            "grant required",
            {
                "lease_token": "lease-private",
                "nested": {
                    "capability_grant_token": "grant-private",
                    "retryable": False,
                },
            },
        ).response("request-a", "raw_cdp")
        serialized = json.dumps(response)
        self.assertNotIn("lease-private", serialized)
        self.assertNotIn("grant-private", serialized)
        self.assertFalse(response["recovery"]["nested"]["retryable"])


if __name__ == "__main__":
    unittest.main()
