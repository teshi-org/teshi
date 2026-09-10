#!/usr/bin/env python3
"""Guard the split between hosted UI delivery and nightly CLI packaging."""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASE = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
NIGHTLY = (ROOT / ".github" / "workflows" / "nightly.yml").read_text(encoding="utf-8")


def require(text: str, needle: str, label: str) -> None:
    if needle not in text:
        raise SystemExit(f"missing {label}: {needle}")


def main() -> int:
    require(NIGHTLY, "run_scope: windows-installer", "nightly installer scope")
    desktop_block_start = RELEASE.index("      - name: Build native desktop")
    desktop_block = RELEASE[desktop_block_start : RELEASE.find("\n      - name:", desktop_block_start + 1)]
    require(
        desktop_block,
        "needs.resolve.outputs.run_scope != 'windows-installer'",
        "nightly guard for native desktop",
    )
    require(
        RELEASE,
        'needs.resolve.outputs.run_scope }}" != "windows-installer"',
        "nightly guard for desktop identity verification",
    )
    require(RELEASE, "prefix-key: teshi-${{ matrix.target }}", "cross-nightly Rust cache key")
    for label in (
        "Install nightly WASM target",
        "Cache wasm-bindgen CLI",
        "Install wasm-bindgen CLI",
        "Build GPUI WASM web UI",
    ):
        block_start = RELEASE.index(f"      - name: {label}")
        block = RELEASE[block_start : RELEASE.find("\n      - name:", block_start + 1)]
        require(
            block,
            "needs.resolve.outputs.run_scope != 'windows-installer'",
            f"nightly guard for {label}",
        )
    require(RELEASE, "$includeWeb = '${{ needs.resolve.outputs.run_scope }}' -ne 'windows-installer'", "setup web gate")
    require(RELEASE, "if ($includeWeb) { $required +=", "setup required-file gate")
    require(RELEASE, "if ($includeWeb) { Copy-Item -Path \"apps/teshi-web/dist/*\"", "setup copy gate")
    require(RELEASE, "$includeDesktop = '${{ needs.resolve.outputs.run_scope }}' -ne 'windows-installer'", "desktop packaging gate")
    require(RELEASE, "if (-not $includeDesktop) { $bundleArgs += \"--omit-desktop\" }", "CLI-only bundle gate")
    require(RELEASE, "if (-not $includeDesktop) { $setupArgs.CliOnly = $true }", "CLI-only installer gate")
    require(RELEASE, "name: Upload web dist artifact", "web artifact step")
    web_upload = RELEASE[RELEASE.index("      - name: Upload web dist artifact") :]
    require(
        web_upload.split("\n      - name:", 1)[0],
        "needs.resolve.outputs.run_scope != 'windows-installer'",
        "web artifact upload gate",
    )
    print("release workflow contract passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
