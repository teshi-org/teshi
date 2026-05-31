# Development Guide

## Prerequisites

- Rust toolchain (stable, via [rustup](https://rustup.rs/))
- A terminal with Unicode and true-color support
- Git hooks enabled (after cloning: `git config core.hooksPath .githooks`)

## Building

```bash
cargo build              # debug build
cargo build --release    # optimized build
cargo run -- [args]      # build + run with arguments
```

## Testing

```bash
cargo test               # run all tests
cargo test -- --nocapture  # show test output
cargo clippy             # lint
cargo fmt -- --check     # check formatting
```

### Test layout

- Unit tests live at the bottom of each source file (in `#[cfg(test)]` modules)
- Integration tests: `tests/feature/*.feature` — BDD scenarios that describe expected behavior
- Key test areas:
  - Gherkin parsing edge cases (`gherkin.rs`)
  - Editor buffer operations (`editor_buffer.rs`)
  - Step navigation and keyword replacement (`bdd_nav.rs`)
  - App integration (navigation, editing, focus, undo, copy/paste) (`app.rs`)

## Project structure

```
src/
├── main.rs              # Entry point, event loop setup
├── app.rs               # Core orchestrator (~4300 lines)
├── ui.rs                # TUI rendering (~1856 lines)
├── gherkin.rs           # Gherkin parser (hand-written)
├── editor_buffer.rs     # ropey::Rope wrapper
├── mindmap.rs           # Prefix trie for tree navigation
├── bdd_nav.rs           # Structured BDD editing operations
├── runner.rs            # Subprocess NDJSON test runner
├── llm.rs               # SSE streaming LLM client
├── highlight.rs         # Gherkin syntax highlighting
├── markdown.rs          # Markdown to ratatui Spans
├── keymap.rs            # KeyEvent → Action dispatch
├── step_index.rs        # Normalized step deduplication
├── gherkin_keywords.rs  # Shared keyword constants
├── agent/
│   ├── mod.rs           # Tool dispatch, agent loop control
│   └── tools.rs         # LLM tool implementations
├── auth/
│   ├── mod.rs           # Module declarations
│   └── manager.rs       # Credential storage in auth.json
├── cli/
│   ├── mod.rs           # Clap CLI definitions
│   └── auth.rs          # Auth subcommands
└── config/
    ├── mod.rs           # Config loading and resolution
    └── types.rs         # Config structs
```

## Key dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `ratatui` | 0.29 | Terminal UI framework |
| `crossterm` | 0.29 | Terminal raw-mode control |
| `tui-tree-widget` | 0.23 | Tree component for mind map |
| `ropey` | 1.6 | Efficient text buffer (rope data structure) |
| `clap` | 4.x | CLI argument parsing |
| `tokio` | 1.x | Async runtime (LLM, runner threads) |
| `reqwest` | 0.12 | HTTP client for LLM APIs |
| `serde` / `serde_json` | 1.x | Serialization (runner protocol, LLM) |
| `toml` | 0.8 | Config file parsing |

## Code conventions

- No doc comments on private items unless the logic is non-obvious
- No comments that restate the code
- Tests live at the bottom of source files
- Functions that mutate the project AST should also rebuild derived indexes (`step_index`, `mindmap_index`)
- Editor mutations should call `push_undo()` before modifying the buffer
- Use the `Action` enum for all user input — never call application logic directly from `keymap.rs`

## Adding a new LLM tool

1. Define the function in `agent/tools.rs` with a JSON Schema
2. Add the tool name/payload to the tools list sent to the LLM
3. Add a match arm in `agent/mod.rs` `execute_tool()`
4. If it mutates files, return `ToolResult::Queued` with an `AgentPendingChange`
5. Handle the confirmation flow in `app.rs` `accept_agent_change()` / `reject_agent_change()`

## Adding a new keybinding

1. Add an `Action` variant if needed
2. Add the mapping in `keymap.rs` `Action::from_key_event()` with appropriate context gating
3. Handle the action in `app.rs` `handle_action()` match
4. Update the help text if it's a user-facing binding

## Release workflow

Full-stack releases publish the CLI, Windows desktop app, and Chrome extension under a single `vX.Y.Z` tag. All component versions (`Cargo.toml`, `desktop/src-tauri`, `extension/teshi-bridge/manifest.json`) must match before tagging.

### Release assets

| Asset | Platform | Contents |
|-------|----------|----------|
| `teshi-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | Linux x64 | `teshi` + README + LICENSE |
| `teshi-vX.Y.Z-aarch64-apple-darwin.tar.gz` | macOS ARM | `teshi` + README + LICENSE |
| `teshi-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Windows x64 | `teshi.exe` + `teshi-desktop.exe` + README + LICENSE |
| `teshi-vX.Y.Z-x64.msi` | Windows x64 | CLI WiX installer (cargo-wix) |
| `teshi-desktop-vX.Y.Z-x64.msi` | Windows x64 | Tauri desktop installer |
| `teshi-bridge-vX.Y.Z.zip` | All | Chrome extension (load unpacked) |
| `SHA256SUMS` | All | Checksums for every archive above |

Workflow: [`.github/workflows/release.yml`](../.github/workflows/release.yml)

WinGet submission (CLI MSI only) runs automatically when `WINGET_TOKEN` is configured. Legacy standalone workflow: [`.github/workflows/winget.yml`](../.github/workflows/winget.yml).

Windows installer sources: `wix/` (CLI MSI via WiX Toolset), Tauri bundle (desktop MSI).

### Publishing with GitHub CLI

**Option A — push a tag (recommended):**

```powershell
# Confirm CI is green
gh run list --workflow=ci.yml --limit 3

# Local quality gates (match CI)
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all --locked

# Tag and push (triggers release workflow)
git tag v0.2.2
git push origin v0.2.2

# Watch the release build
gh run list --workflow=release.yml --limit 1
gh run watch

# Verify published assets
gh release view v0.2.2
gh release download v0.2.2 --dir ./release-check
```

**Option B — re-run for an existing tag:**

```powershell
gh workflow run release.yml -f release_tag=v0.2.2
gh run list --workflow=release.yml --limit 1
gh run watch
```

### Post-release checks

- `gh release view vX.Y.Z` lists 7 assets (2 tar.gz, 1 win zip, 2 msi, 1 bridge zip, SHA256SUMS)
- Windows zip: both `teshi.exe` and `teshi-desktop.exe` in the same folder; `teshi desktop` works without PATH setup
- Extension zip loads in `chrome://extensions` via **Load unpacked**
- `SHA256SUMS` verifies with `sha256sum -c SHA256SUMS` (Linux) or equivalent on other platforms

## Debugging

- Run with `RUST_LOG=debug cargo run` for detailed logging
- The footer displays current mode/context information
- Status messages appear in the status bar for 3 seconds
