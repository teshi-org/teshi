## Context

The teshi project currently has the following structural problems:

- **Root crate (`src/`) is the monolith**: `src/app.rs` is 7,412 lines, holding all TUI state alongside domain logic. Agent tools in `src/agent/` accept `&mut App` and directly call TUI runner, browser sidecar, and filesystem — making them impossible to reuse across frontends.
- **Duplicated modules**: `src/gherkin.rs`, `src/gherkin_lang.rs`, `src/highlight.rs` duplicate functionality already in `crates/teshi-gherkin`. Two separate `llm.rs` files exist in `src/` and `crates/teshi-runtime/src/`.
- **Kitchen-sink `teshi-runtime`**: Bundles project management, terminal PTY, browser sidecar, locator persistence, LLM transport, file watching, venv probing, daemon manifest, and event bus into one undifferentiated crate.
- **Confused `desktop/` directory**: Mixes TypeScript frontend (`desktop/src/`), Tauri Rust shell (`desktop/src-tauri/`), and GPUI stub (`desktop/src-gpui/`). The Tauri package is confusingly named `teshi-desktop`.
- **No enforced layer discipline**: The dependency graph has no guard rails — UI crates import runtime internals, runtime imports appear at every layer.

The project is transitioning from Tauri to GPUI as the primary desktop shell. Before that migration proceeds, the module boundaries must be clarified so the new GPUI shell can share engine services without copying code or importing TUI internals.

## Goals / Non-Goals

**Goals:**
- Define a clear 4-layer architecture: core → agent → engine → shells
- Eliminate all duplicated Gherkin, highlighting, and LLM modules
- Extract the TUI application into its own crate, leaving the root package as a thin binary or workspace placeholder
- Decouple agent from TUI `App` via a trait-based port interface
- Rehome TypeScript frontend under the Tauri app directory
- Centralise workspace dependency versions
- Ensure every intermediate step compiles (no "big bang" refactor)

**Non-Goals:**
- Refactoring `src/app.rs` internals (deferred to a follow-up change after the crate exists)
- Changing any user-facing behaviour or CLI interface
- Adding new features or capabilities
- Rewriting the GPUI shell beyond its current stub
- Bringing `runner/` into the workspace (blocked on `tests/steps/tui` compatibility)

## Decisions

### 1. Layer architecture: core → agent → engine → shells

```
┌──────────────────────────────────────────────────────────────┐
│                     apps/ (application shells)                │
│  ┌───────────────┐  ┌──────────────┐  ┌───────────────────┐  │
│  │  teshi-cli    │  │ teshi-tauri  │  │  teshi-desktop    │  │
│  │ (CLI + TUI)   │  │  (Tauri+Web) │  │  (GPUI native)    │  │
│  └───────┬───────┘  └──────┬───────┘  └────────┬──────────┘  │
│          │                 │                    │             │
│          └─────────────────┼────────────────────┘             │
│                            │                                  │
│  ┌─────────────────────────┼──────────────────────────────┐  │
│  │              crates/    │    (library layer)            │  │
│  │                         ▼                               │  │
│  │  ┌──────────────────┐  ┌──────────────────┐            │  │
│  │  │  teshi-engine    │  │  teshi-agent     │            │  │
│  │  │  (effects/I/O)   │  │  (policy/tools)  │            │  │
│  │  └────────┬─────────┘  └────────┬─────────┘            │  │
│  │           │                     │                       │  │
│  │           └──────────┬──────────┘                       │  │
│  │                      ▼                                  │  │
│  │           ┌──────────────────┐                          │  │
│  │           │   teshi-core     │                          │  │
│  │           │  (pure domain)   │                          │  │
│  │           └──────────────────┘                          │  │
│  └─────────────────────────────────────────────────────────┘  │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐ │
│  │              apps/ (thin transport binaries)              │ │
│  │  ┌──────────────────┐  ┌──────────────────────────────┐  │ │
│  │  │  teshi-daemon    │  │ teshi-terminal-sidecar       │  │ │
│  │  │  (HTTP/WS)       │  │ (standalone terminal relay)  │  │ │
│  │  └──────────────────┘  └──────────────────────────────┘  │ │
│  └──────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

**Rationale**: Each layer has a single responsibility and clear dependency direction (downward only). Shells depend on engine and optionally agent; agent depends only on core; engine depends only on core. No upward or sideways dependencies.

The distributable `teshi` binary is owned by a thin `apps/teshi-cli`
composition package. It depends on the `teshi-tui` library and
`teshi-daemon`, preserving the hidden daemon fork and `teshi web` routing
without creating a forbidden `teshi-tui` → `teshi-daemon` dependency.

**Alternatives considered**:
- *3-layer (merge agent into engine)*: Rejected because agent policies (approval, validation, tool definitions) should be testable without PTYs, WebSockets, or filesystem access.
- *Flat crates (no layers)*: Rejected because it provides no guidance on where new code belongs and encourages spaghetti dependencies.

### 2. Daemon and terminal-sidecar as independent thin binaries

Both `crates/teshi-daemon` and `crates/teshi-terminal-sidecar` move to `apps/` as independent packages that depend on `teshi-engine`. They are NOT merged into engine.

**Rationale**: Their transport dependencies (`axum`, `tower-http`, `tokio-tungstenite`) are not needed by TUI, Tauri, or GPUI shells. Merging them into engine would recreate the kitchen-sink problem. As independent crates they can be packaged, versioned, and tested separately.

**Alternatives considered**:
- *Multiple `[[bin]]` targets in engine behind features*: Rejected because it weakens dependency boundaries and complicates feature-flag management.
- *Merge sidecar into daemon*: Rejected because the terminal sidecar has a completely independent lifecycle and protocol.

### 3. `AgentHost` trait for agent-shell decoupling

`teshi-agent` will NOT depend on `teshi-engine` or any UI crate. Instead, it defines a port trait:

```rust
pub trait AgentHost {
    fn project_snapshot(&self) -> Result<ProjectSnapshot>;
    fn read_feature(&self, path: &ProjectPath) -> Result<String>;
    fn propose_change(&mut self, change: ProposedChange) -> Result<ProposalId>;
    fn run_tests(&mut self, request: RunRequest) -> Result<RunId>;
    fn browser_command(&mut self, command: BrowserCommand) -> Result<BrowserResult>;
}
```

Each shell (TUI, Tauri, GPUI) implements `AgentHost` by delegating to `teshi-engine` services.

**Rationale**: The current `execute_tool(&mut App, ...)` signature is the root cause of agent-TUI coupling. A trait port lets the agent define *what* it needs without knowing *who* provides it.

**Alternatives considered**:
- *Agent depends on engine directly*: Rejected because it forces engine types into agent and makes agent harder to test.
- *Put `AgentHost` in engine*: Rejected because agent would then depend on engine, violating the dependency direction.

### 4. `src/gherkin.rs` etc. → re-export then delete

Root duplicate modules (`src/gherkin.rs`, `src/gherkin_lang.rs`, `src/highlight.rs`) will be replaced by compatibility re-exports:

```rust
// Temporary src/gherkin.rs
pub use teshi_core::gherkin::*;
```

Then call sites migrate to import directly from `teshi_core`, and the compatibility modules are deleted.

**Rationale**: This is the lowest-risk approach — the re-exports keep existing code compiling while imports are migrated incrementally.

### 5. Module assignment: core vs engine

| Current module (teshi-runtime) | → teshi-core | → teshi-engine |
|---|---|---|
| `gherkin` | None (already in teshi-gherkin) | IO: read files, watch, emit events |
| `locator` | DTOs, types, normalisation, matching | Persistence, watchers, sidecar commands |
| `project` | Platform-neutral snapshot | `ProjectState`, directory ops |
| `project_settings` | DTOs, defaults, validation | Load/save |
| `screen` | Cell/Color/Grid (only if multi-consumer) | PTY ownership, event forwarding |
| `venv` | Parsing, error classification | Filesystem probing, commands |
| `daemon` | `DaemonManifest` data shape | I/O, TCP checks, spawning |
| `events` | Serializable event payloads | Tokio broadcast bus |
| `fs_util` | Nothing | All |
| `sidecar` | Command/response data contracts | WebSocket, process lifecycle |
| `terminal` | Command/event contracts | PTY spawn/resize/write/shutdown |
| `watcher` | Nothing | All |
| `llm` | Message/tool-call DTOs | HTTP transport |
| `app_data` | Geometry validation | Directory discovery, persistence |

### 6. Migration order: phased, every step compiles

The 11-phase plan from the architectural analysis is adopted. Key principle: **each commit changes ownership OR behaviour, never both**. Temporary re-exports, package aliases, and type aliases are used liberally.

See `tasks.md` for the detailed phase breakdown.

## Risks / Trade-offs

- **[R] Large diff, easy to miss an import** → Each phase runs `cargo check --workspace` before proceeding. CI must pass on every commit.
- **[R] `src/app.rs` (7412 lines) touches everything** → Deferred: the crate move happens first with `app.rs` unchanged; internal refactoring is a follow-up.
- **[R] Breaking external consumers of `teshi-gherkin`** → Provide a thin compatibility crate `crates/teshi-gherkin` that re-exports `teshi_core::*` until all consumers migrate. Delete after Phase 11.
- **[R] Tauri config paths break when frontend moves** → Update `tauri.conf.json` `frontendDist` and dev URL in the same commit as the directory move.
- **[R] `--daemon-internal` fork path breaks when daemon moves** → Route the hidden flag and `teshi web` through `apps/teshi-cli`, the composition root shared by the TUI and daemon.
- **[R] `runner/` has a nested `[workspace]` and depends on `tests/steps/tui`** → Keep `runner/` excluded from the workspace initially. Bring it in during Phase 10 only after resolving the test-step dependency.

## Open Questions

1. **Should `teshi-core` absorb `teshi-gherkin` by renaming, or should `teshi-core` be new with `teshi-gherkin` as a compatibility crate?** Codex recommended a temporary compatibility crate. The implementation will start with renaming `teshi-gherkin` → `teshi-core` and adding missing modules from root/src — simpler and fewer moving parts.

2. **Where do root `src/config/` and `src/profiles/` go?** These are currently TUI-specific (model profiles, provider configs). They should stay in `teshi-tui` for now. If any types become shared across shells, they can move to core later.

3. **Does the TypeScript frontend stay in `apps/teshi-tauri/frontend/` even though the daemon also serves it?** Yes — the daemon consumes the built artifact from the Tauri app. If the web frontend becomes independently deployed, it can be promoted to its own `apps/teshi-web/` later.
