# Project-switch evidence reconciliation — 2026-10-20

Latest acceptance baseline: `40a5894` plus the prior editor-isolation fix. Earlier C evidence retains baseline `5026ab3`. This audit reconciles the original P8 checklist rather than
turning historical empty checkboxes into failure counts. `Verified` below means
the named scope has executable evidence; `Partial` identifies a missing variant,
not a confirmed product failure. Dated reports remain immutable.

Evidence roots (relative to the checkout):

- **D**: `.teshi/artifacts/p7-p8-delivery-20261019/`. Windows/Linux complete
  snapshots each pass 38 deterministic Features / 244 scenarios. Original
  project-switch Features contain 17 scenarios per locale.
- **R**: `.teshi/artifacts/p8-review-20261020/`. The subsequent paired failure
  Features pass 12 cases per OS, including permission/review cancellation.
- **C**: `.teshi/artifacts/p8-reconcile-20261020/`. Current editor-history fix,
  before/final logs, pinned binaries and separate focused reports. Its replay
  results must be read from `*-gates.json` / `*-matrix/matrix.json`; do not add
  its cases to D and label that sum a single complete replay.
- **S**: `.teshi/artifacts/p8-sessions-20261017/`. Earlier session/association
  reproductions and persistence assertions; these scenarios are retained in D.

- **E**: `.teshi/artifacts/p8-boundaries-20261020/`. Both OSes pass 14 new
  boundary scenarios and one complete 40-Feature / 266-scenario deterministic
  replay, with unchanged sources and executable hashes. Native and independent
  runner gate details are in `summary.md` and `verified-summary.json`.

## Original P8 task accounting

All 36 original task bullets are accounted for below. The six formerly partial
rows now have scoped evidence for the five previously open categories. This is
accounting within the declared Windows/Linux contract, not a claim covering
arbitrary Unix escapes, every filesystem or every byte of the dirty checkout.

| Original item | Result | Source / executable evidence / limit |
| --- | --- | --- |
| P8.1 normalized identity and generation | Verified | Engine `project_identity.rs`; same-project and generation tests; D A/B/A |
| P8.1 path aliases, casing and symlinks | Verified within supported scope | E actual relative/dot aliases; Windows junction and existing case alias; Linux symlink and distinct case-sensitive roots. Alias activation preserves dirty A without confirmation; no macOS or case-sensitive Windows volume claim |
| P8.1 reuse engine/app-data contracts | Verified by source | Shared `ProjectIdentity`, recent-entry service; no desktop teardown dependency |
| P8.1 cohesive context and ownership inventory | Verified by source; regression fixed in C | `PreparedProjectContext`, switch state and `doc/project-switch-contract.md`; C catches old undo/redo/source-view leakage |
| P8.1 root/cwd audit | Verified for delivered paths | Explicit runner/backend/project roots; D root-fix regression and A/B cwd traces. No claim for unrelated later WinApp changes |
| P8.2 recent entry metadata and legacy migration | Verified | `app_data` migration/dedup/metadata units; timestamped entries |
| P8.2 locking/atomic persistence | Verified for recorded cases | Shared exclusive lock/atomic writer; concurrent-migration regression. No multi-process stress claim |
| P8.2 write history after activation; warn on write failure | Verified within supported scope | E history destination blocked by an owned directory; B activates with the persistence warning and unchanged B source |
| P8.2 header name/full path and duplicate labels | Verified within supported scope | E equal display names, both paths visible in the picker, pointer selection of B path, B source viewport and unchanged A/B files; D retains narrow/wrapped path coverage |
| P8.2 picker/open/cancel/keyboard/pointer/same-project | Verified | D original scenarios 0–4 and narrow picker; R empty-history picker; same-project state unit |
| P8.2 missing/unreadable targets and invalid history | Verified within supported scope | D missing target/corrupt config; corrupt-history unit. E invalid UTF-8 plus an unreadable nested source (Windows exclusive handle; Linux denied directory with DAC bypass removed), recovery and retry |
| P8.3 one pending request and stable dirty objects | Verified | `PendingProjectSwitch`, `DirtyProjectObject`, request reentrancy check and dirty-state units |
| P8.3 save/discard/cancel and save failure | Verified for the listed dirty object types | D Feature cancel/discard and R save recovery; E independently blocks requirement Markdown and test-point JSON persistence, cancels without losing edits, restores destinations and retries; changed test points remain Proposed |
| P8.3 unapplied Agent proposals | Verified for recorded scope | Proposal-preservation unit; R Proposed test-point human-review replay. No implicit approval |
| P8.3 task inventory | Verified by source for TUI paths | Contract inventory: runner, native/ACP/bridge, generation/exploration, synchronous dispatch and read-only Git worker |
| P8.3 cancellation ownership per task | Verified within supported containment | D runner PID/cwd/stable writes, native socket closure, ACP ordinary-child shutdown; R dispatch timeout/retry. Deliberate Unix group escape is explicitly unsupported |
| P8.3 stop/cancel/bounded timeout | Verified | D/R real timeout and retry plus state units; C retains ordinary shutdown integration checks |
| P8.3 only owned processes stopped | Verified for recorded fixtures | D unrelated-process survival and PID-exit evidence; no executable-name selection |
| P8.4 detached B construction | Verified by source and failure replay | `prepare_project_context`, D malformed-config and discard-failure cases |
| P8.4 missing/unreadable/corrupt/parse-failure handling | Verified within supported scope | E try_parse_project fails on traversal/file/UTF-8 errors and preserves A edits/history/generation. Incomplete Gherkin remains intentionally editable; syntax errors are not mislabeled as fatal filesystem reads. D retains malformed target-config coverage |
| P8.4 no external side effects during preparation | Verified by source | Candidate builds model/buffers/config before installing/starting target agents; D failed-prepare source assertions |
| P8.4 replace context/increment generation/rebuild views | Verified for recorded fields; fixed in C | D A/B/A and unit stamps; C resets old editor history, source coordinates and Explore state only on success |
| P8.4 explicit runner/config/tool roots and overrides | Verified for recorded paths | D runner cwd/config and backend context; CLI explicit overrides remain session-wide |
| P8.4 reject old identity/generation results | Verified | `ProjectSwitchState` units; stamped Git refresh; deferred-history-restore binding. Runner/Agent work is quiesced rather than relabeled |
| P8.4 mutation callbacks retain old roots | Verified for recorded tasks | D ACP/runner writes; R controlled synchronous dispatch completes against A before B activation |
| P8.5 filter sessions/resume by project | Verified | D scenarios 13–14; S persistence assertions; ownership units |
| P8.5 explicit Legacy/Unassigned view | Verified | D scenario 15; no implicit assignment |
| P8.5 reject foreign/rootless restoration/checkpoint | Verified | S reproducer and ownership units retained in native suite |
| P8.5 checkpoint A/reset B backend/history/UI | Verified | D/R process/context/prompt traces and session replay; C editor-history correction |
| P8.5 preserve global store/reload local links | Verified | D scenario 16 and file-hash assertions; shared-root unit |
| P8.5 clear local link selection/highlight | Verified | Link-reload unit and D A/B/A association scenario |
| P8.6 source acceptance scenarios have executable carriers | Verified for original paired Features | D 34-case original switch replay and R failure additions; no claim that these cover every variant in this plan |
| P8.6 negative save/timeout/corrupt/stale/duplicate-name flows | Verified within supported scope | D/R save/timeout/corrupt product replays and stale-event units; E closes duplicate-name pointer and additional requirement/test-point persistence-failure flows |
| P8.6 deterministic A/B filesystem markers | Verified | D/R traces, cwd, process exit, saved A bytes and unchanged B bytes |
| P8.6 real keyboard and pointer terminal flows | Verified | D original paired Features; R empty-history and permission-modal pointer cases |
| P8.6 architecture/lifecycle documentation | Updated in C | `doc/project-switch-contract.md` includes field ownership, activation reset and supported shutdown boundary; temporary compatibility callers remain visible in source |

## Decisions and remaining acceptance work

The user explicitly selected ordinary Unix descendant support on 2026-10-20.
Deliberate `setsid`/process-group escape is now an **accepted unsupported
boundary**, not unfinished cgroup/sandbox implementation. C's
`unix-boundary.json` is a self-cleaning mechanism probe only, not ACP product
acceptance. The ordinary ACP shutdown integrations supply the product evidence.

All five previously open categories now have E evidence:

1. Unreadable traversal and file loading fail before activation. Old-binary
   before runs visibly activate B despite invalid encoding; the new runs retain
   A's unsaved edit, show the path error and support recovery/retry.
2. A recent-history write error leaves B active with a warning.
3. Identical display names remain distinguishable by full path and pointer selection.
4. Supported Windows junction/case aliases and Linux symlinks preserve A's dirty
   state; relative/dot identities and Linux case-sensitive separation have units.
5. Requirement and test-point saves independently fail, retain edits on cancel,
   and save successfully on retry. Test-point meaning changes remain Proposed.

P6 stays deferred. Two live CodeBuddy Features stay blocked without opt-in.
P4 user aesthetic acceptance remains pending. These are separate from the P8
variants above. Local commits and old full snapshots do not certify the entire
dirty main checkout or imply a push. This plan is not ready for archival.
