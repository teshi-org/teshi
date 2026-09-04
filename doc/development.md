# Development Guide

## Prerequisites

- Rust toolchain (stable, via [rustup](https://rustup.rs/))
- A terminal with Unicode and true-color support
- (Optional) Nightly toolchain + `wasm32-unknown-unknown` target + `wasm-bindgen` CLI, only for building `teshi-web`

> The `.githooks/` in this repo are stale and not enabled by default. Rely on the commands below as the real quality gates.

## Building

`teshi-web` is the web GPUI shell and compiles only for `wasm32-unknown-unknown`. On native hosts (Linux/macOS/Windows), exclude it from workspace commands:

```bash
cargo build --workspace --exclude teshi-web --locked              # debug build
cargo build --workspace --exclude teshi-web --locked --release    # optimized build
cargo run -p teshi-cli -- [args]                                  # build + run CLI
```

To build the GPUI WASM frontend for `teshi web`:

```bash
bash scripts/build-teshi-web.sh
```

This requires the nightly toolchain, the `wasm32-unknown-unknown` target, and the `wasm-bindgen` CLI.

## Testing

```bash
cargo test --workspace --exclude teshi-web --locked               # run all tests
cargo test --workspace --exclude teshi-web --locked -- --nocapture  # show test output
cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings  # lint
cargo fmt --all --check                                           # check formatting
```

### Test layout

- Unit tests live at the bottom of each source file (in `#[cfg(test)]` modules)
- Integration tests: `tests/feature/*.feature` — BDD scenarios that describe expected behavior
- Key test areas:
  - Gherkin parsing edge cases (`crates/teshi-core/src/gherkin.rs`)
  - Editor buffer operations (`crates/teshi-tui/src/editor_buffer.rs`)
  - Step navigation and keyword replacement (`crates/teshi-tui/src/bdd_nav.rs`)
  - App integration (navigation, editing, focus, undo, copy/paste) (`crates/teshi-tui/src/app.rs`)

Requirement documents used by the TUI live in the user-level store (`<app_data>/requirements`, overridable with `TESHI_REQUIREMENTS_DIR` or `--requirements-root`). Tests that touch settings or the requirement library should set `TESHI_APP_DATA_DIR` to a temp directory. Legacy `<project>/requirements/` fixtures are only for `teshi requirements import-project` migration tests.

## Running `teshi web`

The browser GUI is served by `apps/teshi-daemon` and requires the prebuilt GPUI WASM bundle:

```bash
bash scripts/build-teshi-web.sh
./target/debug/teshi web --project <dir> --host 127.0.0.1 --port 20253 --no-open --dist apps/teshi-web/dist
```

When run from the repo root, `--dist apps/teshi-web/dist` is optional because the daemon auto-resolves that path.

## Project structure

This repo is a Cargo workspace.

```
apps/
├── teshi-cli/               # Terminal CLI entry point (`teshi`)
├── teshi-daemon/            # `teshi web` server
├── teshi-terminal-sidecar/  # Terminal integration helper
├── teshi-desktop/           # Native desktop shell (GPUI, Windows-primary)
└── teshi-web/               # GPUI-in-browser WASM shell (wasm32-only)

crates/
├── teshi-core/              # Core domain: Gherkin parser, project model, indexes
├── teshi-tui/               # Terminal UI (ratatui), keymap, editing operations
├── teshi-agent/             # LLM agent loop, tool definitions, approvals
├── teshi-engine/            # Engine: runner, terminal, file watcher, sidecar
└── teshi-ui/                # Shared GPUI UI components (desktop + web)
```

## Key dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `ratatui` | 0.29 | Terminal UI framework |
| `crossterm` | 0.28 | Terminal raw-mode control |
| `tui-tree-widget` | 0.23 | Tree component for mind map |
| `ropey` | 1.x | Efficient text buffer (rope data structure) |
| `clap` | 4.x | CLI argument parsing |
| `tokio` | 1.x | Async runtime (LLM, runner threads) |
| `reqwest` | 0.13 | HTTP client for LLM APIs |
| `serde` / `serde_json` | 1.x | Serialization (runner protocol, LLM) |
| `toml` | 0.8 | Config file parsing |

## Code conventions

- No doc comments on private items unless the logic is non-obvious
- No comments that restate the code
- Tests live at the bottom of source files
- Functions that mutate the project AST should also rebuild derived indexes (`step_index`, `mindmap_index`)
- Editor mutations should call `push_undo()` before modifying the buffer
- Use the `Action` enum for all user input — never call application logic directly from `crates/teshi-tui/src/keymap.rs`

## Adding a new LLM tool

1. Define the function in `crates/teshi-agent/src/tools.rs` with a JSON Schema
2. Add the tool name/payload to the tools list sent to the LLM
3. Add a match arm in `crates/teshi-agent/src/lib.rs` `execute_tool()`
4. If it mutates files, return `ToolResult::Queued` with an `AgentPendingChange`
5. Handle the confirmation flow in `crates/teshi-tui/src/app.rs` `accept_agent_change()` / `reject_agent_change()`

## Adding a new keybinding

1. Add an `Action` variant if needed
2. Add the mapping in `crates/teshi-tui/src/keymap.rs` `Action::from_key_event()` with appropriate context gating
3. Handle the action in `crates/teshi-tui/src/app.rs` `handle_action()` match
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
   - `extension/teshi-bridge/manifest.json`
3. **Commit and tag**: `git commit -m "chore: bump version to vX.Y.Z"` then `git tag vX.Y.Z`
4. **Push**: `git push origin main && git push origin vX.Y.Z`

Run `python scripts/release.py` (dry-run) to verify version consistency across component files before committing.

### Release assets

| Asset | Platform | Contents |
|-------|----------|----------|
| `teshi-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | Linux x64 | `teshi` + README + LICENSE |
| `teshi-vX.Y.Z-aarch64-apple-darwin.tar.gz` | macOS ARM | `teshi` + README + LICENSE |
| `teshi-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Windows x64 | `teshi.exe` + GPUI WASM `share/web/` + README + LICENSE |
| `teshi-vX.Y.Z-x64.msi` | Windows x64 | Full WiX installer: CLI + GPUI WASM web UI under `Program Files\teshi` |
| `teshi-bridge-vX.Y.Z.zip` | All | Chrome extension (load unpacked) |
| `SHA256SUMS` | All | Checksums for every archive above |

Workflow: [`.github/workflows/release.yml`](../.github/workflows/release.yml)

WinGet submission (CLI MSI only) runs automatically when `WINGET_TOKEN` is configured. Legacy standalone workflow: [`.github/workflows/winget.yml`](../.github/workflows/winget.yml).

### Nightly (pre-release) builds

Nightly builds publish the same asset set as stable releases, but from the `dev` branch as GitHub **pre-releases** (no WinGet submission).

| Trigger | Workflow |
|---------|----------|
| Push to `dev` | [`.github/workflows/nightly.yml`](../.github/workflows/nightly.yml) |
| Daily 06:00 UTC | same (uses workflow file on the **default branch**; sync `dev` → `main` to keep schedule active). Catch-up only: skipped when `dev` HEAD already has a nightly tag. |
| Manual | `gh workflow run nightly.yml` |

Tag format: `v{semver}-nightly.{YYYYMMDD}.{short_sha}` (for example `v0.7.9-nightly.20260801.dc6c942`), derived from `apps/teshi-cli/Cargo.toml` version + UTC date of first publish + commit. The workflow skips when this commit already has a `v*-nightly.*` tag, so an unchanged `dev` tip does not get a new tag on the next calendar day. To rebuild the same commit, delete that nightly tag/release first.

```powershell
gh run list --workflow=nightly.yml --limit 3
gh release list --prerelease
gh release download v0.7.9-nightly.20260801.dc6c942 --dir ./nightly-check
```

Stable releases remain tag-driven on `main` (`vX.Y.Z`). Merge `dev` → `main` and run `scripts/release.py` for production releases.

Windows installer source: `wix/` (WiX Toolset).

Local full MSI build (Windows, matches CI): install [WiX Toolset](https://wixtoolset.org/) and `cargo install cargo-wix --locked --version 0.3.9`, stage `staging/msi-root/bin`, `share/web`, and `share/teshi-bridge`, run `heat` on both trees into `wix/web-files.wxs` and `wix/bridge-files.wxs`, then:

```powershell
cargo wix --package teshi-cli --nocapture --no-build -C -dStagingRoot=staging/msi-root -C -dWebRoot=staging/msi-root/share/web -C -dBridgeRoot=staging/msi-root/share/teshi-bridge -o target/wix/teshi-local-x64.msi
```

cargo-wix 0.3.x no longer accepts `--define`; pass WiX preprocessor variables with `-C -dName=value` (candle).

### Publishing with GitHub CLI

**Option A — push a tag (recommended):**

```powershell
# Confirm CI is green
gh run list --workflow=ci.yml --limit 3

# Local quality gates (match CI)
cargo fmt --all --check
cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
cargo test --workspace --exclude teshi-web --locked

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
- Windows zip / MSI: `teshi web` loads the bundled GPUI WASM UI and `teshi desktop` works without a separate frontend build
- Extension zip loads in `chrome://extensions` via **Load unpacked**
- `SHA256SUMS` verifies with `sha256sum -c SHA256SUMS` (Linux) or equivalent on other platforms

## Debugging

- Run with `RUST_LOG=debug cargo run -p teshi-cli` for detailed logging
- The footer displays current mode/context information
- Status messages appear in the status bar for 3 seconds
