# Teshi TUI runner

Rust terminal testing adapter using `tui-test-rs` 0.1.0. It launches actual
applications in a PTY and asserts the rendered viewport, without shell inference.

This crate has an independent Cargo workspace because its terminal backend's
`unicode-width` dependency conflicts with the application's Ratatui version.

```powershell
cargo build --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
cargo build -p teshi-cli --locked
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked -- --ignored
```

Install with `cargo install --path crates/teshi-tui-runner --profile dev --locked`.
Run `teshi tui path/to/test.feature --runner-cmd <runner-executable>` or set
`TESHI_TUI_RUNNER`. `teshi run` automatically selects this runner when all selected
features have `.tui.json` sidecars and no external runner is configured. Explore
uses the same fallback. An explicit external runner retains precedence in those
existing workflows.

Each `name.feature` has a `name.tui.json` sidecar:

```json
{
  "version": 1,
  "mode": "tui",
  "steps": [
    {
      "line": 3,
      "text": "the application is running",
      "confirmed": true,
      "actions": [
        {
          "action": "launch",
          "config": { "program": "my-app", "args": [], "cols": 100, "rows": 30 }
        },
        { "action": "expect_text", "text": "Ready", "visible": true, "timeout_ms": 5000 }
      ]
    }
  ]
}
```

Actions are `launch`, `write`, `key`, `resize`, `expect_text`, and `expect_exit`.
Keys use tui-test names such as `Enter`, `Escape`, and `q`. Launch accepts optional
`cwd` and `env` pairs; cwd defaults to the project root. Arguments are passed
directly. Assertion deadlines must be 1–120000 milliseconds. Each scenario owns
one session; its bindings are validated before launch. Source line and exact
parsed step body must match, and each binding must be confirmed and nonempty.
After Escape, wait for an observable UI state before sending another key: a
terminal can interpret adjacent Escape and character bytes as an Alt shortcut.

The binary reads one NDJSON `run` request from stdin and emits standard Teshi
case/step events to stdout. Failed cases return exit status 1; request/protocol
errors return 2. Failure screens are retained under `.teshi/tui-results` or
`meta.artifacts_dir` and attached to the failure event. Feature Background and
Rule-nested scenarios are supported. Scenario Outlines, Rule Background and
multiple Background blocks fail explicitly. Bindings are authored as JSON; the
repository self-tests generate their bindings inside isolated fixture directories.
