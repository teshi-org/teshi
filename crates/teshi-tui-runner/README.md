# Teshi TUI runner

## ACP model preference acceptance

`teshi-acp-model-feature-runner` binds the paired
`tui_acp_model_persistence.feature` / `ACP模型选择持久化.feature` files through
`tests/fixtures/model_feature_steps.py`. It reads source steps with Teshi's
Gherkin parser, expands each English Examples row for both locales, and rejects
unknown steps. This external semantic runner does not use `.tui.json` sidecars.
The Python bindings drive a Rust-owned real PTY and assert the rendered controls,
replies, and the controlled ACP agent's model/session trace. Rejection, removed
models, and restoration timeouts are deterministic fixture behaviors, not live
provider tests. Every case uses a temporary project, profile store, and app data.

```powershell
cargo build -p teshi-cli --locked
cargo build --manifest-path crates/teshi-tui-runner/Cargo.toml --bin teshi-acp-model-feature-runner --locked
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
$runner = (Resolve-Path crates/teshi-tui-runner/target/debug/teshi-acp-model-feature-runner.exe).Path
teshi run (Resolve-Path features/en-US/tui_acp_model_persistence.feature).Path --runner-cmd $runner
teshi run (Resolve-Path features/zh-CN/ACP模型选择持久化.feature).Path --runner-cmd $runner
```

Each locale executes seven isolated terminal flows across five scenario
definitions (the outline has three rows). Step results use the standard runner
protocol. Screens and ACP traces are retained under `.teshi/tui-results/model`,
or the requested artifacts directory. A failed assertion fails its case.

Rust terminal testing adapter using `tui-test-rs` 0.1.0. It launches actual
applications in a PTY and asserts the rendered viewport, without shell inference.

This crate has an independent Cargo workspace because its terminal backend's
`unicode-width` dependency conflicts with the application's Ratatui version.

```powershell
cargo build --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
cargo build -p teshi-cli --locked
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
$env:TESHI_VT_INPUT_TEST_BIN = & ./scripts/prepare-windows-vt-test.ps1
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

Actions are `launch`, `write`, `key`, `resize`, `click_text`, `expect_text`,
`expect_stable_text`, `expect_file`, and `expect_exit`.
Use `expect_file` with `path` and `timeout_ms` (1–120000) to wait for a nonempty
regular fixture file; relative paths resolve from the project root. This lets
external process fixtures publish readiness before later terminal actions run,
without assuming that a running indicator proves process startup.
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
Rule-nested scenarios and Scenario Outlines (including Chinese dialect keywords)
are supported. Each Examples row runs all Background and scenario steps in its
own terminal. Bindings retain the exact template text and source line; placeholders
are substituted in action strings, step text, doc strings, and step tables after
template validation. Missing parameters, malformed tables, empty Examples, and
stale or unconfirmed bindings fail before any terminal launches.

The NDJSON protocol keeps the requested parent case ID. Outline rows additionally
emit `start_example` and `end_example` with `example_line`, status, and failure
attachments. A failing row does not stop later rows; the parent fails if any row
fails. `end_run` includes `examples_passed` and `examples_failed` in addition to
the existing parent-case counts. A request's `line_number` can select either the
Outline header (all rows) or a specific Examples data-row source line.

Rule Background and multiple Background blocks still fail explicitly. Bindings are authored as JSON; the
repository self-tests generate their bindings inside isolated fixture directories.

## Project-directory loading acceptance

`teshi-directory-feature-runner` binds the source steps and Examples in
`features/zh-CN/项目目录加载.feature` to isolated real PTY flows.
Unknown steps fail, each Examples row has its own temporary project and data stores,
and success/failure screens, launch identities, and per-step results are retained.
Language variants use the explicit `TESHI_FEATURE_LANGUAGE` fixture override;
unit tests separately exercise selection from the resolved locale.

```powershell
cargo build -p teshi-cli --locked
cargo build --manifest-path crates/teshi-tui-runner/Cargo.toml --bin teshi-directory-feature-runner --locked
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
$runner = (Resolve-Path crates/teshi-tui-runner/target/debug/teshi-directory-feature-runner.exe).Path
& $env:TESHI_SELF_TEST_BIN run features/zh-CN/项目目录加载.feature --runner-cmd $runner
```

The configuration/session scenario opens the real project-session browser and executes
an owned probe runner to verify the selected project's relative runner cwd. Its probe
result is separate from the acceptance run's actual per-step outcomes.
