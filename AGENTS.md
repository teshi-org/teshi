# AGENTS.md

## Cursor Cloud specific instructions

**teshi** is a terminal-based (TUI) Gherkin `.feature` file editor written in Rust. It is a standalone offline application with no external service dependencies (no databases, Docker, or network services).

### Rust toolchain

The project uses `edition = "2024"`, which requires **Rust >= 1.85**. The VM update script handles upgrading the toolchain via `rustup`.

### Key commands

Standard `cargo` commands documented in the CI workflow (`.github/workflows/release.yml`):

| Task | Command |
|------|---------|
| Build | `cargo build --bin teshi` |
| Run | `cargo run -- tests/features/editor.feature` |
| Test (all) | `cargo test --all` |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` |
| Format check | `cargo fmt --all -- --check` |

### Testing notes

- E2E tests in `tests/steps/tui/` use `portable-pty` to spawn the `teshi` binary in a pseudo-terminal. The debug binary (`target/debug/teshi`) must be built before running `cargo test --all` — `cargo test` builds it automatically.
- The `runner` crate is an NDJSON-protocol test runner binary. It is built as part of the workspace but has no standalone test suite yet.
- Unit tests are in inline `#[cfg(test)]` modules across the main `teshi` crate.

### Workspace crates

| Crate | Path | Purpose |
|-------|------|---------|
| `teshi` | `/workspace/` | Main TUI editor binary |
| `runner` | `/workspace/runner/` | NDJSON BDD test runner |
| `teshi-tui-steps` | `/workspace/tests/steps/tui/` | PTY-based E2E step definitions |
