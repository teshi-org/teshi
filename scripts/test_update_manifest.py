"""Publication fixtures run without modifying a real installation."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import update_manifest as manifest


class UpdateManifestTests(unittest.TestCase):
    @patch.dict("os.environ", {"TESHI_RELEASE_TIMESTAMP": "2026-09-08T01:02:03Z"})
    def test_build_identity_and_nightly_tag(self):
        sha = "a" * 40
        stable = manifest.identity("v0.7.10", sha, 10)
        nightly = manifest.identity("v0.7.10-nightly.20260908.aaaaaaa", sha, 11)
        self.assertEqual(stable["channel"], "stable")
        self.assertEqual(nightly["build_sequence"], 11)
        self.assertEqual(stable["git_sha"], nightly["git_sha"])
        for tag, commit, sequence in [("v0.7.10", "a" * 7, 1), ("v0.7.10", sha, 0), ("v0.7.10-nightly.20260908.bbbbbbb", sha, 1)]:
            with self.assertRaises(ValueError):
                manifest.identity(tag, commit, sequence)

    def test_bundle_requires_desktop_and_excludes_self_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("teshi.exe", "teshi-update-helper.exe"):
                (root / name).write_bytes(b"executable")
            with self.assertRaises(ValueError):
                manifest.bundle(root, {}, manifest.TARGETS[0], "portable")
            (root / "teshi-desktop.exe").write_bytes(b"desktop")
            result = manifest.bundle(root, {}, manifest.TARGETS[0], "portable")
            self.assertEqual(len(result["files"]), 3)
            self.assertNotIn(manifest.MANIFEST, [f["path"] for f in result["files"]])
            self.assertEqual(json.loads((root / manifest.MANIFEST).read_text()), result)

    def test_windows_exe_bundle_can_omit_desktop_for_nightly_cli(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            for name in ("teshi.exe", "teshi-update-helper.exe"):
                (root / "bin" / name).write_bytes(b"executable")
            result = manifest.bundle(root, {}, manifest.TARGETS[0], "exe", require_desktop=False)
            self.assertEqual(
                [file["path"] for file in result["files"]],
                ["bin/teshi-update-helper.exe", "bin/teshi.exe"],
            )

    def test_publication_requires_every_target_and_checksums_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(FileNotFoundError):
                manifest.release(root, {}, "v0.7.10")
            for target in manifest.TARGETS:
                ext = "zip" if "windows" in target else "tar.gz"
                (root / f"teshi-v0.7.10-{target}.{ext}").write_bytes(b"fixture")
            (root / "teshi-v0.7.10-x64.msi").write_bytes(b"msi fixture")
            (root / "teshi-v0.7.10-x64-setup.exe").write_bytes(b"setup fixture")
            manifest.release(root, {}, "v0.7.10")
            self.assertIn("update-manifest.json", (root / "SHA256SUMS").read_text())
            self.assertIn("teshi-v0.7.10-x64-setup.exe", (root / "SHA256SUMS").read_text())
            self.assertEqual(len(json.loads((root / "update-manifest.json").read_text())["assets"]), 5)
            manifest.verify_release(root)
            (root / "teshi-v0.7.10-x64.msi").write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                manifest.verify_release(root)

    def test_publication_can_include_only_the_windows_setup_exe(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "teshi-v0.7.10-x64-setup.exe").write_bytes(b"setup fixture")
            manifest.release(root, {}, "v0.7.10", include=("exe",))
            assets = json.loads((root / "update-manifest.json").read_text())["assets"]
            self.assertEqual([asset["kind"] for asset in assets], ["exe"])
            self.assertEqual(
                [asset["name"] for asset in assets],
                ["teshi-v0.7.10-x64-setup.exe"],
            )
            self.assertIn("teshi-v0.7.10-x64-setup.exe", (root / "SHA256SUMS").read_text())
            self.assertNotIn(".msi", (root / "SHA256SUMS").read_text())
            manifest.verify_release(root)


if __name__ == "__main__":
    unittest.main()
