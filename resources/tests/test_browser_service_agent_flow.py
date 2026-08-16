"""End-to-end tests for ChromeBridge registration and locator orchestration."""

from __future__ import annotations

import asyncio
import base64
import json
import sys
import tempfile
import unittest
from pathlib import Path

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

from browser_agent_broker import BrokerError  # noqa: E402
from browser_service import (  # noqa: E402
    MAX_BROWSER_ARTIFACT_BYTES,
    MAX_BROWSER_WS_MESSAGE_BYTES,
    ChromeBridge,
    _http_response,
    authenticated_http_post,
    authenticated_websocket_path,
    cleanup_managed_browser_artifacts,
    load_browser_privileged_policy,
    managed_browser_artifact_path,
    persist_managed_browser_artifact,
    validate_raw_cdp_method,
    validate_browser_upload_files,
)

from test_browser_agent_broker import heartbeat, target  # noqa: E402


class ChromeBridgeAgentFlowTests(unittest.IsolatedAsyncioTestCase):
    def test_encoded_artifact_fits_bounded_command_transport(self) -> None:
        encoded_bytes = ((MAX_BROWSER_ARTIFACT_BYTES + 2) // 3) * 4
        envelope_allowance = 1024 * 1024
        self.assertLess(
            encoded_bytes + envelope_allowance, MAX_BROWSER_WS_MESSAGE_BYTES
        )

    def test_http_bridge_mutations_require_broker_token(self) -> None:
        self.assertTrue(
            authenticated_http_post({"x-teshi-broker-token": "secret"}, "secret")
        )
        self.assertFalse(authenticated_http_post({}, "secret"))
        self.assertFalse(
            authenticated_http_post({"x-teshi-broker-token": "wrong"}, "secret")
        )
        self.assertTrue(authenticated_http_post({}, "secret", "/heartbeat?token=secret"))
        self.assertFalse(
            authenticated_http_post({}, "secret", "/heartbeat?token=secret&token=secret")
        )

    def test_http_cors_is_limited_to_chrome_extension_origins(self) -> None:
        extension_origin = "chrome-extension://" + ("a" * 32)
        trusted = _http_response(200, b"{}", cors_origin=extension_origin)
        self.assertIn(
            f"Access-Control-Allow-Origin: {extension_origin}".encode(), trusted
        )
        self.assertNotIn(b"Access-Control-Allow-Origin: *", trusted)
        untrusted = _http_response(
            200, b"{}", cors_origin="https://attacker.example"
        )
        self.assertNotIn(b"Access-Control-Allow-Origin", untrusted)

    async def test_privileged_grant_cli_flow_is_default_deny_and_lists_no_token(self) -> None:
        await self.register("profile-a")
        lease = self.bridge.broker.acquire_lease("profile-a", "teshi-cli", 60)
        command = {
            "cmd": "create_browser_capability_grant",
            "request_id": "grant-1",
            "target": target("profile-a"),
            "lease_token": lease["lease_token"],
            "capability": "javascript",
            "caller_label": "teshi-cli",
            "non_interactive": True,
            "acknowledged_capability": "javascript",
        }
        denied = await self.bridge.forward_command(command)
        self.assertEqual(denied["code"], "browser_capability_denied")
        policy = self.project_root / ".teshi" / "browser-policy.json"
        policy.parent.mkdir(parents=True, exist_ok=True)
        policy.write_text(
            '{"privileged":{"allow":["javascript"]}}', encoding="utf-8"
        )
        self.assertEqual(load_browser_privileged_policy(self.project_root), {"javascript"})
        created = await self.bridge.forward_command({**command, "request_id": "grant-2"})
        token = created["grant"]["grant_token"]
        listed = await self.bridge.forward_command(
            {"cmd": "list_browser_capability_grants", "request_id": "grant-list"}
        )
        serialized = str(listed)
        self.assertNotIn(token, serialized)
        self.assertNotIn("grant_token", serialized)

    async def test_javascript_and_raw_cdp_require_grant_policy_and_bounded_results(self) -> None:
        payload = self.profile_heartbeat("profile-a")
        payload["features"].extend(
            [
                {"feature": "p2.javascript", "available": True},
                {"feature": "p2.raw_cdp", "available": True},
            ]
        )
        payload["supported_operations"].extend(
            ["execute_privileged_javascript", "execute_privileged_cdp"]
        )
        await self.bridge.handle_heartbeat(payload)
        lease = self.bridge.broker.acquire_lease("profile-a", "teshi-cli", 60)
        policy = self.project_root / ".teshi" / "browser-policy.json"
        policy.parent.mkdir(parents=True, exist_ok=True)
        policy.write_text(
            '{"privileged":{"allow":["javascript","raw-cdp"],"raw_cdp_methods":["Page.getLayoutMetrics"]}}',
            encoding="utf-8",
        )
        js_grant = self.bridge.broker.create_capability_grant(
            extension_instance_id="profile-a",
            lease_token=lease["lease_token"],
            capability="javascript",
            project_root=self.project_root,
            caller_label="teshi-cli",
            interactive_confirmed=True,
        )
        cdp_grant = self.bridge.broker.create_capability_grant(
            extension_instance_id="profile-a",
            lease_token=lease["lease_token"],
            capability="raw-cdp",
            project_root=self.project_root,
            caller_label="teshi-cli",
            interactive_confirmed=True,
        )

        async def direct(instance_id: str, command: dict) -> bool:
            response = {
                "type": "response",
                "request_id": command["request_id"],
                "extension_instance_id": instance_id,
                "target": command["target"],
                "ok": True,
                "result": 42 if command["cmd"].endswith("javascript") else {"id": "isolate"},
                "result_bytes": 2,
            }
            asyncio.create_task(self.bridge.handle_extension_response(response))
            return True

        self.bridge._direct_command_callback = direct
        common = {
            "target": target("profile-a"),
            "lease_token": lease["lease_token"],
            "caller_label": "teshi-cli",
        }
        denied = await self.bridge.forward_command(
            {
                **common,
                "cmd": "execute_privileged_javascript",
                "request_id": "js-denied",
                "expression": "40 + 2",
                "capability_grant_token": "invalid",
            }
        )
        self.assertEqual(denied["code"], "browser_capability_denied")
        executed = await self.bridge.forward_command(
            {
                **common,
                "cmd": "execute_privileged_javascript",
                "request_id": "js-ok",
                "expression": "40 + 2",
                "capability_grant_token": js_grant["grant_token"],
            }
        )
        self.assertEqual(executed["result"], 42)
        blocked = await self.bridge.forward_command(
            {
                **common,
                "cmd": "execute_privileged_cdp",
                "request_id": "cdp-blocked",
                "method": "Target.createTarget",
                "params": {"url": "about:blank"},
                "capability_grant_token": cdp_grant["grant_token"],
            }
        )
        self.assertEqual(blocked["code"], "browser_capability_denied")
        allowed = await self.bridge.forward_command(
            {
                **common,
                "cmd": "execute_privileged_cdp",
                "request_id": "cdp-ok",
                "method": "Page.getLayoutMetrics",
                "params": {},
                "capability_grant_token": cdp_grant["grant_token"],
            }
        )
        self.assertEqual(allowed["result"]["id"], "isolate")
        audit = self.bridge.broker.list_privileged_audit(10)
        self.assertFalse(any("40 + 2" in str(item) for item in audit))
        self.assertFalse(any(js_grant["grant_token"] in str(item) for item in audit))

    async def test_cookie_setting_and_extension_metadata_are_separately_gated(self) -> None:
        payload = self.profile_heartbeat("profile-a")
        payload["features"].extend(
            [
                {"feature": "p2.cookies", "available": True},
                {"feature": "p2.content_settings", "available": True},
                {"feature": "p2.extension_management", "available": True},
            ]
        )
        payload["supported_operations"].extend(
            ["list_browser_cookies", "access_browser_content_setting", "list_browser_extensions"]
        )
        payload["optional_permissions"] = {
            "cookies": True,
            "content_settings": True,
            "extension_management": True,
        }
        await self.bridge.handle_heartbeat(payload)
        lease = self.bridge.broker.acquire_lease("profile-a", "teshi-cli", 60)

        def grant(capability: str) -> dict:
            return self.bridge.broker.create_capability_grant(
                extension_instance_id="profile-a",
                lease_token=lease["lease_token"],
                capability=capability,
                project_root=self.project_root,
                caller_label="teshi-cli",
                interactive_confirmed=True,
            )

        grants = {name: grant(name) for name in (
            "cookies", "cookie-values", "content-settings", "extension-management"
        )}
        delivered: list[dict] = []

        async def direct(instance_id: str, command: dict) -> bool:
            self.assertEqual(instance_id, "profile-a")
            delivered.append(command)
            bodies = {
                "list_browser_cookies": {
                    "ok": True,
                    "origin": "https://profile-a.example.test",
                    "cookies": [{"name": "sid", "value": "secret", "partition_key": {"top_level_site": "https://example.test"}}],
                    "count": 1,
                    "truncated": False,
                },
                "access_browser_content_setting": {
                    "ok": True, "setting": command.get("setting"),
                    "value": command.get("value") or "ask", "scope": "selected-origin",
                },
                "list_browser_extensions": {
                    "ok": True,
                    "extensions": [{"id": "ext-a", "name": "Fixture", "enabled": True}],
                    "mutations_enabled": False,
                },
            }
            asyncio.create_task(self.bridge.handle_extension_response({
                "type": "response", "request_id": command["request_id"],
                "extension_instance_id": instance_id, "target": command["target"],
                **bodies[command["cmd"]],
            }))
            return True

        self.bridge._direct_command_callback = direct
        base = {
            "target": target("profile-a"), "lease_token": lease["lease_token"],
            "caller_label": "teshi-cli",
        }
        metadata = await self.bridge.forward_command({
            **base, "cmd": "list_browser_cookies", "request_id": "cookies-meta",
            "capability_grant_token": grants["cookies"]["grant_token"],
        })
        self.assertNotIn("value", metadata["cookies"][0])
        self.assertTrue(metadata["cookies"][0]["value_redacted"])
        denied_values = await self.bridge.forward_command({
            **base, "cmd": "list_browser_cookies", "request_id": "cookies-values-denied",
            "capability_grant_token": grants["cookies"]["grant_token"], "include_values": True,
        })
        self.assertEqual(denied_values["code"], "browser_capability_denied")
        values = await self.bridge.forward_command({
            **base, "cmd": "list_browser_cookies", "request_id": "cookies-values",
            "capability_grant_token": grants["cookies"]["grant_token"],
            "value_capability_grant_token": grants["cookie-values"]["grant_token"],
            "include_values": True,
        })
        self.assertEqual(values["cookies"][0]["value"], "secret")
        setting = await self.bridge.forward_command({
            **base, "cmd": "access_browser_content_setting", "request_id": "setting-set",
            "capability_grant_token": grants["content-settings"]["grant_token"],
            "setting": "notifications", "value": "block",
        })
        self.assertEqual(setting["scope"], "selected-origin")
        denied_setting = await self.bridge.forward_command({
            **base, "cmd": "access_browser_content_setting", "request_id": "setting-denied",
            "capability_grant_token": grants["content-settings"]["grant_token"],
            "setting": "javascript", "value": "allow",
        })
        self.assertEqual(denied_setting["code"], "browser_capability_denied")
        extensions = await self.bridge.forward_command({
            **base, "cmd": "list_browser_extensions", "request_id": "extensions-list",
            "capability_grant_token": grants["extension-management"]["grant_token"],
        })
        self.assertFalse(extensions["mutations_enabled"])
        self.assertFalse(any("capability_grant_token" in item for item in delivered))
        audit_text = str(self.bridge.broker.list_privileged_audit(20))
        self.assertNotIn("secret", audit_text)
        self.assertFalse(any(grant_record["grant_token"] in audit_text for grant_record in grants.values()))

        payload["optional_permissions"]["cookies"] = False
        await self.bridge.handle_heartbeat(payload)
        revoked = await self.bridge.forward_command({
            **base, "cmd": "list_browser_cookies", "request_id": "cookies-revoked",
            "capability_grant_token": grants["cookies"]["grant_token"],
        })
        self.assertEqual(revoked["code"], "browser_capability_unavailable")

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

    def test_managed_artifact_path_is_bounded_and_cannot_escape_project(self) -> None:
        path = managed_browser_artifact_path(
            self.project_root,
            target("profile-a"),
            "../../secret/" + ("x" * 300),
            "../png",
        )
        root = (self.project_root / ".teshi" / "artifacts" / "browser").resolve()
        self.assertEqual(path.parent, root)
        self.assertLessEqual(len(path.name), 160)

    def test_websocket_command_route_requires_exact_bearer_token(self) -> None:
        self.assertEqual(
            authenticated_websocket_path("/?token=secret", "secret"), "/"
        )
        self.assertEqual(
            authenticated_websocket_path(
                "/extension/frames?token=secret", "secret"
            ),
            "/extension/frames",
        )
        self.assertIsNone(authenticated_websocket_path("/", "secret"))
        self.assertIsNone(
            authenticated_websocket_path("/?token=wrong", "secret")
        )
        self.assertIsNone(
            authenticated_websocket_path("/?token=secret&token=secret", "secret")
        )

    def test_raw_cdp_cannot_cross_separate_privilege_boundaries(self) -> None:
        policy = self.project_root / ".teshi" / "browser-policy.json"
        policy.parent.mkdir(parents=True, exist_ok=True)
        methods = [
            "Runtime.evaluate",
            "Storage.getCookies",
            "Network.getCookies",
            "DOM.setFileInputFiles",
        ]
        policy.write_text(
            json.dumps({"privileged": {"raw_cdp_methods": methods}}),
            encoding="utf-8",
        )
        for method in methods:
            with self.subTest(method=method), self.assertRaises(BrokerError) as caught:
                validate_raw_cdp_method(self.project_root, method)
            self.assertEqual(caught.exception.code, "browser_capability_denied")

    async def test_request_project_root_controls_cleanup_and_grant_policy(self) -> None:
        other_root = self.project_root / "other-project"
        other_root.mkdir()
        artifact = persist_managed_browser_artifact(
            other_root,
            target("profile-a"),
            "other-cleanup",
            "revision-1",
            "png",
            b"fixture",
        )
        cleaned = await self.bridge.forward_command(
            {
                "cmd": "cleanup_browser_artifacts",
                "request_id": "other-cleanup",
                "project_root": str(other_root),
                "paths": [artifact["path"]],
            }
        )
        self.assertTrue(cleaned["ok"])
        self.assertFalse(Path(artifact["path"]).exists())

        await self.register("profile-a")
        lease = self.bridge.broker.acquire_lease("profile-a", "teshi-cli", 60)
        policy = other_root / ".teshi" / "browser-policy.json"
        policy.parent.mkdir(parents=True, exist_ok=True)
        policy.write_text(
            '{"privileged":{"allow":["javascript"]}}', encoding="utf-8"
        )
        created = await self.bridge.forward_command(
            {
                "cmd": "create_browser_capability_grant",
                "request_id": "other-grant",
                "project_root": str(other_root),
                "target": target("profile-a"),
                "lease_token": lease["lease_token"],
                "capability": "javascript",
                "caller_label": "teshi-cli",
                "non_interactive": True,
                "acknowledged_capability": "javascript",
            }
        )
        self.assertEqual(
            created["grant"]["project_root"].casefold(), str(other_root).casefold()
        )

    def test_managed_artifact_metadata_correlates_request_target_and_revision(self) -> None:
        artifact = persist_managed_browser_artifact(
            self.project_root,
            target("profile-a"),
            "capture-1",
            "revision-1",
            "png",
            b"not-a-real-png",
        )
        self.assertEqual(artifact["request_id"], "capture-1")
        self.assertEqual(artifact["target"], target("profile-a"))
        self.assertEqual(artifact["page_context_revision"], "revision-1")
        self.assertEqual(artifact["size"], 14)
        self.assertTrue(Path(artifact["path"]).is_file())

    def test_managed_artifact_rejects_oversized_payload_before_write(self) -> None:
        with self.assertRaises(BrokerError) as caught:
            persist_managed_browser_artifact(
                self.project_root,
                target("profile-a"),
                "oversized",
                "revision-1",
                "png",
                b"x" * (MAX_BROWSER_ARTIFACT_BYTES + 1),
            )
        self.assertEqual(caught.exception.code, "browser_artifact_failure")

    def test_cleanup_removes_only_explicit_managed_files(self) -> None:
        artifact = persist_managed_browser_artifact(
            self.project_root,
            target("profile-a"),
            "cleanup",
            "revision-1",
            "png",
            b"fixture",
        )
        outside = self.project_root / "keep.png"
        outside.write_bytes(b"keep")
        cleaned = cleanup_managed_browser_artifacts(
            self.project_root, [artifact["path"]]
        )
        self.assertEqual(cleaned["removed"], [artifact["path"]])
        self.assertTrue(outside.is_file())
        with self.assertRaises(BrokerError):
            cleanup_managed_browser_artifacts(self.project_root, [outside])
        self.assertTrue(outside.is_file())

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

    async def test_negotiated_direct_channel_bypasses_heartbeat_queue(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")
        delivered: list[dict] = []

        async def direct(instance_id: str, command: dict) -> bool:
            self.assertEqual(instance_id, "profile-a")
            delivered.append(command)
            asyncio.create_task(
                self.respond_to_snapshot("profile-a", command, "Save Direct")
            )
            return True

        self.bridge._direct_command_callback = direct
        task = asyncio.create_task(
            self.bridge.forward_command(
                {
                    "cmd": "get_page_snapshot",
                    "request_id": "direct-snapshot",
                    "target": target("profile-a"),
                    "lease_token": lease,
                }
            )
        )
        result = await task
        self.assertTrue(result["ok"])
        self.assertEqual(delivered[0]["request_id"], "direct-snapshot")
        heartbeat_result = await self.bridge.handle_heartbeat(
            self.profile_heartbeat("profile-a")
        )
        self.assertIsNone(heartbeat_result["cmd"])

    async def test_structured_candidate_is_reverified_and_forwarded_without_css_loss(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")
        delivered: list[dict] = []
        candidate = {
            "kind": "role",
            "arguments": {"role": "button", "name": "Save"},
            "expression": "page.getByRole('button', { name: 'Save' })",
            "context": {"frame": "checkout-frame", "shadow_root": "#host"},
        }

        async def direct(instance_id: str, command: dict) -> bool:
            delivered.append(command)
            if command["cmd"] == "verify_playwright_locators":
                payload = {
                    "type": "response",
                    "request_id": command["request_id"],
                    "extension_instance_id": instance_id,
                    "target": command["target"],
                    "ok": True,
                    "verification": [
                        {
                            "expression": candidate["expression"],
                            "match_count": 1,
                            "visible": True,
                            "enabled": True,
                        }
                    ],
                }
            else:
                payload = {
                    "type": "response",
                    "request_id": command["request_id"],
                    "extension_instance_id": instance_id,
                    "target": command["target"],
                    "ok": True,
                    "page_context_revision": "revision-a",
                }
            asyncio.create_task(self.bridge.handle_extension_response(payload))
            return True

        self.bridge._direct_command_callback = direct
        result = await self.bridge.forward_command(
            {
                "cmd": "execute_browser_action",
                "request_id": "structured-action",
                "target": target("profile-a"),
                "lease_token": lease,
                "action": "click",
                "element": {
                    "candidate": candidate,
                    "page_context_revision": "revision-a",
                },
            }
        )
        self.assertTrue(result["ok"])
        self.assertEqual([item["cmd"] for item in delivered], [
            "verify_playwright_locators",
            "execute_locator",
        ])
        self.assertEqual(delivered[1]["candidate"], candidate)
        self.assertEqual(delivered[1]["locator_context"], candidate["context"])
        self.assertIsNone(delivered[1]["selector"])

    async def test_tab_window_and_group_mutations_stay_on_selected_profile(self) -> None:
        await self.register("profile-a")
        await self.register("profile-b")
        lease = await self.acquire("profile-a")
        delivered: list[dict] = []

        async def direct(instance_id: str, command: dict) -> bool:
            self.assertEqual(instance_id, "profile-a")
            delivered.append(command)
            body = {"ok": True}
            if command["cmd"] in {"open_tab", "create_window"}:
                body["new_target"] = target("profile-a", 99, 11)
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        **body,
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        operations = [
            {"cmd": "open_tab", "url": "https://example.test/", "active": True},
            {"cmd": "activate_tab", "focus_window": True},
            {"cmd": "create_window", "url": "https://example.test/new", "focused": True},
            {"cmd": "group_tabs", "tab_ids": [42, 43], "title": "Agents"},
            {"cmd": "close_tab"},
        ]
        results = []
        for index, operation in enumerate(operations):
            results.append(
                await self.bridge.forward_command(
                    {
                        **operation,
                        "request_id": f"tab-op-{index}",
                        "target": target("profile-a"),
                        "lease_token": lease,
                    }
                )
            )
        self.assertTrue(all(result["ok"] for result in results))
        self.assertEqual(results[0]["new_target"], target("profile-a", 99, 11))
        self.assertEqual(results[2]["new_target"], target("profile-a", 99, 11))
        self.assertTrue(
            all(item["target"]["extension_instance_id"] == "profile-a" for item in delivered)
        )

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

    async def test_viewport_screenshot_is_persisted_as_managed_artifact(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")
        commands = []

        async def direct(instance_id: str, command: dict) -> bool:
            self.assertEqual(instance_id, "profile-a")
            self.assertEqual(command["cmd"], "capture_browser_screenshot")
            commands.append(command)
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        "ok": True,
                        "artifact_data": base64.b64encode(b"png-fixture").decode(),
                        "page_context_revision": "revision-a",
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        result = await self.bridge.forward_command(
            {
                "cmd": "capture_browser_screenshot",
                "request_id": "viewport-a",
                "target": target("profile-a"),
                "lease_token": lease,
                "format": "png",
            }
        )
        self.assertTrue(result["ok"])
        self.assertEqual(result["artifact"]["format"], "png")
        self.assertEqual(result["artifact"]["size"], len(b"png-fixture"))
        self.assertTrue(Path(result["artifact"]["path"]).is_file())
        self.assertEqual(
            set(result["artifact"]),
            {"path", "size", "format", "target", "request_id", "page_context_revision", "warnings"},
        )

        element_result = await self.bridge.forward_command(
            {
                "cmd": "capture_browser_screenshot",
                "request_id": "element-a",
                "target": target("profile-a"),
                "lease_token": lease,
                "format": "png",
                "element": {"css": "#save"},
            }
        )
        self.assertTrue(element_result["ok"])
        self.assertEqual(commands[-1]["selector"], "#save")

    async def test_pdf_is_persisted_and_unsupported_backend_error_is_preserved(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")

        async def direct(instance_id: str, command: dict) -> bool:
            self.assertEqual(command["cmd"], "generate_browser_pdf")
            if command["paper_format"] == "Unsupported":
                body = {
                    "ok": False,
                    "code": "browser_capability_unavailable",
                    "error": "PDF generation is unavailable",
                }
            else:
                body = {
                    "ok": True,
                    "artifact_data": base64.b64encode(b"%PDF-fixture").decode(),
                    "page_context_revision": "revision-a",
                }
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        **body,
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        result = await self.bridge.forward_command(
            {
                "cmd": "generate_browser_pdf",
                "request_id": "pdf-a",
                "target": target("profile-a"),
                "lease_token": lease,
                "paper_format": "A4",
                "scale": 1.0,
            }
        )
        self.assertTrue(result["ok"])
        self.assertEqual(result["artifact"]["format"], "pdf")
        self.assertTrue(Path(result["artifact"]["path"]).is_file())
        self.assertNotIn("artifact_data", result)
        unsupported = await self.bridge.forward_command(
            {
                "cmd": "generate_browser_pdf",
                "request_id": "pdf-unsupported",
                "target": target("profile-a"),
                "lease_token": lease,
                "paper_format": "Unsupported",
                "scale": 1.0,
            }
        )
        self.assertFalse(unsupported["ok"])
        self.assertEqual(unsupported["code"], "browser_capability_unavailable")

    async def test_console_capture_start_list_clear_and_stop_flow(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")
        start_task = asyncio.create_task(
            self.bridge.forward_command(
                {
                    "cmd": "start_console_capture",
                    "request_id": "console-start",
                    "target": target("profile-a"),
                    "lease_token": lease,
                    "levels": ["info", "error"],
                    "max_entries": 2,
                    "max_bytes": 4096,
                    "max_age_ms": 60_000,
                }
            )
        )
        start_command = await self.next_command("profile-a", "start_console_capture")
        await self.bridge.handle_extension_response(
            {
                "type": "response",
                "request_id": start_command["request_id"],
                "extension_instance_id": "profile-a",
                "target": start_command["target"],
                "ok": True,
            }
        )
        started = await start_task
        self.assertTrue(started["ok"])
        self.assertTrue(started["capture"]["active"])

        for index in range(3):
            accepted = await self.bridge.handle_extension_response(
                {
                    "type": "console_event",
                    "extension_instance_id": "profile-a",
                    "target": target("profile-a"),
                    "event": {"level": "info", "text": f"console-{index}"},
                }
            )
            self.assertTrue(accepted["accepted"])
        ignored = await self.bridge.handle_extension_response(
            {
                "type": "console_event",
                "extension_instance_id": "profile-a",
                "target": target("profile-a"),
                "event": {"level": "debug", "text": "filtered"},
            }
        )
        self.assertFalse(ignored["accepted"])

        listed = await self.bridge.forward_command(
            {
                "cmd": "list_console_events",
                "request_id": "console-list",
                "target": target("profile-a"),
                "lease_token": lease,
            }
        )
        self.assertEqual(
            [event["text"] for event in listed["events"]],
            ["console-1", "console-2"],
        )
        cleared = await self.bridge.forward_command(
            {
                "cmd": "clear_console_capture",
                "request_id": "console-clear",
                "target": target("profile-a"),
                "lease_token": lease,
            }
        )
        self.assertEqual(cleared["removed_entries"], 2)

        stop_task = asyncio.create_task(
            self.bridge.forward_command(
                {
                    "cmd": "stop_console_capture",
                    "request_id": "console-stop",
                    "target": target("profile-a"),
                    "lease_token": lease,
                }
            )
        )
        stop_command = await self.next_command("profile-a", "stop_console_capture")
        await self.bridge.handle_extension_response(
            {
                "type": "response",
                "request_id": stop_command["request_id"],
                "extension_instance_id": "profile-a",
                "target": stop_command["target"],
                "ok": True,
            }
        )
        stopped = await stop_task
        self.assertTrue(stopped["ok"])
        self.assertFalse(stopped["active"])

    async def test_network_capture_metadata_redaction_and_explicit_body_flow(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")

        async def direct(instance_id: str, command: dict) -> bool:
            body = {"ok": True}
            if command["cmd"] == "get_network_response_body":
                body.update({"body": "x" * 2_000, "base64_encoded": False})
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        **body,
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        started = await self.bridge.forward_command(
            {
                "cmd": "start_network_capture",
                "request_id": "network-start",
                "target": target("profile-a"),
                "lease_token": lease,
                "max_body_bytes": 1024,
                "sensitive_fields": ["x-private"],
            }
        )
        self.assertTrue(started["ok"])
        await self.bridge.handle_extension_response(
            {
                "type": "network_event",
                "extension_instance_id": "profile-a",
                "target": target("profile-a"),
                "event": {
                    "event_type": "request",
                    "request_id": "cdp-request-1",
                    "url": "https://example.test/data",
                    "method": "GET",
                    "headers": {"Authorization": "private", "X-Private": "private"},
                },
            }
        )
        await self.bridge.handle_extension_response(
            {
                "type": "network_event",
                "extension_instance_id": "profile-a",
                "target": target("profile-a"),
                "event": {
                    "event_type": "response",
                    "request_id": "cdp-request-1",
                    "status": 200,
                    "headers": {"Set-Cookie": "private"},
                },
            }
        )
        listed = await self.bridge.forward_command(
            {
                "cmd": "list_network_requests",
                "request_id": "network-list",
                "target": target("profile-a"),
                "lease_token": lease,
            }
        )
        self.assertEqual(listed["requests"][0]["request_id"], "cdp-request-1")
        self.assertNotIn("headers", listed["requests"][0])
        detail = await self.bridge.forward_command(
            {
                "cmd": "get_network_request_detail",
                "request_id": "network-detail",
                "target": target("profile-a"),
                "lease_token": lease,
                "network_request_id": "cdp-request-1",
            }
        )
        self.assertNotIn("private", str(detail))
        body = await self.bridge.forward_command(
            {
                "cmd": "get_network_request_detail",
                "request_id": "network-body",
                "target": target("profile-a"),
                "lease_token": lease,
                "network_request_id": "cdp-request-1",
                "include_body": True,
            }
        )
        self.assertEqual(len(body["body"]), 1024)
        self.assertTrue(body["truncated"])
        cleared = await self.bridge.forward_command(
            {
                "cmd": "clear_network_capture",
                "request_id": "network-clear",
                "target": target("profile-a"),
                "lease_token": lease,
            }
        )
        self.assertEqual(cleared["removed_entries"], 1)
        stopped = await self.bridge.forward_command(
            {
                "cmd": "stop_network_capture",
                "request_id": "network-stop",
                "target": target("profile-a"),
                "lease_token": lease,
            }
        )
        self.assertTrue(stopped["ok"])
        self.assertFalse(stopped["active"])

    async def test_two_profiles_capture_console_concurrently_without_cross_delivery(self) -> None:
        await asyncio.gather(self.register("profile-a"), self.register("profile-b"))
        lease_a, lease_b = await asyncio.gather(
            self.acquire("profile-a"), self.acquire("profile-b")
        )

        async def direct(instance_id: str, command: dict) -> bool:
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        "ok": True,
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        starts = await asyncio.gather(
            self.bridge.forward_command(
                {
                    "cmd": "start_console_capture",
                    "request_id": "console-start-a",
                    "target": target("profile-a"),
                    "lease_token": lease_a,
                }
            ),
            self.bridge.forward_command(
                {
                    "cmd": "start_console_capture",
                    "request_id": "console-start-b",
                    "target": target("profile-b"),
                    "lease_token": lease_b,
                }
            ),
        )
        self.assertTrue(all(result["ok"] for result in starts))
        await asyncio.gather(
            self.bridge.handle_extension_response(
                {
                    "type": "console_event",
                    "extension_instance_id": "profile-a",
                    "target": target("profile-a"),
                    "event": {"level": "info", "text": "only-a"},
                }
            ),
            self.bridge.handle_extension_response(
                {
                    "type": "console_event",
                    "extension_instance_id": "profile-b",
                    "target": target("profile-b"),
                    "event": {"level": "info", "text": "only-b"},
                }
            ),
        )
        listed_a, listed_b = await asyncio.gather(
            self.bridge.forward_command(
                {
                    "cmd": "list_console_events",
                    "request_id": "console-list-a",
                    "target": target("profile-a"),
                    "lease_token": lease_a,
                }
            ),
            self.bridge.forward_command(
                {
                    "cmd": "list_console_events",
                    "request_id": "console-list-b",
                    "target": target("profile-b"),
                    "lease_token": lease_b,
                }
            ),
        )
        self.assertEqual([item["text"] for item in listed_a["events"]], ["only-a"])
        self.assertEqual([item["text"] for item in listed_b["events"]], ["only-b"])

    async def test_debugger_conflict_rolls_back_console_capture_state(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")

        async def direct(instance_id: str, command: dict) -> bool:
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        "ok": False,
                        "code": "debugger_conflict",
                        "error": "another debugger owns the target",
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        result = await self.bridge.forward_command(
            {
                "cmd": "start_console_capture",
                "request_id": "console-conflict",
                "target": target("profile-a"),
                "lease_token": lease,
            }
        )
        self.assertFalse(result["ok"])
        self.assertEqual(result["code"], "debugger_conflict")
        self.assertEqual(self.bridge.broker.sessions["profile-a"].console_captures, {})

    async def test_monitored_mutation_dispatches_once_and_preserves_structured_diff(self) -> None:
        await self.register("profile-a")
        lease = await self.acquire("profile-a")
        delivered: list[dict] = []

        async def direct(instance_id: str, command: dict) -> bool:
            delivered.append(command)
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        "ok": True,
                        "action_outcome": {"ok": True},
                        "wait_outcome": {
                            "ok": False,
                            "code": "browser_wait_timeout",
                        },
                        "monitoring": {
                            "before": {"ok": True, "visible_text": ["Before"]},
                            "after": {"ok": True, "visible_text": ["After"]},
                            "diff": {
                                "available": True,
                                "added_text": ["After"],
                                "removed_text": ["Before"],
                            },
                        },
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        result = await self.bridge.forward_command(
            {
                "cmd": "execute_browser_action",
                "request_id": "monitored-action",
                "target": target("profile-a"),
                "lease_token": lease,
                "action": "click",
                "element": {"css": "#save"},
                "monitor": True,
            }
        )
        actions = [item for item in delivered if item["cmd"] == "execute_locator"]
        self.assertEqual(len(actions), 1)
        self.assertTrue(actions[0]["monitor"])
        self.assertTrue(result["action_outcome"]["ok"])
        self.assertEqual(result["wait_outcome"]["code"], "browser_wait_timeout")
        self.assertEqual(result["monitoring"]["diff"]["added_text"], ["After"])

    async def test_upload_validates_project_files_before_single_dispatch_without_path_echo(self) -> None:
        upload = self.project_root / "upload.txt"
        upload.write_text("fixture", encoding="utf-8")
        self.assertEqual(
            validate_browser_upload_files(self.project_root, ["upload.txt"]),
            [str(upload)],
        )
        await self.register("profile-a")
        lease = await self.acquire("profile-a")
        delivered: list[dict] = []

        async def direct(instance_id: str, command: dict) -> bool:
            delivered.append(command)
            asyncio.create_task(
                self.bridge.handle_extension_response(
                    {
                        "type": "response",
                        "request_id": command["request_id"],
                        "extension_instance_id": instance_id,
                        "target": command["target"],
                        "ok": True,
                        "action_outcome": {"ok": True, "uploaded_files": 1},
                        "wait_outcome": None,
                    }
                )
            )
            return True

        self.bridge._direct_command_callback = direct
        result = await self.bridge.forward_command(
            {
                "cmd": "execute_browser_action",
                "request_id": "upload-valid",
                "target": target("profile-a"),
                "lease_token": lease,
                "action": "upload",
                "element": {"css": "input[type=file]"},
                "files": ["upload.txt"],
            }
        )
        self.assertTrue(result["ok"])
        self.assertEqual(delivered[0]["files"], [str(upload)])
        self.assertNotIn(str(upload), str(result))

        with tempfile.TemporaryDirectory(prefix="teshi-upload-outside-") as outside_dir:
            outside = Path(outside_dir) / "private.txt"
            outside.write_text("private", encoding="utf-8")
            denied = await self.bridge.forward_command(
                {
                    "cmd": "execute_browser_action",
                    "request_id": "upload-denied",
                    "target": target("profile-a"),
                    "lease_token": lease,
                    "action": "upload",
                    "element": {"css": "input[type=file]"},
                    "files": [str(outside)],
                }
            )
            self.assertFalse(denied["ok"])
            self.assertEqual(denied["code"], "browser_capability_denied")
            self.assertNotIn(str(outside), str(denied))


if __name__ == "__main__":
    unittest.main()
