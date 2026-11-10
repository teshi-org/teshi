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
| `cmd` (+ `args`, `cwd`, `env`) | An external process that speaks protocol v1 |

`[runners.match]` must declare `tags`, `paths`, or both; when both are set a
scenario must satisfy both. Use `paths = ["**"]` to match everything.

- **Tags** are compared against the scenario's effective tags (Feature tags
  plus scenario tags), case-insensitively and with or without the leading `@`.
  Any listed tag matches.
- **Paths** are glob patterns with `/` separators, relative to the project
  root. `*` never crosses a `/`; `**` matches any number of directories,
  including none (`**/*.feature` matches `sample.feature`).

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

`teshi run` exits with:

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
