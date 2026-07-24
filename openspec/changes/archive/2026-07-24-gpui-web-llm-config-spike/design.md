## Context

Teshi already has:

- `apps/teshi-desktop`: native GPUI Hello World on crates.io `gpui = "0.2"`.
- `apps/teshi-daemon`: Axum HTTP + WS host that serves a static `dist/` (today: React Tauri frontend) and exposes `/api/v1/*`.
- React UI with `TeshiRuntimeApi` (`tauri` vs `web` fetch) as the production surface.
- Marketing site in `teshi-org.github.io` (Hugo) — stays as `teshi.org/`.
- Engine LLM transport: `teshi_engine::llm::LlmConfig` (`api_key`, `base_url`, `model`, …), primarily from `TESHI_LLM_*` env; TUI has richer provider/credential flows under `teshi-tui`.

Strategic lock-in from exploration:

- Replace React with GPUI long-term; desktop = native GPUI; `teshi.org/app` = GPUI WASM.
- Round-1 communication path = **Path 1**: daemon hosts GPUI static assets (same-origin).
- Round-1 product surface = **LLM config only**.

## Goals / Non-Goals

**Goals:**

- One shared GPUI view crate builds and runs on native desktop and `wasm32-unknown-unknown`.
- Daemon can serve that WASM build; opening the daemon URL loads the GPUI shell same-origin.
- WASM shell can `fetch` LLM config get/set against daemon; desktop shell can load/save via native backend.
- UI fields: base URL, model, API key (masked on read); save + “configured?” status.
- Document crate dependency rules so WASM never links `teshi-engine` / `teshi-agent`.

**Non-Goals:**

- React panel migration or Tauri removal.
- WebSocket `/api/v1/events` in this spike.
- Feature editor, agent chat, browser, terminal, mind map.
- Hugo `/app` publish pipeline (can follow after spike).
- Full TUI provider profile / `teshi auth` parity (optional later consolidation).
- New large crate graph (`teshi-protocol`, `teshi-platform-*`, etc.) beyond what the spike needs.

## Decisions

### D1 — Path 1 hosting only

**Choice:** Validate WASM↔daemon over same-origin by having `teshi-daemon` `ServeDir` the GPUI web `dist` (via existing `--dist` or an updated default resolver).

**Why:** Avoids CORS / Private Network Access issues of `teshi.org/app` → `127.0.0.1`. Proves the transport Teshi will use for local `teshi web`.

**Alternatives:** Pages-hosted `/app` + cross-origin daemon (deferred).

### D2 — Shared `teshi-ui`, thin entry crates

**Choice:**

```text
apps/teshi-desktop  → native Application::run + NativeBackend
apps/teshi-web      → gpui_platform::single_threaded_web + WasmBackend
crates/teshi-ui     → RootView / LlmConfigView (GPUI only)
```

`teshi-ui` depends on GPUI + a small backend trait (in `teshi-ui` or a tiny `teshi-client` module). It MUST NOT depend on `teshi-engine`.

**Why:** Matches “one UI + two backends”; keeps WASM link graph clean.

### D3 — Pin one GPUI git revision in workspace

**Choice:** Root `[workspace.dependencies]` pins `gpui` + `gpui_platform` to one Zed rev (start from a rev known to support `FetchHttpClient` / `single_threaded_web`). Desktop and web both use `*.workspace = true`.

**Why:** View types are not shareable across crates.io `0.2` vs git web platform.

### D4 — Backend trait, reuse daemon HTTP (no new protocol crate yet)

**Choice:** Minimal trait, e.g.:

```text
get_llm_config() -> LlmConfigView
set_llm_config(LlmConfigUpdate) -> ()
```

- **WasmBackend:** `gpui_web::FetchHttpClient` / `web_sys` fetch to `/api/v1/llm/config`.
- **NativeBackend:** read/write the same on-disk store (or call a shared helper in engine/daemon code paths without pulling engine into `teshi-ui`).

Do **not** introduce `teshi-protocol` as a separate crate in this change unless serialization types need sharing; DTOs can live next to the daemon handlers and be duplicated thinly in the WASM client for the spike, or shared via `teshi-core` if they are pure data.

**Why:** Fastest proof; React already proved HTTP shape.

### D5 — LLM config persistence for the spike

**Choice:** User-level store under Teshi app data (e.g. `llm-config.json`) with fields aligned to `LlmConfig`: `base_url`, `model`, `api_key` (and optionally `max_tokens` / `temperature` later). Daemon:

- `GET /api/v1/llm/config` → returns config with **masked** API key (e.g. last 4 chars or `configured: true` + empty key).
- `PUT /api/v1/llm/config` → saves plaintext key on disk (local daemon trust model).

Native backend writes the same file so desktop and web agree when using the same machine.

**Why:** Env-only `TESHI_LLM_*` cannot be updated from UI; full TUI credential manager is coupled to `teshi-tui`. Spike store is enough; a follow-up can merge with `CredentialManager` / providers.

**Alternatives considered:** Only wrap env vars (insufficient); call into `teshi-tui` auth (wrong dependency direction).

### D6 — UI scope

**Choice:** Single screen / panel: title, three inputs (base URL, model, API key), Save, status text (Configured / Not configured / Saved / Error). No chat, no test completion call required for acceptance (optional “Test connection” can be a stretch task).

### D7 — React dist coexistence

**Choice:** Spike uses explicit `--dist <gpui-web-dist>` (or env) so React default resolver can remain until migration. Do not delete Tauri frontend in this change.

## Risks / Trade-offs

- [GPUI rev churn] → Pin explicitly; upgrade as a dedicated chore.
- [API key on disk via daemon] → Acceptable for local admin daemon; document no cloud deployment of this store; never log the raw key.
- [Duplicate DTOs vs core] → Prefer `teshi-core` only if types stay IO-free; otherwise spike-local structs OK.
- [Daemon still defaults to React dist] → Operators must pass GPUI dist; document in tasks / README snippet.
- [Unsafe WASM AppCell keep-alive patterns upstream] → Copy upstream hello_web / prior website bootstrap carefully; keep single-threaded.
- [TUI/engine still read env] → Spike UI may not feed TUI until a follow-up wires the store into `LlmConfig::from_*`; call this out in open questions.

## Migration Plan

1. Land workspace GPUI pin + empty `teshi-ui` + dual entries showing a static label.
2. Add LLM config view + native save/load.
3. Add daemon routes + WasmBackend fetch.
4. Build WASM → point `teshi daemon` / `teshi web --dist` at it → manual verify.
5. Later changes: publish under Hugo `/app`, migrate panels, retire React, unify credential stores.

Rollback: keep React as default `dist`; GPUI crates can remain unused.

## Open Questions

1. Should saving from the GPUI spike also update/override `TESHI_LLM_*` effective config for `teshi-engine` in the same process (daemon) so Requirements/Agent immediately pick it up? **Recommend yes for daemon process** via in-memory override + file; document for follow-up if engine only reads env today.
2. Exact Zed `rev` to pin — pick at implement time from a rev that builds `teshi-web` and desktop on the team’s toolchain.
3. Whether `teshi-client` is a separate crate or a module inside `teshi-ui` for the spike — **prefer module/file in `teshi-ui` + daemon DTOs** until a second consumer appears.
