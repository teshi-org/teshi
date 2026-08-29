## Why

Teshi can author and run UI BDD (browser/WinApp) but cannot run HTTP API tests as first-class Gherkin, and neither the TUI nor GPUI can show the full request/response of each call. Teams that mix UI and API in one product need one editor, one run button, and one inspector—without embedding Jinja2 or HTTP clients in Rust.

## What Changes

- Add a Python API sidecar and helper (same package for interactive use and CI) that renders one `.json.j2` file per HTTP interface, executes the request, then evaluates Jinja2 `extract` and `assert` against the response.
- Python step definitions orchestrate multiple `call(...)` invocations per Gherkin step; captured Gherkin parameters and extracted variables flow through scenario `vars` without explicit `call` kwargs.
- Gherkin uses `[API]` on API steps and `@api` / `@ui` engine tags (Scenario overrides Feature; both tags means mixed). Teshi dispatches mixed scenarios step-by-step across API and UI sidecars.
- Extend the NDJSON run protocol with step and `http_exchange` events.
- TUI Explore and a new GPUI surface subscribe to the same events: run a scenario and inspect each exchange (sensitive fields redacted by default).

## Capabilities

### New Capabilities

- `api-bdd-testing`: Gherkin conventions, `.json.j2` envelope, Python sidecar/helper, `vars` / extract / assert, mixed-scenario dispatch, runner events, environment injection.
- `api-run-inspect-ui`: TUI Explore and GPUI run/inspect surfaces for HTTP exchanges (no Gherkin editing in GPUI).

### Modified Capabilities

- `gpui-shell`: Add a shared Run/API inspect surface alongside Browser, WinApp, and Settings.

## Impact

- New: `resources/api_service.py` (and helper module), optional `teshi api` CLI, bundled like browser/winapp sidecars.
- Engine/CLI: step `[API]` stripping, `@api`/`@ui` mode resolution, Teshi step dispatcher for mixed runs, `teshi.toml` `[api]` config.
- Runner protocol: `http_exchange` (and step start/end) on NDJSON / sidecar WebSocket.
- TUI: Explore inspector for exchanges.
- GPUI (`teshi-ui`, desktop, web/daemon): new shell surface; daemon run streaming already exists and must carry the new events.
- Docs: user guide, CLI, development notes.
- Dependencies: Python httpx + Jinja2 in the sidecar environment (not in Rust).
