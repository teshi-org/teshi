#!/usr/bin/env python3
"""Guard phase-one migration: legacy daemon REST routes stay registered."""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SERVER = (ROOT / "apps" / "teshi-daemon" / "src" / "server.rs").read_text(
    encoding="utf-8"
)

REQUIRED_ROUTES = {
    "/api/v1/sessions",
    "/api/v1/sessions/{token}",
    "/api/v1/events",
    "/api/v1/projects/open",
    "/api/v1/projects/teardown",
    "/api/v1/projects/switch-allowed",
    "/api/v1/settings/recent",
    "/api/v1/settings/project",
    "/api/v1/fs/list",
    "/api/v1/fs/read",
    "/api/v1/gherkin/render",
    "/api/v1/gherkin/validate-buffer",
    "/api/v1/gherkin/scenarios",
    "/api/v1/api/exchange",
    "/api/v1/locator/sync-step",
    "/api/v1/locator/active-step",
    "/api/v1/locator/pending",
    "/api/v1/locator/confirm",
    "/api/v1/locator/reject",
    "/api/v1/locator/highlight",
    "/api/v1/steps/statuses",
    "/api/v1/steps/unbind",
    "/api/v1/steps/catalog",
    "/api/v1/llm/config",
    "/api/v1/llm/profiles",
    "/api/v1/llm/profiles/{id}",
    "/api/v1/llm/profiles/{id}/activate",
    "/api/v1/browser/start",
    "/api/v1/browser/stop",
    "/api/v1/browser/sessions",
    "/api/v1/browser/activate-tab",
    "/api/v1/browser/stream",
    "/api/v1/terminal/spawn",
    "/api/v1/terminal/stop",
    "/api/v1/terminal/resize",
    "/api/v1/terminal/write",
    "/api/v1/daemon/run",
    "/api/v1/daemon/shutdown",
}


def main() -> int:
    missing = sorted(route for route in REQUIRED_ROUTES if route not in SERVER)
    if missing:
        raise SystemExit("legacy REST routes disappeared: " + ", ".join(missing))
    print(f"legacy REST preservation contract passed ({len(REQUIRED_ROUTES)} routes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
