## 1. Python sidecar and helper

- [x] 1.1 Add `resources/api_service.py` (loopback WebSocket/JSON commands) and a helper module step defs can `import` for `call(template_id)`
- [x] 1.2 Implement two-pass Jinja2 render of `.json.j2` envelopes and httpx send with optional `timeout_ms`
- [x] 1.3 Implement `extract` / `assert` after response injection; merge extract into scenario `vars`
- [x] 1.4 Seed `vars` from `teshi.toml` `[api]` and `TESHI_API_*` env; clear `vars` at scenario start
- [x] 1.5 Sandbox `{% include %}` / `{% import %}` to template roots; reject paths outside
- [x] 1.6 Emit `http_exchange` (and step correlation ids) on sidecar WS and as NDJSON for behave
- [x] 1.7 Default-redact sensitive headers/fields on emit; keep raw values only for explicit inspector expand

## 2. Gherkin, tags, and Teshi dispatch

- [x] 2.1 Parse and strip `[API]` after Gherkin keywords; fail tag/step mismatches (`@api`-only vs UI steps, `@ui`-only vs `[API]`)
- [x] 2.2 Resolve `@api` / `@ui` with Scenario override (no Feature∪Scenario union); mixed = both tags on Scenario
- [x] 2.3 Teshi step dispatcher: mixed and interactive runs walk steps; `[API]` → API sidecar; else browser/WinApp bindings
- [x] 2.4 Pure `@api` `teshi run` path through behave using the same helper; document `[runner]` example
- [x] 2.5 Wire `teshi.toml` `[api]` template root override (`api/` default) and `features/steps/` discovery

## 3. CLI and engine

- [x] 3.1 Add `teshi api` (or equivalent) to start/stop/doctor the API sidecar, analogous to browser sidecar lifecycle
- [x] 3.2 Extend NDJSON `RunEvent` parsing for `http_exchange` and step start/end without breaking existing case events
- [x] 3.3 Stream the new events from daemon `POST /api/v1/daemon/run` (same NDJSON body, extended types)

## 4. TUI Explore

- [x] 4.1 Attach exchanges to the current step in Explore; show pass/fail from envelope asserts
- [x] 4.2 Inspector pane: full envelope fields from the spec; redacted by default with expand-to-plaintext
- [x] 4.3 Run mixed `@api` `@ui` scenarios from Explore using the Teshi dispatcher

## 5. GPUI Run/API surface

- [x] 5.1 Add `ShellSurface::Run` (name as implemented) and header nav in `teshi-ui` `AppShell`
- [x] 5.2 Add a backend trait for list-scenarios / start-run / subscribe-events (no `teshi-engine` in `teshi-ui`)
- [x] 5.3 Implement inspect tree (steps → exchanges) with the same redaction rules as TUI
- [x] 5.4 Desktop and web hosts: wire backends (daemon HTTP/WS for web; engine/sidecar for desktop)

## 6. Tests and fixtures

- [x] 6.1 Python tests: two-pass render, extract chaining across two `call`s, assert failure, include sandbox
- [x] 6.2 Rust tests: `[API]` strip, Scenario tag override, mismatch failure
- [x] 6.3 Sample project under `api/` + `features/steps/` used by tests (create user then get by extracted id)
- [x] 6.4 GPUI/TUI tests or smoke: exchange JSON fixture renders inspector without leaking redacted values by default

## 7. Docs

- [x] 7.1 Document Gherkin conventions, envelope schema, `teshi.toml` `[api]`, env vars, mixed dispatch
- [x] 7.2 Point user-guide / CLI docs at API BDD; note GPUI is run+inspect only

## 8. Verification

- [x] 8.1 `cargo fmt --all` and targeted `cargo test` / Python sidecar tests for this change
