## Why

The current codebase has accumulated structural debt: a 7,412-line god object (`src/app.rs`), duplicated Gherkin/LLM modules across root and crates, a confused `desktop/` directory that mixes TypeScript frontend with two Rust shells, a "kitchen-sink" `teshi-runtime` crate that bundles everything from PTY management to LLM transport, and an agent module that directly couples to TUI-specific types. This makes the codebase hard to navigate, test in isolation, and extend with new frontends (GPUI). A clear layered architecture is needed before the GPUI migration proceeds further.

## What Changes

- **Rename and refine crates**: `teshi-gherkin` → `teshi-core` (pure domain), `teshi-runtime` → `teshi-engine` (effectful orchestration)
- **Extract `teshi-agent`**: Pull agent tool definitions, approval pipeline, and validation out of the TUI crate into a standalone library crate with clean ports (no dependency on TUI or engine)
- **Extract `teshi-tui`**: Move the root crate's TUI application (ratatui/crossterm) into `crates/teshi-tui`; a thin `apps/teshi-cli` composition package owns the `teshi` binary and daemon routing
- **Reorganise `desktop/` → `apps/`**: Tauri shell (`desktop/src-tauri/`) → `apps/teshi-tauri/` (with TypeScript frontend at `frontend/`), GPUI shell (`desktop/src-gpui/`) → `apps/teshi-desktop/`
- **Move daemon and sidecar to `apps/`**: `crates/teshi-daemon` → `apps/teshi-daemon`, `crates/teshi-terminal-sidecar` → `apps/teshi-terminal-sidecar`, both as thin binaries over `teshi-engine`
- **Eliminate duplication**: Remove root `src/gherkin.rs`, `src/gherkin_lang.rs`, `src/highlight.rs` (defer to `teshi-core`); consolidate two `llm.rs` implementations into a single engine transport
- **Introduce `AgentHost` trait**: Decouple agent from TUI `App` via a port interface so `teshi-agent` only depends on `teshi-core`
- **Centralise workspace dependency versions** with `[workspace.dependencies]`

## Capabilities

### New Capabilities
- `module-boundaries`: Defines the new crate and application-layer boundary contracts — which types live where, dependency direction rules, and the port interfaces between layers.

### Modified Capabilities
<!-- No existing spec-level behavior changes. This is a pure internal restructuring. -->

## Impact

- **Every Rust source file** — imports will change as modules move between crates
- **Workspace `Cargo.toml`** — new members, dependency aliases, version centralisation
- **`desktop/src-tauri/tauri.conf.json`** — frontend path updates
- **Build scripts (WiX, CI, MSI)** — binary and resource paths change
- **`runner/Cargo.toml`** — may be brought into workspace later
- **TypeScript frontend** — `desktop/src/` moves to `apps/teshi-tauri/frontend/`
