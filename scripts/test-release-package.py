#!/usr/bin/env python3
"""Inspect installed/staged runtime resources and hashed bundle contents.

Run on staging roots before packaging, and on extracted release artifacts.
This deliberately does not bootstrap the user-wide broker or change pairing.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
from pathlib import Path
import subprocess
import time
import tomllib

from package_runtime import probe_rust_broker, runtime_resources


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {"x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "aarch64-apple-darwin"}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def compiled_identity(binary: Path, root: Path) -> dict:
    """Read metadata, allowing a bounded Windows sharing violation after copy."""
    deadline = time.monotonic() + 10
    while True:
        try:
            result = subprocess.run([str(binary), "--update-identity"], cwd=root,
                                    capture_output=True, text=True, timeout=20, check=True)
            return json.loads(result.stdout)
        except OSError as error:
            # Windows scanners can briefly open a freshly copied PE exclusively.
            # Only this read-only identity query retries; browser actions do not.
            if getattr(error, "winerror", None) != 32 or time.monotonic() >= deadline:
                raise
            time.sleep(0.1)


def inspect(root: Path, target: str, kind: str, cli_only: bool = False,
            run_cli: bool = False) -> dict:
    """Reject missing dependencies, stale resources, state, and hash drift."""
    require(target in TARGETS, f"Unsupported target: {target}")
    require(kind in {"msi", "exe", "portable", "agent"}, f"Unsupported kind: {kind}")
    require(not cli_only or (kind == "exe" and "windows" in target), "CLI-only is a Windows EXE layout")
    root = root.resolve()
    require(root.is_dir(), f"Package root does not exist: {root}")
    windows = "windows" in target
    standalone = kind == "agent"
    share = root / ("runtime" if standalone else "share")
    entries = ("browser_service.py",) if standalone else (
        ("browser_service.py", "api_service.py", "winapp_service.py") if windows
        else ("browser_service.py", "api_service.py")
    )
    embedded_tree = ast.parse((share / "browser_service.py").read_text(encoding="utf-8"))
    require(not any(isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef))
                    and node.name in {"ChromeBridge", "_LegacyChromeBridge", "run_chrome", "run_http_discovery"}
                    for node in embedded_tree.body),
            "Production package contains the retired Python Chrome transport")
    dependencies = runtime_resources(entries)
    expected_python = {source.name for source in dependencies}
    actual_python = {path.name for path in share.glob("*.py")}
    require(actual_python == expected_python,
            f"Runtime resource mismatch: missing={sorted(expected_python - actual_python)}, "
            f"unexpected={sorted(actual_python - expected_python)}")
    for source in dependencies:
        require(digest(share / source.name) == digest(source), f"Runtime source drift: {source.name}")

    package = root if standalone else share / "teshi-browser-testing"
    plugin = json.loads((package / ".codex-plugin/plugin.json").read_text(encoding="utf-8"))
    compatibility = json.loads((package / "compatibility.json").read_text(encoding="utf-8"))
    bridge = package / "extension/teshi-bridge"
    extension = json.loads((bridge / "manifest.json").read_text(encoding="utf-8"))
    require(plugin["version"] == compatibility["package_version"] == extension["version"],
            "Plugin/extension compatibility version mismatch")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    require(plugin["version"] == version, "Plugin/workspace version mismatch")
    require(compatibility["broker_protocol"] == 1, "Broker protocol mismatch")
    runtime = compatibility.get("chrome_runtime", {})
    require(runtime.get("default_implementation") == "rust"
            and runtime.get("endpoint_bridge") == "rust"
            and runtime.get("python_required") is False
            and runtime.get("automatic_fallback") is False
            and runtime.get("rejected_legacy_selector") == "TESHI_BROWSER_BROKER_IMPLEMENTATION=python"
            and "legacy_implementation_selector" not in runtime,
            "Chrome runtime compatibility mismatch")
    require("key" not in extension, "Packaging must preserve the unpacked extension identity contract")
    for relative in (".mcp.json", "skills/playwright-locator/SKILL.md", "skills/teshi/SKILL.md",
                     "skills/bdd-feature/SKILL.md", "skills/winapp-regression/SKILL.md"):
        require((package / relative).is_file(), f"Missing plugin asset: {relative}")
    bridges = [bridge] if standalone else [bridge, share / "teshi-bridge"]
    for destination in bridges:
        for relative in ("manifest.json", "background.js", "popup.js", "popup.html"):
            require(digest(destination / relative) == digest(ROOT / "extension/teshi-bridge" / relative),
                    f"Packaged extension drift: {destination.name}/{relative}")

    inventory = list(root.rglob("*"))
    forbidden = {".git", ".venv", "venv", "__pycache__", ".teshi", ".teshi-update"}
    for path in inventory:
        require(path.name != "chrome_broker_oracle.py", "Test-only Chrome oracle in package")
        require(not path.is_symlink() and not (hasattr(path, "is_junction") and path.is_junction()),
                f"Linked package entry: {path.relative_to(root)}")
        relative = path.relative_to(root)
        require(not any(part in forbidden or part.startswith(".env") for part in relative.parts),
                f"Local state in package: {relative}")
    paths = [path for path in inventory if path.is_file()]
    for path in paths:
        relative = path.relative_to(root)
        require(path.suffix.lower() not in {".pyc", ".pyo"}, f"Python cache in package: {relative}")

    if not standalone:
        suffix = ".exe" if windows else ""
        prefix = root / "bin" if kind in {"msi", "exe"} else root
        binaries = [prefix / (name + suffix) for name in ("teshi", "teshi-update-helper")]
        if windows and not cli_only:
            binaries.append(prefix / "teshi-desktop.exe")
        for binary in binaries:
            require(binary.is_file(), f"Missing shipped executable: {binary.name}")
            with binary.open("rb") as stream:
                magic = stream.read(4)
            valid = magic[:2] == b"MZ" if windows else (
                magic == b"\x7fELF" if "linux" in target else
                magic in {b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe"}
            )
            require(valid, f"Wrong executable format for {target}: {binary.name}")
        web = share / "web"
        if windows and not cli_only:
            for relative in ("index.html", "main.js", "pkg/teshi_web.js", "pkg/teshi_web_bg.wasm"):
                require((web / relative).is_file(), f"Missing GPUI WASM artifact: {relative}")
        else:
            require(not web.exists(), "This channel must not carry hosted Web UI")
        if cli_only:
            require(not (prefix / "teshi-desktop.exe").exists(), "Nightly must not carry native Desktop")
        manifest = json.loads((root / "teshi-bundle.json").read_text(encoding="utf-8-sig"))
        require(manifest["target"] == target and manifest["kind"] == kind, "Bundle layout metadata mismatch")
        listed = {item["path"]: item for item in manifest["files"]}
        require(len(listed) == len(manifest["files"]), "Duplicate bundle manifest entry")
        actual = {path.relative_to(root).as_posix() for path in paths if path.name != "teshi-bundle.json"}
        require(actual == set(listed), "Bundle inventory differs from files on disk")
        for name, item in listed.items():
            require(item["size"] == (root / name).stat().st_size and item["sha256"] == digest(root / name),
                    f"Bundle checksum mismatch: {name}")
        if run_cli:
            for binary in binaries:
                require(compiled_identity(binary, root) == manifest["identity"], f"Compiled identity drift: {binary.name}")
            probe_rust_broker(binaries[0])

    return {"target": target, "kind": kind, "cli_only": cli_only,
            "files": len(paths), "python_resources": sorted(expected_python),
            "compiled_identity_verified": run_cli and not standalone}


def main() -> None:
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--target", choices=sorted(TARGETS), required=True)
    parser.add_argument("--kind", choices=("msi", "exe", "portable", "agent"), required=True)
    parser.add_argument("--cli-only", action="store_true")
    parser.add_argument("--run-cli", action="store_true")
    args = parser.parse_args()
    print(json.dumps(inspect(args.root, args.target, args.kind, args.cli_only, args.run_cli), indent=2))


if __name__ == "__main__":
    main()
