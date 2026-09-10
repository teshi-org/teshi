# Gherkin validation and diagnostics

Teshi keeps two deliberately separate contracts for `.feature` files:

- `teshi-core` parses source permissively so an in-progress buffer remains
  renderable and editable.
- Validation reports correctness problems before binding or execution. It does
  not rewrite the source or make the parser strict.

## Check scopes

Run the canonical validator from a project directory:

```text
teshi check
teshi check --feature features/login.feature
teshi check --all
teshi check --all --json
```

Without a selector, `check` validates every `.feature` file discovered below
the current project root. `--feature` validates exactly one source, while
`--all` makes the all-files scope explicit. `--feature` and `--all` cannot be
used together.

Human output includes the source location, severity, stable code, message, an
optional repair suggestion, and aggregate counts:

```text
error features/zh-CN/login.feature:4:6 [missing_step_separator] Step keyword '当' must be followed by a space
  suggestion: 当 用户登录
Summary: 1 error(s), 0 warning(s), 0 suggestion(s)
```

## JSON contract

Successful validation reports have this shape:

```json
{
  "scope": ["features/login.feature"],
  "summary": {"errors": 0, "warnings": 1, "suggestions": 0},
  "diagnostics": [
    {
      "path": "features/login.feature",
      "line": 4,
      "column": 6,
      "severity": "warning",
      "code": "scenario_starts_without_given",
      "message": "Scenario starts without a Given step",
      "suggestion": null
    }
  ]
}
```

`line` and `column` are one-based source positions. Diagnostics are ordered by
normalized path, line, column, severity, and code. `suggestion` is omitted from
JSON when no repair text is available. I/O or invocation failures use exit
status `2` and an error envelope with `error.code: "check_io_error"`.

## Severity and stable codes

- `error` means the source cannot safely be treated as the intended executable
  Feature. Any error fails `check` and blocks binding or execution.
- `warning` identifies an authoring-quality issue but does not block a command.
- `suggestion` is an optional improvement hint and does not block a command.

Core syntax and structure codes include:

`missing_feature_header`, `empty_feature_name`,
`malformed_structural_header`, `duplicate_feature_header`, `empty_rule_name`,
`empty_scenario_name`, `scenario_without_steps`,
`scenario_outline_missing_examples`, `examples_without_outline`,
`examples_missing_headers`, `missing_step_separator`,
`unrecognized_executable_line`, `step_outside_scenario`, `empty_step`, and
`unterminated_doc_string`.

Authoring guidance codes include `duplicate_scenario_name`, `missing_when`,
`missing_then`, `scenario_starts_without_given`, `scenario_too_many_steps`,
and `cross_scenario_dependency`. These are warnings or suggestions; in
particular, a legal scenario containing repeated `Given`/`And` steps is not
blocked merely because it has no `When` or `Then` step.

The missing-separator check uses the active Gherkin dialect from
`teshi-core`. For example, `当用户登录` is diagnosed as a prefix of `当` and
the suggested source is `当 用户登录`; valid prose before a Scenario's first
step, comments, tags, tables, and DocStrings remain legal.

## Binding and execution gates

The direct CLI path and daemon-backed path use the same Core report. Step
selection, unbound-step discovery, next-unbound selection, binding resolution,
`teshi run`, browser replay, and WinApp replay fail before state mutation or
target execution when their selected Feature scope contains an error. Warnings
and suggestions are surfaced but remain non-blocking.

Daemon REST responses keep the normal JSON body and add warnings in the
`x-teshi-validation-report` response header. NDJSON execution emits an
additional first `{"type":"validation","report":...}` event. Hosted control
responses keep the existing result shape and publish the same report as a
`gherkin.validation` event; validation failures use a structured error object:

```json
{
  "error": {
    "code": "feature_validation_failed",
    "message": "Feature validation failed",
    "report": {"scope": [], "summary": {}, "diagnostics": []}
  }
}
```

The daemon validates the selected Feature or directory, not unrelated sibling
Features. This keeps CLI, REST, control, and direct execution scopes aligned.

## Editor behavior

The TUI validates the raw current buffer on redraw and shows locations,
messages, and suggestions in a diagnostics pane. Error rows are marked in the
source view. The buffer, partial AST, Browse/Insert navigation, and save path
remain available while the source is incomplete.

Desktop and Web use the same `GherkinEditorBackend` contract and transport
`bdd.validate_buffer`; render payloads also carry the same diagnostics. Shells
may debounce or schedule requests, but validation rules and diagnostic codes
belong only to `teshi-core`.

## Skills and CI

The packaged `teshi` and `bdd-feature` Skills call
`teshi check --feature <path> --json` before binding or execution and consume
the returned codes and locations. Repository CI additionally runs
`teshi check --all`; the existing Rust and Web quality gates remain required.

## Self-bootstrap E2E

The deterministic CLI contract is also tested through Teshi's own BDD runner:

```text
cargo test --locked -p teshi-cli --test validation_cli_bdd -- --nocapture
```

This command builds `teshi-validation-cli-runner`, runs the paired
`@validation-e2e` Features through `teshi run`, and passes the current
`CARGO_BIN_EXE_teshi` path as `TESHI_BIN`. Each scenario creates a fresh
temporary project and app-data directory. The runner then invokes the target
binary for `check`, repeated-Given lifecycle warnings, selected-directory run
scope, daemon-backed REST warning/error reports, step binding gates, BDD run
preflight, and browser/WinApp replay preflight, inspecting child-process
status and output. The paired Features currently contain 17 executable cases
per locale; daemon control-channel parity remains covered by the daemon's
protocol integration tests because the CLI runner exercises the public CLI/REST
surface.

Invalid Gherkin is generated inside those temporary projects because the test
Feature that drives `teshi run` must itself be valid. This gives black-box
coverage of the executable CLI contract without making Teshi both the system
under test and the assertion oracle. Parser partial-AST and editor rendering
remain covered by Rust-level tests and live-surface checks.
