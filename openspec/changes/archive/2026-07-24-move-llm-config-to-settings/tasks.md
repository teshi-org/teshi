## 1. Shell structure

- [x] 1.1 Add a root `AppShell` (or equivalent) view in `teshi-ui` with navigation state `Main` | `Settings`
- [x] 1.2 Implement an empty/placeholder main surface as the default landing content
- [x] 1.3 Add a settings host surface with open/back (or close) navigation from the shell chrome
- [x] 1.4 Export the root shell from `teshi-ui` and keep `LlmConfigBackend` injection at the shell boundary

## 2. Move LLM config into settings

- [x] 2.1 Embed existing `LlmConfigView` (or shared form child) under the settings surface as the LLM section
- [x] 2.2 Ensure main surface does not show base URL / model / API key editors as primary content
- [x] 2.3 Verify focus and keybindings (`Tab` / `Enter` / `Backspace`) still work when the form is nested under settings

## 3. Entry points

- [x] 3.1 Update `teshi-desktop` to mount the new root shell instead of `LlmConfigView` directly
- [x] 3.2 Update `teshi-web` WASM bootstrap to mount the same root shell with the existing WASM backend
- [x] 3.3 Confirm `bind_llm_config_keys` (or successor) is still registered once at app startup

## 4. Verification

- [ ] 4.1 Smoke-test desktop: launch → empty main → Settings → load/save LLM → masked key behavior
- [ ] 4.2 Smoke-test web (daemon-hosted if available): same navigation and save round-trip via HTTP API
- [x] 4.3 Run `cargo fmt --all`, `cargo check` for affected crates, and fix any clippy issues introduced by the UI move
