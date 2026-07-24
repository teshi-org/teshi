## 1. Workspace GPUI pin and crate skeleton

- [x] 1.1 Add `gpui` and `gpui_platform` to root `[workspace.dependencies]` pinned to one Zed git revision that supports `single_threaded_web` and `FetchHttpClient`
- [x] 1.2 Create `crates/teshi-ui` package (library) with empty/root placeholder view; no `teshi-engine` / `teshi-agent` deps
- [x] 1.3 Create `apps/teshi-web` (`cdylib` + `rlib`) with WASM `run` entry using `gpui_platform` web init; depend on `teshi-ui`
- [x] 1.4 Switch `apps/teshi-desktop` to `gpui.workspace = true`, depend on `teshi-ui`, remove direct `teshi-engine`/`teshi-agent` deps if unused
- [x] 1.5 Register `crates/teshi-ui` and `apps/teshi-web` in workspace `members`
- [x] 1.6 Verify `cargo check -p teshi-desktop` and `cargo check -p teshi-ui`; document WASM build command for `teshi-web`

## 2. Shared LLM config UI

- [x] 2.1 Define backend trait in `teshi-ui` (`get_llm_config` / `set_llm_config`) and view DTOs (`base_url`, `model`, masked key / `api_key_configured`)
- [x] 2.2 Implement `LlmConfigView` (inputs + Save + status) as the `teshi-ui` root content
- [x] 2.3 Wire desktop and web entries to show `LlmConfigView` with injectable backends

## 3. Persistence and daemon API

- [x] 3.1 Implement user-level `llm-config.json` store helper (shared path under Teshi app data) readable/writable from daemon and native code
- [x] 3.2 Add `GET /api/v1/llm/config` and `PUT /api/v1/llm/config` to `teshi-daemon` (masked key on GET; persist on PUT; do not log raw key)
- [x] 3.3 Implement NativeBackend against the same store for `teshi-desktop`
- [x] 3.4 Optionally apply saved config as in-process override for daemon-side engine LLM calls when present (if low-cost; otherwise document follow-up)

## 4. WASM HTTP backend and Path 1 serve

- [x] 4.1 Implement WasmBackend using browser fetch / `FetchHttpClient` against `/api/v1/llm/config`
- [x] 4.2 Add web static build pipeline for `teshi-web` (wasm-bindgen + minimal HTML/JS loader → `dist/`)
- [x] 4.3 Confirm daemon `--dist` can point at that `dist/`; document the Path 1 run command (keep React as default resolver)
- [x] 4.4 Manual acceptance: start daemon with GPUI dist, open UI, save LLM config, reload and confirm masked/configured status via UI and `GET`

## 5. Verification and docs touch-up

- [x] 5.1 Run `cargo fmt` / `cargo check` for touched packages; fix clippy issues introduced by the spike
- [x] 5.2 Add a short English note in `doc/` or README snippet: Path 1 spike commands, out-of-scope (React still production UI, Hugo unchanged)
- [x] 5.3 Confirm `cargo tree -p teshi-ui` and `cargo tree -p teshi-web` exclude `teshi-engine` / `teshi-agent`
