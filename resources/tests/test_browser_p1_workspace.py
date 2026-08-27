"""Real two-Profile Chromium acceptance for P1 monitoring and workspace controls."""

from __future__ import annotations

import asyncio
import json
import unittest

from test_browser_two_profile_p0 import BrowserTwoProfileP0Tests


class BrowserP1WorkspaceTests(BrowserTwoProfileP0Tests):
    @unittest.skip("P0 control loop is covered by test_browser_two_profile_p0.py")
    async def test_two_profiles_execute_concurrently_without_cross_routing(self) -> None:
        return

    async def test_monitor_upload_focus_group_and_two_profile_isolation(self) -> None:
        sessions = await self.wait_for_new_profiles(self.baseline)
        targets = [self.active_target(session) for session in sessions]
        leases = await asyncio.gather(
            self.cli("lease", "acquire", "--session", targets[0]["session"], "--owner", "p1-workspace-a"),
            self.cli("lease", "acquire", "--session", targets[1]["session"], "--owner", "p1-workspace-b"),
        )
        tokens = [lease["lease"]["lease_token"] for lease in leases]

        try:
            port = self.http.server_address[1]
            await asyncio.gather(
                self.cli("navigate", f"http://127.0.0.1:{port}/monitor-a", *self.target_args(targets[0], tokens[0])),
                self.cli("navigate", f"http://127.0.0.1:{port}/monitor-b", *self.target_args(targets[1], tokens[1])),
            )

            monitored = await asyncio.gather(
                self.cli("execute", "--selector", "#action", "--action", "click", "--monitor", *self.target_args(targets[0], tokens[0])),
                self.cli("execute", "--selector", "#action", "--action", "click", "--monitor", *self.target_args(targets[1], tokens[1])),
            )
            for result in monitored:
                self.assertTrue(result["action_outcome"]["ok"])
                diff = json.dumps(result["monitoring"]["diff"])
                self.assertIn("clicked-monitor-", diff)
                self.assertIn("idle-monitor-", diff)

            uploads = await asyncio.gather(
                self.cli("execute", "--selector", "#upload", "--action", "upload", "--file", "README.md", "--monitor", *self.target_args(targets[0], tokens[0])),
                self.cli("execute", "--selector", "#upload", "--action", "upload", "--file", "README.md", "--monitor", *self.target_args(targets[1], tokens[1])),
            )
            self.assertTrue(all(result["action_outcome"]["uploaded_files"] == 1 for result in uploads))
            self.assertTrue(all("README.md" in json.dumps(result["monitoring"]["diff"]) for result in uploads))

            observed = sorted(
                await asyncio.gather(
                    *(context.pages[0].locator("#upload-status").text_content() for context in self.contexts)
                )
            )
            self.assertEqual(observed, ["uploaded-README.md", "uploaded-README.md"])

            opened = await self.cli(
                "tab", "open", f"http://127.0.0.1:{port}/organized", *self.target_args(targets[0], tokens[0])
            )
            new_target = opened["new_target"]
            await self.wait_for_target(new_target)
            inactive_activation = await self.cli(
                "tab", "activate", *self.target_args(targets[0], tokens[0])
            )
            self.assertFalse(inactive_activation["focus_requested"])
            self.assertFalse(inactive_activation["window_focused"])
            focused_activation = await self.cli(
                "tab", "activate", "--focus-window", *self.target_args(targets[0], tokens[0])
            )
            self.assertTrue(focused_activation["focus_requested"])
            self.assertTrue(focused_activation["window_focused"])

            grouped = await self.cli(
                "tab", "group",
                "--tab-id", targets[0]["tab"],
                "--tab-id", str(new_target["tab_id"]),
                "--title", "P1 Workspace",
                *self.target_args(targets[0], tokens[0]),
            )
            self.assertTrue(grouped["ok"])
            self.assertIn("organized", grouped)
            if not grouped["organized"]:
                self.assertEqual(grouped["warning"]["code"], "tab_group_unavailable")
        finally:
            await asyncio.gather(
                self.cli("lease", "release", "--session", targets[0]["session"], "--lease-token", tokens[0]),
                self.cli("lease", "release", "--session", targets[1]["session"], "--lease-token", tokens[1]),
            )


if __name__ == "__main__":
    unittest.main()
