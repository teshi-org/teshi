## Context

Teshi's TUI currently exposes Explore, MindMap, and AI tabs. Requirement gathering and scenario planning exist only as in-memory fields on `App`, and `generate_plan` advances directly to feature writing. The current specification calls Gherkin scenarios and steps "test points," which collapses verification intent and executable implementation into one artifact.

The desired authoring workflow has two complementary views over the same traceability graph:

- Requirements view: requirement hierarchy and source text, with linked test points for the selected text.
- Test Points view: test-point hierarchy and intent, with linked requirement excerpts.

Most authoring remains in the TUI. GPUI/web remains an execution-artifact presentation surface and does not become a second authoring implementation.

## Goals / Non-Goals

**Goals:**

- Make requirement documents and non-Gherkin test points durable, source-controlled project artifacts.
- Support arbitrary text-range links with resilient re-anchoring after requirement edits.
- Require explicit human approval of generated test points before scenario planning.
- Allow users to add, edit, reject, approve, group, and cross-navigate test points in the TUI.
- Preserve traceability from requirements through approved test points to generated scenarios.
- Recover authoring and review state after restarting the TUI.

**Non-Goals:**

- Adding requirement or test-point authoring to GPUI, React web, or daemon APIs.
- Implementing screenshot or video capture and presentation.
- Restoring FreeMind `.mm`, generated mock HTML, or word-token segmentation.
- Turning a test point into an executable Gherkin fragment.
- Automatically importing existing `.feature` scenarios as approved test points.
- Multi-user concurrent editing or remote synchronization.

## Decisions

### 1. Test points are verification intents

A test point records what behavior must be verified, independently of how a runner will execute it. It contains:

- stable ID and title;
- objective;
- optional preconditions and expected outcomes in natural language;
- hierarchy path used by the Test Points tree;
- trace links to one or more requirement ranges;
- review state;
- zero or more downstream scenario references.

It does not contain Gherkin keywords or ordered execution steps. One approved test point may produce multiple scenarios, and a scenario may cite multiple test points when it realizes a combined behavior.

This keeps requirement analysis reviewable before the AI commits to automation details.

### 2. Authoring artifacts are source-controlled project files

The default layout is:

```text
requirements/
├── _teshi.json
└── **/*.md

testpoints/
└── testpoints.json
```

`requirements/_teshi.json` maps stable document IDs to Markdown paths, display titles, and the last observed content revision. The directory structure provides the Requirements tree. Markdown remains editable with ordinary tools.

`testpoints/testpoints.json` stores test points, hierarchy paths, review states, requirement anchors, and scenario references. A single canonical file makes cross-link validation and atomic replacement straightforward. Writers use a temporary file plus rename so interruption cannot leave partially serialized state.

The roots may become configurable later, but this change uses these defaults. Runtime data and binary execution artifacts remain under ignored `.teshi/` storage.

### 3. Anchors combine position and quote selectors

Each requirement link stores:

```text
document_id
document_revision
position: start/end character offsets
quote: exact text plus bounded prefix/suffix context
resolution: resolved or stale
```

This follows the selector strategy used by text-annotation systems:

1. If the document revision matches and the position still selects the exact quote, use it.
2. Otherwise, locate exact quote matches.
3. Use prefix and suffix context to disambiguate multiple matches.
4. If no unique match remains, mark the link stale instead of silently linking the wrong text.

Persisted offsets count Unicode scalar values, not UTF-8 bytes, so files and TUI rendering agree across multibyte text. Runtime code may cache byte offsets after validating character boundaries.

Arbitrary ranges are allowed, but an empty selection cannot create a link. A test point can link to multiple ranges in the same or different documents.

### 4. Requirement edits invalidate only affected review decisions

After a requirement document changes, the resolver re-anchors all links for that document.

- A successfully re-anchored link whose exact quote is unchanged remains valid.
- A changed or ambiguous quote becomes stale.
- An approved test point with any stale link becomes `NeedsReview`.
- An unrelated requirement edit does not reset other approvals.

Editing the intent-bearing fields or requirement links of an approved test point changes it to `Proposed`. Moving it between hierarchy groups does not reset approval because grouping does not change verification meaning.

### 5. Test-point approval is a hard business gate

Review states are:

```text
Proposed ──approve──▶ Approved
    │                    │
    ├──reject──────▶ Rejected
    └──edit────────▶ Proposed

Approved ──meaningful edit/stale anchor──▶ NeedsReview
NeedsReview ──approve/reject──▶ Approved/Rejected
```

The TUI supports individual approval and an explicit batch approval action. Batch approval is still a human action; entering the tab or selecting a row never implies approval.

Scenario planning may begin only when:

- at least one test point is `Approved`;
- every test point included in the generation request is `Approved`;
- no included test point has a stale anchor.

`ApprovalMode::{Auto, Bypass}` continues to control ordinary agent tool/file mutations, but it does not affect this gate. The pipeline waits for a dedicated TUI review action and cannot approve test points through an LLM tool call.

### 6. The generation state machine gains proposal and review phases

The authoritative flow becomes:

```text
Idle
  → Gathering
  → GeneratingTestPoints
  → ReviewingTestPoints
  → Planning
  → Writing
  → Confirming
  → Validating
  → Complete
```

`submit_requirements` records the selected requirement documents and advances to `GeneratingTestPoints`. A new `propose_test_points` tool validates and persists proposed test points, then advances to `ReviewingTestPoints`.

While reviewing, the agent loop pauses. Human review in the Test Points tab supplies the only transition to `Planning`. Rejection remains visible to the next generation attempt so the agent can avoid proposing the same unwanted intent.

`generate_plan` accepts approved test-point IDs and records them on `ScenarioPlan`. It rejects unknown, unapproved, or stale IDs. Feature mutation tools carry those references into scenario metadata using a stable Teshi-owned convention that remains valid Gherkin. The exact serialization convention is selected during implementation after checking parser compatibility.

### 7. Two TUI tabs present the same traceability graph

The existing direct keys remain stable:

- `1`: Explore
- `2`: MindMap
- `3`: AI
- `4`: Requirements
- `5`: Test Points

The Requirements tab uses three panes:

```text
┌──────────────────┬──────────────────────────────┬─────────────────────┐
│ Requirement tree │ Markdown source/editor       │ Linked test points  │
│ folders/documents│ selection and highlights     │ filtered by range   │
└──────────────────┴──────────────────────────────┴─────────────────────┘
```

Selecting text filters the right pane to links overlapping that range. With no active selection, it shows all test points linked to the document. Selecting a test point highlights all its anchors in the center pane. Users can create a test point from the active range.

The Test Points tab uses:

```text
┌──────────────────┬──────────────────────────────┬─────────────────────┐
│ Test-point tree  │ Intent/details/review state  │ Requirement excerpts│
│ hierarchy/status │ edit and approval actions    │ source highlights   │
└──────────────────┴──────────────────────────────┴─────────────────────┘
```

The tree is organized by an explicit business-domain/function/category path rather than copying the requirement tree. The right pane lists every linked excerpt. Activating an excerpt opens its document and range in the Requirements tab.

Both tabs use explicit pane focus and reuse existing tree, editor-buffer, scrolling, and highlight primitives where their behavior matches.

### 8. Persistence validation precedes UI and generation

Readers validate:

- unique document and test-point IDs;
- referenced documents exist;
- position boundaries and quote selectors are well formed;
- hierarchy paths contain no empty segment;
- state transitions are valid;
- scenario references use project-relative feature paths.

Malformed records produce actionable diagnostics and remain visible as invalid instead of being silently dropped. Generation is blocked while selected records are invalid.

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| Arbitrary text ranges become stale after edits | Store position and quote/context selectors; fail closed when re-anchoring is ambiguous |
| Source files and index drift after external renames | Detect missing/indexed and unindexed Markdown files, then offer explicit repair rather than assigning new identities silently |
| A single test-point JSON file causes merge conflicts | Use stable ordering and deterministic formatting; splitting storage can be reconsidered if real projects show contention |
| Five tabs increase navigation complexity | Preserve existing `1`/`2`/`3` bindings and add discoverable `4`/`5` bindings |
| Approval semantics conflict with agent Auto/Bypass | Implement review as a separate domain transition that agent approval settings cannot invoke |
| Links embedded in Gherkin become parser-visible noise | Validate the chosen metadata convention against current parsers and keep it Teshi-owned |
| External requirement edits occur during review | Refresh revisions before approval and before scenario planning; mark affected links `NeedsReview` |

## Migration Plan

1. Add models, persistence, validation, and anchor resolution without changing existing generation behavior.
2. Add the Requirements and Test Points tabs and manual authoring.
3. Add test-point proposal/review stages and the mandatory gate.
4. Add scenario trace references and update MindMap/Explore detail presentation.
5. Change the authoritative specification from “test points are scenarios” to the new staged model.

Existing projects require no migration. The new directories are created only when users initialize requirement authoring or accept generated test points.

## Open Questions

None. The accepted defaults are arbitrary-range hybrid anchors, explicit business hierarchy for test points, and mandatory human approval before scenario planning.
