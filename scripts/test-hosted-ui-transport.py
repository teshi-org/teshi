#!/usr/bin/env python3
"""Static contract checks for the hosted GPUI WASM transport boundary.

This is intentionally a source/artifact smoke gate, not a replacement for a
real Chromium test. It catches accidental reintroduction of the old bundled
REST/XHR client while the hosted Pages workflow builds the current UI.
"""

from __future__ import annotations

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
WEB_SOURCE = (ROOT / "apps" / "teshi-web" / "src" / "lib.rs").read_text(
    encoding="utf-8"
)
WEB_LOADER = (ROOT / "apps" / "teshi-web" / "web" / "main.js").read_text(
    encoding="utf-8"
)
WEB_E2E_LOADER = (ROOT / "apps" / "teshi-web" / "web" / "e2e-launch.js").read_text(
    encoding="utf-8"
)
# Mirrors the forbidden-marker scan the Pages deploy runs over `public/app`,
# which ships `index.html` and `main.js` but not the local E2E module.
HOSTED_FORBIDDEN_MARKERS = re.compile(r"port=[^& ]*token=|X-Teshi-Token|/api/v1/")
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


def quoted_requests(text: str) -> set[str]:
    """Extract every control RPC literal used by the hosted WASM backend."""
    return set(re.findall(r'\.request\(\s*"([^"]+)"', text))


HOSTED_CONTROL_INVENTORY = {
    "llm.get_config",
    "llm.set_config",
    "llm.list_profiles",
    "llm.get_profile",
    "llm.save_profile",
    "llm.delete_profile",
    "llm.activate_profile",
    "browser.start",
    "browser.stop",
    "browser.list_sessions",
    "browser.activate_tab",
    "project.open",
    "project.teardown",
    "project.switch_allowed",
    "project.list_recent",
    "project.get_settings",
    "filesystem.list",
    "filesystem.read",
    "bdd.render_feature",
    "bdd.validate_buffer",
    "bdd.list_scenarios",
    "bdd.run",
    "bdd.begin_replay",
    "bdd.replay_history",
    "bdd.replay_snapshot",
    "bdd.replay_image",
    "api.get_exchange",
    "locator.sync_step",
    "locator.active_step",
    "locator.pending",
    "locator.confirm",
    "locator.reject",
    "locator.highlight",
    "steps.catalog",
    "steps.statuses",
    "steps.unbind",
    "terminal.spawn",
    "terminal.stop",
    "terminal.resize",
    "terminal.write",
    "runtime.shutdown",
}
# This executable set is the hosted UI contract. Keep it in the test rather
# than duplicating it in a generated requirements document.


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
    ui_methods = quoted_requests(WEB_SOURCE)
    expected_ui_methods = {
        "browser.start",
        "browser.list_sessions",
        "browser.activate_tab",
        "llm.get_config",
        "llm.set_config",
        "llm.list_profiles",
        "llm.get_profile",
        "llm.save_profile",
        "llm.delete_profile",
        "llm.activate_profile",
        "bdd.validate_buffer",
        "bdd.list_scenarios",
        "bdd.run",
        "bdd.begin_replay",
        "bdd.replay_history",
        "bdd.replay_snapshot",
        "bdd.replay_image",
        "api.get_exchange",
    }
    if ui_methods != expected_ui_methods:
        raise SystemExit(
            "hosted UI control inventory drifted: "
            f"missing={sorted(expected_ui_methods - ui_methods)}, "
            f"unexpected={sorted(ui_methods - expected_ui_methods)}"
        )

    server_methods = set(
        re.findall(r'^\s*"([a-z][a-z0-9_.]+)"\s*=>', SERVER_SOURCE, re.MULTILINE)
    )
    missing_server_methods = sorted(ui_methods - server_methods)
    if missing_server_methods:
        raise SystemExit(
            "hosted UI methods are not registered by daemon control dispatch: "
            + ", ".join(missing_server_methods)
        )
    missing_inventory_methods = sorted(HOSTED_CONTROL_INVENTORY - server_methods)
    if missing_inventory_methods:
        raise SystemExit(
            "operation inventory methods are not registered by daemon control dispatch: "
            + ", ".join(missing_inventory_methods)
        )

    require(WEB_SOURCE, "Channel::Preview", "preview hello")
    require(WEB_SOURCE, "start_winapp", "WinApp preview branch")
    require(WEB_SOURCE, 'query_parameter("winapp_preview")', "WinApp preview selector")
    require(WEB_SOURCE, "fn open_wasm_preview_socket", "browser preview socket")
    for mode in ("BrowserMode::Chrome", "BrowserMode::Embedded", "BrowserMode::WinApp"):
        require(SERVER_SOURCE, mode, f"{mode} preview relay")

    # The hosted production client must not call the legacy REST surface. The
    # local ?e2e=1 harness may mint its own ephemeral session exactly once;
    # that bootstrap is not a hosted business workflow and lives in a module
    # the Pages bundle does not ship.
    forbid(WEB_SOURCE, "/api/v1/", "hosted UI business REST request")
    for name, text in (("index.html", WEB_INDEX), ("main.js", WEB_LOADER)):
        marker = HOSTED_FORBIDDEN_MARKERS.search(text)
        if marker:
            raise SystemExit(
                f"hosted {name} contains a forbidden launch/auth/REST marker: "
                f"{marker.group(0)}"
            )
    loader_fetches = re.findall(r'fetch\(\s*["\']([^"\']+)', WEB_LOADER)
    if loader_fetches:
        raise SystemExit(
            "unexpected hosted loader HTTP endpoints: " + ", ".join(loader_fetches)
        )
    gate_start = WEB_LOADER.find("async function prepareLocalE2eLaunch")
    gate_end = WEB_LOADER.find("\n}\n", gate_start)
    gate = WEB_LOADER[gate_start:gate_end] if gate_start >= 0 else ""
    require(gate, 'url.searchParams.get("e2e") !== "1"', "local E2E bootstrap gate")
    require(gate, "url.hostname", "local E2E loopback gate")
    require(gate, 'import("./e2e-launch.js")', "gated local E2E module import")
    if WEB_LOADER.count("e2e-launch.js") != 1:
        raise SystemExit("hosted loader must reference the local E2E module only behind its gate")
    e2e_fetches = re.findall(r'fetch\(\s*["\']([^"\']+)', WEB_E2E_LOADER)
    if e2e_fetches != ["/api/v1/sessions"]:
        raise SystemExit(
            "unexpected local E2E loader HTTP endpoints: " + ", ".join(e2e_fetches)
        )
    require(WEB_E2E_LOADER, 'method: "POST"', "local E2E session bootstrap")
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
    print(
        "hosted UI transport contract passed: "
        f"{len(ui_methods)} control RPCs, browser+WinApp preview, "
        "and no hosted business REST requests"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
