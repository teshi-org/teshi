"""Real two-Profile Chromium acceptance for P1 screenshot and PDF artifacts."""

from __future__ import annotations

import asyncio
import unittest
from pathlib import Path

from test_browser_two_profile_p0 import BrowserTwoProfileP0Tests


class BrowserP1ArtifactTests(BrowserTwoProfileP0Tests):
    @unittest.skip("P0 control loop is covered by test_browser_two_profile_p0.py")
    async def test_two_profiles_execute_concurrently_without_cross_routing(self) -> None:
        return

    async def test_two_profiles_capture_viewport_fullpage_element_pdf_and_cleanup(self) -> None:
        sessions = await self.wait_for_new_profiles(self.baseline)
        sessions.sort(key=lambda item: item["identity"]["extension_instance_id"])
        targets = [self.active_target(session) for session in sessions]
        leases = await asyncio.gather(
            self.cli("lease", "acquire", "--session", targets[0]["session"], "--owner", "p1-agent-a"),
            self.cli("lease", "acquire", "--session", targets[1]["session"], "--owner", "p1-agent-b"),
        )
        tokens = [lease["lease"]["lease_token"] for lease in leases]
        artifacts: list[Path] = []
        try:
            screenshots = await asyncio.gather(
                self.cli("screenshot", "--format", "png", *self.target_args(targets[0], tokens[0])),
                self.cli("screenshot", "--format", "jpeg", "--quality", "75", "--full-page", *self.target_args(targets[1], tokens[1])),
            )
            paths = [Path(result["artifact"]["path"]) for result in screenshots]
            artifacts.extend(paths)
            self.assertEqual(paths[0].read_bytes()[:8], b"\x89PNG\r\n\x1a\n")
            self.assertEqual(paths[1].read_bytes()[:2], b"\xff\xd8")

            snapshots = await asyncio.gather(
                self.cli("snapshot", *self.target_args(targets[0], tokens[0])),
                self.cli("snapshot", *self.target_args(targets[1], tokens[1])),
            )
            refs = [snapshot["interactive_elements"][0]["ref"] for snapshot in snapshots]
            elements = await asyncio.gather(
                self.cli("screenshot", "--reference", refs[0], *self.target_args(targets[0], tokens[0])),
                self.cli("screenshot", "--reference", refs[1], *self.target_args(targets[1], tokens[1])),
            )
            artifacts.extend(Path(result["artifact"]["path"]) for result in elements)

            pdfs = await asyncio.gather(
                self.cli("pdf", "--paper", "A4", "--print-background", *self.target_args(targets[0], tokens[0])),
                self.cli("pdf", "--paper", "Letter", "--landscape", "--scale", "0.9", *self.target_args(targets[1], tokens[1])),
            )
            pdf_paths = [Path(result["artifact"]["path"]) for result in pdfs]
            artifacts.extend(pdf_paths)
            self.assertTrue(all(path.read_bytes().startswith(b"%PDF") for path in pdf_paths))

            await self.cli("artifact-cleanup", *(item for path in artifacts for item in ("--path", str(path))))
            self.assertTrue(all(not path.exists() for path in artifacts))
        finally:
            await asyncio.gather(
                self.cli("lease", "release", "--session", targets[0]["session"], "--lease-token", tokens[0]),
                self.cli("lease", "release", "--session", targets[1]["session"], "--lease-token", tokens[1]),
            )


if __name__ == "__main__":
    unittest.main()
