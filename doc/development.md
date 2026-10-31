# Development Guide

## Prerequisites

- Rust toolchain (stable, via [rustup](https://rustup.rs/))
- A terminal with Unicode and true-color support
- (Optional) Nightly toolchain + `wasm32-unknown-unknown` target + `wasm-bindgen` CLI, only for building `teshi-web`

> The `.githooks/` in this repo are stale and not enabled by default. Rely on the commands below as the real quality gates.

## Building

### Terminal application tests

The Rust TUI runner uses the existing `tui-test-rs` framework and has an isolated
Cargo workspace because its Unicode dependency conflicts with Ratatui 0.29.
Native workspace checks do not include this runner; run its gates explicitly:

```bash
cargo build -p teshi-cli --locked
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
cargo clippy --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --all-targets -- -D warnings
TESHI_SELF_TEST_BIN="$PWD/target/debug/teshi" cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked -- --ignored
```

On Windows, set `$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path`
and `$env:TESHI_VT_INPUT_TEST_BIN = & ./scripts/prepare-windows-vt-test.ps1`
before the final command. The latter selects Cargo's exact unit-test executable
for the console lifecycle fixture. CI runs the real PTY backend and self-regressions on
Windows and Linux. The self-tests launch the actual Teshi TUI with isolated data
directories, replay the English and Chinese `@tui` Features, and verify saved file
contents after navigation, running a scenario from Explore, editing, saving and
exit. These regressions have also been verified locally on Windows and Linux
(WSL). The CI matrix keeps both platforms covered independently. WSL is an
optional development environment for Linux shell scripts, not a prerequisite
for the Windows Rust build or the TUI runner.

For another application, install the runner with
`cargo install --path crates/teshi-tui-runner --profile dev --locked`, record a
`name.tui.json` sidecar next to `name.feature`, then run `teshi tui name.feature`.
Use `--scenario NAME` to select a scenario and `--runner-cmd PATH` or
`TESHI_TUI_RUNNER` to locate the runner. `teshi run` and Explore select it when all
selected Features have TUI sidecars and no external runner is configured.
Explicit external runner configuration retains precedence. See
[the runner guide](../crates/teshi-tui-runner/README.md) for the action schema,
failure evidence and current Gherkin limitations.

`teshi-web` is the web GPUI shell and compiles only for `wasm32-unknown-unknown`. On native hosts (Linux/macOS/Windows), exclude it from workspace commands:

```bash
cargo build --workspace --exclude teshi-web --locked              # debug build
cargo build --workspace --exclude teshi-web --locked --release    # optimized build
cargo run -p teshi-cli -- [args]                                  # build + run CLI
```

To build the GPUI WASM artifact for the hosted Pages deployment or local
artifact diagnostics:

```bash
bash scripts/build-teshi-web.sh
```

This requires the nightly toolchain, the `wasm32-unknown-unknown` target, and the `wasm-bindgen` CLI.

## Installing the local build (development loop)

Install the CLI from the checkout so `teshi` resolves from any directory and every
agent tool picks up the same binary:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/install-local.ps1   # Windows
bash scripts/install-local.sh                                                   # Linux / WSL
```

Both wrappers run `cargo install --path apps/teshi-cli --profile dev`.

`--profile dev` is not optional. `cargo install` defaults to `release`, and
`[profile.release]` enables `lto = true`, so a bare install rebuilds and relinks the
whole workspace every iteration (~4 min) instead of reusing the `target/debug` cache
that `cargo build` maintains. Measured with the dev profile: ~14 s after a one-line
edit, ~4 s when nothing changed.

No Cargo config key changes the default install profile — Cargo ignores unknown keys
silently, so `[install] profile = "dev"` has no effect and the omitted flag silently
costs a full release build. Use the wrappers, or pass the flag explicitly.

Reserve the packaged `setup.exe` flow below for work that targets the installer or the
update stream; that path cannot be exercised any other way, and it is what pays for the
full release build.

## Local source build identity

`crates/teshi-core/build.rs` derives a complete `local` identity automatically
whenever the build injects no `TESHI_BUILD_*` variable at all, so plain `cargo run`,
`cargo build`, and `cargo install --path apps/teshi-cli` already report as local
source builds; no manual export is needed for day-to-day iteration. The fallback is
all-or-nothing: if any `TESHI_BUILD_*` variable is set, the rest are passed through
untouched, so a half-configured CI job still fails closed to `dev` rather than
looking like a deliberate local build.

To pin identity explicitly, which is what release automation does, use
`TESHI_BUILD_CHANNEL=local` for a deliberately identified source build,
separate from GitHub stable/nightly streams. Set `TESHI_GIT_SHA` to the full
source commit, `TESHI_BUILD_TIMESTAMP` to a UTC RFC3339 timestamp,
`TESHI_BUILD_DATE` to its `YYYYMMDD` date, and `TESHI_BUILD_SEQUENCE` to a positive
local build sequence (for example Unix time). Compile every bundled executable
with the same values. Record uncommitted source changes separately when building
a dirty checkout.

Set `TESHI_BUILD_LOCAL_TIMESTAMP` to the same instant in the build machine's
current timezone, including its RFC3339 offset. The displayed local version
includes that local minute and offset, for example
`0.7.10-local.20261013.2003+0800 (26fd0b6)`. The bundle's machine-readable channel is
`local`; the manifest retains the full timestamp. Local builds do not perform
automatic release checks. Rebuild locally for another local version, or explicitly
choose `teshi update --channel stable` / `--channel nightly` to switch streams.
Local identities are rejected by published release manifests.

## Testing

```bash
cargo test --workspace --exclude teshi-web --locked               # run all tests
cargo test --workspace --exclude teshi-web --locked -- --nocapture  # show test output
cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings  # lint
cargo fmt --all --check                                           # check formatting
```

### Test layout

- Unit tests live at the bottom of each source file (in `#[cfg(test)]` modules)
- Product Gherkin: `features/en-US/`, `features/zh-CN/`, and `features/winapp/`; every Feature has an explicit verification route and runner tag (`@tui`, `@web-ui`, `@cli`, `@validation-e2e`, `@api`, or `@winapp`)
- Feature catalog ownership is checked by `python scripts/check-feature-e2e-catalog.py`; Core, architecture, CI packaging, and protocol-internal contracts belong in Rust unit/integration tests, workflow contract tests, or documentation. See [Test Strategy](test-strategy.md)
- Requirement CLI E2E is executed by `teshi run` and `teshi-requirement-cli-runner` (`tests/steps/requirement-cli`) against requirement-tagged CLI files (English `requirement_*.feature`, Chinese titles such as `需求列表.feature` and `需求标签页.feature`)
- Validation self-bootstrap E2E is executed by `validation_cli_bdd.rs` with `teshi-validation-cli-runner` (`tests/steps/validation-cli`). It creates malformed source as isolated fixture data, then starts the exact built Teshi binary as a child process; it does not use the target validator API for assertions.
- Key test areas:
  - Gherkin parsing edge cases (`crates/teshi-core/src/gherkin.rs`)
  - Editor buffer operations (`crates/teshi-tui/src/editor_buffer.rs`)
  - Step navigation and keyword replacement (`crates/teshi-tui/src/bdd_nav.rs`)
  - App integration (navigation, editing, focus, undo, copy/paste) (`crates/teshi-tui/src/app.rs`)

Requirement documents used by the TUI live in the user-level store (`<app_data>/requirements`, overridable with `TESHI_REQUIREMENTS_DIR` or `--requirements-root`). Tests that touch settings or the requirement library should set `TESHI_APP_DATA_DIR` to a temp directory. Legacy `<project>/requirements/` fixtures are only for `teshi requirements import-project` migration tests.

## Running `teshi web`

The production launcher starts the loopback daemon and opens the latest hosted
UI at `https://teshi-org.github.io/app/#port=<port>&token=<session-token>`. It no longer
requires an installed `share/web` directory or a local WASM build:

```bash
./target/debug/teshi web --project <dir>
```

Use `--no-open` for daemon/session lifecycle checks. `--port` and `--dist` are
explicit diagnostic controls; `--dist` is not part of the nightly package or
the production hosted launch path. See [hosted-web-ui.md](hosted-web-ui.md).

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
   - `Cargo.toml` (`[workspace.package] version`, inherited by `teshi-cli` and `teshi-tui`)
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

### Nightly (pre-release) builds

Nightly builds publish the Windows per-user **setup.exe** from the `dev` branch as GitHub **pre-releases** (no Linux/macOS archives or MSI). Stable releases still publish the full asset set.

| Trigger | Workflow |
|---------|----------|
| Push to `dev` | [`.github/workflows/nightly.yml`](../.github/workflows/nightly.yml) |
| Daily 06:00 UTC | same (uses workflow file on the **default branch**; sync `dev` → `main` to keep schedule active). Catch-up only: skipped when `dev` HEAD already has a nightly tag. |
| Manual | `gh workflow run nightly.yml` |

Tag format: `v{semver}-nightly.{YYYYMMDD}.{short_sha}` (for example `v0.7.9-nightly.20260801.dc6c942`), derived from `[workspace.package] version` in the root `Cargo.toml` + UTC date of first publish + commit. The workflow skips when this commit already has a `v*-nightly.*` tag, so an unchanged `dev` tip does not get a new tag on the next calendar day. To rebuild the same commit, delete that nightly tag/release first. Nightly CI injects its build identity without writing the tag into Cargo.toml. The published asset is `teshi-<tag>-x64-setup.exe` plus `update-manifest.json` and `SHA256SUMS`.

New Nightly builds also show a workflow run number, for example `0.7.10-nightly.101 (2026.09.07, cb4361a)`, in `teshi -V` and the TUI. The GitHub release title includes the same number. Numbers increase with Nightly workflow runs and can have gaps when a run is skipped or fails; reruns retain their number. The tag retains its existing format so already-installed updaters can validate and install the release. The update manifest retains its separate globally ordered build sequence. Stable version bumps remain independent.

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

Per-user setup.exe (in-app updates): install [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`choco install innosetup`), stage the same tree as `staging/exe-root` with `python scripts/update_manifest.py bundle ... --kind exe`, then `./scripts/build-exe-setup.ps1`.

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
