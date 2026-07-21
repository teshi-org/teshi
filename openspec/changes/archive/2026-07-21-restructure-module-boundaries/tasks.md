## 1. Baseline

- [x] 1.1 Run `cargo check --workspace && cargo test --workspace` and record any pre-existing failures
- [x] 1.2 Run `cargo tree --depth 1` for each workspace member and save the output for comparison
- [x] 1.3 Identify all public APIs used between crates (root → runtime, daemon → runtime, tauri → runtime, sidecar → runtime)

## 2. Introduce `teshi-core` (absorb `teshi-gherkin`)

- [x] 2.1 Rename `crates/teshi-gherkin` → `crates/teshi-core`; update `Cargo.toml` package name to `teshi-core`
- [x] 2.2 Create a compatibility crate `crates/teshi-gherkin` that re-exports `pub use teshi_core::*` (for external consumers)
- [x] 2.3 Update root `Cargo.toml`: rename dependency `teshi-gherkin` → `teshi-core`, add `teshi-core` to workspace members
- [x] 2.4 Update all `use teshi_gherkin::*` imports across the workspace to `use teshi_core::*` (or keep the alias: `teshi-gherkin = { package = "teshi-core", path = "..." }`)
- [x] 2.5 Move pure domain modules from root into `teshi-core`: `mindmap` types (tree, filter, highlight rules), `diff` algorithms, `markdown` algorithms
- [x] 2.6 Verify: `cargo check --workspace && cargo test --workspace`

## 3. Eliminate root Gherkin duplication

- [x] 3.1 Compare `src/gherkin.rs` / `src/gherkin_lang.rs` / `src/highlight.rs` public APIs with `teshi-core` equivalents; add any missing items to core
- [x] 3.2 Replace `src/gherkin.rs` with `pub use teshi_core::gherkin::*` (compatibility re-export)
- [x] 3.3 Replace `src/gherkin_lang.rs` with `pub use teshi_core::gherkin_lang::*`
- [x] 3.4 Replace `src/highlight.rs` with `pub use teshi_core::highlight::*`
- [x] 3.5 Migrate call sites one module at a time to import from `teshi_core` directly
- [x] 3.6 Delete the three compatibility re-export modules after all references are migrated
- [x] 3.7 Verify: `cargo check --workspace && cargo test --workspace`

## 4. Rename `teshi-runtime` → `teshi-engine` (mechanical only)

- [x] 4.1 Rename directory `crates/teshi-runtime` → `crates/teshi-engine`; update its `Cargo.toml` package name to `teshi-engine`
- [x] 4.2 Update root `Cargo.toml` workspace member path and dependency alias: `teshi-runtime = { package = "teshi-engine", path = "crates/teshi-engine" }`
- [x] 4.3 Update `crates/teshi-daemon/Cargo.toml`: change `teshi-runtime` → `teshi-engine`
- [x] 4.4 Update `crates/teshi-terminal-sidecar/Cargo.toml`: change `teshi-runtime` → `teshi-engine`
- [x] 4.5 Update `desktop/src-tauri/Cargo.toml`: change `teshi-runtime` → `teshi-engine`
- [x] 4.6 Update root `src/` and `desktop/src-tauri/src/` imports: `teshi_runtime::` → `teshi_engine::`
- [x] 4.7 Add `pub type TeshiRuntime = TeshiEngine;` alias in engine's `lib.rs` (temporary)
- [x] 4.8 Verify: `cargo check --workspace && cargo test --workspace`

## 5. Split pure contents from engine into core

- [x] 5.1 Move locator DTOs and normalisation (`ActiveStep`, `FeatureStepRef`, `StepBinding`, `StepBindingStatus`, `LocatorCandidate`, `PendingLocator` types + `normalize_step_text`, `resolve_step_context` pure functions) → `teshi-core::locator`
- [x] 5.2 Move project settings DTOs (`ProjectSettings`, `DEFAULT_LOCATOR_AUTO_CONFIRM_SEC`) → `teshi-core::project_settings`
- [x] 5.3 Move `DaemonManifest` data shape → `teshi-core::daemon` (serialisation only, no I/O)
- [x] 5.4 Move event payload types (serialisable `RuntimeEvent` variants) → `teshi-core::events`
- [x] 5.5 Move sidecar/terminal command DTOs (serialisable JSON contracts) → `teshi-core::sidecar`, `teshi-core::terminal` (skip — tightly coupled, not worth extracting)
- [x] 5.6 Move LLM DTOs (`ChatMessage`, `ToolCall`, `ToolDefinition`, completion request/response types) → `teshi-core::llm`
- [x] 5.7 Move venv parsing (`parse_pyvenv_cfg`, error classification helpers) → `teshi-core::venv`
- [x] 5.8 Add temporary re-exports from `teshi-engine` for each moved module; update consumers incrementally
- [x] 5.9 Verify: `cargo check --workspace && cargo test --workspace`

## 6. Consolidate LLM transport

- [x] 6.1 Define shared LLM completion/tool DTOs in `teshi-core::llm` (if not already done in 5.6)
- [x] 6.2 Implement a single async HTTP/SSE client in `teshi-engine::llm` that replaces both old implementations
- [x] 6.3 Create a compatibility wrapper in `teshi-tui` that translates the new engine async API into the old `LlmHandle`/`LlmEvent` channel-based API (so `App` doesn't need to change yet)
- [x] 6.4 Switch root `src/llm.rs` to use the wrapper; delete its own HTTP logic
- [x] 6.5 Delete the old `crates/teshi-runtime/src/llm.rs` (now `crates/teshi-engine/src/llm.rs`) after its replacement is stable
- [x] 6.6 Verify: `cargo check --workspace && cargo test --workspace`

## 7. Extract `teshi-agent`

- [x] 7.1 Create `crates/teshi-agent/Cargo.toml` with dependencies only on `teshi-core`, `serde`, `serde_json`, `serde_yaml`, `anyhow`
- [x] 7.2 Move agent pure modules first: `approval.rs`, `definition.rs`, `pipeline.rs`, `validator.rs`, tool schemas, registry data structures
- [x] 7.3 Add re-exports from root `src/agent/mod.rs` → `teshi_agent::*` to keep existing code compiling
- [x] 7.4 Define `AgentHost` trait and `ProposedChange` enum in `teshi-agent`
- [x] 7.5 Implement `AgentHost` as a thin adapter in `teshi-tui` that delegates to `teshi-engine` services
- [x] 7.6 Refactor tool dispatch (`execute_tool`) to accept `&mut dyn AgentHost` instead of `&mut App`
- [x] 7.7 Move tool definitions, loader, and registry to `teshi-agent`
- [x] 7.8 Delete root `src/agent/` compatibility shims after all references use `teshi_agent::`
- [x] 7.9 Verify: `cargo check --workspace && cargo test --workspace`

## 8. Extract `teshi-tui`

- [x] 8.1 Create `crates/teshi-tui/Cargo.toml` with TUI-specific deps (`ratatui`, `crossterm`, `clap`, `ropey`, `tui-tree-widget`, `inquire`, `arboard`, `dirs`, `toml`) and workspace deps (`teshi-core`, `teshi-agent`, `teshi-engine`)
- [x] 8.2 Move `src/` modules to `crates/teshi-tui/src/`: `app.rs`, `ui.rs`, `keymap.rs`, `editor_buffer.rs`, `engine.rs`, `session.rs`, `runner.rs`, `bdd_nav.rs`, `config/`, `profiles/`, `cli/`, `auth/`, `main.rs`
- [x] 8.3 Update root `Cargo.toml`: change `[package]` to a virtual workspace manifest (remove all `[dependencies]`) or keep as a thin binary that just calls `teshi_tui::main()`
- [x] 8.4 Preserve binary name `teshi` in the thin `apps/teshi-cli` composition package and expose `teshi-tui` as a library
- [x] 8.5 Fix all `crate::` imports to use the new crate structure
- [x] 8.6 Verify: `cargo check --workspace && cargo test --workspace`

## 9. Thin daemon and sidecar, move to `apps/`

- [x] 9.1 Create `apps/teshi-daemon/`; move `crates/teshi-daemon/src/` and `Cargo.toml` there
- [x] 9.2 Update daemon to use `teshi-engine` service APIs instead of direct `TeshiRuntime` field access where possible
- [x] 9.3 Move reusable daemon manifest/spawn utilities from daemon into `teshi-engine` (if not already there)
- [x] 9.4 Create `apps/teshi-terminal-sidecar/`; move `crates/teshi-terminal-sidecar/` there
- [x] 9.5 Update sidecar to use `teshi-engine` terminal service APIs
- [x] 9.6 Update root `Cargo.toml` workspace members to point to `apps/` paths
- [x] 9.7 Delete old `crates/teshi-daemon/` and `crates/teshi-terminal-sidecar/` directories
- [x] 9.8 Verify: `cargo check --workspace && cargo test --workspace`

## 10. Move desktop applications to `apps/`

- [x] 10.1 Create `apps/teshi-tauri/`; move `desktop/src-tauri/` contents (Rust source, Cargo.toml, build.rs, tauri.conf.json, capabilities/, icons/, resources/)
- [x] 10.2 Move TypeScript frontend: `desktop/src/` → `apps/teshi-tauri/frontend/src/`; also move `package.json`, `vite.config.ts`, `tsconfig.json`, `index.html`
- [x] 10.3 Update `apps/teshi-tauri/tauri.conf.json`: set `frontendDist` to `frontend/dist`, `devUrl` to `http://localhost:5173` (or whatever Vite dev server uses)
- [x] 10.4 Update any build scripts, CI config, MSI/WiX paths referencing `desktop/src-tauri/` or `desktop/src/`
- [x] 10.5 Rename Tauri package from `teshi-desktop` to `teshi-tauri` in its `Cargo.toml` (the old name was confusing — the GPUI one is the real desktop)
- [x] 10.6 Create `apps/teshi-desktop/`; move `desktop/src-gpui/` contents there; rename package to `teshi-desktop`
- [x] 10.7 Update root `Cargo.toml` workspace members: remove `desktop/src-tauri`, `desktop/src-gpui`; add `apps/teshi-tauri`, `apps/teshi-desktop`
- [x] 10.8 Delete old `desktop/` directory
- [x] 10.9 Verify: `cargo check --workspace && cargo test --workspace`

## 11. Workspace polish and remove compatibility layers

- [x] 11.1 Add `[workspace.dependencies]` to root `Cargo.toml` with common external deps (`serde`, `serde_json`, `tokio`, `anyhow`, `tracing`, `base64`, `chrono`, `reqwest`, `clap`)
- [x] 11.2 Update each member crate to use `dep.workspace = true` for shared dependency versions
- [x] 11.3 Delete the `crates/teshi-gherkin` compatibility crate (all consumers now use `teshi-core` directly)
- [x] 11.4 Remove the `teshi-runtime = { package = "teshi-engine" }` alias from all Cargo.toml files (all uses are now `teshi-engine` directly)
- [x] 11.5 Remove the `pub type TeshiRuntime = TeshiEngine;` alias from engine
- [x] 11.6 Delete any remaining root `src/` compatibility re-exports
- [x] 11.7 Run `cargo tree` to verify dependency direction:
  - `teshi-core` has no internal deps
  - `teshi-agent` → only `teshi-core`
  - `teshi-engine` → only `teshi-core`
  - `teshi-tui` → `teshi-engine` (and optionally `teshi-agent`, `teshi-core`)
  - `teshi-tauri` → `teshi-engine` (and optionally `teshi-core`)
  - `teshi-desktop` → `teshi-engine` (and optionally `teshi-agent`, `teshi-core`)
  - `teshi-daemon` → `teshi-engine`
  - `teshi-terminal-sidecar` → `teshi-engine`
- [x] 11.8 Final verification: `cargo check --workspace && cargo test --workspace && cargo build --workspace --release`
