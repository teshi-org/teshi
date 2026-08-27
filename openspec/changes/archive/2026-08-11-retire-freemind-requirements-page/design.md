## Context

Today, desktop/web ships a Requirements→Testpoints page (FreeMind `.mm` + mock HTML + segment linking) as the default view, backed by duplicated LLM prompts in `teshi-daemon` and `teshi-tauri`. The TUI already has a generation pipeline (`submit_requirements` → `generate_plan` → feature writing) that produces Gherkin, plus a Gherkin-based MindMap that is unrelated to FreeMind.

Product direction: authoring (requirements + test points) belongs in the TUI; web/desktop present runner artifacts (screenshots). This change is the first slice: remove GUI generation and FreeMind; keep Screenshots gallery as-is.

## Goals / Non-Goals

**Goals:**

- Delete FreeMind/mock HTML generation paths from GUI and backends.
- Desktop/web start in Workspace only (no Requirements mode toggle).
- Document that TUI Agent pipeline is the requirements→test-points (scenarios) path.
- Minimal TUI prompt/entry alignment if needed so paste-and-generate works in the terminal.

**Non-Goals:**

- Step-by-step pass/fail confirmation UI on web/desktop.
- Video review UI.
- New intermediate mindmap formats replacing `.mm`.
- Migrating or converting historical `.teshi/testpoints/` data.
- Unifying TUI and desktop Gherkin editors beyond removing the Requirements page.

## Decisions

### 1. Hard-delete FreeMind stack (not feature-flag)

**Choice**: Remove Requirements React components, generate API, and FreeMind prompts entirely.

**Alternatives**: Hide behind a flag; keep API for CLI clients.

**Rationale**: No remaining consumers once GUI is gone; flags leave dead surface area. Historical `.teshi/testpoints/` remains on disk unused.

### 2. Test points = Gherkin scenarios/steps

**Choice**: Reuse `teshi_agent::pipeline` and existing tools; TUI MindMap is the browse surface.

**Alternatives**: Invent a JSON test-point tree; keep `.mm` only in TUI.

**Rationale**: Aligns with runner/replay and avoids a second tree model. FreeMind is explicitly retired.

### 3. Spec strategy for `requirements-testpoints-page`

**Choice**: Delta with REMOVED requirements for page/API/FreeMind/mock; ADDED requirement that GUI MUST NOT offer generation. New capability `tui-requirements-generation` owns the TUI path. At archive time, main spec becomes a thin retirement note or is replaced by the TUI capability.

**Rationale**: Clear migration from archived design that explicitly excluded TUI.

### 4. TUI changes stay minimal

**Choice**: Prefer prompt wording that says “test points” mean scenarios; only add a slash/command hint if there is no discoverable path today.

**Rationale**: Pipeline already implements the flow; this change’s core delivery is GUI subtraction.

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| Users expect Requirements page on desktop launch | Workspace becomes default; docs/spec state generation is TUI-only |
| Half-deleted API (daemon vs Tauri) | Delete both generate endpoints/commands in the same change |
| Confusion between TUI MindMap and FreeMind | Specs/docs use “Gherkin MindMap”; remove FreeMind wording |
| Unused `.teshi/testpoints/` dirs | Document as unsupported; no auto-delete |

## Migration Plan

1. Land OpenSpec artifacts and implementation together.
2. Deploy: no data migration; old generate clients get 404 / missing command.
3. Rollback: revert commit (components + routes restored).

## Open Questions

None for this slice. Step-review UI is a future change.
