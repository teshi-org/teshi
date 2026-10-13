"""Negative artifact inventory tests; headers below are layout fixtures only."""

import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

import update_manifest
from package_runtime import copy_runtime


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("release_package", ROOT / "scripts/test-release-package.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


def stage(root, target, kind, cli_only=False):
    """Create a structural fixture, never a runnable cross-platform build."""
    windows = "windows" in target
    prefix = root / "bin" if kind in {"msi", "exe"} else root
    prefix.mkdir(parents=True, exist_ok=True)
    magic = b"MZ\x00\x00" if windows else (b"\x7fELF" if "linux" in target else b"\xcf\xfa\xed\xfe")
    names = ["teshi", "teshi-update-helper"]
    if windows and not cli_only:
        names.append("teshi-desktop")
    for name in names:
        (prefix / (name + (".exe" if windows else ""))).write_bytes(magic)
    entries = ("browser_service.py", "api_service.py") + (("winapp_service.py",) if windows else ())
    copy_runtime(root / "share", entries)
    agent = root / "share/teshi-browser-testing"
    shutil.copytree(ROOT / "agent-packages/teshi-browser-testing", agent)
    for skill in ("teshi", "bdd-feature", "winapp-regression"):
        shutil.copytree(ROOT / "skills" / skill, agent / "skills" / skill)
    for destination in (root / "share/teshi-bridge", agent / "extension/teshi-bridge"):
        shutil.copytree(ROOT / "extension/teshi-bridge", destination)
    if windows and not cli_only:
        web = root / "share/web"
        (web / "pkg").mkdir(parents=True)
        for relative in ("index.html", "main.js", "pkg/teshi_web.js", "pkg/teshi_web_bg.wasm"):
            (web / relative).write_bytes(b"fixture")
    update_manifest.bundle(root, {}, target, kind, require_desktop=not cli_only)


class PackageInventoryTests(unittest.TestCase):
    def test_all_channel_layouts(self):
        cases = [(target, "portable", False) for target in update_manifest.TARGETS]
        cases += [(update_manifest.TARGETS[0], "msi", False),
                  (update_manifest.TARGETS[0], "exe", False),
                  (update_manifest.TARGETS[0], "exe", True)]
        for target, kind, cli_only in cases:
            with self.subTest(target=target, kind=kind, cli_only=cli_only), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                stage(root, target, kind, cli_only)
                result = package.inspect(root, target, kind, cli_only)
                self.assertIn("update_participant.py", result["python_resources"])
                self.assertFalse(result["compiled_identity_verified"])

    def test_stale_chrome_runtime_metadata_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[1], "portable")
            metadata = root / "share/teshi-browser-testing/compatibility.json"
            value = json.loads(metadata.read_text(encoding="utf-8"))
            value["chrome_runtime"]["default_implementation"] = "python"
            metadata.write_text(json.dumps(value), encoding="utf-8")
            update_manifest.bundle(root, {}, update_manifest.TARGETS[1], "portable")
            with self.assertRaisesRegex(ValueError, "Chrome runtime compatibility mismatch"):
                package.inspect(root, update_manifest.TARGETS[1], "portable")

    def test_chrome_oracle_is_not_shipped(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[1], "portable")
            (root / "share/chrome_broker_oracle.py").write_text("pass", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "Runtime resource mismatch"):
                package.inspect(root, update_manifest.TARGETS[1], "portable")

    def test_missing_live_import_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[1], "portable")
            (root / "share/update_participant.py").unlink()
            with self.assertRaisesRegex(ValueError, "Runtime resource mismatch"):
                package.inspect(root, update_manifest.TARGETS[1], "portable")

    def test_unreferenced_python_resource_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[1], "portable")
            (root / "share/legacy_unused.py").write_text("pass", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "Runtime resource mismatch"):
                package.inspect(root, update_manifest.TARGETS[1], "portable")

    def test_wrong_binary_format_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[2], "portable")
            (root / "teshi").write_bytes(b"MZ\x00\x00")
            with self.assertRaisesRegex(ValueError, "Wrong executable format"):
                package.inspect(root, update_manifest.TARGETS[2], "portable")

    def test_nightly_hosted_ui_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[0], "exe", True)
            (root / "share/web").mkdir()
            with self.assertRaisesRegex(ValueError, "must not carry hosted Web UI"):
                package.inspect(root, update_manifest.TARGETS[0], "exe", True)

    def test_tampered_asset_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[1], "portable")
            (root / "share/teshi-browser-testing/.mcp.json").write_text("{}", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "Bundle checksum mismatch"):
                package.inspect(root, update_manifest.TARGETS[1], "portable")

    def test_local_state_is_rejected_even_when_in_manifest(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage(root, update_manifest.TARGETS[1], "portable")
            (root / "share/.env").write_text("fixture", encoding="utf-8")
            update_manifest.bundle(root, {}, update_manifest.TARGETS[1], "portable")
            with self.assertRaisesRegex(ValueError, "Local state in package"):
                package.inspect(root, update_manifest.TARGETS[1], "portable")


if __name__ == "__main__":
    unittest.main()
