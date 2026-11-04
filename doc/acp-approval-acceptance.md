# ACP approval command acceptance

Requirement: `doc-41` (ACP approvals through a slash command).
Validated on Windows on 2026-11-03, using the working tree based on `2f9547a`.

## Behavior

- In ACP conversations, `/approval`, `/approve`, and `/app` open the agent's
  advertised approval/permission settings. Advertised session modes are the
  fallback when no explicit approval selector exists.
- Choices and current values come from the owning remote session. Modern
  settings use `session/set_config_option`; legacy modes use `session/set_mode`.
- Discovery may finish after the command opens. Multiple approval selectors
  remain separately selectable. Rejection retains the confirmed remote value.
- An agent without approval settings or modes gets an explicit unavailable
  message. Unknown setting types remain unsupported rather than acquiring
  fabricated values.
- `/approval local` retains Teshi's Manual/Auto/Bypass picker. Native `/approval`
  and the local policy badge retain their existing behavior.

## Executed evidence

The English and Chinese Features are bound by
`scripts/run-tui-sidecar-feature.py` and executed through the real ConPTY runner.
Both locales passed all five scenarios: successful modern mutation, rejected
mutation, legacy modes, missing capability, and local/remote policy separation.
They assert visible terminal state after reopening the picker and clean exit.

Reports under `.teshi/feature-results/acp-approval-20261103/`:

| Report | Result |
| --- | --- |
| `en-final.json` | 5/5 passed |
| `zh-final.json` | 5/5 passed |
| `live-codex-result.json` | 1/1 passed against the installed Codex ACP agent |

The live test opened `/approval`, observed the agent's Read-only, Workspace
access, Auto review, and Full access modes, selected Read-only, reopened the
picker, and verified Read-only was confirmed. It used an isolated project and
Teshi stores, sent no model prompt, and executed no agent tools. The confirmed
viewport is stored at
`.teshi/logs/replay-screenshots/6b727520-5adf-4a07-ba32-1184711d7a27/frame-BRn0Iv.png`.
Other real ACP agents were not exercised.

Earlier failures are retained as `en-first.json`, `en-second.json`, and
`en-third.json`. They exposed Windows extended-path comparison and shared
evidence-directory handling in the adapter, then assertions that incorrectly
expected wrapped terminal messages to occupy one line. The adapter and
assertions were corrected before the final passing replays.

Additional checks passed:

- Requirement Feature syntax checks for both locales: zero diagnostics.
- `cargo test -p teshi-acp -p teshi-agent-runtime -p teshi-tui --lib --locked -- --test-threads=1`:
  651 passed, 5 ignored.
- `cargo test -p teshi-agent-runtime --test acp_backend --locked -- --test-threads=1`:
  17 passed, including pending permissions, session isolation, and mutation errors.
- Python sidecar adapter tests: 18 passed.
- Scoped strict Clippy, native workspace check excluding `teshi-web`, formatting,
  and whitespace checks passed. No full workspace test suite, WASM validation,
  or Linux terminal replay is claimed.

The tested CLI was installed as `0.7.10-local.20261103.1601+0800 (2f9547a)`.
Cargo's replacement attempt failed because the existing executable was in use.
The previous executable was retained beside it, and the built binary was copied
to the normal launch path with matching SHA-256 verification. All four existing
Teshi processes were preserved. New launches use the new build; existing
conversations keep running their previous build.

## Reproduction

Build the CLI and `crates/teshi-tui-runner` with `--locked`. Set
`TESHI_SELF_TEST_BIN` to the built CLI and `TESHI_GENERIC_TUI_RUNNER` to the built
runner, then execute each locale:

```text
teshi run features/en-US/tui_acp_approval.feature --runner-cmd python --runner-arg scripts/run-tui-sidecar-feature.py --report en-approval.json
teshi run features/zh-CN/ACP斜杠审批.feature --runner-cmd python --runner-arg scripts/run-tui-sidecar-feature.py --report zh-approval.json
```
