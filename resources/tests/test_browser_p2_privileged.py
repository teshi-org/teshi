"""Real two-Profile Chromium acceptance for explicitly granted P2 execution."""

from __future__ import annotations

import asyncio
import json
import subprocess
import unittest
from pathlib import Path

from test_browser_two_profile_p0 import BrowserTwoProfileP0Tests, REPO_ROOT, TESHI_CLI


class BrowserP2PrivilegedTests(BrowserTwoProfileP0Tests):
    async def asyncSetUp(self) -> None:
        self.policy_path = REPO_ROOT / ".teshi" / "browser-policy.json"
        self.original_policy = self.policy_path.read_bytes() if self.policy_path.exists() else None
        self.policy_path.parent.mkdir(parents=True, exist_ok=True)
        self.policy_path.write_text(
            json.dumps(
                {
                    "privileged": {
                        "allow": ["javascript", "raw-cdp", "cookies"],
                        "raw_cdp_methods": ["Page.getLayoutMetrics"],
                    }
                }
            ),
            encoding="utf-8",
        )
        await super().asyncSetUp()

    async def asyncTearDown(self) -> None:
        try:
            await super().asyncTearDown()
        finally:
            if self.original_policy is None:
                self.policy_path.unlink(missing_ok=True)
            else:
                self.policy_path.write_bytes(self.original_policy)

    @unittest.skip("P0 control loop is covered by test_browser_two_profile_p0.py")
    async def test_two_profiles_execute_concurrently_without_cross_routing(self) -> None:
        return

    async def test_two_profiles_execute_only_with_profile_bound_grants(self) -> None:
        sessions = await self.wait_for_new_profiles(self.baseline)
        targets = [self.active_target(session) for session in sessions]
        leases = await asyncio.gather(
            self.cli("lease", "acquire", "--session", targets[0]["session"], "--owner", "p2-a"),
            self.cli("lease", "acquire", "--session", targets[1]["session"], "--owner", "p2-b"),
        )
        tokens = [lease["lease"]["lease_token"] for lease in leases]
        try:
            grants = []
            for index in range(2):
                target_args = self.target_args(targets[index], tokens[index])
                js = await self.cli("grant", "create", "--capability", "javascript", "--yes", *target_args)
                cdp = await self.cli("grant", "create", "--capability", "raw-cdp", "--yes", *target_args)
                grants.append((js["grant"]["grant_token"], cdp["grant"]["grant_token"]))

            results = await asyncio.gather(
                self.cli("javascript", "--expression", "document.title", "--grant-token", grants[0][0], *self.target_args(targets[0], tokens[0])),
                self.cli("javascript", "--expression", "document.title", "--grant-token", grants[1][0], *self.target_args(targets[1], tokens[1])),
                self.cli("cdp", "Page.getLayoutMetrics", "--grant-token", grants[0][1], *self.target_args(targets[0], tokens[0])),
                self.cli("cdp", "Page.getLayoutMetrics", "--grant-token", grants[1][1], *self.target_args(targets[1], tokens[1])),
            )
            titles = sorted(result["result"] for result in results[:2])
            self.assertEqual(titles, ["Profile bootstrap-a", "Profile bootstrap-b"])
            self.assertTrue(
                all(
                    result["result"].get("cssLayoutViewport")
                    or result["result"].get("layoutViewport")
                    for result in results[2:]
                )
            )

            listed = await self.cli("grant", "list")
            serialized = json.dumps(listed)
            for js_token, cdp_token in grants:
                self.assertNotIn(js_token, serialized)
                self.assertNotIn(cdp_token, serialized)
            audit = await self.cli("audit", "--limit", "20")
            audit_text = json.dumps(audit)
            self.assertNotIn("document.title", audit_text)
            self.assertTrue(all(token not in audit_text for pair in grants for token in pair))
        finally:
            await asyncio.gather(
                self.cli("lease", "release", "--session", targets[0]["session"], "--lease-token", tokens[0]),
                self.cli("lease", "release", "--session", targets[1]["session"], "--lease-token", tokens[1]),
            )

    async def test_fresh_profiles_deny_cookie_access_without_popup_permission(self) -> None:
        sessions = await self.wait_for_new_profiles(self.baseline)
        target = self.active_target(sessions[0])
        lease = await self.cli("lease", "acquire", "--session", target["session"], "--owner", "p2-permission-denial")
        lease_token = lease["lease"]["lease_token"]
        try:
            feature = next(
                item for item in sessions[0]["capabilities"]["features"]
                if item["feature"] == "p2.cookies"
            )
            self.assertFalse(feature["available"])
            self.assertEqual(feature["reason"], "permission_not_granted")
            grant = await self.cli(
                "grant", "create", "--capability", "cookies", "--yes",
                *self.target_args(target, lease_token),
            )

            def invoke_cookie_command() -> subprocess.CompletedProcess[str]:
                return subprocess.run(
                    [
                        str(TESHI_CLI), "browser", "cookies", "--grant-token",
                        grant["grant"]["grant_token"],
                        *self.target_args(target, lease_token),
                    ],
                    cwd=REPO_ROOT, capture_output=True, text=True, timeout=30, check=False,
                )

            result = await asyncio.to_thread(invoke_cookie_command)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("browser_capability_unavailable", result.stderr)
            self.assertNotIn(grant["grant"]["grant_token"], result.stderr)
        finally:
            await self.cli("lease", "release", "--session", target["session"], "--lease-token", lease_token)


if __name__ == "__main__":
    unittest.main()
