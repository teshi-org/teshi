#!/usr/bin/env python3
"""Verify that a windows-installer staging root has no hosted Web UI payload."""

from __future__ import annotations

import argparse
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    if not root.is_dir():
        raise SystemExit(f"staging root does not exist: {root}")

    required = [
        root / "bin" / "teshi.exe",
        root / "share" / "browser_service.py",
        root / "share" / "api_service.py",
    ]
    missing = [str(path.relative_to(root)) for path in required if not path.is_file()]
    if missing:
        raise SystemExit(f"nightly staging is missing runtime files: {', '.join(missing)}")

    web_dir = root / "share" / "web"
    if web_dir.exists() and any(web_dir.rglob("*")):
        raise SystemExit("nightly installer staging contains share/web payload")

    # Browser-extension HTML (for example teshi-bridge/popup.html) is a
    # legitimate runtime asset. Reject the hosted GPUI payload by its known
    # names and WASM files instead of banning every HTML file in the bundle.
    forbidden_suffixes = {".wasm"}
    forbidden_names = {
        "index.html",
        "main.js",
        "teshi_web.js",
        "teshi_web_bg.js",
        "ui-manifest.json",
    }
    offenders = []
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix.lower() in forbidden_suffixes or path.name in forbidden_names:
            offenders.append(str(path.relative_to(root)))
    if offenders:
        raise SystemExit(
            "nightly installer staging contains hosted UI files: " + ", ".join(offenders)
        )
    print(f"nightly artifact content check passed: {root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
