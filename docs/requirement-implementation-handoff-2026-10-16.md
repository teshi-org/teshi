# Requirement implementation handoff — engineering closeout

Closed: 2026-10-20. Original planning date: 2026-10-16.

The engineering handoff is closed for the accepted scoped candidate. P0–P5 and deterministic P7–P9 delivery are recorded below; P6, live-provider acceptance, and user aesthetic acceptance are carried forward explicitly. This is not a claim that every original requirement or the combined dirty checkout has passed acceptance.

This document is the final status record. The [historical handoff records](requirement-implementation-handoff-2026-10-16-history.md) preserve the previous document in full, including original task contracts, failed attempts, intermediate results, and old checklists. Those historical instructions do not reopen completed work.

## Accepted scope and evidence

The final isolated candidate is based on `40a5894` plus the prior scoped editor-isolation fix. It excludes deferred P6 and unrelated later WinApp work. Evidence belongs to the recorded candidate, source fingerprints, and pinned executables, rather than to every file currently present in the main checkout.

| Platform | Complete deterministic Features / scenarios | Native tests passed / ignored | Independent runner tests passed / ignored |
| --- | --- | --- | --- |
| Windows | 40 / 266 | 1,294 / 4 | 23 / 113 |
| Linux | 40 / 266 | 1,291 / 4 | 23 / 112 |

Both platforms passed native fmt/check/test/Clippy/doc and independent terminal-runner fmt/tests/Clippy. Python contracts and the 47-Feature catalog passed. Ignored tests are not counted as executed acceptance. Source and pinned executable fingerprints remained unchanged during each complete deterministic replay.

Primary evidence:

- [Final five-boundary acceptance summary](../.teshi/artifacts/p8-boundaries-20261020/summary.md), with `verified-summary.json`, `*-full-matrix/matrix.json`, and `*-full-execution.json` in the same evidence directory.
- [Original 36-row project-switch evidence reconciliation](project-switch-evidence-reconciliation-2026-10-20.md).
- [Project-switch ownership and shutdown contract](../doc/project-switch-contract.md).
- Earlier P3/P4/P5/P7 acceptance and exact binary/platform limits remain indexed in the [historical records](requirement-implementation-handoff-2026-10-16-history.md).

## Final package inventory

| Package | Current result | Remaining boundary |
| --- | --- | --- |
| P0 | Isolated candidate and exact source/binary records; live source doc-2 revision `8b413fffee3e58df` and doc-6 revision `dd7dab4d004f9b93` reconfirmed read-only | Keep final replay identity separate from the remaining dirty checkout |
| P1/P2 | Creation/title/restart/deletion regressions retained in the earlier scoped PTYs and current native suites | No new implementation gap identified in this continuation |
| P3 | Windows clipboard/selection and Linux X11 acceptance retained | No Wayland or post-exit clipboard durability claim |
| P4 | Scoped pointer/modal acceptance retained | User aesthetic acceptance remains pending |
| P5 | Documented bounded Vim subset retained | Full Vim grammar and unclaimed platform/input paths remain outside the delivered subset |
| P6 | Deferred by the user | No design approval or delivery claim; priority changes remain outside the candidate and commits |
| P7 | Deterministic work committed; six paired Features / 64 scenarios pass; native and runner acceptance retained | Two real CodeBuddy Features remain blocked without explicit opt-in |
| P8 | All 36 original task bullets have scoped evidence; original switching 34, prior failure boundaries 14 and new persistence/identity boundaries 14 all pass on each OS | Scoped implementation and acceptance complete. Deliberate Unix group escape remains explicitly unsupported by user decision; no universal filesystem/platform claim |
| P9 | Both native workspace gate sets, independent terminal-runner gates, 47-Feature catalog and complete 40-Feature / 266-scenario deterministic replays pass on the final candidate | Deterministic engineering validation complete. Real-provider/user acceptance and excluded external/rendering targets retain their separate status; no whole-dirty-tree claim |

## Carried-forward items

These three items remain open independently of the closed engineering handoff. None is marked as passed or waived.

| Item | Status | Next action and completion condition |
| --- | --- | --- |
| P6 requirement priority | Deferred by the user | Resume only after explicit priority design approval; implement and validate as a separate change. Existing priority edits remain outside the accepted candidate. |
| Two real CodeBuddy Features | Blocked pending explicit live-provider opt-in | After authorization, run the two real-provider Features with isolated stores and record provider/runtime identity, actual results, and evidence. Deterministic ACP fixtures do not substitute for this acceptance. |
| P4 appearance | User aesthetic acceptance pending | Present the recorded normal/narrow rendered views for user review. Record the user's judgment; any requested visual changes receive their own implementation and validation. Pointer/modal test success does not imply aesthetic approval. |

Unsupported or unexercised platform paths remain scope limits, not new work automatically assigned by this closeout: full Vim grammar, Wayland and post-exit clipboard durability, macOS, deliberate Unix process-group escape, and Linux Desktop/Web rendering. External browser/WinApp results retain their own snapshot and target scope; they are not included in the 40-Feature deterministic count.

## Delivery state and future work

Earlier scoped commits remain identified in the historical records. The accepted code delivery is committed locally as `ebbd898` (12 files), including the prerequisite editor-isolation fix and five-boundary changes. This documentation closeout is delivered in a separate documentation commit. No push has been performed. Unrelated dirty changes and OpenSpec deletions remain preserved. Tests did not mutate the real requirement store.

Code delivery remains a separate operation: reconcile the accepted candidate against the intended staged diff, preserve unrelated changes, and obtain separate commit/push authorization. Do not certify the entire dirty checkout from isolated-candidate evidence.

Future work starts from one of the three carried-forward items or a separately requested delivery action. Do not restart the five completed P8 boundary categories or rerun the full accepted matrix solely because historical checkboxes remain unchecked. Rerun affected validation when a subsequent code change invalidates its evidence.

## Five P8 boundaries and complete deterministic replay — 2026-10-20

Evidence: `.teshi/artifacts/p8-boundaries-20261020/summary.md`, `verified-summary.json`, `*-full-matrix/matrix.json` and `*-full-execution.json`. Source candidate: `review/`, baseline `40a5894` plus the prior scoped editor fix. Each full replay has unchanged source and executable fingerprints; native validation records the same source snapshot. These complete runs replace no historical failures and are not assembled from partial reports.

- `try_parse_project` makes project-switch preparation fail on unreadable directories/files or invalid UTF-8, with the affected path in the error. It never returns a partial project. A's dirty buffer, undo history and project generation survive failed preparation. Directory aliases are cycle-checked; incomplete Gherkin remains editable.
- The old pinned CLI fails the English invalid-encoding before case on each OS: Discard activates B and silently omits the bad file. The viewport and failed assertion supply the reproduction; the CLI's nonzero runner completion is retained separately. The new paired boundary Features pass 14/14 on each OS, including actual read restriction/recovery (Windows owned exclusive handle; Linux denied nested directory with DAC bypass removed).
- History write failure leaves B active with a warning. Equal names are selected through distinct rendered paths. Windows junction/case and Linux symlink aliases keep A's unsaved edit without confirmation. Engine units cover actual relative/dot aliases and distinct case-sensitive Linux roots.
- Requirement Markdown and test-point JSON persistence fail independently; cancelling retains the unsaved viewport state. Restoring the destination and retrying saves to the shared requirement store or A's point file. The changed test point remains Proposed; no human approval or review continuation is synthesized. B source bytes remain unchanged.
- One complete deterministic replay per OS passes 40 Features / 266 scenarios, zero failures/skips/blocks in this scope. All 62 project-switch scenarios (34 original + 14 prior failure + 14 new boundaries) are included. Real CodeBuddy and external Web/WinApp Features are excluded explicitly, not counted as passes.
- Both native fmt/check/test/Clippy/doc sets pass. Windows native tests/doctests: 1294 passed / 4 ignored; Linux: 1291 passed / 4 ignored. Independent runner regular tests pass 23 / 23, with 113 / 112 ignored respectively; selected product behavior is represented by the actual Feature reports, not those ignored counts. Runner formatting/Clippy, Python contracts and the 47-Feature catalog pass. Syntax has zero errors/warnings and two nonblocking scenario-length suggestions per new locale Feature. Existing rustdoc warnings remain recorded.
- All 36 original P8 task bullets now have explicit scoped accounting. No P8 implementation/evidence category from the five-item queue remains open. Deferred priority, real-provider and aesthetic decisions retain the status above. No commit/push or real requirement-library mutation occurred; unrelated dirty files, processes, outputs and worktrees are preserved.
