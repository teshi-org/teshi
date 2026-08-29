## Context

Teshi runs Gherkin by spawning an external NDJSON runner (`teshi run`, TUI Explore, daemon `POST /api/v1/daemon/run`). Events are scenario-level (`start_case` / `case_passed`). Browser and WinApp use Python sidecars and `.teshi/step-bindings`; GPUI (`teshi-ui`) only has Browser, WinApp, and Settings. There is no first-class HTTP API test engine and no request/response inspector.

Rust stays the orchestrator. Jinja2 rendering, HTTP, extract, and assert live in Python, matching `browser_service.py` / `winapp_service.py`.

## Goals / Non-Goals

**Goals:**

- One Gherkin step can invoke one or more HTTP interfaces; each interface is one `.json.j2` envelope.
- Step definitions only sequence `call(...)`; variables flow through scenario `vars` (Gherkin captures, extract, env).
- Pure `@api` / `@ui` and mixed `@api @ui` scenarios (Scenario tags override Feature; no union).
- Mixed runs: Teshi dispatches per step using `[API]` vs UI bindings.
- Same Python package for a long-lived sidecar (UI) and behave (CI `teshi run` on pure API).
- TUI Explore and GPUI inspect full exchanges with default secret redaction.

**Non-Goals:**

- Gherkin editing in GPUI.
- Storing API bindings in `.teshi/step-bindings`.
- Embedding Jinja2 or an HTTP client in Rust.
- Redirect/TLS/phase timing (dns/connect/ttfb) in v1 exchanges.
- Mixed UI+API inside a single Python behave process for CI (mixed always uses Teshi step dispatch).
- JSONPath extractors, free-form Python in templates, or retry/backoff beyond optional `timeout_ms`.

## Decisions

### 1. One `.json.j2` file per HTTP interface

The file is a Jinja2 template that renders to a JSON envelope: `method`, `url`, `headers`, `body`, `extract`, `assert`, optional `timeout_ms`. Sidecar renders in two passes: request fields without `response`, then `extract` / `assert` after the HTTP round-trip with `response` injected (`status`, `json`, `headers`, `body`).

Alternative: one YAML file listing N requests per step. Rejected: user required one file = one interface; orchestration stays in the step def.

### 2. Python step defs orchestrate; `call` takes only a template id

`features/steps/` holds behave-style `@when` / `@then`. Each handler calls `call("create_user.json.j2")` (resolved under `api/` or a project-relative path). It MUST NOT pass extracted ids as kwargs. Gherkin `{name}` captures are merged into `vars` before any `call`.

Alternative: Karate-style templates without Python functions. Rejected: user wanted BDD step definitions calling multiple APIs.

### 3. `vars` are scenario-scoped

At scenario start, Teshi/sidecar clears `vars`, injects `teshi.toml` `[api]` plus process env (e.g. `TESHI_API_TOKEN` → `token`), then runs Background in that scenario. Extract writes into `vars` for later `call`s and later Gherkin steps. Feature-wide leakage across scenarios is forbidden.

### 4. Engine tags and `[API]` markers

After the Gherkin keyword, an optional `[API]` token marks an HTTP step; it is stripped before behave matching. `@api` and `@ui` select engines: Scenario tags win if any engine tag is present; otherwise Feature tags apply. Mixed = both `@api` and `@ui` on the Scenario. A pure `@api` scenario that contains a non-`[API]` UI step (or pure `@ui` with `[API]`) MUST fail before execution.

Pure `@api` CI may run the whole scenario through behave. Mixed and interactive runs use Teshi stepping: `[API]` → API sidecar; otherwise → existing browser/WinApp bindings.

### 5. Dual packaging, one engine

Ship `resources/api_service.py` plus a helper imported by step defs. Interactive TUI/GPUI talk to a loopback sidecar WebSocket (browser pattern). `teshi run` for pure `@api` spawns behave with the same helper emitting NDJSON `http_exchange` (and step start/end) on stdout.

### 6. No API step-bindings

Locator JSON remains UI-only. Preview of “which templates this step will call” is out of v1; the inspector shows actual `call` order from runtime events.

### 7. Exchange event and UI redaction

Each HTTP attempt emits one `http_exchange`: template path, rendered method/url, request headers/body, status, response headers/body, duration_ms, extract map, per-assert pass/fail. TUI and GPUI use the same schema. Default redaction matches browser network capture (`Authorization`, `Cookie`, token/password/secret substrings, plus `teshi.toml` extras); the user can expand to plaintext in the inspector.

### 8. Jinja2 include sandbox

Templates MAY `{% include %}` / `{% import %}` only files under `api/` or configured template roots. Templates MUST NOT access Python objects or the OS. Shared headers live in fragments under `api/`.

### 9. GPUI surface

`AppShell` adds a Run/API inspect surface (not the default landing view). It lists runnable scenarios from the open project, starts a run through the daemon/engine, and shows step + exchange trees. Gherkin editing stays in the TUI.

### 10. HTTP client

Sidecar uses httpx. Optional `timeout_ms` on the envelope; v1 has no automatic retries.

## Risks / Trade-offs

- [Mixed dispatch complexity] → Teshi already steps browser replay; reuse that loop and branch on `[API]`. Fail fast on tag/step mismatch.
- [Two-pass Jinja2 mistakes] → Unit-test: `extract` expressions MUST NOT evaluate before `response` exists; request pass MUST NOT require `response`.
- [Secret leakage in NDJSON] → Redact on the emit path (sidecar) so CI logs and UI share the same default; plaintext only behind an explicit UI expand flag, never in default file artifacts.
- [Behave vs Teshi matcher drift] → Strip `[API]` in one shared helper used by sidecar and behave.
- [GPUI scope creep] → v1 is run + inspect only; no editor, no locator recording.

## Migration Plan

- Additive: projects without `@api` / `[API]` behave as today.
- Document `teshi.toml` `[api]`, env var names, directory layout, and mixed-tag rules.
- Bundle the sidecar in release `share/` next to browser/winapp scripts.
- Rollback: omit the new surface and runner; existing UI BDD unchanged.

## Open Questions

- Exact env-var → `vars` key mapping table (document in CLI help; default `TESHI_API_*` prefix).
- Whether daemon `POST /api/v1/daemon/run` streams new event types on the same NDJSON body or a WebSocket (prefer same NDJSON stream as today, extended types).
