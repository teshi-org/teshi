# Runner routing and protocol v1

Teshi does not execute steps itself for most projects. A **runner** executes
scenarios and reports results as NDJSON events. One project can use several
runners (for example the built-in API dispatch for `@api`, the TUI runner for
`@tui`, and a project-specific `behave` or `pytest` adapter for the rest).

This document is the contract for `teshi run`, the TUI `r` key and the agent
`run_tests` tool.

## Routing: `[[runners]]` in `teshi.toml`

```toml
[[runners]]
name = "api"
builtin = "dispatch"
[runners.match]
tags = ["@api"]

[[runners]]
name = "backend"
cmd = "python"
args = ["-m", "my_runner"]
cwd = "."                       # relative to the project root
env = { PYTHONUTF8 = "1" }
[runners.match]
paths = ["features/backend/**/*.feature"]
```

Each rule has a unique `name` and exactly one executor:

| Field | Meaning |
|-------|---------|
| `builtin = "dispatch"` | In-process API/mixed dispatch (`[API]` steps and Jinja templates) |
| `builtin = "tui"` | The packaged `teshi-tui-runner` for `.tui.json` sidecars (`TESHI_TUI_RUNNER` overrides its path) |
| `builtin = "browser"` | `teshi browser replay` of confirmed web bindings, one process per scenario |
| `builtin = "winapp"` | `teshi winapp replay` of confirmed Windows UI bindings, one process per scenario |
| `builtin = "manual"` | Nothing runs; each scenario is reported as `case_skipped` with reason `manual verification required`, so manual scenarios have an owner without counting as passed or failed |
| `cmd` (+ `args`, `cwd`, `env`) | An external process that speaks protocol v1 |

A relative `cmd` that contains a path separator (`target/debug/my-runner`) is
resolved against the project root; a bare name (`python`) is looked up on
`PATH`. On Windows `.exe` is implied. `cwd` is relative to the project root.
Teshi exports `TESHI_BIN` (its own executable) to external runners unless the
environment or the rule's `env` already sets it, so runners that drive Teshi
test the binary that started them.

`[runners.match]` must declare `tags`, `paths`, or both; when both are set a
scenario must satisfy both. Use `paths = ["**"]` to match everything.

- **Tags** are compared against the scenario's effective tags (Feature tags
  plus scenario tags), case-insensitively and with or without the leading `@`.
  Any listed tag matches.
- **Paths** are glob patterns with `/` separators, relative to the project
  root. `*` never crosses a `/`; `**` matches any number of directories,
  including none (`**/*.feature` matches `sample.feature`).
- **`exclude_tags` / `exclude_paths`** disqualify a scenario that would
  otherwise match. They let a broad rule (`tags = ["@cli"]`) step aside for a
  narrower one (`tags = ["@validation-e2e"]`) so each scenario keeps exactly
  one owner. There is no rule priority.

### Browser and WinApp executors

The `browser` and `winapp` builtins replay confirmed bindings scenario by
scenario and report step events and one result per scenario. Target
availability is checked before any step runs, and an unavailable target is
reported as **blocked**, never as a failure:

| Category | Browser | WinApp |
|----------|---------|--------|
| `target_unconfigured` | No `.teshi/cdp-endpoint.json` for the project | — |
| `target_unavailable` | Endpoint unreachable or page snapshot fails (`teshi browser doctor`) | WinApp service cannot start, or no window is attached |
| `platform_unsupported` | — | Not running on Windows |

Once steps run, any error is a failure. Each scenario keeps its own replay
evidence run under `.teshi/logs/replay-screenshots/`.

### Project root

Relative paths given to `teshi run` are rooted at the working directory. For an
absolute path, the project is the nearest ancestor containing `teshi.toml` or
a `.teshi` directory; the search stops at the user's home directory, so a
stray `~/.teshi` never captures Features below it.

### Every scenario routes to exactly one runner

Before anything starts, every selected scenario is resolved. A scenario that
matches no rule, more than one rule, or cannot be read rejects the whole run
with a list of offending scenarios. Nothing is guessed, and there is no
fallback runner.

Use `teshi run --explain` (or `--explain --json`) to print the decision for each
scenario without executing anything.

Matched scenarios are grouped into one batch per rule, in order of first
appearance, and the batches run sequentially. The combined stream has one
`start_run`, one aggregated `end_run` and every case receives a terminal
event: a case its runner never reported is failed, and cases left behind by a
cancellation are skipped.

### Removed configuration

The following are rejected with a migration message instead of being
translated:

- the `[runner]` section of `teshi.toml`
- `TESHI_RUNNER_CMD`, `TESHI_RUNNER_ARGS`, `TESHI_RUNNER_CWD`
- implicit selection of the TUI runner from `.tui.json` sidecars, implicit
  dispatch of `@api` scenarios, Python engine auto-detection and daemon hand-off

`--runner-cmd` (with `--runner-arg`, `--runner-cwd`) still exists for ad-hoc
runs: it sends every selected scenario to one external v1 runner and ignores
`[[runners]]`.

In the TUI a migration or rule error is shown when a run is requested, so a
project with an old configuration still opens. A `teshi.toml` that cannot be
read or parsed still blocks opening the project.

## Protocol v1

### Request

Teshi writes one JSON line to the runner's stdin:

```json
{"command":"run","protocol_version":1,"cases":[{"id":"f0:s1","feature_path":"/abs/login.feature","scenario":"Successful login","line_number":12,"until_line":15}],"meta":{"project_root":"/abs","runner_name":"backend","evidence_run_directory":"/abs/.teshi/logs/replay-screenshots/<run>"}}
```

`case.id` is opaque; echo it back unchanged.

### Handshake

The first protocol event on stdout **must** be `start_run` with the protocol
version and the optional event families the runner will emit:

```json
{"type":"start_run","total":1,"protocol_version":1,"capabilities":["steps","artifacts"]}
```

| Capability | Unlocks |
|------------|---------|
| `steps` | `start_step`, `end_step` |
| `artifacts` | `artifact` |
| `blocked` | `case_blocked` |
| `http_exchange` | `http_exchange` |

Unknown capability names are ignored. A runner without optional events
declares `"capabilities": []`.

Only stdout lines that are JSON objects with a string `type` are protocol
events. Other lines are kept in `stdout.ndjson` as diagnostics.

A run is rejected as a **protocol violation** when:

- the first protocol event is not `start_run`,
- `protocol_version` is missing or not `1`,
- `capabilities` is missing, or
- an event from an undeclared family is emitted.

On a violation Teshi records a `protocol_error`, stops the runner process tree
and ignores its remaining output. A runner that exits without any handshake is
reported the same way.

### Events

```json
{"type":"start_case","case_id":"f0:s1","name":"Successful login"}
{"type":"start_step","case_id":"f0:s1","step_id":"f0:s1:L13","text":"I log in"}
{"type":"end_step","case_id":"f0:s1","step_id":"f0:s1:L13","status":"passed"}
{"type":"case_passed","case_id":"f0:s1","duration_ms":1234}
{"type":"case_failed","case_id":"f0:s2","duration_ms":567,"error":{"message":"Assertion failed","stack":"...","attachments":[{"kind":"screenshot","path":"/abs/a.png"}]}}
{"type":"case_skipped","case_id":"f0:s3","reason":"Not implemented"}
{"type":"case_blocked","case_id":"f0:s4","reason":"browser session unavailable","category":"target_unavailable"}
{"type":"log","case_id":"f0:s1","message":"..."}
{"type":"end_run","passed":1,"failed":1,"skipped":1,"blocked":1}
```

### Blocked

`case_blocked` means the scenario could not be judged because its target was
unavailable (no browser session, no window, missing credentials). It is not a
pass and not an assertion failure. `category` is a short machine-readable
reason such as `target_unavailable` or `credentials_unavailable`.

## `teshi run` output

`teshi run` prints the merged stream of all batches as protocol v1 NDJSON on
stdout, so its output can itself be consumed as a v1 runner stream:

- it opens with one `start_run` declaring every capability,
- runner events are forwarded, including runner-specific events Teshi does not
  interpret (for example `end_example`),
- it closes with one aggregated `end_run`, then `runner_exit`
  (`{"type":"runner_exit","success":false}`) or `runner_error`
  (`{"type":"runner_error","message":"..."}`) when a runner failed to run or
  violated the protocol.

Runner stderr is forwarded as `log` events. Teshi's own errors go to stderr.

Exit codes:

| Exit code | Meaning |
|-----------|---------|
| `0` | Every scenario passed |
| `1` | A scenario failed, a runner errored, or the run did not complete |
| `3` | No failures, but at least one scenario was blocked |

## Evidence

Each external batch writes its own run under
`.teshi/logs/replay-screenshots/<run>/` with `stdout.ndjson`, `stderr.txt`
and the recorded events, including `runner_handshake` and any
`protocol_error`. See [replay-evidence.md](replay-evidence.md).
