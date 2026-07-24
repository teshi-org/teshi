## Why

Teshi is moving the product UI from React/Tauri to shared native GPUI (desktop) and GPUI WASM (`teshi.org/app`), while Hugo continues to own the marketing site. Before rewriting panels, we need a minimal closed loop that proves: one GPUI UI crate builds for both targets, and the WASM shell can talk to `teshi-daemon` over same-origin HTTP when the daemon hosts the app assets.

## What Changes

- Add `crates/teshi-ui` with a shared GPUI root view whose only product feature is **LLM configuration** (base URL, model, API key; load/save/status).
- Add `apps/teshi-web` (GPUI WASM entry) and evolve `apps/teshi-desktop` to launch the same `teshi-ui` root on native GPUI.
- Unify desktop and web on one workspace GPUI git revision (replace crates.io `gpui = "0.2"` on desktop).
- Introduce a thin client/backend boundary so WASM uses browser `fetch` (GPUI `FetchHttpClient`) against existing daemon HTTP, and native can call engine/config helpers directly.
- Extend `teshi-daemon` to:
  - Serve the GPUI WASM static build (Path 1: same-origin `http://127.0.0.1:<port>/`).
  - Expose minimal LLM config HTTP APIs (get/set/status) backed by the same config surface used by the engine/TUI where practical.
- Keep Hugo marketing site (`teshi-org.github.io`) separate; this change does not migrate `/` to GPUI. Public `teshi.org/app` packaging can follow later; this spike validates communication via daemon-hosted assets.
- **Out of scope**: replacing React panels, WebSocket event bus, feature editor, agent chat, browser/terminal, cloud workspaces, inventing a full new protocol crate graph.

## Capabilities

### New Capabilities

- `gpui-shell`: Shared GPUI application shell for native desktop and WASM, including workspace GPUI pinning, dual entry points, and daemon-hosted static serving for same-origin Path 1.
- `gpui-llm-config`: LLM settings UI and persistence/API used by that shell (load current config, save updates, show configured/masked status; no chat UI).

### Modified Capabilities

- `module-boundaries`: Allow new GPUI shell crates (`teshi-ui`, `teshi-web`) and document that GPUI WASM must not depend on `teshi-engine` / `teshi-agent` directly.

## Impact

- **Apps**: `apps/teshi-desktop`, new `apps/teshi-web`, `apps/teshi-daemon` (static serve + LLM config routes).
- **Crates**: new `crates/teshi-ui`; root `Cargo.toml` workspace members and `[workspace.dependencies]` for `gpui` / `gpui_platform`.
- **APIs**: new daemon `/api/v1/...` LLM config endpoints (exact paths in design).
- **Frontend**: React/Tauri remains the current production UI until a later change; this spike does not remove it.
- **Docs/site**: Hugo site unchanged; optional later CI to publish WASM under `/app`.
- **Risk**: GPUI pre-1.0 pinning; WASM HTTP limited to buffered `fetch` (acceptable for config payloads).
