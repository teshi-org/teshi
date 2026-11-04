# Historical handoff records — archived 2026-10-20

This is a verbatim snapshot of the handoff before documentation closeout. All status statements, unchecked task lists, and next-agent instructions below are historical. They are not an active work queue. Use the [closed engineering handoff](requirement-implementation-handoff-2026-10-16.md) for final status and carried-forward items. Failures, intermediate runs, and original acceptance contracts remain preserved for traceability.

---

# Requirement implementation handoff

Planning date: 2026-10-16. Intended implementer: a subsequent agent, including Luna Max.
Baseline checkout: `D:/Dev/Rust/teshi/dev`, initial HEAD `ec830c4`, with substantial existing uncommitted changes.

Latest continuation (2026-10-20, five-boundary acceptance): all five previously open P8 categories now have executable Windows/Linux evidence. The candidate is based on `40a5894` plus the prior editor-isolation fix, excluding deferred P6 and unrelated WinApp changes. Each OS passes the new 14-case pair and one complete immutable deterministic replay of 40 Features / 266 scenarios. Native workspace and independent runner gates pass with unchanged source fingerprints. P8 scoped implementation/acceptance and P9 deterministic gates are complete; P6 remains deferred, real CodeBuddy two Features remain blocked without opt-in, and user aesthetic acceptance remains pending. No commit or push; the combined dirty checkout is not certified.
Status (2026-10-19, P7/P8 scoped delivery): P7 deterministic session/provider-error work is committed as `0ba2032`; ACP process-container ownership and paired failure Features are committed as `c8ae768`; CLI validation/replay root isolation is committed as `6a7d8dc`. The isolated candidate excludes deferred P6 and later WinApp features. Windows and Linux native fmt/check/test/Clippy/doc pass; Windows independent runner gates pass. Windows tests/doctests: 1,285 passed / 4 ignored; Linux: 1,280 passed / 4 ignored under the recorded capability constraint. The 45-Feature catalog passes. Each OS now has one complete immutable replay of 38 deterministic Features / 244 scenarios, with unchanged source fingerprints. A subsequent empty-history picker correction (`1798507`) passes eight scoped PTY cases plus affected units/fmt/Clippy on each OS; it is not relabeled as a complete 246-case replay. Two live CodeBuddy Features remain blocked without opt-in, user aesthetic acceptance remains pending, and P6 remains deferred. P8 still has the explicit boundaries below. No push; unrelated working-tree changes remain preserved.


## Authoritative scoped delivery inventory — 2026-10-20

This section supersedes every older “current”, “remaining”, “next work” and unchecked list in this temporary plan. Original checklists retain their execution contract; do not infer missing implementation from an unchecked historical box. The plan is not fully complete or ready for archival.

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

### P7/P8 continuation evidence and corrections

Evidence root: `.teshi/artifacts/p7-p8-delivery-20261019/`. The candidate under `review/` contains the committed baseline plus exactly the delivered changes, without P6 priority or later WinApp shell/tray work. Its source-bound runners retain that source path. Main-checkout output executables and unrelated worktrees/processes were preserved.

- P7: `p7-matrix/matrix.json` passes six Features / 64 scenarios. `p7-affected-tests.log` passes 762 unit/integration/doctests with one ignored; `p7-gates.json` records build, Python contracts, catalog and affected Clippy. The initial extraction mistakenly included a P6-only `priority: None`; it was excluded before acceptance. The initial main-target binary build failed because that output executable was in use; `p7-build-locked-exe.log` is retained. The isolated output build passed without stopping that process. Formatting was corrected using each crate's declared edition; final workspace formatting passes.
- ACP ownership: the old implementation fails `p8-descendant-before.log` because shutdown is not acknowledged within four seconds while an ordinary descendant holds the inherited transport. It is not evidence of a successful old acknowledgement followed by writes. Windows now spawns suspended, assigns a non-breakaway Job before resuming, terminates descendants even after primary exit, and waits for an empty Job. Unix uses a dedicated process group. Shutdown errors retain worker ownership and remain retryable. Both OSes pass all three runtime shutdown integrations and standalone ACP tests/Clippy (`p8-final-gates.json`, `p8-linux-gates.json`). Deliberate Unix `setsid`/group escape is not covered by that mechanism.
- New paired failure Features exercise actual file-save failure and recovery, normal ACP exit with a surviving tool child, and synchronous dispatch stop timeout followed by safe retry. `p8-final-matrix/matrix.json` passes four Features / 40 scenarios on Windows; final fixture-only process-exit/stdlib-transport refinements pass the new six cases in `p8-owned-process-final/` and `p8-linux-pty/`. Evidence includes viewport/SVG, saved A bytes, unchanged B sources, child PID exit, stable writes and a visible five-second timeout while A remains active. The controlled dispatch completes against A before B becomes active.
- Earlier new-Feature attempts remain in `p8-matrix/`, `p8-failure-final/` and `p8-failure-dispatch/`. One save fixture first failed to acknowledge the actual external-change prompt; one controlled API transport initially omitted the required typed response. The original busy-ACP header click also failed to open the picker. A later controlled comparison (`busy-header-pointer.json`) confirms that an empty recent list prevents opening, while seeding a recent B entry opens it on the same binary. `1798507` removes the four-line empty-list early return and retains the existing empty-list rendering/Open path control. The new paired scenario verifies header click, visible empty notice, pointer Open path, actual B activation and unchanged B source. Both OSes pass all eight failure cases plus affected TUI units/fmt/Clippy; see `windows-picker-*` and `linux-picker-*`. The earlier ACP stop scenario remains accepted through the keyboard project command; it is not retroactively counted as the empty-history pointer regression.
- The broader candidate gate exposed pre-existing CLI root drift. `check-root-before.json` reproduces it inside an owned ancestor `.teshi` directory. `integrated-test-root-failure.log` retains the three failing check tests and the two scanning tests terminated through the recorded owned process tree. `integrated-final-test.log` retains the separate replay preflight failure. `6a7d8dc` scopes relative check/steps/browser-replay/WinApp-replay inputs to the caller's project and adds a deterministic ancestor regression. Only the replay-root portion of WinApp was staged; other WinApp edits remain untouched.
- Final Windows gates: `integrated-rootfix-gates.json`; Linux: `linux-integrated-gates.json`. Native tests/doctests pass 1,285/1,280 with four ignored on each OS. Linux tests remove only `CAP_DAC_OVERRIDE` and `CAP_DAC_READ_SEARCH` from the root subprocess bounding set so the permission assertions actually execute. No failing gate is waived. Documentation exits zero with recorded warnings. Ignored product PTYs are represented only by explicitly executed reports, not by the normal Cargo test counts.


### Complete replay identity and subsequent narrow correction

`windows-final-integrated-matrix/matrix.json` and `linux-final-integrated-matrix/matrix.json` each pass 38 Features / 244 scenarios in one run. Their `*-final-matrix-execution.json` files record exit zero and unchanged source fingerprints; the associated binary/source JSON files retain exact hashes. `candidate-vs-commits.json` verifies all 28 changed candidate files against committed source at `c8ae768` (line endings normalized for Git blob comparison). These runs are distinct from the earlier source-changing preliminary matrices and from historical external-target replays.

The later empty-history correction changes only `app.rs`, its paired failure Feature text and that Feature's fixture; `picker-changed-files.json` records those four paths. `picker-source-before.json` and `picker-source-after.json` match. Both OSes pass the eight scoped cases, TUI units and affected formatting/Clippy. The complete workspace/38-Feature snapshot predates this four-line behavioral correction. `verified-summary.json` and `summary.md` preserve that distinction rather than synthesizing a new full replay. P6 and later WinApp changes remain excluded from all these candidate claims.

The source-bound review worktree is retained within the artifact directory so the pinned semantic runners can resolve their exact fixtures. No unrelated worktree was removed. `source-requirements.json` retains the read-only stable-ID/revision refresh; no user-library data was edited.

### Work still open after this scoped delivery

1. P6 remains explicitly deferred; priority design/implementation is not delivered by this candidate.
2. Two real CodeBuddy Features remain blocked without explicit opt-in.
3. P4 user aesthetic acceptance remains pending.
4. The five P8 boundary categories from the prior reconciliation are now closed within the supported Windows/Linux scope. Deliberate Unix process-group escape remains unsupported by explicit user decision. See [the updated 36-row reconciliation](project-switch-evidence-reconciliation-2026-10-20.md).
5. Delivery is local and uncommitted. Later WinApp work, the entire dirty checkout, external browser/WinApp targets, Linux Desktop/Web rendering, Wayland and macOS have no new acceptance claim. This plan is not fully complete or ready for archival while the separate deferred/blocked/user decisions remain.

## Five P8 boundaries and complete deterministic replay — 2026-10-20

Evidence: `.teshi/artifacts/p8-boundaries-20261020/summary.md`, `verified-summary.json`, `*-full-matrix/matrix.json` and `*-full-execution.json`. Source candidate: `review/`, baseline `40a5894` plus the prior scoped editor fix. Each full replay has unchanged source and executable fingerprints; native validation records the same source snapshot. These complete runs replace no historical failures and are not assembled from partial reports.

- `try_parse_project` makes project-switch preparation fail on unreadable directories/files or invalid UTF-8, with the affected path in the error. It never returns a partial project. A's dirty buffer, undo history and project generation survive failed preparation. Directory aliases are cycle-checked; incomplete Gherkin remains editable.
- The old pinned CLI fails the English invalid-encoding before case on each OS: Discard activates B and silently omits the bad file. The viewport and failed assertion supply the reproduction; the CLI's nonzero runner completion is retained separately. The new paired boundary Features pass 14/14 on each OS, including actual read restriction/recovery (Windows owned exclusive handle; Linux denied nested directory with DAC bypass removed).
- History write failure leaves B active with a warning. Equal names are selected through distinct rendered paths. Windows junction/case and Linux symlink aliases keep A's unsaved edit without confirmation. Engine units cover actual relative/dot aliases and distinct case-sensitive Linux roots.
- Requirement Markdown and test-point JSON persistence fail independently; cancelling retains the unsaved viewport state. Restoring the destination and retrying saves to the shared requirement store or A's point file. The changed test point remains Proposed; no human approval or review continuation is synthesized. B source bytes remain unchanged.
- One complete deterministic replay per OS passes 40 Features / 266 scenarios, zero failures/skips/blocks in this scope. All 62 project-switch scenarios (34 original + 14 prior failure + 14 new boundaries) are included. Real CodeBuddy and external Web/WinApp Features are excluded explicitly, not counted as passes.
- Both native fmt/check/test/Clippy/doc sets pass. Windows native tests/doctests: 1294 passed / 4 ignored; Linux: 1291 passed / 4 ignored. Independent runner regular tests pass 23 / 23, with 113 / 112 ignored respectively; selected product behavior is represented by the actual Feature reports, not those ignored counts. Runner formatting/Clippy, Python contracts and the 47-Feature catalog pass. Syntax has zero errors/warnings and two nonblocking scenario-length suggestions per new locale Feature. Existing rustdoc warnings remain recorded.
- All 36 original P8 task bullets now have explicit scoped accounting. No P8 implementation/evidence category from the five-item queue remains open. Deferred priority, real-provider and aesthetic decisions retain the status above. No commit/push or real requirement-library mutation occurred; unrelated dirty files, processes, outputs and worktrees are preserved.

## P8 checklist reconciliation and editor-state isolation — 2026-10-20

Concurrent main-checkout advance: `40a5894` (ACP interruption/continuation) appeared after this candidate was created. Its changes are preserved during integration. All new acceptance results below still belong to the `5026ab3` candidate plus this package; they do not certify that concurrent commit or the complete combined dirty checkout.

Evidence: `.teshi/artifacts/p8-reconcile-20261020/summary.md` and `verified-summary.json`. Baseline `5026ab3`; final source candidate is `review/`. Exact executable hashes are in `windows-v4-binaries.json` / `linux-v3-binaries.json`; source fingerprints match before/after native gates and scoped replays. Source revision metadata from earlier requirement-store reads remains historical; this continuation does not modify the real store.

- The before-product unit regression fails because A's undo history survives B activation. Successful activation now clears undo/redo, step input/picker, pending key, folds, source selection, external-change prompt, stale source hit regions, Explore offsets/drag/edit state and scenario dropdown state. These resets occur after detached preparation succeeds. A separate regression proves failed preparation preserves A's history and selection.
- The real terminal scenario verifies undo/redo in B cannot reveal A, then performs a single-character B edit/undo/redo positive control while retaining unchanged B source bytes. Legacy undo bytes map to Ctrl+7 on Unix and Ctrl+Shift+minus on ConPTY; keymap tests cover the aliases, and `key-events.txt` preserves actual ConPTY events.
- Both final four-Feature replays pass 48/48 scenarios: original switching 34 plus failure boundaries 14. Windows TUI units pass 472 with one ignored; Linux passes 469 with one ignored. Ordinary ACP shutdown integrations pass 3/3 per OS. Python adapter contracts pass 20/20. Both locale syntax checks pass.
- Final native fmt/check/test/Clippy/doc and catalog pass on each OS. Windows tests/doctests: 1290 passed / 4 ignored; Linux: 1285 passed / 4 ignored. The Linux permission assertions run with the two DAC bypass capabilities removed, as in the prior accepted environment. Existing documentation warnings remain visible. Windows final native gates use one Cargo build job after parallel linking exhausted memory.
- Initial new-test compilation and fixture failures remain recorded. The fixture required leaving chat focus, avoiding a status message absent from the viewport, waiting for editor updates, and using a single-character positive control because undo snapshots are per character. The real undo-byte alias defect was fixed in production. Windows also exposed a stale priority-bearing Core dependency in the shared build cache; only candidate Rust mtimes were refreshed to force the P6-free rebuild, with no shared-output deletion. The first final native attempt failed with linker memory exhaustion and remains under `windows-native-first-attempt/`; it was not waived.
- `doc/project-switch-contract.md` records field ownership/reset and shutdown scope. All 36 original P8 task bullets are mapped in `docs/project-switch-evidence-reconciliation-2026-10-20.md`: 30 scoped evidence rows and six partial rows, not 30 fully accepted E2E requirements. The partial rows consolidate into the five open categories above. This plan is still not ready for archival.

## P8 permission and human-review acceptance — 2026-10-20

Evidence: `.teshi/artifacts/p8-review-20261020/`. Commit: `62b14a4`. The detached `review/` candidate starts at `3b63e7e` and excludes deferred P6 and later WinApp work. This continuation changes tests and fixtures only; the immutable CLI is copied from the previously accepted picker snapshot, with its SHA256 recorded in `windows-binaries.json` / `linux-binaries.json`. The semantic runner is rebuilt from this candidate. Do not infer source identity from the older version caption embedded in that CLI. `staged-vs-tested.json` verifies all five delivered source blobs against the tested candidate while excluding the existing priority dispatch arm.

- Both locale failure Features pass six scenarios each on Windows and Linux (`windows-final-v2-matrix/matrix.json`, `linux-final-v2-matrix/matrix.json`). The final source fingerprints match before/after each replay. This is a scoped 12-case replay per OS, not a new complete 38-Feature replay.
- Permission: a real pending ACP permission modal owns pointer input; clicking the project header neither switches nor approves the tool. Explicit Escape delivers a cancelled permission outcome. Stop-and-switch waits for the old ACP process exit; B has no stale permission prompt, old reply, or source mutation. A separate App regression exercises requesting a switch while permission is pending: cancel retains A and the same request; stop clears the request and waits before replacing context.
- Human review: the fixture invokes real authoring tools to persist a Proposed test point, and the actual Test Points viewport shows it. Cancelling the busy switch preserves A and its unapproved bytes. Stop-and-switch leaves those bytes unchanged, creates no B test-point file, and generates neither blocked nor approved Feature files. The App regression also returns to A and verifies its Proposed state. No review continuation or approval is simulated.
- Final affected gates (`*-final-gates.json`, excluding its recorded replay-directory preflight): Windows TUI units 469 passed / 1 ignored; Linux 466 passed / 1 ignored; workspace formatting and affected all-targets/all-features Clippy pass on both OSes. Syntax and final replay commands pass separately in `*-final-v2-execution.json` with `source_unchanged: true`. No production Rust change required another full native workspace gate set.
- Failed attempts remain recorded: `*-permission-policy-before-*` and `*-permission-host-before-*` expose the new test helper's missing interactive permission host. Setting Manual alone was insufficient; the corrected helper installs `ChannelPermissionHost`, matching the product path. The `*-final-replay.log` directory collision is a fail-closed harness preflight, not a product failure; the successful replay uses fresh `*-final-v2-matrix/` directories.

P8 remains acceptance pending for deliberate Unix group escape and the original checklist reconciliation. P6 remains deferred, real CodeBuddy remains blocked without opt-in, and aesthetic acceptance stays pending. No real requirement-store mutation or push occurred; unrelated dirty changes remain preserved.

## Delivery commits and fresh Linux replay — 2026-10-19

Browser commit `796cb0f` contains the disconnect and asynchronous text assertion fixes, paired Run Inspect Features/bindings and real-browser regressions. A closeout rerun exposed a deadline race: a final locator read could replace an already observed text mismatch with generic `timeout`. The fix retains `assert_text_failed` and the last observed text after a subsequent read timeout. The initial 23-test run had one failure; two corrected 23-test runs pass. The final log is `.teshi/artifacts/linux-clean-replay-20261019/windows-python-checks.log`. The earlier 24-scenario Web replay predates this deadline-only refinement; it is not relabeled as a new Web replay.

Linux commit `94c1e37` contains the bounded readiness operation and regressions, paired project-switch templates, parent/child readiness publication, broker connection isolation and platform Python selection. The existing Linux/Windows gate evidence remains in the sections below.

Fresh replay evidence is isolated under `.teshi/artifacts/linux-clean-replay-20261019/`; one complete run passes 36 Features / 238 scenarios, with zero failures, skips or blocks within this deterministic scope (run `20261019T142013.788640Z-580`). Before/after hashes cover the Feature inputs and exact binaries and are unchanged. `verified-summary.json` records the aggregate and identity check. `source-comparison.json` separately records differences from the previous delivery source snapshot, including unrelated WinApp CLI/service/test changes. This replay validates the pinned acceptance executable and current replay inputs; it does not certify those later WinApp edits or the entire dirty working tree. No real library or unrelated process was modified.

## P9 Linux native and terminal acceptance — 2026-10-19

Evidence: `.teshi/artifacts/p9-linux-20261019/summary.md`, `execution.json`, `source-before.json`, `source-after.json`, and `effective-matrix.json` with its provenance reports. Ubuntu 26.04.1 LTS in WSL2 uses the pre-existing isolated rustc 1.99.0 toolchain and native caches. The pinned CLI SHA256 is `e0f961f07d107d2966d707fd2104aaaece8394f8cc3e944f9f87a7943cd85636`; the final generic/semantic runners are separately pinned under `bin-readiness/`, with hashes in `execution.json`. Cached local version strings are not source identity proof.

- Native workspace fmt/check/test/Clippy/doc pass, excluding wasm-only `teshi-web`. The final full test replay passes 1,250 tests/doctests with 2 ignored. The initial broker suite fails an HTTP limit probe; a later root replay fails the Unix permission precondition in `portable_handoff`. Root bypasses `chmod 000`, so the final full replay removes `CAP_DAC_OVERRIDE` and `CAP_DAC_READ_SEARCH` from the bounding set while retaining UID root. Its helper subprocess inherits those bounds. The original assertions run and pass; no failure is allowlisted or test disabled. See `permission-gate.json` and `workspace-test-permission-gated.log`.
- The independent runner passes 23 regular tests / 111 ignored on Linux, plus formatting and all-target Clippy. Fourteen selected real PTY suites initially pass 94 checks and fail one Explore fixture; the corrected Explore suite passes 3/3, yielding 95 selected checks with explicit first-attempt history. These cover native/ACP clipboard, normal/narrow pointer controls, Vim, creation/deletion/title, and project switching. Both fixture Xvfb instances run in private mount namespaces, with executable-verified cleanup records; no WSLg clipboard is used by these clipboard checks.
- Current deterministic coverage is 36 Features / 238 successful scenarios. The complete delivery replay passes 35 Features and fails one (237 passed / 1 failed scenarios); its sole failure uses the independent Chinese template that initially missed the new readiness action. After correcting that template, its full 17-case scoped replay passes. `effective-matrix.json` consolidates the 35 passing reports and corrected Feature report with source links, without overwriting either original run or claiming one clean full replay. The original complete matrix remains 34 passed / 2 failed Features (236 passed / 2 failed scenarios). Both failures stop A before the external runner has published process evidence. The corrected fixture writes a nonempty readiness marker only after parent/child identities and write evidence exist; the source-bound template waits for that marker before switching. The generic runner adds bounded `expect_file` (1–120000 ms), with missing/empty/delayed-file regressions and pre-launch validation. Process-exit, cwd, stable-write and unrelated-process assertions remain intact. Failed attempts to read readiness through the running log panel are retained separately.
- Two other fixture corrections preserve product assertions: independent broker HTTP-limit probes use fresh connections after early oversized-body rejection, and Explore respects `TESHI_TEST_PYTHON` with platform defaults. The old broker case passes 10 isolated repetitions but fails one of five complete parallel package replays; the corrected package passes ten replays and the final workspace replay. Windows rechecks pass the focused broker case 1/1, Explore 3/3, independent runner 23 tests / 112 ignored, and both project-switch Features 34/34. Python template contracts pass 18/18, including bilingual action parity; that new regression fails before the missing Chinese binding is corrected. No production Teshi CLI behavior changed in this continuation; the test runner gained the explicit readiness operation documented in its README.

All binaries, runtime conditions, source fingerprints and intermediate results remain distinct; do not turn scoped Windows external acceptance plus Linux terminal evidence into a single 43-Feature pass. This closes the previously missing Linux native/terminal matrix for the exercised scope. X11 fixture evidence does not claim Wayland clipboard behavior, Linux Desktop/Web rendering, or macOS acceptance; those are scope limits, not newly invented implementation requirements. P9 itself requests no new web behavior. The two real CodeBuddy Features still require explicit live opt-in; P6 and user aesthetic acceptance remain separate. No real requirement-library data, unrelated process/worktree, commit or remote was changed.

## P9 Windows external-target acceptance — 2026-10-19

Evidence: `.teshi/artifacts/p9-external-20261019/summary.md`, `shared-closeout.json`, `web-fixed-standard/matrix/matrix.json`, and `native-result.json`. The final Web matrix uses the standard, unmodified `scripts/run-feature-matrix.py`, an owned headed Chromium, isolated project/stores, checkout-built GPUI WASM, and the pinned CLI SHA256 `d2297a95c5a9641fcc8a946165230b2a64f44d457b61b9b931f90c92f0e96069`. Python runtime hashes and Web bundle hashes are recorded separately. No Rust product source changed in this continuation.

- Four bilingual Browser Profiles / Run Inspect Features pass all 24 scenarios in a single browser. Earlier isolated runs also passed 24/24, but used the pre-disconnect-fix Python fingerprint; their evidence is preserved rather than relabeled.
- Both Run Inspect Features now explicitly open Run after the Browser Background; the secret-display scenario explicitly starts its run. Sidecars contain 32 CLI-confirmed bindings per locale. Embedded literal text assertions wait within their existing timeout for asynchronous DOM projection without repeating actions.
- The shared-browser stall was caused by repeated abrupt CLI WebSocket disconnects logging tracebacks into an unread stderr pipe. Once full, synchronous logging blocked the Python event loop, making navigation/snapshot commands appear stuck. Redirecting stderr into an owned diagnostic file lets the original batch pass and captures the exceptions. The real-sidecar regression fails before the fix and passes 48 abrupt connections after handling `ConnectionClosed` with optional metadata diagnostics and normal cleanup. Unexpected non-connection errors remain visible.
- Focused Python results: real Chromium action/assertion tests 4/4, HTTP contracts 10/10, Agent flow 32/32, and real connection regression 1/1 (47 total). The 43-Feature catalog and diff whitespace check pass. Native/WASM builds and hosted transport smoke checks passed during provisioning; full native workspace gates were not repeated for Python/Feature changes.
- The native negative-assertion Feature passes one scenario / two UIA steps against an owned checkout-built Desktop window, with screenshots. Desktop startup may start/reuse the shared per-user Rust browser broker; that shared service is preserved, and no unrelated profile is controlled. Final Web daemon and all owned descendants are reaped.

Earlier failed/blocked attempts remain in phase directories, including software WebGPU adapter failure, initial missing manifest/origin setup, and pre-fix shared-browser timeouts. `execution.json` retains the earlier isolated-run snapshot; `shared-closeout.json` is the latest source-specific result. Do not combine the historical 36 deterministic Features and scoped external runs into a claim of one fresh full-matrix pass. Remaining P9 boundaries are live-provider opt-in and other-platform acceptance; user aesthetic acceptance and P6 remain separate. No new commit or push was performed.

## AI pending-change HTTP fixture correction — 2026-10-19

Evidence: `.teshi/artifacts/ai-timeout-20261019/summary.md`, `execution.json`, per-run viewport/input/provider traces and original failure logs. This continuation changes only `crates/teshi-tui-runner/tests/ai_controls_pointer.rs` and this handoff. Production source, the P9 pinned CLI hash and user routing remain unchanged; tests use isolated owned projects and stores.

The original failed run's `send-after.txt` already shows `AI: HTTP request failed` before that status expires. Twenty-five diagnostic repetitions all pass, but two provider traces expose immediate Windows `WouldBlock` / 10035 after only 14,600 request bytes. The old server treats that read error as request completion and sends its SSE response early. Accepted sockets inherit nonblocking behavior on this Windows host. Separately, an empty connection consumes its first tool-call response. Two deterministic TCP regressions fail on the old server: response before the final request-body byte and loss of the first tool call after a closed connection.

The fixture now explicitly makes accepted sockets blocking with a bounded read timeout, replies only after the declared request body is complete, and skips incomplete/closed requests without advancing the conversation turn. Added metadata-only connection/input logs and a timeout screenshot retain diagnostics without recording request bodies, authorization headers or environment credentials. A third regression covers truncated bodies.

Validation: deterministic regression baseline 0 passed / 2 failed; corrected regressions 3/3 passed. On the unchanged P9 CLI, repeated pending-change rejection passes 25/25 and the full normal/narrow AI control suite passes 14/14. The 25 fixed traces record 50 complete responses with no read errors, incomplete requests or write errors. Independent runner regular tests pass 21/21 with 112 ignored; runner formatting and all-target Clippy pass. All other source fingerprints match the prior pinned build. Full workspace/Feature/platform gates were not repeated for this separate-workspace test-only change.

The confirmed fixture defects explain a premature-response/request-failure path consistent with the original symptom. The original failed run has no provider trace, so this evidence does not prove that path was its exclusive cause. Preserve the original P9 timeout and all intermediate results. Next continue isolated external-target/platform acceptance; if this symptom recurs, use the new traces to classify transport, input and product-event failures before changing production behavior. No commit or push was performed.

## P9 current-source validation refresh — 2026-10-19

Evidence: `.teshi/artifacts/p9-current-20261019/execution.json` and `summary.md`. The pinned CLI is `integrated/bin/teshi.exe` under that directory, SHA256 `d2297a95c5a9641fcc8a946165230b2a64f44d457b61b9b931f90c92f0e96069`. Use this hash rather than the cached local version string. Source hashes are unchanged throughout gates and between build and final verification. This run verifies the current dirty working source, including previously existing deferred-priority changes; it does not redefine the scope of prior independent commits or approve P6.

- Workspace fmt/check/test/Clippy/doc, independent runner fmt/Clippy/test, and the 43-Feature catalog gate pass. Workspace tests/doctests: 1,291 passed / 4 ignored. Independent runner regular tests: 18 passed / 112 ignored; selected product PTY tests were executed explicitly below. Documentation exits zero with four existing warnings.
- The same pinned CLI/runner set passes 36 deterministic Features / 238 scenarios. The two live CodeBuddy Features were invoked and report blocked without live opt-in. Five browser/WinApp Features remain not run because this continuation has no provisioned isolated target; no user profile/window was driven. See `integrated/matrix/`, `integrated/matrix-live/` and `external-prerequisites.json`.
- Fourteen direct PTY suites initially pass 95 checks and fail `pending_change_reject_normal` waiting for `Change Summary`. The unchanged binary passes that isolated case (1/1) and a complete AI control-suite replay (14/14). Keep `integrated/pty-ai_controls_pointer.log`, original screenshots, `ai-recheck.log` and `ai-suite-recheck.log`; do not describe this as a clean first attempt or a diagnosed fix. All other direct suites pass, including clipboard, Vim, pointer controls, creation/deletion/title and project switching.
- Python matrix/sidecar contracts pass 3 + 17 tests. Diff whitespace checks pass. A local evidence-script aggregate filename exceeded Windows limits after the individual PTY suites finished; per-suite results remain intact, and the aggregate filename has been shortened. This was an artifact-script failure, separate from the product timeout.

Next work: the AI fixture correction above supersedes this refresh's undiagnosed-fixture status. Provision isolated browser/WinApp targets; obtain live-provider opt-in before provider execution; and run the remaining platform acceptance. User aesthetic acceptance and P6 remain separate. Historical unchecked lists and older platform-pending notes below must be read against the current inventory and evidence, not counted as a fresh implementation backlog.

## Historical closeout inventory — superseded by authoritative inventory above

This inventory supersedes historical status, next-step instructions, and unchecked implementation lists below. Those lists retain the original execution contract; an unchecked box alone is not proof that implementation is missing. Scoped evidence is not a full-workspace or cross-platform pass.

| Package | Current evidence | Remaining boundary |
| --- | --- | --- |
| P0 | Baseline and isolated acceptance inventory exist | Refresh source identity for final integrated acceptance |
| P1/P2 | Windows creation, title/restart and deletion regressions passed | Preserve these regressions in final integration |
| P3 | Windows scoped chat selection committed; Linux X11 replay passed 7 PTY tests with 24 independent clipboard readbacks | Final integrated/platform coverage; no claim of clipboard survival after application exit |
| P4 | Scoped commits b6929e2/d96c7d8; 75 pointer PTY checks plus 16 chat/Vim regressions passed | User aesthetic acceptance, other platforms and integrated P9 |
| P5 | Scoped commits 8995866/7f779a0; bounded Vim subset passed 8 PTY tests | Other platforms and integrated P9; excluded Vim grammar is not promised |
| P6 | Explicitly deferred by user | Resume only after product-design approval |
| P7 | Deterministic Agent workflows and Windows/Linux runner evidence exist | Live-provider acceptance and final integrated replay |
| P8 | Scoped commits 68e93c3/b9c4e3f/731d24c; 34 bilingual scenarios and 94 combined PTY checks passed | Full workspace/platform/provider acceptance and P9 |
| P9 | Windows scoped external acceptance passes 5 Features / 25 scenarios; Linux native gates, 36 deterministic Features / 238 scenarios and 95 selected PTYs verified | Two live-provider Features blocked; aesthetic acceptance pending; preserve platform and binary/runtime scope |

P3 Linux evidence is `.teshi/artifacts/p3-linux-20261017/summary.md` and `execution.json`. The isolated Xvfb replay verifies clipboard ownership while Teshi is alive; the Windows clipboard-lock case is excluded on Linux. Earlier Linux-pending statements below predate this evidence.

The current P9 continuation records commands, immutable binary hashes, source hashes, failures and results under `.teshi/artifacts/p9-current-20261019/`. All nine recorded gates passed; see the refresh above for the PTY timeout and missing acceptance boundaries. Existing dirty source, OpenSpec deletions and deferred priority changes remain outside the scoped-commit delivery claims. No commit or push was performed in this closeout continuation.

Current queue: run the current-source native workspace and independent runner gates, build one pinned CLI/runner set, replay the deterministic Feature matrix, retain explicit live-provider/external-target limitations, then record the final results here. Do not restart completed P4/P5/P8 implementation from the older queue. Do not count the historical 36/43 Feature classification as a current replay.

## P8 scoped project switching delivery — 2026-10-19

Local commits `68e93c3` (identity/history and acknowledged backend teardown), `b9c4e3f` (transactional TUI switching and session isolation), and `731d24c` (paired Features, PTY tests and documentation) exclude unrelated P6/P7/OpenSpec changes. Evidence: `.teshi/artifacts/p8-scoped-20261019/summary.md`, `summary.json`, `matrix-initial/matrix.json`, and `pty-final-results.json`.

Tests use isolated project/requirement/session/application-data stores and a SHA256-pinned Windows CLI. Project preparation failures preserve A; switching checks dirty/busy decisions, runner/native/ACP teardown, session ownership and restart/local-link behavior. Concurrent recent-history migration and ACP reconnect shutdown ownership were repaired. Final regressions pass; an initial registry-pointer fixture timeout remains recorded alongside isolated and full-suite successful rechecks without changing the binary. This scoped result does not close full workspace, other platforms, live providers, aesthetic acceptance or P9. P6 remains deferred; no push.

## P4 scoped controls delivery — 2026-10-19

Local commits `b6929e2` (implementation) and `d96c7d8` (terminal tests/fixtures/documentation) deliver P4 pointer controls and modal input ownership without P6 priority or P8 project switching/session ownership. Independent committed-baseline validation passes 75 P4 real-PTY checks plus 16 chat-selection/Vim regressions on one pinned Windows CLI, TUI 442 passed / 1 ignored, affected-package formatting and Clippy. Evidence: `.teshi/artifacts/p4-scoped-20261019/summary.md` and `summary.json`.

The first isolated matrix and all failure artifacts are retained in `initial/`. The final candidate fixes the omitted Registry URL override and separates P4 fixture assumptions from P8 cwd/title behavior. The project-header picker portions remain outside this commit scope. No push. User aesthetic acceptance, other platforms, live providers and the complete P9/Feature acceptance matrix remain separate work. P8 remains uncommitted and P6 remains deferred.

## P5 scoped Vim delivery — 2026-10-19

Local commits `8995866` (implementation) and `7f779a0` (real-PTY tests/documentation) deliver the bounded Requirements Vim subset without P6 priority, general P4 pointer controls or P8 project switching. Visual-line selection now highlights every selected source line. Independent committed-baseline checks pass: 440 TUI tests / 1 ignored, 8/8 Windows real-PTY scenarios, affected-package formatting and Clippy. Exact Markdown, restart and viewport evidence: `.teshi/artifacts/p5-scoped-20261019/summary.md`.

No push. Full workspace and Linux/macOS acceptance were not repeated. Other P4/P8 edits remain uncommitted; P6 remains deferred. Counts/macros/register grammar, search/replace and Ex commands remain outside this documented Vim subset.

## Chat selection scoped delivery — 2026-10-18

Commits `d717284` (chat selection implementation) and `8d74584` (real-PTY regression suite, mouse helpers and documentation) are local; no push. Selection copies rendered plain text in AI and embedded MindMap, survives valid wrapping/scrolling changes, clears on conversation/content replacement, and remains retryable after clipboard write failure. Chat hit testing independently respects modal ownership without relying on uncommitted P4 pointer dispatch.

Independent committed-baseline validation: TUI 432 passed / 1 ignored, Windows real PTY 8/8, runner library 8/8, affected-package formatting and Clippy pass. Evidence: `.teshi/artifacts/chat-selection-commits-20261018/summary.md`. This does not repeat Linux acceptance or close all remaining P3/P4/global acceptance. Existing P4/P5/P8 changes remain uncommitted and P6 stays deferred.

## Diff opening closeout — 2026-10-18

Evidence: `.teshi/artifacts/diff-fix-20261018/summary.md`, `pty.log`, `scoped-suite.log`, and `scoped-clippy.log`. Commit `631eb1b` changes only the Diff toggle condition and two state regressions. Cached diff lines no longer prevent opening Explore from MindMap or AI. The 14-case PTY suite uses the full working checkout; isolated unit/Clippy validation uses the committed baseline plus this fix. The extended pointer suite remains uncommitted with its P4 dependencies. No push was performed.

## Targeted bindings and history closeout — 2026-10-18

Evidence: `.teshi/artifacts/bindings-history-20261018/summary.json`, `matrix-final/`, and `overlay-pty.log`. This continuation implements the user-selected test/binding and history-verification work; it does not claim the independent P4 Diff defect is fixed.

- Overlay: 8/8 actual PTY checks pass, including generation scope and keyword save at normal/narrow sizes. Tests now read the serialized `named` iteration key, use a command that fits the narrow input, and wait for exact saved bytes. These corrections remain in the uncommitted P4 overlay test file because its other controls depend on uncommitted P4 product code.
- Permissions: both locales pass allow/reject Features. The scenario checks the earlier reply after the permission modal closes; bindings assert visible choices and complete the rejection before checking the missing follow-up. Commit `88833c5` contains only this bounded change.
- History: restored chat text is distinguished from the conversation title. Commands wait for input acceptance/consumption, and restoration waits for modal ownership to end before another command. Both selection-memory Features pass 9/9; conversation history passes 14/14 and activity 13/13 per locale. Commit `2aa1823` contains the bounded fixture correction and asynchronous regressions.
- Independent committed-baseline compatibility: 6 Features / 22 scenarios pass without the uncommitted P4 product changes. The committed Python regression tests pass; the original fixture fails the new regressions. Overlay Clippy and the 43-Feature catalog check pass.

No product Rust source changed in this follow-up. No push or live-provider run was performed. The prior acceptance sections below remain historical, and their failures must not override these focused results or be silently counted as passes for unrelated packages.

Historical status (2026-10-17): P1/P2 regressions and the bounded P5 subset are verified on Windows. P3 Windows acceptance is verified; Linux clipboard lifetime is under investigation. P4 now has 58 passing real-PTY checks, including 32 additional pointer checks; management forms and aesthetic approval remain open. Native Windows gates pass. The latest Feature replay is 35/36 passed, with one project-runner shutdown verification failure still under investigation; the earlier 36/36 result belongs to a different binary. P7 live-provider and P9 external-target/platform acceptance remain open. P6 is explicitly deferred by the user. P8 is outside the current continuation scope.

## Latest P4 controls continuation — 2026-10-17

The current immutable Windows acceptance binary is `.teshi/artifacts/p4-controls-20261017/bin-final/teshi.exe`, SHA256 `09075a5d12e96b7950b0961cbeb261e323cb849d5ffba046fd88958d7999be6e`. Its version string is unchanged; use the hash. Product source was integrated by the root agent while three sub-agents supplied independent pointer tests and audits.

- `pty-final.log`: 58/58 explicit real-PTY checks pass: the earlier 26 regressions plus Requirements 15, TestPoints 5, navigation/pickers 9, and Explore 3. Artifacts under `final/` include exact saved files, viewport text and SVG captures.
- All five tabs have actual pointer access at 150×30 and 55×16. Requirement creation/save/filter/group/iteration/delete/test-point actions, dirty guards, linked rows, TestPoints fields/review/excerpts/scenarios, MindMap focus/preview/location, model/session rows, approval ownership, and Explore run/details/deleted rows now have recorded pointer checks. Priority semantics remain deferred P6.
- Fixes include hit regions from the rendered geometry, modal ownership, iteration text accepting `j`/`k`, independent wheel targets, compact tab/action labels, and synchronized TestPoints tree selection/ancestor expansion. Selected tree styling was also inspected in a rendered PNG.
- `gates.json`: fmt/check/test/Clippy/doc all exit zero; 1,287 native tests pass / 4 ignored. Independent runner formatting and Clippy pass. Rustdoc retains the four previously recorded warnings.
- `matrix-delivery-agent-en` and `matrix-delivery-agent-zh`: 11/11 Features pass each. `matrix-delivery-cli-regressions`: 13/14 pass. The Chinese project-switch Feature fails case `f0:s10` because shutdown verification reports the old runner still alive. Preserve this result; process identity and failure evidence are being improved before replay. Do not infer PID reuse from PID-only evidence or count this failure as passed.
- Catalog remains 43 Features / 265 scenarios with zero syntax errors. The two live-provider Features are explicitly blocked; five browser/WinApp Features lack configured isolated targets.

P3 Linux acceptance is in progress under `.teshi/artifacts/p3-linux-20261017/`. The earlier `/tmp` toolchain/dependencies did not survive the WSL environment reset; recovery uses owned persistent paths without changing the system toolchain or real user stores. No Linux clipboard pass is claimed until a separate process reads the actual clipboard. Model/authentication/Agent management forms are the remaining P4 pointer audit boundary.

The closeout below is a historical passing baseline, not the latest replay. P6 remains deferred. No commit or push was performed.

## Historical non-P8 closeout baseline — 2026-10-17

The user directed this continuation toward the other packages and explicitly answered **“暂缓 P6，先完成其他项”**. Three sub-agents audited and expanded P3, P4/P5, and P7 acceptance. The root agent integrated product fixes and replayed the final acceptance. Do not count pre-existing priority code as approved P6 delivery or restart P8 by default.

Final evidence: `.teshi/artifacts/p9-closeout-20261017/execution.json`. The exact binary is `.teshi/artifacts/p9-closeout-20261017/bin-delivery/teshi.exe`, SHA256 `d04abeb03411d7557fcb45722523185395125cec5efce0f292cc20f34aa8f806`. All 36 deterministic Feature reports and 26 direct PTY checks below use this binary. The version remains `0.7.10-local.20261016.1852+0800 (c173415)`; use the hash to distinguish builds.

| Package | Current result | Acceptance and remaining boundary |
| --- | --- | --- |
| P1/P2 | verified on Windows | Creation 3/3, deletion 1/1, title-save-restart 1/1 real PTY regressions pass; existing identity/concurrency tests remain passing. |
| P3 | verified on Windows; Linux acceptance pending | 8/8 real PTY checks with exact OS clipboard readback: Native/ACP, Unicode and combining characters, code projection, reverse drag, resize/MindMap, scroll, session replacement, stream invalidation, invalid `/copy`, and actual clipboard contention/retry. Linux clipboard ownership/lifetime is not verified. |
| P4 | focused Windows acceptance verified | 5/5 real PTY checks cover all five tabs at 150×30 and 55×16, pane-prefix cancellation, modal pointer ownership, seeded project header, independent Requirements wheel/Shift-wheel/scrollbar drag/resize, and Explore column scrolling/click offsets. Horizontal bars now use `─`/`━`; final normal/narrow PNGs were inspected. Exhaustive individual control/picker/prompt pointer coverage and the user's aesthetic judgment remain open. |
| P5 | bounded subset verified on Windows | 8/8 real PTY checks verify word/line/visual operators, wrapped Unicode movement with logical `dd`, internal yank/paste, `o/O`, undo/redo after save, document-local history, and edit-save-restart. Multiline CRLF/tab/emoji bracketed paste is verified specifically through the existing Windows VT/Zellij path; ordinary Win32 console paste packet boundaries are not claimed. |
| P6 | deferred by user | No design approval and no delivery claim. Preserve existing dirty implementation without assigning real-library priorities. |
| P7 | Windows Agent and Windows/Linux runner acceptance verified | 22 Agent Features pass across both locales, including activity 13/13, history 14/14 and model persistence 5/5 per locale. Selected runner tests pass 24/24, including real PTY Outline/CLI consumption. Live CodeBuddy: 2 Features blocked without opt-in. System probes initially lacked cargo/rustc. An isolated Ubuntu WSL2 toolchain subsequently passed runner unit 8/8, bilingual Outline 2/2 and terminal PTY 6/6 (16 total), using stable rustc 1.99.1. The owned toolchain/cache/target paths are recorded in `linux-toolchain-path.txt`; no system toolchain/profile was changed. Linux CLI/full-workspace and clipboard acceptance are separate and are not claimed. |
| P9 | native Windows gates and deterministic matrix verified | Current catalog: 43 Features / 265 scenarios, zero syntax failures. Runtime results: 36 Features / 238 scenarios passed, zero failures; 2 live Features blocked; 5 browser/WinApp Features not run because no isolated target was configured. Native fmt/check/test/Clippy/doc pass; 1,286 tests pass / 4 ignored. The 26 selected ignored product PTY checks were run explicitly. Rustdoc exits zero with four existing documentation warnings. |

Final report locations:

- `p9-closeout-20261017/execution.json`: complete 43-Feature classification, binary/source hashes, gate results and platform limits.
- `p9-closeout-20261017/matrix-delivery-{agent-en,agent-zh,cli-regressions}/matrix.json`: final 36-Feature replay; `matrix-live/matrix.json`: two explicit live-provider blocks.
- `p9-closeout-20261017/pty-delivery-final.log`, `delivery-*.log`, `gates.json`, `runner-verified.log`, `runner-ignored-verified.log`, `runner-clippy-final.log`: final test and gate evidence.
- `p3-closeout-20261017/delivery-final/` and `p4-p5-closeout-20261017/delivery-final/`: exact clipboard assertions, saved terminal screens/SVGs, Markdown snapshots and normal/narrow PNG review.
- `p7-closeout-20261017/linux-probe.json`: initial read-only WSL probe; `p9-closeout-20261017/linux-outline.log`, `linux-outline.sh` and `linux-toolchain-path.txt`: successful isolated Linux runner replay and reproducible toolchain location.

All paths above are relative to `.teshi/artifacts/`. Intermediate failing runs are retained for diagnosis and must not replace the final reports. Tests use isolated project/library/profile/session stores. Existing OpenSpec deletions and unrelated dirty work remain; nothing was committed or pushed.

Next work: close P4's remaining control-by-control pointer inventory, verify Linux clipboard ownership/lifetime with an isolated display, and provision isolated browser/WinApp/live-provider targets for P9. Start by reading `execution.json` and `doc/tui-interaction-matrix.md`; do not repeat the already-passing Windows matrix without a relevant change or new failure. P6 stays deferred; resume P8 only when explicitly selected.

Historical P8 continuation evidence (2026-10-17; superseded by the non-P8 closeout above): checkout HEAD `c17341531a196f849975b9fb07739d16b4ebc0d0`, branch `dev`, with substantial pre-existing and continuation changes still uncommitted. The final acceptance binary and its SHA256 are pinned in `.teshi/artifacts/p8-sessions-20261017/execution.json`. Its version is still `0.7.10-local.20261016.1852+0800 (c173415)`; the version string alone does not distinguish dirty builds or Cargo feature unification. Focused real-PTY checks use isolated temporary projects; no real requirement store was modified.

Historical audit and P8.5 continuation (2026-10-17; superseded for current gates by the closeout above): the historical reconciled matrix at `.teshi/feature-results/reconciled-20261016-1158/summary.md` records 41 Features: 36 passed and 5 blocked (WebGPU browser initialization and an unavailable WinApp target). It predates the current project-switch implementation. The current catalog contains 43 Features. On the final source build, paired project-switch Features passed 17/17 scenarios per locale and paired terminal-editor regressions passed 2/2 per locale (4 Features / 38 scenarios). New acceptance covers A/B session lists/resume, restart under B, legacy history remaining unassigned, and one global requirement showing distinct project-local links. P8.3 task shutdown/cwd/context cases were also replayed on this same binary. Native workspace check, tests (1,282 passed / 4 ignored), Clippy and docs passed; workspace format still fails at the same 17 prior locations. Do not combine historical runs into a claim that all 43 current Features passed. Latest evidence is under `.teshi/artifacts/p8-sessions-20261017/`, with final replay at `matrix-final-binary/matrix.json`; earlier runner/Agent evidence remains preserved.

## 1. Read this first

This document is a temporary execution and handoff plan, not a second authoritative specification tree. The source requirements are the user-level Teshi library; executable product behavior belongs in Features and tests under the current `doc/test-strategy.md` workflow. Retire or archive this plan after delivery rather than maintaining competing specifications.

Read `AGENTS.md`, `CLAUDE.md`, `doc/test-strategy.md`, `features/README.md`, and `crates/teshi-tui-runner/README.md` before implementation. Resolve any newer applicable instructions first.

The working tree already contains ACP activity/history changes, runner changes, new tests, and deletion of OpenSpec artifacts. Preserve all of them. Do not restore `openspec/`, reset files, regenerate unrelated artifacts, or treat existing untracked files as disposable. `openspec list --json` still reports directories with no tasks; that is not evidence of active implementation plans.

The installed CLI reported `0.7.10-local.20261016.0336+0800 (85468d7)`, which differs from checkout HEAD. Use an explicitly built checkout binary for acceptance; record its path, version, HEAD, and dirty-state description. Do not silently test the PATH binary and attribute results to this checkout.

Do not change the real user requirement store for tests. Create isolated projects, requirement stores, session stores, and profile stores. Use the existing fixture conventions and inspect all environment override mechanisms. A planning request does not authorize commits, pushes, release publication, or deleting user documents.

## 2. Inventory and corrected baseline

Library inspected with `teshi requirements path`, `list --json`, and `show <id> --json`:

- Store: `C:/Users/lilin/AppData/Roaming/teshi/requirements`.
- Store ID: `reqstore-18d3e4036ad71b3800002674`.
- 15 documents; doc-1 is placeholder content; doc-3 and doc-5 overlap.
- Metadata has no completion status. The categories below are source-review judgments, not persisted statuses or test results.

| ID | Subject | Planning classification | Work package |
| --- | --- | --- | --- |
| doc-1 | Placeholder | Exclude; do not delete it | None |
| doc-2 | Project switching and isolation | TUI workflow incomplete; engine infrastructure exists | P8 |
| doc-3 | H1/title/tree synchronization | Implemented in source; preserve regression | P1/P2 guards |
| doc-4 | Requirements scrollbars | Implemented; editor soft wrap is intentional | P4 guards |
| doc-5 | Rename requirement | Covered by doc-3; no separate rename dialog | P1/P2 guards |
| doc-6 | Agent activity and conversation history | Implementation exists; finish product acceptance | P7 |
| doc-7 | Requirements Vim editing | Limited current subset | P5 |
| doc-8 | Delete requirement | Implemented and verified in P2; preserve regression | P2 |
| doc-9 | Requirement priority | Missing model/UI; explicit product design checkpoint | P6 |
| doc-10 | New document tree selection | Implemented and verified in P1; preserve regression | P1 |
| doc-11 | Horizontal scrollbar appearance | Visual complaint; capture baseline before changing | P4 |
| doc-12 | Consistent keyboard/mouse interaction | Partial implementation; inventory all supported tabs | P4 |
| doc-13 | `/copy` does not work | Dispatch and clipboard write exist; diagnose actual failure | P3 |
| doc-14 | Select/copy chat text | Missing chat selection mapping | P3 |
| doc-15 | Native Scenario Outline runner | Windows/Linux runner acceptance passed; Linux full CLI matrix is not claimed | P0/P7 |

Correction to the initial audit: doc-10's selection-order defect is fixed. `create_document()` rebuilds the tree before selecting the new stable ID, and the shared selection helper expands ancestors and scrolls the selected tree row into view. Focused state tests plus real PTY creation, dirty-guard, and title-restart tests passed against `target/debug/teshi.exe` at HEAD `c173415`.

Correction to the initial audit: doc-2 must reuse or adapt existing `teshi-engine/src/project.rs` and `app_data.rs` capabilities. Recent-project persistence and engine project opening already exist. They do not establish a safe TUI switch transaction.

Working backlog: seven implementation gaps including doc-10, two issues requiring diagnosis (doc-11/doc-13), and two acceptance-closeout items (doc-6/doc-15). These are eleven documents, not eleven independent code projects; the packages below deliberately combine shared work.

## 3. Execution order and task boundaries

Recommended sequence:

```text
P0 baseline and acceptance inventory
  -> P1 new-document selection
  -> P2 requirement deletion and identity safety
  -> P3 copy command and chat selection
  -> P4 keyboard/mouse parity and scrollbar presentation
  -> P5 bounded Vim editing
  -> P6 priority model and UI, after its design checkpoint
  -> P7 activity/history and Outline acceptance closeout
  -> P8.1 ... P8.6 project switching
  -> P9 integrated acceptance and handoff
```

P7's Linux Outline validation can be done after P0 without waiting for UI implementation. P6 design can be reviewed early. These are dependency allowances, not instructions to launch parallel agents. The same large `app.rs`, `keymap.rs`, and UI files are shared by many packages; execute implementation serially unless explicitly coordinated.

Each agent turn should implement one numbered package or one P8 subpackage. Stop at its acceptance boundary, report evidence, and leave an exact next step. Do not use a single session to rewrite all application state, all editors, and all rendering.

Use the statuses `not started`, `in progress`, `implemented / acceptance pending`, `verified`, and `blocked by <specific dependency>`. Check a task only when its listed evidence exists. A test being present or a provider returning `end_turn` is not acceptance evidence.

## 4. Code map

Paths are authoritative starting points; line numbers change. Search for the symbols after refreshing the checkout.

| Concern | Starting points |
| --- | --- |
| Requirement metadata and H1 rules | `crates/teshi-core/src/authoring/requirements.rs`: `RequirementDocumentMeta`, `requirement_title` |
| Requirement persistence | `crates/teshi-engine/src/authoring/service.rs`, `store.rs`: store facade, lock, index and Markdown writers |
| Requirement document state | `crates/teshi-tui/src/authoring_tab.rs`: `create_document`, `rebuild_tree`, `select_document_by_id`, `select_document_tree_path`, `guard_unsaved`, `reload_from_disk` |
| Requirement CLI | `crates/teshi-tui/src/cli/requirements.rs`; tests in `apps/teshi-cli/tests/requirement_cli_bdd.rs` |
| Linked test points | `crates/teshi-core/src/authoring/resolve.rs`, `testpoints.rs`; `crates/teshi-tui/src/cli/testpoints.rs` |
| Actions and input | `crates/teshi-tui/src/keymap.rs`, `app.rs`: `Action`, `KeyContext`, `handle_action`, `handle_mouse_event` |
| Requirement layout | `crates/teshi-tui/src/ui.rs`: `render_requirement_scrollbars`; wrap/hit-test helpers in `authoring_tab.rs` |
| Chat layout and copying | `crates/teshi-tui/src/ui/chat.rs`; `app.rs`: `cmd_copy`, `selection_text`, `screen_to_buffer_pos` |
| Editing | `crates/teshi-tui/src/editor_buffer.rs`, requirement editor state in `authoring_tab.rs` |
| AI history | `crates/teshi-tui/src/session.rs`; `app.rs`: `AgentThread`, checkpoints, `project_sessions`, `restore_session` |
| ACP runtime | `crates/teshi-agent-runtime/src/acp_backend.rs`, corresponding runtime tests |
| Runner | `crates/teshi-tui/src/runner.rs`: `spawn_runner`, `spawn_teshi_dispatch`, `RunEvent`; consumers in `app.rs` |
| Engine project/recent history | `crates/teshi-engine/src/project.rs`, `app_data.rs` |
| TUI launch/configuration | `crates/teshi-tui/src/lib.rs`, `config/`, `generation_state.rs`, `agent_selection.rs` |
| Real terminal acceptance | Independent workspace `crates/teshi-tui-runner`; `tests/requirement_title.rs`, `outline.rs`, `acp_activity.rs`, `cli.rs`, `terminal.rs` |

Do not take approximate field counts in older module documentation as current architecture. Inspect actual ownership and call sites.

## 5. P0 — Refresh baseline and build the acceptance inventory

Deliverable: a compact execution record for the selected package, with reproducible commands and an evidence location.

- [ ] Record `git status --short`, HEAD, branch, and `git worktree list --porcelain`; inspect diffs in files the package will touch.
- [ ] Resolve the CLI from `TESHI_CLI` or PATH; inspect version and relevant help; reread the source documents by stable ID.
- [ ] Record relevant document revisions so later requirement changes are detectable. Do not infer completion from document age or iteration.
- [ ] Build the checkout CLI once for product acceptance and record the executable path.
- [ ] Map existing English/Chinese Features to their actual runner, bindings, scenario/example counts, and latest available results.
- [ ] Identify `#[ignore]` PTY tests that require the built binary. A normal `cargo test` run skipping them is not enough.
- [ ] Capture the doc-10 baseline with at least two saved documents and a long/scrolling tree.
- [ ] Capture doc-13 separately for native and ACP assistant replies, and doc-11 at normal and narrow terminal sizes.
- [ ] Classify each baseline result as reproduced, not reproduced with stated environment, or untested. Preserve failures as evidence.

If an existing defect is already fixed by newer local changes, retain a regression and report the evidence instead of implementing another path.

## 6. P1 — Synchronize new-document selection (doc-10)

Contract: after creating a document, selected document ID, tree selection path, visible tree highlight, and editor body all refer to that document. Its ancestors are expanded and its tree row is visible. Do not require moving keyboard focus away from the editor solely to show tree selection.

Suggested implementation:

- [x] Reproduce the `rebuild_tree -> select_document_by_id` ordering defect with an existing selection.
- [x] Introduce or reuse one explicit selection helper that synchronizes document state and tree path without recursive calls between tree rebuild and document selection.
- [x] Make creation select the new ID after tree construction and reveal it in the viewport. Preserve existing dirty-buffer guards.
- [x] Define filtered creation consistently: recommended behavior is to inherit a concrete active iteration filter; for non-concrete filters, reveal the draft without silently hiding it. Do not silently change a persisted document's iteration merely to reveal it.
- [x] Preserve numbering/path collision checks, first-line H1 synchronization, and stable identity.

Acceptance:

1. Create with zero, two, and enough documents to scroll; selected row and body match.
2. Create under collapsed grouping and an active iteration filter; the new document is visible.
3. Dirty current document: Cancel preserves all state; Save/Discard then New behaves correctly.
4. Rename the new H1, save, restart; the same ID/path and new label remain.
5. Exercise both the shortcut and the clickable creation control if present; P4 supplies any missing pointer entry.

Evidence: focused state test for all four selection values plus real PTY viewport evidence. Checking only `selected_document_id` does not cover this bug.

P1 verification: `teshi-tui` authoring tests passed (25/25); the real PTY creation suite passed (2/2) for empty, two-document, scroll-to-document-30, and dirty Cancel/Discard flows; the title save/restart PTY passed (1/1). The latest P4 continuation also adds a clickable New footer action and verifies its normal/narrow pointer and dirty-guard behavior.

## 7. P2 — Delete requirements safely (doc-8)

Contract: TUI can remove a selected requirement from the global library, with an explicit in-product confirmation. Cancel changes nothing. Existing test points and Features are not deleted. References to the removed document become missing/unresolved, never silently retargeted.

Recommended persistence design:

- Use the existing engine store facade and lock. Do not implement filesystem deletion directly in a key handler.
- Use a recoverable store-local tombstone/trash operation for saved documents. Preserve document ID, metadata, and body in the recovery record. Active listings omit tombstones. A restoration UI is outside this package.
- Keep deleted IDs reserved. Current gap-filling `doc-N` allocation would otherwise bind an old cross-project link to a newly created document. Include tombstones/reserved IDs in allocation or migrate to a collision-safe monotonic allocator. Preserve existing IDs; do not renumber the library.
- Define the transaction commit point and recovery from interruption between body relocation and index replacement. Two file operations are not automatically one transaction. Reuse atomic-write helpers; add explicit journal/recovery only where needed.
- Reload current index under the lock. Detect stale body/metadata rather than overwriting concurrent edits. Body revisions currently do not cover all metadata changes; do not pretend they do.
- Validate paths with existing store rules. Never recursively remove paths supplied by requirement content.

Tasks:

- [x] Add engine removal result/errors and tests for success, not found, revision conflict, and write/rename failure.
- [x] Add recovery/reserved-ID behavior, including restart after partial transaction. Ensure concurrent creation/deletion cannot allocate the same ID.
- [x] Add a tree-context Delete action and clickable control. In the editor, Delete remains character deletion. The confirmation names the document and states that the library is global.
- [x] Resolve dirty state first. Cancel deletion preserves draft/body/selection. An unsaved draft can be discarded without creating a persisted tombstone unless its ID has already escaped into persisted references.
- [x] After removal select the next visible document, otherwise previous, otherwise the empty state. Clear stale editor selection, tree nodes, and linked-pane caches.
- [x] Keep links from the active project's test points intact and show missing-source diagnostics. Other projects discover missing sources when loaded; do not scan and rewrite unknown project roots.
- [x] Refresh external-store readers without silently discarding their dirty buffers.

Acceptance matrix:

| Case | Required result |
| --- | --- |
| Delete first/middle/last/only document | Correct remaining selection or empty state; no panic |
| Cancel confirmation | Body, index, selection, and links unchanged |
| Dirty or unsaved draft | Explicit guard; no silent data loss |
| Linked requirement in projects A and B | Test points retained; references report missing in both projects when opened |
| Delete then create | New document never acquires deleted identity |
| Two Teshi instances | Conflicts reported; unrelated documents and metadata preserved |
| Injected persistence failure/restart | Active or recoverable document remains; no silent unrecoverable body loss |
| H1 rename then delete | Stable ID selected; no dependence on title uniqueness |

Do not add CLI deletion solely as a testing shortcut. Add it only if deliberately included in package scope; the engine API is the shared boundary regardless.

P2 verification: the engine authoring suite passed 54/54, the Requirements state suite passed 26/26, the app deletion tests passed 2/2, and the real PTY delete flow passed 1/1. Evidence covers tombstone rollback/recovery, reserved IDs, first/middle/last/only selection, cancel/dirty guards, renamed-ID deletion, two project readers, stale-instance revision conflict, and preservation of a dirty buffer during external reload.

## 8. P3 — Copy command and chat text selection (doc-13/doc-14)

### P3.1 Diagnose and repair `/copy`

Existing `cmd_copy` gathers assistant messages and calls `arboard`. Find the real failure before replacing it.

- [ ] Trace command parsing, message availability after native/ACP streaming, clipboard lifetime, error propagation, and the success status seen in the actual terminal host.
- [ ] Define `/copy` as copying the latest completed nonempty assistant answer. `/copy N` copies the last N eligible answers in chronological order. Do not count activity/reasoning-only records as answers.
- [ ] Keep tool/activity text available through text selection; do not silently include hidden reasoning in `/copy`.
- [ ] Handle invalid/zero arguments explicitly and report no eligible answer without claiming success. Document whether a live partial answer is excluded; recommended default is exclusion until completion.
- [ ] Centralize clipboard write/error handling for command and selection. On Linux, verify clipboard ownership/lifetime instead of assuming a successful API call makes the data persist.
- [ ] Verify by reading/pasting the clipboard outside the source selection. Preserve and restore the user's previous clipboard during live diagnostics where practical.

### P3.2 Implement chat selection

Do not reuse `App::selection_text()` unchanged: it reads the Gherkin editor buffer. Chat rendering already performs explicit wrapping; extend that projection rather than independently recomputing wrap geometry in mouse handling.

Suggested state:

- A chat selection belongs to an agent thread and a content/layout generation.
- Anchors identify stable message/block positions and character ranges, not only screen cells.
- The render projection maps each visible cell range to source text; decorations such as role rails and borders are not copied.
- Mouse down/drag/up selects within chat; clicks on controls continue to activate controls.
- Visible selected text is highlighted. Selection survives scrolling/resizing when source anchors remain valid. A content replacement or session switch clears invalid selection explicitly.
- Streaming updates must not change the meaning of already selected text. Use a pinned snapshot or stable source ranges; document and test the choice.
- Copy reconstructs source newlines, not soft-wrap newlines. Include code indentation and multiline text accurately. Rendered Markdown that hides source syntax needs an explicit plain-text projection contract; do not mix display columns with raw Markdown offsets.

Tasks:

- [ ] Add a reusable text projection/selection model for chat, keeping the requirement editor's Unicode wrap map intact.
- [ ] Implement selection highlighting and scroll/resize updates in both AI tab and MindMap's AI panel.
- [ ] Provide a discoverable Copy selection action and pointer equivalent. Preserve Ctrl+C cancellation semantics where already defined; choose an available chord after P4's key audit rather than taking Ctrl+C globally.
- [ ] Specify cross-message copying, reverse drags, empty selection, CJK, wide characters, combining marks, emoji, tabs, code blocks, and long lines.
- [ ] Keep internal selection separate from system clipboard failure; failure must not destroy the user's selected text.

Acceptance: native and ACP answers; multiple messages; wrapped and unwrapped text; resize; scroll; tool rows; stream updates; restart/session switch; clipboard failure. Use pure projection tests plus real PTY input/viewport tests and OS clipboard readback where available. Headless clipboard absence is a recorded environment gap, not a successful clipboard acceptance result.

Historical verification (2026-10-16; see current closeout above): the command path now accepts the latest completed answer or `/copy N` in chronological order, rejects invalid/zero counts and empty history, and uses the same clipboard writer for command and chat-selection copies. The chat projection has stable block/character anchors, reverse-drag and source-newline reconstruction, fingerprint invalidation, and AI/MindMap support; session restoration clears stale selection. Pure tests cover projection, wrapping, reverse drags, and invalid command cases. Real Windows PTY tests passed for both a deterministic ACP fixture and a native local OpenAI-compatible fixture, including exact OS clipboard readback outside the source selection; the native test exercises two completed responses. Evidence: `crates/teshi-tui-runner/tests/chat_selection.rs` and the corresponding focused test output. Linux clipboard ownership/lifetime, clipboard-write failure, full resize/scroll/tool-row selection, and the complete acceptance matrix remain open.

## 9. P4 — Shared interaction rules and scrollbars (doc-11/doc-12)

Before editing, make an interaction matrix for Explore, MindMap, AI, Requirements, TestPoints, and modal overlays. List visible panes, keyboard focus order, selectable controls, mouse hit regions, wheel target, and text-selection support. Avoid claiming universal mouse parity after testing one tab.

Recommended interaction contract:

| Input | Behavior |
| --- | --- |
| Ctrl+W then h/l | Previous/next visible pane in the active tab; clamp at boundary |
| Ctrl+W then j/k | Nearest visible pane below/above; no-op where layout has no neighbor |
| Esc after prefix | Cancel prefix without editing text or switching tabs |
| Pointer click | Activate the same semantic action as the corresponding keyboard control |
| Wheel | Scroll the pane under the pointer, not a different focused pane |
| Shift+wheel | Horizontal scroll where supported |
| Scrollbar drag | Scroll the owning pane; no text selection or control activation |
| Modal open | Modal owns input; underlying pane receives no navigation or click |

Tasks:

- [x] Move pane-navigation prefix handling out of the Requirements-only branch. Account for Windows uppercase Ctrl+W events and text input mode precedence.
- [x] Model visible pane neighbors centrally enough to avoid five unrelated key-handler implementations. Do not create an abstract framework for future nonexistent panes.
- [x] Build click regions from the exact rendered control rectangles. Dispatch the same actions used by the keyboard; fix stale rectangles on resize, tab switch, and modal close.
- [ ] Cover current keyboard-accessible lists, buttons, pickers, session controls, requirement controls, test-point review controls, and ACP prompts. Record intentionally noninteractive labels separately.
- [x] Capture existing horizontal bars before changing styling. Use current theme colors and a low-noise track/thumb consistent with borders; avoid a new status row solely for a scrollbar.
- [x] Keep tree/linked panes horizontally scrollable and the middle Markdown editor soft-wrapped. Vertical bars appear on overflow; horizontal bars appear only on applicable overflow.
- [x] Verify zero-length and one-cell tracks, full-width thumb, minimum terminal size, CJK width, and independent offsets for each pane.
- [x] Update `doc/keybindings.md` and relevant style guidance with the delivered behavior.

Acceptance: complete the matrix at normal and narrow sizes; capture each affected tab's viewport. Include keyboard and real pointer sequences. If current runner sidecars cannot send mouse input, extend the test adapter minimally or use an explicitly recorded real terminal mouse acceptance; a direct Rust handler call alone is not end-to-end mouse evidence. Visual appearance requires actual rendering inspection, not only string snapshots.

Historical verification (2026-10-16; see current closeout above): the shared keymap/scrollbar tests passed (`scrollbar` 7/7 and keymap 16/16), and the runner now sends coordinate-targeted SGR wheel input with Shift support. The real PTY Requirements acceptance passed all three scenarios: tree/editor synchronization, dirty-guard cancel/discard, and a wheel over the Requirements tree that changed the tree viewport while retaining the editor selection. Evidence is under `target/acceptance/p4/` (including `wheel-tree-after.txt` and the generated SVG viewport artifact). The full five-tab normal/narrow interaction matrix, scrollbar visual review, and complete pointer coverage are not closed.

## 10. P5 — A bounded Vim subset for Requirements (doc-7)

The source requirement does not define full Vim compatibility. Deliver an explicit supported subset and document exclusions. Do not describe this as a full Vim implementation.

Recommended first release:

| Category | Keys |
| --- | --- |
| Modes | `i`, `a`, `o`, `O`, Esc |
| Movement | `h/j/k/l`, `w/b/e`, `0`, `^`, `$`, `gg`, `G` |
| Editing | `x`, `dd`, `yy`, `p`, `P` |
| History | `u`, Ctrl+R |
| Selection | `v`, `V`, `y`, `d` in visual mode |
| Save | Existing Ctrl+S |

Counts, macros, registers, search/replace, arbitrary operator-motion grammar, and Ex commands are separate scope. If basic numeric counts are inexpensive, implement them in a later explicit task rather than inconsistently supporting some commands.

Tasks:

- [ ] Audit current key conflicts. Uppercase `I` is already iteration editing; do not silently steal it. Keep it until iteration editing is deliberately moved and documented. Do not advertise Vim `I` in the initial subset.
- [ ] Use an explicit normal/insert/visual/pending-command state. Invalid sequences reset predictably; tab/document changes clear pending state.
- [ ] Add requirement-local undo/redo. Never reuse the active Gherkin buffer's undo stack. Group one insertion session or paste as a meaningful undo operation.
- [ ] Keep source positions separate from display positions. Recommended `j/k` behavior follows visible wrapped rows consistently with existing arrow behavior; line operators operate on logical source lines. Document the deliberate Vim deviation.
- [ ] Maintain Unicode correctness and selection/link-source ranges; never slice strings by display-cell offsets.
- [ ] Define internal yank register and explicit system-copy action separately; clipboard failures must not break local yank/paste.
- [ ] Keep invalid H1 edits in the dirty buffer and reject save using the existing title validator; undo can restore a valid heading.

Acceptance: each supported command, empty document, first/last line, final newline, CJK/emoji, wrapped rows, paste, undo/redo after save, document switching, invalid command sequence, and no changes to Gherkin editing behavior. Include a real PTY edit-save-restart flow, not only buffer unit tests.

Historical verification (2026-10-16; see current closeout above): the bounded Requirements Vim implementation and keymap tests passed (authoring state suite 26/26, keymap suite 16/16, plus focused bounded-mode and Requirements mutation guards). The real PTY title edit/save/restart regression passed 1/1 against the checkout CLI. This verifies the delivered subset and persistence boundary, not full Vim compatibility; per-command PTY coverage, all CJK/emoji/wrap cases, paste/undo/redo restart coverage, and the complete acceptance matrix remain open.

## 11. P6 — Requirement priority (doc-9)

This requirement explicitly says its design should be confirmed. Prepare the following concrete mockup and semantics for review before implementing the persisted schema/UI. Other packages need not wait for this checkpoint.

Recommended proposal, not an already approved requirement:

- One optional **priority** field: `P0`, `P1`, `P2`, `P3`, or unset.
- P0 means immediate attention; P1 next planned work; P2 normal backlog; P3 deferred improvement. This is scheduling priority, not incident severity.
- If the user wants severity, confirm separate severity semantics instead of overloading P0 with both concepts.
- Existing documents remain unset; do not infer priority from titles or iteration names.
- Show a compact `[P1]` badge before the tree title, and a clickable `Priority: P1` control in the document header/footer area. Unset is represented neutrally.
- Keep existing default tree ordering; optional priority ordering is out of this first package. This avoids selection jumps during editing.
- Picker supports keyboard and mouse and a Clear value action. Use an available shortcut established by the key audit.

Review mockup:

```text
Requirements                 Document                                  Linked points
  [P1] Project switching     Priority: P1  [Change]                     ...
> [P2] Login                 # Login
  [--] Notes                 ...
```

After the checkpoint:

- [ ] Extend `RequirementDocumentMeta` with a backward-compatible optional typed field and shared validation.
- [ ] Preserve the new field across every constructor, Markdown save, iteration edit, import, reload, and CLI edit path. Audit struct literals and metadata merge functions.
- [ ] Decide metadata concurrency explicitly: a body hash alone cannot detect two priority edits. Use a metadata/index revision or expected-value comparison under the store lock.
- [ ] Persist through the engine service, not by TUI JSON writes. Ensure CLI list/show expose the value; a new CLI mutation command is optional scope.
- [ ] Render tree and editor metadata consistently; changing priority does not alter body, H1, document ID, path, or requirement-link resolution.
- [ ] Test older index files, unset value, invalid value, concurrent edit, persistence failure, restart, narrow layout, and monochrome readability.

Do not fill priorities for the user's real library automatically.

Current status (2026-10-17): the user explicitly chose “暂缓 P6，先完成其他项”. The checkout contains a P0-P3 priority model/UI implementation and focused tests from existing dirty work, but the required design checkpoint has not been approved. It is not counted as delivered, and no priority values were written to the user's real library. Keep P6 deferred until the user resumes it.

## 12. P7 — Finish existing acceptance (doc-6/doc-15)

### P7.1 Agent activity and durable sessions

Inspect existing uncommitted implementation and fixtures before adding new ones. Existing source includes activity grouping, session checkpoints, `Session.project_root`, backend identity, and a project-filtered browser.

- [ ] For native and ACP flows, verify tool start/update/success/error, reasoning where intentionally visible, permission allow/reject, cancellation, interrupted response, and long/wrapped conversations.
- [ ] Assert a response-only marker in the current terminal viewport so echoed prompts or a protocol log cannot satisfy the assertion.
- [ ] Check history after normal completion, explicit save/exit, cancellation, and interruption. Inspect the persisted snapshot and reopen the session.
- [ ] Confirm stable session identity, no duplicate records after repeated checkpoints, and a visible error on persistence failure.
- [ ] Preserve credentials and transient ACP controls outside saved session content. Test with synthetic markers, not real secrets.
- [ ] Keep deterministic fixture acceptance separate from optional live-provider results. Report provider exhaustion separately from cancellation or an unexplained interrupt.
- [ ] Record each English/Chinese Feature scenario's executed status and evidence; do not mark the product Feature covered because the runtime unit tests pass.

### P7.2 Scenario Outline

Existing `scenario.rs` and `tests/outline.rs` implement native row expansion and row-specific reporting. Do not expand product Outlines into copied ordinary Scenarios as the final solution.

- [x] Run the independent runner tests on Windows and Linux. Initial WSL probes lacked Cargo; an isolated temporary Ubuntu toolchain passed 16 runner unit/Outline/terminal tests. Windows selected runner acceptance passed 24/24. See current closeout evidence above.
- [ ] Verify English and Chinese syntax, multiple Examples tables, Background per row, text/table/doc-string substitutions, and per-row isolated terminal sessions.
- [ ] Verify one failed row does not suppress later rows, parent/row totals are correct, row filtering works, and failure evidence identifies its data row.
- [ ] Verify missing, stale, and unconfirmed bindings and malformed/empty Examples fail explicitly before launch.
- [ ] Retain ordinary Scenario regression and the explicitly unsupported Rule Background diagnostic unless support is separately scoped.
- [ ] Validate consuming Feature runner/reporting paths, not just the runner crate. Parent case IDs and row identity must remain coherent through CLI/UI summaries.

Completion requires recorded platform evidence. If Linux cannot be exercised, label it `implemented / Linux acceptance pending`; do not quietly waive the requirement.

Historical verification (2026-10-16; see current closeout above): the independent runner suite passed on Windows (runner unit 8/8, Outline 2/2, terminal 6/6); the public CLI Outline acceptance passed 1/1 with parent failure, later-row execution, row totals/filtering, and evidence assertions. The ACP activity PTY checks passed, including response color/event order and narrow/expanded activity rendering. The deterministic product Feature runner passed the complete English `tui_agent_activity.feature` at 13/13 scenarios, and the self-test PTY suite passed wrapped six-turn/30-turn conversations plus permission allow/reject flows. Windows evidence is therefore strong for the exercised flows, but Linux has not been run in this continuation; the broader session persistence/reopen matrix, all English/Chinese paired Features, and every cancellation/interruption/persistence-failure case remain acceptance-pending.

## 13. P8 — Safe TUI project switching (doc-2)

This is the largest package. Implement its subpackages separately. The final feature remains unavailable until the switch transaction and lifecycle guards work together.

### P8.1 State ownership and project identity

Deliver an actual field inventory before moving fields. For every project-related `App` field, list owner, initialization source, dirty/persistence rule, cancellation rule, and reset rule.

| Ownership | Examples |
| --- | --- |
| Global | Theme, credentials/model profiles, installed ACP agent registry, requirement library location/identity, recent projects |
| Project | BddProject, buffers and undo, StepIndex, MindMap data, test points, requirement associations, runner config/results, agent working directory and pending changes, generation state, Git refresh, exploration memory |
| View/session | Focus, selection, overlays, scroll, chat session selection; explicitly decide reset versus preservation |

Important: global requirement **storage** does not make the whole `authoring_ui` global. It currently combines global documents with project-local test points and UI state. Existing `reload_from_disk()` deliberately preserves in-memory test points; do not call it unchanged for A -> B switching or A's points can survive into B.

Tasks:

- [ ] Define normalized `ProjectIdentity` and `ProjectGeneration` types. Canonicalize existing paths; normalize Windows extended-path representation without blindly lowercasing every platform's paths.
- [ ] Test relative/absolute/trailing-separator aliases and Windows casing/symlinks as supported by the filesystem. Equivalent paths must not trigger a new switch.
- [ ] Reuse existing engine/app-data facilities where their contracts fit. Avoid routing the TUI through desktop engine teardown just to reuse a function name.
- [ ] Introduce an owned project context or cohesive aggregate with explicit construction and replacement. Migrate fields in reviewable groups; keep adapters temporarily if needed.
- [ ] Search project-scoped operations for `current_dir`, `set_current_dir`, relative paths, configuration loads, and captured roots. Make root-dependent work explicit rather than switching process-global cwd.

Acceptance: startup behavior unchanged; field-inventory coverage complete; state constructors compile; project-local configuration is resolved from explicit roots. Existing project identity in saved sessions is retained and normalized, not replaced by a second incompatible field.

### P8.2 Recent-project persistence and switcher UI

Existing `app_data.rs` stores a maximum-ten path list and no timestamps. Evolve it compatibly instead of creating a second recent-project registry.

- [ ] Add normalized identity/root, display name, and last-opened time with an explicit migration from legacy path-only data.
- [ ] Keep read/write adapters compatible with existing engine consumers; use locking/atomic persistence so two shells do not corrupt history.
- [ ] Update recent history only after successful activation. A history-write failure after activation should warn without pretending the project switch itself failed.
- [ ] Add a discoverable Project control showing name and accessible full path. Duplicate names must be distinguishable by path.
- [ ] Add recent-project picker, Open path, Cancel, keyboard navigation, and pointer controls. Same-project selection closes without rebuilding state or interrupting work.
- [ ] Missing/unreadable paths produce a useful error and preserve A. No recent entry may make normal startup fail.

Acceptance: restart, aliases, duplicate directory names, missing directory, corrupt history, history write failure, and same-project no-op. Do not enable unsafe replacement while P8.3/P8.4 are incomplete.

### P8.3 Dirty-state and task lifecycle guards

Switch preparation must cover all dirty Feature buffers, requirement drafts, pending test-point edits, and unapplied Agent proposals. The requirement store is global but unsaved requirement editor data still needs protection.

- [ ] Build one pending-switch request; collect dirty objects with stable identities.
- [ ] Offer Save and Switch / Discard and Switch / Cancel. Save failure stays in A and reports which object failed. Already saved objects need not be rolled back, but unsaved ones must remain accessible.
- [ ] Treat unapplied Agent proposals explicitly: apply/reject them or cancel switching; never carry them into B.
- [ ] Inventory running Agent, ACP session, generation, external runner, Git/index workers, and any browser/terminal workers reachable from TUI. Distinguish harmless background reads from side-effecting tasks.
- [ ] Audit cancellation/process ownership handles for every task type. External runners now expose `spawn_runner_with_control()` and have Windows process-tree shutdown evidence; the receiver-only compatibility wrapper is not sufficient for other callers. Agent/backend and synchronous-dispatch quiescence still need review.
- [ ] Offer Stop tasks and Switch / Cancel when needed. Wait for stop acknowledgement with a bounded timeout; timeout keeps A active with an actionable message.
- [ ] Stop only processes owned by the task. Never kill all processes with an executable name or stop unrelated daemons.

Ignoring late events is not sufficient: old tasks may still write to A or operate through shared mutable roots. They must be quiescent before committing the switch unless their safe isolation is explicitly proven.

Acceptance: each busy-task type, cancel success, cancel failure/timeout, dirty save error, dirty-discard followed by failed B load, and no accidental file writes into B.

### P8.4 Prepare and commit the new context

Recommended state machine:

```text
Active(A) -> ResolveTarget -> ResolveDirty -> QuiesceTasks
          -> Prepare(B) -> Commit(B) -> Active(B)

Cancel or failure before Commit -> Active(A)
```

Keep A's context until B is fully prepared. A cancelled task is not automatically restarted after failure, but A's documents and UI must remain usable. Defer destructive discard of in-memory buffers until commit; a failed B load should not unnecessarily erase A's edits.

- [ ] Build B's parsed project, buffers, indices, test points, runner settings, generation state, and agent setup in a detached candidate.
- [ ] Define preparation failure consistently with doc-2: nonexistent/unreadable paths, corrupt settings, and blocking parse failures keep A active and show diagnostics.
- [ ] Avoid external side effects in detached preparation. Defer launching agents/runners/watchers until commit where possible; clean up only candidate-owned resources on failure.
- [ ] Commit by replacing the complete project context, incrementing its generation, refreshing view state, and rebuilding derived maps from B.
- [ ] Pass explicit B roots to runner/config/tool services. Audit the CLI-override precedence intentionally: session-wide explicit overrides may persist, but project configuration must reload.
- [ ] Tag asynchronous results with project identity and generation; check both before applying them. A -> B -> A must reject results from the first A generation.
- [ ] Audit mutation boundaries as well as event consumers. Never let a deferred callback obtain the new project root for an old task.

Acceptance: A -> B -> A with different Feature names, test points, runner configs, and Agent fixtures; delayed events from A; same path with a newer generation; failed B load; repeated rapid requests; and cancellation while preparation is in progress. An event-rejection unit test must be paired with file-write and real UI evidence.

### P8.5 Sessions and global requirements across projects

Implemented with focused Windows acceptance on 2026-10-17. Default lists already filtered by project identity; the remaining restoration/checkpoint holes and ambiguous legacy presentation are now corrected. Evidence: `.teshi/artifacts/p8-sessions-20261017/execution.json` and `matrix-final-binary/matrix.json`.

- [x] Default session list and `/resume` show only matching normalized project identity. Startup does not implicitly restore a conversation.
- [x] Legacy sessions remain accessible through `/sessions legacy` under `Legacy / Unassigned`. Loading and checkpointing preserve the absent owner; no assignment action is offered.
- [x] Restoration rejects foreign ownership and rootless sessions outside explicit legacy access. Checkpointing independently rejects a foreign project root.
- [x] Checkpoint A before switching; B gets fresh backend/history state. Global profile definitions remain shared. The old session model caption, permissions, visual observations, tool status, and pending restoration do not carry across the boundary.
- [x] Preserve the global requirement store identity/root; load B's associations/test points afresh.
- [x] Reset project-local link selection and highlight state. The same global document can remain selected.

Acceptance: same global requirement linked to different points in A/B, no cross-project session restoration, legacy visibility without implicit assignment, backend cwd verification, and restart under B.

The paired project-switch Features now pass 17/17 scenarios per locale (34 total), plus the paired terminal-editor regressions (4 total). Four new scenarios per locale cover A/B history lists and resume, restart under B, legacy checkpoint ownership, and shared-global-document/local-link isolation. `session-isolation.json` in cases `f0_s13`–`f0_s16` records actual saved session ownership/content and unchanged requirement/association hashes. Unit tests enforce foreign-root rejection, explicit legacy restoration, checkpoint ownership, deferred restoration cancellation/agent/generation ownership, and link-highlight reset. Backend cwd/process/context evidence remains covered by the replayed P8.3 cases `f0_s11`/`f0_s12` on this same binary.

Restoration requests nonblocking backend shutdown, keeps the old handle until exit acknowledgement, and resumes only for the captured agent/project generation. Escape cancels the pending restore. The previous binary reproduces legacy auto-assignment in `matrix-before-legacy`; the first new replay exposed a stale model caption and invisible legacy notice, both fixed before the successful `matrix-final-v2` replay. Full native gates changed the debug binary through Cargo feature unification; the CLI was rebuilt with default package features and all four selected Features passed again in `matrix-final-binary`. Intermediate failures remain saved. P8 as a whole remains acceptance pending for its remaining failure paths and platform matrix.

### P8.6 Product acceptance and architecture documentation

- [ ] Cover all seven doc-2 acceptance scenarios with executable Features and actual runner ownership.
- [ ] Add negative flows for dirty save failure, busy task timeout, corrupt B, stale-generation events, and duplicate project names.
- [ ] Use deterministic agents/runners that write identifiable markers to isolated A/B folders so the tests detect cross-project side effects.
- [ ] Exercise both keyboard and pointer switcher flows in a real terminal.
- [ ] Update the nearest architecture/development docs with final ownership and lifecycle contracts. Remove temporary adapters only after their consumers are migrated and tested.

Do not claim completion for a project path label and a file-list reload. The complete state and task invariants above are the feature.

Historical verification (2026-10-16; see current closeout above): project identity/generation types, normalized target aliases, recent-project migration/locking, explicit-root runner configuration, candidate preparation, and commit-time state replacement are implemented with focused tests. The latest paired project-switch Features passed 20/20 real Windows PTY scenarios, including missing/corrupt target errors, normal/narrow pointer controls, and retention of unsaved Feature edits after Discard followed by failed preparation. This is focused acceptance; P8 remains `implemented / acceptance pending` for the complete source requirement.

### P8.6 focused acceptance continuation

Source: `doc-2`, revision `8b413fffee3e58df` (read from the global library without mutation). HEAD remains `c17341531a196f849975b9fb07739d16b4ebc0d0`; all continuation work is uncommitted.

Changes:

- Normalize absolute CLI Feature paths to paths relative to the source project before copying them into a fixture. Otherwise joining an absolute path silently bypasses the temporary root and writes generated bindings into the source tree. Regression coverage checks absolute, relative, and outside-root requests; final replay leaves no generated project-switch sidecars in the source tree.
- Replace the two stale generated project-switch sidecars with source-bound `*.tui.template.json` bindings. The external adapter creates fresh projects and expands only the binary/root tokens; changed source steps, line numbers, and unknown tokens fail before launch.
- Fix long recent-project paths hiding `Open path` and shifting pointer targets. Rendering and hit testing now use the same wrapped physical rows, with space reserved for controls and selection scrolling.
- Publish switch failure messages in editing views. Discard followed by invalid target preparation now leaves A's edit accessible and shows the failure.
- Add paired corrupt-settings, narrow-picker, and failed-preparation-with-unsaved-edit scenarios. Verify errors in Then steps and verify A remains usable after failure.
- Fingerprint A/B Feature and runner-config files around every scenario; unexpected writes or creation of a missing target fail acceptance.

Execution:

```text
python scripts/run-feature-matrix.py --teshi .teshi/artifacts/p8-feature-20261016-232551/bin-accepted/teshi.exe --feature features/en-US/tui_project_switch.feature --feature features/zh-CN/项目切换.feature --output-dir .teshi/artifacts/p8-feature-20261016-232551/matrix-isolated
```

Results and evidence:

- Final matrix exit 0: English 10/10 and Chinese 10/10; 20/20 filesystem assertions passed. Windows real PTYs use 120x35 normally and 64x24 for the narrow scenario.
- `execution.json` records the exact command, requirement revision, HEAD, platform, source hashes, binary hashes, and limits. The accepted CLI snapshot matches `target/debug/teshi.exe` by SHA256; its version string is unchanged, so use the hashes to distinguish these dirty builds.
- `matrix-isolated/matrix.json` contains the final results. Its `artifacts/` tree contains 118 successful step viewport captures and 20 `fixture-files.json` records.
- Earlier failures remain in `matrix/` (4/8 failed per locale before the layout fix) and `matrix-final/` (1/10 failed per locale before the editor error-message fix). Original stale sidecars were retained as `en-US-original.tui.json` and `zh-CN-original.tui.json` in the artifact root.
- The paired terminal-editor Features also passed 4/4 after the adapter path-isolation fix (`matrix-editor-isolated/matrix.json`); their tracked source/sidecars remained unchanged.
- Final TUI unit suite passed 454 tests with one unrelated ignored test (`tui-tests-final.log`); TUI all-targets/all-features Clippy passed (`tui-clippy.log`).
- Independent runner tests passed 18 tests with 25 explicitly ignored prerequisites; runner Clippy passed. The Python adapter/matrix contract suite passed 7/7; catalog and paired Feature validation passed. Test logs are in the same artifact root.
- Ubuntu WSL was reprobed: neither Cargo nor Rustc is on PATH, and `$HOME/.cargo/bin/cargo` is absent. `linux-probe.json` records exit 1. Linux is blocked, not passed.

Remaining: all seven original doc-2 scenarios are not yet fully covered by these ten focused scenarios. Busy-task stop/timeout, delayed Agent events and writes, backend working-directory/context isolation, legacy/unassigned sessions, global requirement/project-local association isolation, dirty save failure, and the complete platform matrix still need implementation review and executable evidence. This historical next-step recommendation is superseded by later P8 evidence and the current non-P8 closeout queue above. No commit/push or real-library write occurred.

### P8.3 external runner shutdown continuation (2026-10-17)

Scope: the external runner path, one independently verified part of P8.3. P8 as a whole remains `implemented / acceptance pending`. Source requirement: doc-2, revision `8b413fffee3e58df` as recorded in the previous handoff; this continuation did not change the requirement library. HEAD remains `c173415`; changes are uncommitted.

Implemented and verified:

- Cancellation stops the owned external runner tree on Windows (`taskkill /PID /T /F`), or a dedicated process group on Unix. No process-name selection is used. Output readers finish before `RunnerExit` is emitted. Stop failures retain the receiver and process ownership so switching can time out in A.
- The final commit boundary rechecks busy tasks after dirty-state resolution. The five-second timeout retains A's unsaved edits and owned runner handles, and publishes its error to editor views. The busy overlay now labels the action `Stop tasks and switch`.
- Added four state regressions: cancel preserves the runner, stop waits for exit acknowledgement, timeout preserves A/edits/ownership, and dirty resolution cannot bypass a task that became busy.
- Added one paired product scenario with a deterministic parent/child runner fixture. Recorded PID absence and stable writes prove A's processes exited before fixture cleanup. A separate Python process survives cancellation, and B executes its own runner configuration with an actual B working directory.

Evidence: `.teshi/artifacts/p8-runner-20261017/`.

- `matrix-before-corrected/matrix.json`: the old binary passes the original 10 scenarios and fails the new stop/switch scenario, timing out while waiting for B. The earlier `matrix-before/` attempt failed at a stale `Run started` viewport assertion; it is retained as harness diagnostics, not the process-shutdown reproduction.
- `matrix-final/matrix.json`: four Features, 26/26 scenarios passed on the new binary (project switch 22, terminal editor 4).
- `matrix-process-verified/matrix.json`: paired project switch replay, 22/22 passed after adding explicit PID-exit checks. Each locale's `f0_s10/runner-shutdown.json` records parent/child PIDs, actual working directories, stable write counts, and survival of the unrelated process. All 22 source-file fingerprint assertions passed.
- `project-switch-unit.log`: 7/7 focused App tests. `tui-tests.log`: 458 passed, 1 ignored. `tui-clippy.log`: all-targets/all-features Clippy passed. `python-adapter-tests.log`: 7/7; `python-matrix-tests.log`: 3/3. Catalog validation passed for 43 files, and both locale syntax checks passed.
- `fmt-touched.log`: all three touched Rust files pass formatting. `fmt-workspace.log`: the workspace still fails at exactly the pre-existing locations recorded in the previous continuation's `fmt-final.log`. `diff-check.log`: passed.
- `execution.json` records commands, source/binary hashes, platform, and validation limits. The accepted CLI snapshot matches the current `target/debug/teshi.exe` hash.

Remaining: live stop-failure/timeout PTY evidence, Agent/native/ACP shutdown and delayed side effects, synchronous dispatch cancellation, escaped/detached descendants, full session/association isolation, dirty save failure, Linux validation, and the full 43-Feature/workspace gates. The new Unix process-group path is untested on Linux. Do not extend these external-runner results to other task types or mark all of P8.3 complete. Next isolate Agent/backend quiescence before broadening the final matrix. No commit, push, or real requirement-library write occurred.

### P8.3 Agent and ACP shutdown continuation (2026-10-17)

Scope: local Native model workers, the primary ACP process, MCP bridge ownership, and late event suppression during project switching. This is another verified part of P8.3, not completion of the full project-isolation requirement. Requirement revision remains `8b413fffee3e58df` from the previous handoff; the real library was not changed. HEAD remains `c173415`; all changes are uncommitted.

Changes:

- Agent backends now expose nonblocking shutdown request/poll/pending operations. ACP retains its worker and MCP bridge until both exit, rejects queued calls, and disables old events/permission prompts while stopping. Starting or configuring a still-stopping ACP backend returns a retryable error instead of joining it on the UI thread.
- Native cancellation wakes the transport future even when no SSE chunks arrive. Retiring worker handles remain owned until they can be joined; a queued request cannot reset the permanent shutdown flag.
- The switcher tracks backend shutdown across canceled switches, stops idle connected backends before preparation, and retains A and owned handles on timeout. Stopping tasks preserves unapplied Agent proposals for explicit review/discard; the existing human-review/file-approval workflow is not bypassed.
- Added paired ACP cancel-during-stop and stalled Native stream scenarios. Controlled ACP shutdown attempts a late reply and continues writing A until it exits. B's prompt must exclude A's request; ACP OS cwd and native socket closure are recorded independently from viewport assertions.

Evidence root: `.teshi/artifacts/p8-agent-20261017/`.

- `matrix-before-direct/matrix.json`: old binary, 11 passed and 2 failed. ACP cannot dismiss the stopping overlay within 500 ms because teardown blocks the UI; Native reaches B while A's connection remains open. `matrix-before/` and `native-shutdown-diagnostic.log` retain earlier diagnostics, including a proxy-intercepted transport that was not valid direct-connection evidence.
- `matrix-final/matrix.json`: initial accepted snapshot, four Features and 30/30 scenarios passed (project switching 26, terminal editor 4). Workspace feature unification changed the debug binary hash, so the default-profile CLI was rebuilt and accepted separately: `bin-final/teshi.exe` matches `target/debug/teshi.exe`, and `matrix-final-binary/matrix.json` passed the same 30/30 scenarios. `final-binary.json` and `execution.json` identify these exact paths/hashes. Do not substitute another binary just because its version string matches.
- Each locale's `f0_s11/agent-shutdown.json` and `agent-trace.jsonl` prove primary ACP exit before B session creation, stable A writes, correct process cwd, a late reply attempt, and independent B conversation context. `f0_s12/agent-shutdown.json` proves the old Native stream closed before B's reply and B received independent context. Existing runner-tree and source-file assertions also remain passing.
- `affected-tests.log`: Agent Runtime units 26/26, existing ACP integrations 17/17, new teardown integrations 2/2, Engine units 247/247 and integrations 3/3, TUI units 459 passed with 1 ignored, TUI doctests 2/2. The new API tests verify nonblocking requests, retained handles, suppression of old events, and rejection of premature ACP restart. The TUI proposal-preservation regression passed in this full package run.
- `affected-clippy.log`: all-targets/all-features Clippy passed for Engine, Agent Runtime and TUI. Python adapter/matrix contracts passed 9 + 3. Catalog: 43 files. Both locale source checks have zero errors/warnings and one nonblocking length suggestion for the coherent ACP cancel/retry scenario.
- Workspace check and workspace all-targets/all-features Clippy passed. The first full workspace test run failed one of six `portable_handoff` tests with Windows access denied (error 5). That exact test passed on focused rerun, then the complete workspace test rerun passed: 1,279 tests/doctests passed and 4 were explicitly ignored. All three logs are retained; this differs from the older six error-448 failures.
- Touched Rust formatting and diff whitespace checks pass. Workspace formatting still fails at exactly the historical locations outside this continuation. Full workspace docs, independent runner suite, Linux, and WASM web validation were not run here.

Local HTTP fixtures explicitly bypass proxies only in their test child environment. The first socket diagnostic identified the user's proxy as the peer; changing production proxy/routing was neither required nor performed. ACP traces omit MCP descriptors and authentication environment, and HTTP traces omit authorization headers.

Remaining: live backend stop-failure/timeout and permission/review cancellation matrices, ACP descendant-process ownership beyond the primary process, synchronous dispatch, escaped/detached tasks, session reopen/legacy ownership, project-local requirement associations, dirty save failure, Linux, full 43-Feature replay, workspace formatting and docs. Next isolate P8.5 sessions/associations or the remaining P8.3 failure paths. No commit/push or real-library mutation occurred.

## 14. P9 — Validation and completion evidence

Run focused checks as each package changes. Run the integrated gates once the assembled scope is ready, then repeat only what subsequent changes invalidate.

Native workspace gates from `AGENTS.md`:

```text
cargo fmt --all --check
cargo check --workspace --exclude teshi-web --locked
cargo test --workspace --exclude teshi-web --locked
cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
cargo doc --workspace --exclude teshi-web --locked --no-deps --document-private-items
```

The independent terminal runner is not covered by those workspace commands:

```text
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked
cargo clippy --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --all-targets -- -D warnings
python scripts/check-feature-e2e-catalog.py
```

Build and point ignored product PTY tests at the checkout CLI. Example for the existing title regression on Windows:

```powershell
cargo build -p teshi-cli --locked
$env:TESHI_SELF_TEST_BIN = (Resolve-Path target/debug/teshi.exe).Path
cargo test --manifest-path crates/teshi-tui-runner/Cargo.toml --locked --test requirement_title -- --ignored
```

Use the actual runner documented by each Feature; inspect current help before invoking `teshi run` or the matrix script. Validate Feature syntax through the checkout CLI (`check --feature <path> --json`) before execution. New or edited Gherkin must follow the installed `bdd-feature` skill. Keep English/Chinese pairs aligned where the repository expects them.

Do not run all ignored/live-provider tests blindly. Select the package-relevant tests, make required binaries/fixtures explicit, and record skipped prerequisites. Never mark a missing fixture, unavailable clipboard, provider failure, or missing Linux toolchain as a pass.

No web behavior is requested here. Native gates exclude wasm-only `teshi-web`; if shared model changes affect web consumers, compile/validate those consumers with their separate supported workflow and report that scope explicitly.

Earlier P9 baseline (2026-10-16): `cargo check --workspace --exclude teshi-web --locked` passed, and the native workspace Clippy gate passed after fixing four lint errors in the project-switch/chat-selection code. The first parallel workspace test exposed a process-global environment-variable race between `app_data` and `managed_runtime`; a shared crate-level test lock was added, the engine tests then passed 246/246, and the rerun reached the update-helper package where six `portable_handoff` tests fail in this environment with Windows error 448 (`untrusted mount point`). The Requirements/TUI suite passed 453/453 with one unrelated ignored test. `cargo doc` reached only warning-level broken-link/bare-URL diagnostics but was stopped after an unusually long rustdoc phase and is not counted as passed. These are historical results from before the latest P8.6 fixes, not a final integrated gate.

P8.6 validation refresh: the final paired project-switch Features passed 20/20; the final TUI unit suite passed 454 tests with one ignored; `cargo clippy -p teshi-tui --locked --all-targets --all-features -- -D warnings`, independent runner tests/Clippy, catalog, and adapter contracts passed. The toolchain binaries are usable by prepending `C:/Users/lilin/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin` to PATH; the normal Cargo shim fails to launch in this environment. `cargo fmt --all --check` now runs but fails on pre-existing formatting differences outside the two Rust files changed in this continuation; `fmt-before.log` and `fmt-final.log` retain the exact locations. The changed `app.rs` and `ui.rs` were formatted. Full workspace tests/docs, Linux acceptance, the broader normal/narrow interaction matrix, and the final cross-package checklist remain open. This continuation does not rerun the unrelated browser/WinApp blockers or convert the earlier 36/41 result into a current full-suite pass.

P8.3 validation refresh (2026-10-17): external-runner shutdown, dirty-to-busy recheck, and visible timeout reporting are covered by the focused continuation above. The new CLI passed project-switch 22/22 and terminal-editor 4/4; TUI units passed 458 with one ignored, TUI Clippy passed, and Python adapter/matrix contracts passed 10/10. Touched Rust formatting and diff whitespace checks passed; workspace formatting still fails at the same historical locations. The full native workspace gates and the independent runner suite were not rerun in this continuation, and Linux was not exercised. Historical browser/WinApp blockers remain unverified, not passed.

Prior P8.3 Agent refresh (2026-10-17): project-switch 26/26 and editor 4/4 passed, affected package tests/Clippy passed, and native workspace check/Clippy passed. The full workspace tests passed on a complete rerun after one access-denied update-helper failure also passed its focused rerun. Those initial failures remain recorded, not waived. Workspace format still fails at the original locations; docs, Linux, WASM web and the complete 43-Feature matrix remain open. See the Agent continuation and its `execution.json` for final binary/result ownership.

Latest P8.5 validation refresh (2026-10-17): project-switch 34/34 and editor 4/4 passed on the final pinned binary. Native workspace tests/doctests passed 1,282 with four ignored; check and Clippy passed. Documentation generation now passed too, retaining existing rustdoc warnings. Python adapter/matrix contracts passed 14/14. Both locale syntax checks report zero errors, zero warnings, and one existing ACP scenario-length suggestion. Touched Rust formatting and diff whitespace checks passed; workspace formatting still fails at the same 17 historical locations. The old-binary auto-assignment failure, first replay's caption/notice failures, and corrected Clippy failure remain recorded. Linux, WASM web, independent TUI-runner suite, and the complete 43-Feature replay were not run in this continuation.

Historical P8 next-step recommendation (superseded by the current non-P8 queue): isolate the remaining P8 failure paths (live backend stop failure/timeout, permission/review cancellation, ACP descendants beyond the primary process, synchronous dispatch, escaped tasks, dirty save failure), then complete the platform/full-Feature matrix and remaining P9 gates. P6 is now explicitly deferred by the user. No commit/push or real-library mutation occurred.

For each accepted behavior, record:

| Field | Required content |
| --- | --- |
| Source | Requirement ID/revision and package/task ID |
| Code | HEAD, dirty-state summary, relevant files |
| Binary | Absolute path and version |
| Environment | OS, terminal host, dimensions, fixture/backend |
| Execution | Exact command, test/scenario/example identity, exit status |
| Result | Passed / failed / blocked / not run |
| Evidence | Current viewport text, visual capture when needed, relevant fixture trace/persistence assertions |
| Limits | Missing platform/provider/clipboard/UI evidence |

Evidence must be stored in a known artifact directory and referenced by exact path. Do not commit generated terminal logs or screenshots by default; follow repository artifact conventions. Preserve only task-owned fixture processes and clean them by owned handles/PIDs.

Final integrated checklist:

- [ ] Every touched requirement has an explicit result and linked acceptance evidence.
- [ ] Doc-3/doc-4/doc-5 regressions remain intact.
- [ ] All newly visible actions have keyboard and pointer coverage or an explicit unresolved gap.
- [ ] Metadata changes survive save/import/reload and old-library compatibility tests.
- [ ] Project A/B isolation is demonstrated at both UI and filesystem boundaries.
- [ ] Windows/Linux requirements are reported separately and truthfully.
- [ ] Final diff preserves unrelated initial changes; no global library was modified by tests.
- [ ] Handoff records remaining work and next command; no unsupported completion claims.

## 15. Product decisions and bounded assumptions

The plan provides recommended defaults so routine implementation can proceed. These are not retroactively approved changes to requirement text.

| Decision | Recommended default | Handling |
| --- | --- | --- |
| Priority vs severity and placement | Optional P0-P3 priority, tree badge plus metadata control | Explicit doc-9 design review before P6 implementation |
| Delete semantics | Recoverable removal with reserved IDs; links retained as missing | Implement with documented recovery contract; do not delete user data during development |
| Vim scope | Listed subset, no full compatibility claim | Document delivered subset; raise conflicts instead of silently remapping established commands |
| Scrollbar appearance | Current theme, minimal track/thumb, retain soft wrap | Show before/after real render; aesthetic acceptance remains a user judgment |
| Missing native clipboard | Honest error; preserve selection | Do not add OSC52 or another remote clipboard mechanism without a concrete requirement |
| Legacy sessions | Explicit Legacy/Unassigned list | Never auto-assign during switch |
| Failure loading B | Preserve A context and edits; stopped tasks stay stopped | Explain failure; no automatic destructive fallback |

## 16. Copyable next-agent instruction

> Read the authoritative scoped delivery inventory at the top of `docs/requirement-implementation-handoff-2026-10-16.md`, `.teshi/artifacts/p9-current-20261019/`, the scoped P4/P5/P8 summaries, current `AGENTS.md` and `CLAUDE.md`. Read the completed 36-row project-switch evidence reconciliation and latest p8-boundaries-20261020 summary; do not restart the five completed boundary categories; do not restart completed scoped implementation or substitute historical gates for current evidence. P6 is explicitly deferred by the user. Preserve existing dirty files and OpenSpec deletions, use isolated fixture stores, and address one remaining boundary at a time. Do not implement P6 priority semantics without explicit design approval. Record failed, blocked and not-run results separately, with exact evidence paths and platform limits. Do not commit or push unless separately requested.

For later turns, replace `P0 and P1` with exactly one package or P8 subpackage. Require the implementer to reread its dependencies and the latest handoff before editing.

Handoff template:

```text
Package:
Requirement IDs and revisions:
Baseline HEAD and pre-existing dirty files:
Implemented behavior:
Files changed by this task:
Decisions made / decisions still needed:
Tests and Features actually executed:
Evidence paths:
Failures, skips, and platform limitations:
Remaining tasks:
Next package and first command:
Commit/push status:
```
