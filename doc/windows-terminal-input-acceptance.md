# Windows VT input acceptance

On Windows, Teshi reads console input as VT through an owned `CONIN$` handle
instead of crossterm's console-record backend, which never reports bracketed
paste. Windows Terminal answers Ctrl+V with an image-only clipboard by sending
an empty bracketed paste; reading VT input is what lets Teshi see it and attach
the screenshot.

Outside Zellij, Teshi also enables win32-input-mode (`CSI ? 9001 h`), so the
console reports every key record as `CSI Vk;Sc;Uc;Kd;Cs;Rc _` and keys such as
Shift+Enter keep their modifiers. Records that type text, unmodified Enter and
Tab, and the synthetic records that carry terminal sequences are handed to the
VT parser as text, so pastes and mouse reports parse as whole sequences. Other
keys are decoded like crossterm's Windows backend. A console that refuses VT
input falls back to crossterm's backend, and `TESHI_LEGACY_CONSOLE_INPUT` forces
that fallback. Under Zellij, Teshi keeps plain VT keys.

Releasing the event source stops its parser, cancels and joins its console
reader, turns win32-input-mode off, and then restores the input mode. Each
reader owns a fresh handle so cancellation cannot strand subsequent consumers
of standard input. UTF-16 surrogate pairs are retained across console read
boundaries and across key records.

## Reproduce on Windows

```powershell
cargo test -p teshi-tui --lib --locked
$env:TESHI_VT_INPUT_TEST_BIN = & ./scripts/prepare-windows-vt-test.ps1
$env:TESHI_VT_INPUT_ARTIFACTS = "$PWD/.teshi/artifacts/windows-vt-input"
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --test windows_vt_input -- --ignored
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --test requirements_vim -- --ignored --test-threads=1
```

The real ConPTY fixture runs twice: with `ZELLIJ` set (plain VT keys) and
without it (win32-input-mode). Each run reads Chinese and emoji, drops and
recreates the input source three times, verifies that each drop completes
within two seconds and restores the original mode, then successfully reads
through `std::io::stdin`. The win32-input-mode run also requires an empty
bracketed paste to arrive as an empty paste event. The Vim suite also verifies
bracketed multiline Unicode paste, atomic undo/redo, saved Markdown, restart,
and narrow wrapped-row editing.

## November 6, 2026 evidence

- Both ConPTY fixture runs passed, including the empty paste in
  win32-input-mode.
- TUI unit suite: 677 passed.

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
