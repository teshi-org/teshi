## Why

The shared GPUI shell currently mounts `LlmConfigView` as the root window content, which makes LLM configuration look like the product’s primary surface. Configuration is secondary infrastructure and belongs in a settings area so the main shell can evolve into real product UI (left empty for now).

## What Changes

- Introduce a shared GPUI root shell with a primary (main) surface and a settings surface.
- Move the existing LLM configuration UI (base URL, model, API key, save/status) into the settings surface so it is reachable from settings, not as the default root.
- Leave the main surface intentionally empty (placeholder / blank) until product panels are added later.
- Keep existing LLM persistence contracts unchanged: daemon HTTP API, WASM same-origin fetch backend, and native shared store semantics.
- Update `teshi-desktop` and `teshi-web` entry points to mount the new root shell instead of `LlmConfigView` directly.
- **BREAKING** (UI only): users who previously landed on LLM config as the home screen must open Settings to edit LLM config; no API or store format break.

## Capabilities

### New Capabilities

- `gpui-settings`: Settings navigation and host surface inside the shared GPUI shell, including entry from the shell and an LLM settings section that embeds the existing config form.

### Modified Capabilities

- `gpui-shell`: Root view is no longer the LLM form alone; dual entry points present a shell with an empty main surface plus access to settings.
- `gpui-llm-config`: LLM configuration UI requirement relocates from “primary GPUI shell surface” to “settings-hosted section”; load/save/masking and backend API requirements stay.

## Impact

- `crates/teshi-ui`: new root shell / settings views; `LlmConfigView` becomes a child of settings rather than the app root.
- `apps/teshi-desktop`, `apps/teshi-web`: open window / WASM bootstrap mounts the new root view and still injects `LlmConfigBackend`.
- Specs: `openspec/specs/gpui-shell`, `openspec/specs/gpui-llm-config`, plus new `gpui-settings`.
- Untouched: `teshi-daemon` LLM HTTP endpoints, engine store APIs, and backend trait field semantics (unless wiring adjustments for the new root require them).
