## 1. Spec sync (main specs)

- [x] 1.1 Update `openspec/specs/requirements-testpoints-page/spec.md` to match the retirement delta (GUI must not generate; FreeMind/mock removed)
- [x] 1.2 Add `openspec/specs/tui-requirements-generation/spec.md` from the change delta

## 2. Remove desktop/web Requirements UI

- [x] 2.1 Remove Requirements page components (`RequirementsPage`, `RequirementsInput`, `RequirementsText`, `MindMapViewer`, `MockHtmlViewer`, and related CSS)
- [x] 2.2 Simplify `App.tsx`: drop `showRequirements` state, toggle button, and dual-view wrappers; always show Workspace
- [x] 2.3 Remove frontend `requirements/generate` client API usage from platform layer

## 3. Remove backend generate endpoints

- [x] 3.1 Remove `POST /api/v1/requirements/generate` and FreeMind/`generate_testpoints` prompt tooling from `teshi-daemon`
- [x] 3.2 Remove matching Tauri generate command(s) and FreeMind prompts from `teshi-tauri`
- [x] 3.3 Grep for leftover FreeMind / `generate_testpoints` / `requirements/generate` / `.teshi/testpoints` references in app code and clean them

## 4. TUI requirements path alignment

- [x] 4.1 Review TUI Agent pipeline prompts; clarify that test points are Gherkin scenarios (no FreeMind/mock)
- [x] 4.2 Confirm paste into AI input works; add only minimal discoverability (e.g. slash hint) if generation entry is unclear
- [x] 4.3 Smoke-check that ScreenshotsPanel / BottomDock screenshots tab remain intact

## 5. Verification

- [x] 5.1 Frontend typecheck / build for teshi-tauri frontend
- [x] 5.2 `cargo check` (or targeted checks) for daemon and tauri crates touching the removals
