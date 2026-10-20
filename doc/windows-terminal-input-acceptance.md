# Windows VT input acceptance

When `ZELLIJ` is set, Teshi reads Windows console VT input through an owned
`CONIN$` handle. Releasing the event source stops its parser, cancels and joins
its console reader, and then restores the input mode. Each reader owns a fresh
handle so cancellation cannot strand subsequent consumers of standard input.
UTF-16 surrogate pairs are retained across console read boundaries before
conversion to the parser's UTF-8 byte stream.

## Reproduce on Windows

```powershell
cargo test -p teshi-tui --lib --locked
$env:TESHI_VT_INPUT_TEST_BIN = & ./scripts/prepare-windows-vt-test.ps1
$env:TESHI_VT_INPUT_ARTIFACTS = "$PWD/.teshi/artifacts/windows-vt-input"
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --test windows_vt_input -- --ignored
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --test requirements_vim -- --ignored --test-threads=1
```

The real ConPTY fixture reads Chinese and emoji, drops and recreates the input
source three times, verifies that each drop completes within two seconds and
restores the original mode, then successfully reads through `std::io::stdin`.
The Vim suite also verifies bracketed multiline Unicode paste, atomic undo/redo,
saved Markdown, restart, and narrow wrapped-row editing.

## October 20, 2026 evidence

- TUI unit suite: 481 passed; real Chrome integration and the separately run
  console fixture were ignored by the default invocation.
- Windows ConPTY lifecycle: passed, including subsequent standard input.
- Vim real PTY suite: eight passed using the separately built checkout CLI.
- TUI and runner Clippy passed with warnings denied; both formatting checks passed.

Evidence lives under `.teshi/artifacts/windows-input-20261020` and the adjacent
`windows-input-*-20261020` logs. The CLI was built to a separate output to
preserve running instances. The earlier shared-handle lifecycle failure and
successful owned-handle replay remain in separate logs. This is Windows
acceptance; it does not certify a Linux matrix or the full native workspace.
