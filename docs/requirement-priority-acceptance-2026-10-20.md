# Requirement priority (P6) — 2026-10-20

The user resumed P6 and explicitly approved the design on 2026-10-20. The
previous deferral no longer applies. This is a separate continuation from the
original accepted handoff candidate, based on local HEAD `fc0687b`.

## Behavior

- Optional scheduling priority: P0 immediate attention, P1 next planned work,
  P2 normal backlog, P3 deferred improvement. Existing documents remain unset.
- The Requirements tree displays textual badges and keeps its existing order.
  Click the document's `Priority: Unset` / `Priority: P1` header control, or
  press `p` in the tree. Use arrows and Enter or click a picker value. Unset
  clears the value; Esc or Cancel leaves it unchanged.
- The engine persists priority under the store lock with an expected-value
  check. The picker retains its original expectation across disk reloads.
  Failed writes retain the picker for retry. Generic index and Markdown saves
  preserve the persisted priority instead of overwriting it from stale snapshots.
- Priority changes preserve body, H1, ID, path, iteration, revision and unsaved
  editor text. Imports retain source priority for new documents and target
  priority when reusing an existing document. CLI list/show JSON includes
  `priority`; plain show continues to return the original Markdown.

## Evidence

Evidence directory: [p6-priority-20261020](../.teshi/artifacts/p6-priority-20261020/).
The real-terminal runs use `bin/teshi.exe`, SHA256
`6e0401ceaa0fb9ee3996a21f80fe8a5dbe2f5fdf99d71ff38b3cacbebb10ee8d`.
Final validation records live in `final/`, including source fingerprints in
`source-before.json` and `source-after.json`, plus executable identity in
`binary.json`. Earlier root-level results belong to the preceding candidate.

- P6 model/engine/TUI tests: 12 passed in `final/test.log`, including invalid/legacy metadata,
  import, concurrent writers, stale picker after reload, persistence failure and
  stale generic saves and external deletion changing selection. The earlier
  11-test focused run remains in `priority-unit.log`.
- P6 real Windows PTY scenarios: 4/4 passed at 150×30 and 55×16
  (`final/regression.log`). This covers mouse/keyboard, cancel, clear, restart,
  dirty-body preservation and persistence failure/retry.
- Affected real Windows PTY regression: 32/32 passed (`final/regression.log`):
  creation 3, deletion 1, title 1, pointer controls 19 (including P6), Vim 8.
- Initial PTY failures remain in `pty-initial.log` and `pty-initial/`. They
  exposed title overlap at narrow width and pane handling consuming header
  clicks; both were fixed before the final replay.

Rendered P6 views: [normal width](../.teshi/artifacts/p6-priority-20261020/final/pty/pointer/priority-roundtrip-normal/priority-p1-after.svg)
and [narrow width](../.teshi/artifacts/p6-priority-20261020/final/pty/pointer/priority-roundtrip-narrow/priority-p1-after.svg).

All eight final native workspace and independent runner gates pass and are
recorded in `final/gates.json`: native fmt/check/test/Clippy/doc (excluding the
WASM-only `teshi-web` crate), runner fmt/test/Clippy. Native tests and doctests:
1,313 passed, 5 ignored. Runner regular tests: 23 passed, 118 ignored. Ignored
tests are not counted as executed acceptance; the 32 selected real PTY tests
were executed explicitly. Existing rustdoc warnings remain recorded.
The initial workspace compile exhausted Windows virtual memory; serial retry
logs preserve that failure separately. The existing default CLI executable was
in use, so real-terminal acceptance used the independently built executable.
The original executable was restored with its original hash and its process
remains alive (`final/restoration.json`). Final source and pinned executable
hashes are unchanged across the complete final validation; `final/summary.json`
reconciles the results without combining earlier candidate runs.

## Scope

These results cover Windows native/TUI behavior. Linux/macOS and Desktop/Web
rendering were not replayed for P6. The old complete deterministic Feature
matrix belongs to its original candidate and is not recertified by this run.
Live CodeBuddy acceptance and P4 aesthetic acceptance remain separate.
No real requirement-library values were assigned. Validation made no commits or
pushes. After explicit user authorization, the implementation, tests and records
are delivered as one local commit; no push was performed. The existing
`Cargo.toml` status and five-item `TODO.md` were preserved.
