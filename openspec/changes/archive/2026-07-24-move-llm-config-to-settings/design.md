## Context

Today `teshi-desktop` and `teshi-web` both mount `LlmConfigView` as the window root. That made sense for the LLM-config spike, but product direction treats LLM settings as secondary. The shared UI crate (`teshi-ui`) still must not depend on `teshi-engine` / `teshi-agent`; backends inject `LlmConfigBackend` at the app boundary.

## Goals / Non-Goals

**Goals:**

- Introduce a shared root shell with a main surface (empty placeholder) and a settings surface.
- Host the existing LLM config form inside settings without changing load/save/masking semantics or HTTP/store contracts.
- Keep desktop and web on the same root view API from `teshi-ui`.

**Non-Goals:**

- Building real main-panel product features (chat, exploration, etc.).
- Redesigning LLM fields, persistence format, or daemon routes.
- Full settings IA (multiple categories, search, preferences beyond LLM) beyond a minimal host that can grow later.
- Fancy navigation chrome beyond a simple, usable way to open/close settings.

## Decisions

### 1. Root entity: `AppShell` (or equivalent) owns surface switching

- **Choice**: Add a root GPUI view in `teshi-ui` that holds navigation state (`Main` | `Settings`) and renders either an empty main placeholder or the settings host.
- **Why**: Entry points stay thin (backend + open window); shell owns UX structure.
- **Alternatives**: Keep `LlmConfigView` as root and hide it behind a flag — rejected; reinforces config-as-home. Separate windows for settings — rejected; heavier and inconsistent for WASM.

### 2. Settings hosts LLM config as a section, not a second window

- **Choice**: Settings view embeds `LlmConfigView` (or extracts shared form logic into a child entity owned by settings). Prefer embedding/reuse of the current view with minimal API tweak (e.g. optional title / focus handoff).
- **Why**: Preserve existing keybindings, mask behavior, and backend calls with least churn.
- **Alternatives**: Rewrite the form inside settings — unnecessary risk for a layout move.

### 3. Main surface is intentionally blank

- **Choice**: Main shows a minimal empty/placeholder pane (no LLM fields, no fake product widgets).
- **Why**: Matches “main can be empty for now”; avoids implying unfinished features.
- **Alternatives**: Temporary “welcome / go to settings” copy — optional light hint is fine if it helps discovery; do not put config controls on main.

### 4. Navigation affordance: simple shell chrome

- **Choice**: Provide an explicit control on the main surface (and a way back from settings), e.g. “Settings” / “Back” actions or header buttons. Keybindings optional if easy.
- **Why**: Without chrome, an empty main traps users with no path to LLM config.
- **Alternatives**: Menu bar only — GPUI/web parity is harder; defer until needed.

### 5. Public exports and entry wiring

- **Choice**: Export the new root view from `teshi-ui`; apps construct it with `SharedLlmBackend` (or pass backend into shell → settings → LLM view). Stop mounting `LlmConfigView` directly in `main` / WASM `run`.
- **Why**: Single root contract for dual entry points (`gpui-shell` requirement).
- **Alternatives**: Apps compose shell locally — duplicates desktop/web.

### 6. Persistence layer unchanged

- **Choice**: No changes to `LlmConfigBackend`, daemon `GET/PUT /api/v1/llm/config`, or engine store APIs unless compile/wiring forces import path updates.
- **Why**: Scope is UI placement only.

## Risks / Trade-offs

- [Users cannot find LLM config after the move] → Mitigation: visible Settings entry on main; keep labels clear (“LLM” / “Settings”).
- [Focus / keybinding context breaks when nested] → Mitigation: keep `LlmConfigView` key context; verify Tab/Enter/Backspace after nesting; adjust focus handle ownership if needed.
- [Over-building settings IA now] → Mitigation: one settings host + one LLM section only; no category tree yet.
- [Empty main looks “broken”] → Mitigation: optional short placeholder text; document as intentional in UI.

## Migration Plan

1. Add shell + settings views; wire LLM form under settings.
2. Switch desktop and web entry points to the new root.
3. Smoke-test: open app → main empty → open settings → load/save LLM → masked key still correct on desktop and WASM.
4. Rollback: revert entry points to mount `LlmConfigView` if critical regression (UI-only rollback).

## Open Questions

- Exact visual chrome (header vs footer button) — implementer picks the simplest GPUI-consistent pattern.
- Whether `LlmConfigView` remains a public export for tests or becomes crate-private under settings — prefer keep public for now if tests/docs reference it; otherwise re-export only the shell root.
