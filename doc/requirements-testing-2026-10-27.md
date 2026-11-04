# Requirement testing record — 2026-10-27

Baseline: `f271c4a`, with the existing uncommitted implementation described in
[the delivery record](requirements-delivery-2026-10-27.md). This is scoped Windows
validation, not full workspace, cross-platform, or requirement acceptance.
The real user requirement library and its delivery statuses were not modified.

## Executed coverage

| Layer | Result | Evidence |
| --- | --- | --- |
| Rust library tests: agent/core/engine/TUI | 61 / 135 / 274 / 501 passed; 2 TUI tests ignored | `cargo test -p teshi-agent -p teshi-core -p teshi-engine -p teshi-tui --lib --locked`; final TUI rerun includes the new scrolling regression |
| Creation Feature, English | 5 passed | `.teshi/feature-results/requirements-20261027/creation-en-final.json` |
| Creation Feature, Chinese | 5 passed | `.teshi/feature-results/requirements-20261027/creation-zh-final.json` |
| Real PTY requirement creation | 3 passed in the final combined run | `pty-final.log`; `pty-current-binary/after-create.txt`, `.svg`, `saved-index.json` under the evidence directory |
| Real PTY interaction/layout | 6 passed in the final combined run | `pty-final.log`; `pty-layout-current-binary/interaction/` |
| Python WinApp contracts and transport | 148 passed | `python -m unittest discover -s resources/tests -p 'test_winapp*.py' -q` |
| Scoped Rust Clippy, all targets | Passed with `-D warnings` | `clippy-final.log`; agent/core/engine/TUI/requirement CLI runner |
| Separate PTY runner Clippy, all targets | Passed with `-D warnings` | `runner-clippy.log` |
| Workspace and separate runner formatting | Passed | `cargo fmt --all --check`; runner manifest formatting check |

The evidence directory is `.teshi/feature-results/requirements-20261027/`.
`source-manifest.json` records HEAD, dirty-source SHA-256 values, and the binary
SHA-256 used for the final combined PTY run. Test artifacts are local ignored files;
the source and this document remain uncommitted.

The CLI runner launches the checkout binary in child processes against isolated
temporary stores. It checks complete Unicode file bodies, JSON metadata, explicit
planned project membership, absent implicit membership, stdin, concurrent IDs and
paths, tombstone reservation, and rollback after a blocked atomic index write.
Deletion is fixture preparation through the engine; creation assertions cross
the actual CLI boundary. The runner rejects unsupported steps.

Added unit coverage checks timestamp syntax, malformed-index rejection, tombstone
preservation, write rollback, built-in versus custom Default provenance, retained
elapsed time, unknown context usage, and the seventy-percent warning boundary.
New WinApp contracts cover owned handles, PID reuse, stop timeouts, checker
calibration, nonzero exits, blocked results, and timeout evidence. These contracts
do not substitute for native-window or three-Agent acceptance.

## Failures found and corrected

- Six original TUI library failures: updated timestamp and deleted-step expectations,
  adapted the scrollbar fixture to the compact-layout boundary, and restored Feature
  title diffs alongside individually selectable Step changes. Added a regression
  for selection visibility and pointer rows after scrolling past the title diff.
- Creation Feature first run: 2 passed, 3 failed because the new runner assumed a
  nested project-status object and an `ok` field in diagnostics. The established
  contract uses a status string and `code`/`error`. The original report is retained
  as `creation-en.json`; reruns use distinct names.
- The stronger PTY persistence assertion initially observed 29 saved documents
  after accepting an old success message. It now waits for the actual 30-document
  index and verifies every timestamp-named file body.
- WinApp full-suite runs reproduced temporary-runtime cleanup racing executor
  exit. Cleanup now retries file sharing violations for at most five seconds,
  still failing if the runtime stays locked. The final 148-test suite passed.
- Interaction tests used old fixed pane borders and expected all panes on a narrow
  screen. Updated them to observe the rendered border and focused-pane behavior,
  with explicit waits after modal, input-mode, and focus changes. Initial viewport
  evidence remains separate from rerun evidence.

## Remaining acceptance gaps

| Requirements | Remaining work |
| --- | --- |
| doc-23 | Feature replay passes in both locales; no user-library acceptance status changed |
| doc-24 | Timestamp and persistence contracts plus actual TUI creation covered; deterministic equal-clock/rollback and legacy-link Feature replay still missing |
| doc-25–27, doc-29 | Focused unit coverage added; dedicated Feature runner and ACP/provider PTY scenarios still missing |
| doc-28 | Product decision pending; title policy remains unimplemented and unaccepted |
| doc-30 | Dedicated external-add/change/delete/deferred-run/scan-error Feature replay still missing |
| doc-31, doc-33–36 | Scoped real PTY layout/navigation/pointer checks executed; the newly authored individual Features still need runner wiring and per-scenario replay |
| doc-32 | Existing diff tests and new long-list selection/hit-target regression pass; its authored Feature still needs replay wiring |
| doc-20–22 | New Python contracts pass; executor-restart/native-window/Steam workflows and three-Agent acceptance remain unverified |

The Feature catalog check fails: 32 new bilingual files lack execution-owner tags;
the pre-existing bilingual replay-screenshot Features also lack binding sidecars.
Tags or empty bindings were not added merely to make catalog checks pass. Authored
Features remain distinct from executable bindings and actual replay outcomes.

No full workspace test, all-features Clippy, documentation build, Linux PTY, WebGPU,
or native application acceptance is claimed here. No commit, push, installation,
or modification of an unrelated running application was performed.
