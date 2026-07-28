## Why

The TUI generation pipeline currently moves from conversational requirements to a scenario plan and then to `.feature` mutations. It treats test points as Gherkin scenarios, so users have no durable place to review what must be tested before the system decides how to implement those checks. Requirement text, intermediate planning state, and traceability links are also lost when the TUI session ends.

Teshi needs a first-class authoring workflow in which requirements and non-executable test points are durable project artifacts. Users must be able to inspect the source text behind each test point and explicitly approve the proposed test points before AI generation can continue to Gherkin scenarios.

## What Changes

- **BREAKING**: Redefine a test point as a verification intent that does not contain Gherkin Given/When/Then steps. Gherkin scenarios become downstream executable realizations of approved test points.
- Add Requirements and Test Points tabs to the TUI alongside the existing Explore, MindMap, and AI tabs.
- Add source-controlled requirement-document and test-point persistence, including stable identifiers and test-point hierarchy metadata.
- Add bidirectional traceability between arbitrary ranges of requirement text and test points, using position and quote selectors that can recover anchors after document edits.
- Add a dedicated AI tool and pipeline stages for proposing test points before scenario planning.
- Add a mandatory human review gate for proposed or stale test points. This gate cannot be bypassed by the agent's Auto or Bypass file-change approval modes.
- Link generated Gherkin scenarios back to the approved test points they realize.
- Keep GPUI/web focused on presenting execution artifacts. Screenshot and video presentation changes are outside this change.

## Capabilities

### New Capabilities

- `tui-requirements-testpoints-authoring`: Provides durable requirement documents, test points, arbitrary-range traceability, two complementary TUI tabs, and mandatory test-point review.

### Modified Capabilities

- `tui-requirements-generation`: Inserts test-point proposal and review between requirements gathering and Gherkin scenario planning, and changes test points from executable scenarios into non-Gherkin verification intents.

## Impact

- **Core model and persistence**: New shared requirement, anchor, test-point, approval-state, and scenario-link models; new project artifact readers and writers.
- **TUI**: Two new tabs, tree/detail panes, text selection and highlighting, editing and approval actions, navigation between linked artifacts, and updated key bindings/help.
- **Agent pipeline**: New generation stages and tool schema, hard-gate validation, persistence across sessions, and test-point-to-scenario references.
- **Gherkin integration**: Generated scenario plans and written scenarios retain stable links to originating test points without embedding test-point intent as Given/When/Then.
- **Compatibility**: Existing `.feature` projects continue to open. They have no test-point links until users create or generate authoring artifacts.
- **GPUI/web**: No authoring UI is added. Existing artifact viewers remain unchanged.
