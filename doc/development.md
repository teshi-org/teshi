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

Full-stack releases publish the CLI, Windows desktop app, and Chrome extension under a single `vX.Y.Z` tag. All component versions must match before tagging.

### Quick start (recommended)

```bash
# Dry-run: see what version would be released
python scripts/release.py

# Apply version bumps to all component files
python scripts/release.py --apply

# Apply, commit, and tag (full automation)
python scripts/release.py --tag
```

Then push to trigger the CI release workflow:

```bash
git push origin main && git push origin vX.Y.Z
```

### Manual release

If you prefer to do it step by step:

1. **Analyze commits** since last tag to determine bump type
2. **Update version** in component files (must match):
   - `apps/teshi-cli/Cargo.toml`
   - `apps/teshi-web-ui/package.json`
   - `extension/teshi-bridge/manifest.json`
3. **Commit and tag**: `git commit -m "chore: bump version to vX.Y.Z"` then `git tag vX.Y.Z`
4. **Push**: `git push origin main && git push origin vX.Y.Z`

The pre-commit hook (`.githooks/pre-commit`) will verify version consistency across all files.

### Release assets

| Asset | Platform | Contents |
|-------|----------|----------|
| `teshi-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | Linux x64 | `teshi` + README + LICENSE |
| `teshi-vX.Y.Z-aarch64-apple-darwin.tar.gz` | macOS ARM | `teshi` + README + LICENSE |
| `teshi-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Windows x64 | `teshi.exe` + `share/web/` + README + LICENSE |
| `teshi-vX.Y.Z-x64.msi` | Windows x64 | Full WiX installer: CLI + web UI under `Program Files\teshi` |
| `teshi-bridge-vX.Y.Z.zip` | All | Chrome extension (load unpacked) |
| `SHA256SUMS` | All | Checksums for every archive above |

Workflow: [`.github/workflows/release.yml`](../.github/workflows/release.yml)

WinGet submission (CLI MSI only) runs automatically when `WINGET_TOKEN` is configured. Legacy standalone workflow: [`.github/workflows/winget.yml`](../.github/workflows/winget.yml).

### Nightly (pre-release) builds

Nightly builds publish the same asset set as stable releases, but from the `dev` branch as GitHub **pre-releases** (no WinGet submission).

| Trigger | Workflow |
|---------|----------|
| Push to `dev` | [`.github/workflows/nightly.yml`](../.github/workflows/nightly.yml) |
| Daily 06:00 UTC | same (rebuilds `dev` tip if not already published) |
| Manual | `gh workflow run nightly.yml` |

Tag format: `v{semver}-nightly.{YYYYMMDD}.{short_sha}` (for example `v0.7.9-nightly.20260801.dc6c942`), derived from `apps/teshi-cli/Cargo.toml` version + UTC date + commit.

```powershell
gh run list --workflow=nightly.yml --limit 3
gh release list --prerelease
gh release download v0.7.9-nightly.20260801.dc6c942 --dir ./nightly-check
```

Stable releases remain tag-driven on `main` (`vX.Y.Z`). Merge `dev` → `main` and run `scripts/release.py` for production releases.

Windows installer source: `wix/` (WiX Toolset).

Local full MSI build (Windows, matches CI): install [WiX Toolset](https://wixtoolset.org/) and `cargo install cargo-wix --locked --version 0.3.9`, stage `staging/msi-root/bin`, `share/web`, and `share/teshi-bridge`, run `heat` on both trees into `wix/web-files.wxs` and `wix/bridge-files.wxs`, then:

```powershell
cargo wix --package teshi --nocapture --no-build -C -dStagingRoot=staging/msi-root -C -dWebRoot=staging/msi-root/share/web -C -dBridgeRoot=staging/msi-root/share/teshi-bridge -o target/wix/teshi-local-x64.msi
```

cargo-wix 0.3.x no longer accepts `--define`; pass WiX preprocessor variables with `-C -dName=value` (candle).

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

- `gh release view vX.Y.Z` lists 6 assets (2 tar.gz, 1 win zip, 1 msi, 1 bridge zip, SHA256SUMS)
- Windows zip / MSI: `teshi web` and `teshi desktop` work without a separate frontend build
- Extension zip loads in `chrome://extensions` via **Load unpacked**
- `SHA256SUMS` verifies with `sha256sum -c SHA256SUMS` (Linux) or equivalent on other platforms

## Debugging

- Run with `RUST_LOG=debug cargo run` for detailed logging
- The footer displays current mode/context information
- Status messages appear in the status bar for 3 seconds
