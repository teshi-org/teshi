## ADDED Requirements

### Requirement: Layered crate dependency direction

The project SHALL enforce the following dependency direction for internal crates:

```
shells (apps/) → teshi-engine → teshi-core
shells (apps/) → teshi-agent → teshi-core
teshi-agent → teshi-core
```

The `apps/teshi-cli` composition shell MAY depend on both the `teshi-tui`
library and `teshi-daemon` application in order to route the stable `teshi`
command surface. Neither dependency may point back to `teshi-cli`.

No crate in a lower layer SHALL depend on a crate in a higher layer.

#### Scenario: Core has no internal dependencies
- **WHEN** `cargo tree -p teshi-core --depth 1` is run
- **THEN** no `teshi-*` workspace crates appear in the dependency tree (only external crates)

#### Scenario: Agent depends only on core
- **WHEN** `cargo tree -p teshi-agent --depth 1` is run
- **THEN** `teshi-core` appears but `teshi-engine`, `teshi-tui`, and any app crate do NOT appear

#### Scenario: Engine depends only on core
- **WHEN** `cargo tree -p teshi-engine --depth 1` is run
- **THEN** `teshi-core` appears but `teshi-agent`, `teshi-tui`, and any app crate do NOT appear

#### Scenario: Shells depend on engine
- **WHEN** `cargo tree -p teshi-tui --depth 1` is run
- **THEN** `teshi-engine` appears; `teshi-core` or `teshi-agent` may appear but no other shell crate does

### Requirement: `teshi-core` — pure domain, no I/O

The `teshi-core` crate SHALL contain only deterministic, side-effect-free types and functions. It MUST NOT depend on: `tokio`, `reqwest`, `notify`, `portable-pty`, `tungstenite`, `dirs`, `dunce`, `fd-lock`, or any filesystem/network operation.

#### Scenario: Core has no async runtime dependency
- **WHEN** `cargo tree -p teshi-core` is run
- **THEN** `tokio` does NOT appear anywhere in the tree

#### Scenario: Core has no filesystem watcher dependency
- **WHEN** `cargo tree -p teshi-core` is run
- **THEN** `notify` does NOT appear anywhere in the tree

#### Scenario: Core has no PTY dependency
- **WHEN** `cargo tree -p teshi-core` is run
- **THEN** `portable-pty` does NOT appear anywhere in the tree

### Requirement: `teshi-core` owns all Gherkin concepts

The `teshi-core` crate SHALL be the single source of truth for Gherkin parsing, language keywords, syntax highlighting, render payloads, and step indexing. No other crate SHALL contain a duplicate Gherkin implementation.

#### Scenario: No Gherkin duplication in root
- **WHEN** the target structure is in place
- **THEN** `src/gherkin.rs`, `src/gherkin_lang.rs`, and `src/highlight.rs` do NOT exist (or are thin re-exports of `teshi_core`)

#### Scenario: No Gherkin duplication in engine
- **WHEN** `crates/teshi-engine/src/gherkin.rs` exists
- **THEN** it contains only I/O functions (read, watch, emit) and calls into `teshi_core` for parsing and rendering

### Requirement: `teshi-agent` — no UI or engine dependency

The `teshi-agent` crate SHALL depend only on `teshi-core`. It MUST NOT import from `teshi-engine`, `teshi-tui`, `ratatui`, `crossterm`, `tauri`, `gpui`, or any crate providing a `crate::app::App` type.

#### Scenario: Agent has no App import
- **WHEN** `rg "crate::app::App" crates/teshi-agent/src/` is run
- **THEN** zero matches are found

#### Scenario: Agent defines AgentHost trait
- **WHEN** the `teshi-agent` crate is compiled
- **THEN** it exposes a public `AgentHost` trait (or equivalent port interface) that callers implement to provide project, filesystem, browser, and test-runner capabilities

### Requirement: `teshi-engine` — no UI dependency

The `teshi-engine` crate SHALL contain all effectful runtime logic (project lifecycle, terminal PTY, browser sidecar, locator persistence, file watching, LLM transport, event bus, daemon manifest utilities). It MUST NOT import from any UI or application-shell crate (`teshi-tui`, `teshi-tauri`, `teshi-desktop`, `teshi-daemon`, `teshi-terminal-sidecar`).

#### Scenario: Engine has no TUI dependency
- **WHEN** `cargo tree -p teshi-engine` is run
- **THEN** `ratatui` and `crossterm` do NOT appear in the tree

#### Scenario: Engine has no Tauri or GPUI dependency
- **WHEN** `cargo tree -p teshi-engine` is run
- **THEN** `tauri` and `gpui` do NOT appear in the tree

### Requirement: Daemon and terminal-sidecar are standalone apps

The daemon (`teshi-daemon`) and terminal sidecar (`teshi-terminal-sidecar`) SHALL be independent binary crates under `apps/`. They SHALL depend on `teshi-engine` but engine MUST NOT depend on them.

#### Scenario: Daemon is in apps/
- **WHEN** the target directory structure is checked
- **THEN** `apps/teshi-daemon/Cargo.toml` exists, `crates/teshi-daemon/` does NOT exist

#### Scenario: Sidecar is in apps/
- **WHEN** the target directory structure is checked
- **THEN** `apps/teshi-terminal-sidecar/Cargo.toml` exists, `crates/teshi-terminal-sidecar/` does NOT exist

### Requirement: TypeScript frontend under Tauri app

The TypeScript/React frontend (currently `desktop/src/`) SHALL reside at `apps/teshi-tauri/frontend/`. The Tauri configuration SHALL reference it correctly for both development and production builds.

#### Scenario: Frontend directory exists under Tauri
- **WHEN** the target directory structure is checked
- **THEN** `apps/teshi-tauri/frontend/package.json` exists

#### Scenario: Tauri config references correct frontend path
- **WHEN** `apps/teshi-tauri/tauri.conf.json` is read
- **THEN** the `frontendDist` and `devUrl` fields point to `frontend/` (not `../src/`)

### Requirement: Workspace dependency centralisation

The root `Cargo.toml` SHALL define common dependencies in `[workspace.dependencies]` so that member crates use `dep.workspace = true` for shared versions.

#### Scenario: Workspace dependencies section exists
- **WHEN** the root `Cargo.toml` is read
- **THEN** `[workspace.dependencies]` contains entries for at least: `serde`, `serde_json`, `tokio`, `anyhow`, `tracing`, `teshi-core`, `teshi-agent`, and `teshi-engine`

### Requirement: No duplicate LLM implementations

There SHALL be exactly one LLM HTTP transport implementation in the workspace, owned by `teshi-engine`. The root `src/llm.rs` and any duplicate in other crates SHALL be removed or reduced to thin adapters.

#### Scenario: Only one HTTP transport
- **WHEN** `rg "reqwest::Client" --type rust crates/ src/` is run
- **THEN** `reqwest::Client` appears only in `crates/teshi-engine/` (or `apps/teshi-daemon/` for daemon-specific use), not in multiple independent LLM modules
