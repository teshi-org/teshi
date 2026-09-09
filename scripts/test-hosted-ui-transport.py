#!/usr/bin/env python3
"""Static contract checks for the hosted GPUI WASM transport boundary.

This is intentionally a source/artifact smoke gate, not a replacement for a
real Chromium test. It catches accidental reintroduction of the old bundled
REST/XHR client while the hosted Pages workflow builds the current UI.
"""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WEB_SOURCE = (ROOT / "apps" / "teshi-web" / "src" / "lib.rs").read_text(
    encoding="utf-8"
)
WEB_INDEX = (ROOT / "apps" / "teshi-web" / "web" / "index.html").read_text(
    encoding="utf-8"
)
SERVER_SOURCE = (ROOT / "apps" / "teshi-daemon" / "src" / "server.rs").read_text(
    encoding="utf-8"
)
SESSION_SOURCE = (ROOT / "apps" / "teshi-daemon" / "src" / "session.rs").read_text(
    encoding="utf-8"
)
PROTOCOL = (ROOT / "crates" / "teshi-web-protocol" / "src" / "lib.rs").read_text(
    encoding="utf-8"
)


def require(text: str, needle: str, label: str) -> None:
    if needle not in text:
        raise SystemExit(f"missing {label}: {needle}")


def forbid(text: str, needle: str, label: str) -> None:
    if needle in text:
        raise SystemExit(f"forbidden {label}: {needle}")


def main() -> int:
    require(WEB_SOURCE, "struct ControlClient", "unique control client")
    require(WEB_SOURCE, "/ws/control", "control WebSocket endpoint")
    require(WEB_SOURCE, "/ws/preview", "preview WebSocket endpoint")
    require(WEB_SOURCE, "replace_state_with_url", "fragment removal")
    require(WEB_SOURCE, "/app/ui-manifest.json?ts=", "cache-busted manifest compatibility fetch")
    require(WEB_SOURCE, 'option_env!("TESHI_UI_SOURCE_SHA")', "compiled UI source identity")
    require(
        WEB_SOURCE,
        'option_env!("TESHI_UI_MINIMUM_CLI_JSON")',
        "compiled minimum CLI identity",
    )
    require(
        WEB_SOURCE,
        "manifest.minimum_cli != minimum",
        "manifest downgrade resistance",
    )
    require(WEB_SOURCE, "ClientMessage::ClientHello", "control/preview handshake")
    for method in (
        '"bdd.run"',
        '"llm.get_config"',
        '"browser.list_sessions"',
        '"api.get_exchange"',
    ):
        require(WEB_SOURCE, method, "migrated control method")
    for method in (
        '"project.open"',
        '"filesystem.read"',
        '"bdd.run"',
        '"locator.sync_step"',
        '"steps.catalog"',
        '"llm.get_config"',
        '"browser.list_sessions"',
        '"terminal.write"',
        '"api.get_exchange"',
    ):
        require(SERVER_SOURCE, method, "daemon control method")
    require(
        SESSION_SOURCE,
        "Role::HostedWebUi => false",
        "hosted role is excluded from legacy REST",
    )
    require(SERVER_SOURCE, "sanitize_preview_message", "preview relay allowlist")
    require(SERVER_SOURCE, "hosted_read_project_file", "hosted filesystem boundary")
    for forbidden in (
        'format!("connect to preview sidecar: {error}"',
        'format!("attach to target application: {error}"',
    ):
        forbid(SERVER_SOURCE, forbidden, "private sidecar diagnostic leakage")
    for forbidden in (
        "/api/v1/",
        "XmlHttpRequest",
        "localStorage",
        "sessionStorage",
        "document.cookie",
        "console.log(\"token",
    ):
        forbid(WEB_SOURCE, forbidden, "hosted UI transport/persistence marker")

    require(WEB_INDEX, 'name="referrer" content="no-referrer"', "referrer policy")
    require(WEB_INDEX, "Content-Security-Policy", "hosted UI CSP")
    require(WEB_INDEX, "ws://127.0.0.1:*", "WS CSP")
    require(PROTOCOL, "pub const CONTROL_PROTOCOL_VERSION", "control protocol version")
    require(PROTOCOL, "pub const PREVIEW_PROTOCOL_VERSION", "preview protocol version")
    print("hosted UI transport contract passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
