## Why

The desktop/web Requirements→Testpoints page (FreeMind `.mm` + mock HTML) conflicts with the product direction: most authoring happens in the TUI, while web/desktop should present runner results (screenshots) for review. Keeping FreeMind generation in the GUI duplicates work and locks “test points” into a format the TUI does not use.

## What Changes

- **BREAKING**: Remove the desktop/web Requirements page as the default view and remove the Requirements/Workspace toggle.
- **BREAKING**: Remove FreeMind (`.mm`) mindmap generation, mock HTML generation, and the `POST /api/v1/requirements/generate` (and Tauri equivalent) API.
- Retire persistence expectations for `.teshi/testpoints/<slug>/requirements.mm` and `mock.html`.
- Treat “test points” as Gherkin scenarios/steps produced via the existing TUI Agent generation pipeline (Gathering → Planning → Writing), browsable in the TUI MindMap.
- Keep the web/desktop Workspace (editor, locator, Screenshots gallery) unchanged aside from removing the Requirements mode.
- Out of scope: step-by-step pass/fail review UI; video review; migrating historical `.teshi/testpoints/` data.

## Capabilities

### New Capabilities

- `tui-requirements-generation`: Requirements gathering and test-point (scenario) generation live in the TUI Agent pipeline; products are Gherkin `.feature` files, not FreeMind or mock HTML.

### Modified Capabilities

- `requirements-testpoints-page`: Remove FreeMind/mock HTML page requirements; GUI SHALL NOT provide requirements→testpoints generation. Capability is effectively retired in favor of TUI generation (delta documents removals and the new GUI non-requirement).

## Impact

- **Frontend**: `apps/teshi-tauri/frontend` — delete Requirements panels; simplify `App.tsx` startup to Workspace only; remove generate client calls.
- **Backend**: `apps/teshi-daemon` generate endpoint + prompts; `apps/teshi-tauri` Tauri commands mirroring generation.
- **TUI / agent**: Document and lightly align prompts so requirements→`.feature` is the supported path; no new mindmap format.
- **Specs**: Delta for `requirements-testpoints-page`; new `tui-requirements-generation`.
- **Risk**: Users who relied on the Requirements default page lose that flow; historical `.teshi/testpoints/` artifacts become unsupported.
