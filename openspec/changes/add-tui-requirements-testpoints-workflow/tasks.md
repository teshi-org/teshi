## 1. Authoring domain model and persistence

- [ ] 1.1 Add documented shared models for requirement-document metadata, Unicode character positions, quote selectors, requirement links, test points, hierarchy paths, review states, and scenario references.
- [ ] 1.2 Implement readers and validators for `requirements/_teshi.json`, requirement Markdown files, and `testpoints/testpoints.json`, including duplicate-ID, missing-file, invalid-range, empty-hierarchy, and invalid-reference diagnostics.
- [ ] 1.3 Implement deterministic, atomic writers for requirement metadata and test points, preserving the last complete file when replacement fails.
- [ ] 1.4 Add project-loading integration that discovers authoring artifacts when present and leaves existing feature-only projects unchanged.
- [ ] 1.5 Add table-driven unit tests for valid round trips and every validation failure path.

## 2. Text selection, anchors, and invalidation

- [ ] 2.1 Add conversions between editor/runtime byte positions and persisted Unicode scalar offsets, with tests covering ASCII, CJK, emoji, combining characters, and line boundaries.
- [ ] 2.2 Implement anchor creation from any non-empty editor selection, including exact quote, bounded prefix/suffix context, document revision, and character positions.
- [ ] 2.3 Implement deterministic anchor resolution in position, quote, and context-disambiguation order; return stale instead of choosing an ambiguous match.
- [ ] 2.4 Re-resolve document links after external or TUI edits and transition affected approved test points to `NeedsReview` without invalidating unrelated approvals.
- [ ] 2.5 Add unit and integration tests for moved text, duplicate quotes, deleted text, unrelated edits, and restart/reload behavior.

## 3. Requirements tab

- [ ] 3.1 Extend `MainTab`, tab rendering, key handling, and help text with Requirements on direct key `4` while preserving existing `1`/`2`/`3` bindings.
- [ ] 3.2 Build the left requirement tree from indexed document paths and support document creation, rename/move repair, selection, and missing-file diagnostics.
- [ ] 3.3 Reuse or extend the editor buffer to render and edit requirement Markdown with arbitrary range selection, linked-range highlighting, scrolling, and safe saves.
- [ ] 3.4 Implement the right linked-test-point pane, including document-wide results, overlap filtering for the active selection, and highlights driven by selected test points.
- [ ] 3.5 Add the action that creates a `Proposed` test point from the active non-empty range.
- [ ] 3.6 Add focused state/action tests and terminal rendering tests for narrow, normal, and wide layouts.

## 4. Test Points tab and review workflow

- [ ] 4.1 Extend tab rendering, key handling, and help text with Test Points on direct key `5`.
- [ ] 4.2 Implement the explicit business hierarchy tree with grouping, status indicators, filtering, and stable test-point selection.
- [ ] 4.3 Implement center-pane editing for title, objective, preconditions, expected outcomes, hierarchy, and review state, resetting approval only for meaning-bearing changes.
- [ ] 4.4 Implement the requirement-excerpt pane with resolved/stale indicators and cross-navigation to the exact highlighted range in Requirements.
- [ ] 4.5 Add explicit approve, reject, and batch-approve actions; prevent implicit approval through focus, selection, or agent tool execution.
- [ ] 4.6 Add state-transition tests covering Proposed, Approved, Rejected, and NeedsReview, including hierarchy-only edits and meaningful edits.

## 5. Agent test-point generation and hard gate

- [ ] 5.1 Extend `GenerationStage` with Generating Test Points and Reviewing Test Points, including labels, prompt guidance, resume behavior, and transition tests.
- [ ] 5.2 Replace scenario descriptions in gathered requirements with source-document/range references while retaining compatibility for pasted conversational input.
- [ ] 5.3 Define and register `propose_test_points`, validate its structured arguments, persist only `Proposed` test points, and stop the agent loop in review.
- [ ] 5.4 Implement the human-only transition from Reviewing Test Points to Planning and ensure `ApprovalMode::Auto` and `ApprovalMode::Bypass` cannot trigger it.
- [ ] 5.5 Update `generate_plan` to require approved, resolved test-point IDs and reject unknown, rejected, proposed, stale, or invalid records.
- [ ] 5.6 Persist and restore the active generation source, review phase, and plan without granting approval during restart.
- [ ] 5.7 Add end-to-end agent-tool tests for propose, pause, reject/edit, approve, resume, restart, and blocked-bypass paths using deterministic mocked LLM responses.

## 6. Scenario realization traceability

- [ ] 6.1 Evaluate valid Gherkin metadata conventions against the current parser and select a stable Teshi-owned encoding for one-or-many test-point IDs.
- [ ] 6.2 Add test-point IDs to `ScenarioPlan` and propagate them through feature creation and scenario insertion without changing executable step semantics.
- [ ] 6.3 Parse scenario references back into test-point records or a derived traceability index, tolerating existing scenarios with no metadata.
- [ ] 6.4 Add navigation from a test point to every realized scenario and surface originating test-point links in existing MindMap/Explore details where space permits.
- [ ] 6.5 Add parser, writer, round-trip, and backward-compatibility tests for unlinked, singly linked, and multiply linked scenarios.

## 7. Documentation and verification

- [ ] 7.1 Update user and keybinding documentation for the five tabs, source-controlled artifact layout, review states, hard approval gate, stale anchors, and scenario links.
- [ ] 7.2 Update AI tool and generation-flow documentation to distinguish test points from Gherkin scenarios and remove obsolete direct Planning assumptions.
- [ ] 7.3 Run `cargo fmt --all --check`.
- [ ] 7.4 Run targeted tests for the affected core, agent, and TUI crates, then run `cargo test --workspace --exclude teshi-web --locked`; record any known pre-existing unrelated failure.
- [ ] 7.5 Run `cargo check --workspace --exclude teshi-web --locked`.
- [ ] 7.6 Run `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings`; distinguish the documented pre-existing `teshi-tui` lint if it remains.
- [ ] 7.7 Manually exercise requirement selection, linked highlighting, AI proposal, mandatory review, scenario generation, restart recovery, and bidirectional navigation in a real TTY, and capture a concise walkthrough artifact.
