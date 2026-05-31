---
name: bdd-locator
description: Infer DOM locators for the selected Gherkin step in teshi-desktop using CDP context files
---

# BDD Locator Skill

Use this skill when the user is recording BDD step locators in **teshi-desktop**.

## Prerequisites

1. A project is open in teshi-desktop.
2. A browser session is running in the Browser panel:
   - **Connect Chrome** — for logged-in apps (requires `extension/teshi-bridge` loaded in Chrome; user must activate the target tab), or
   - **Start Embedded** — headless Playwright preview for local/staging URLs.
3. `.teshi/cdp-endpoint.json` exists and `page_url` matches the page under test.
4. The user selected a Gherkin **step** in the left panel (Desktop writes `.teshi/active-step.json`).
5. You are working from the project root in the embedded terminal.

If any prerequisite is missing, stop and tell the user what to do in the Desktop UI first (see [doc/browser-modes.md](../../doc/browser-modes.md)).

## Context files (read-only unless noted)

| File | Purpose |
|------|---------|
| `.teshi/active-step.json` | Selected feature path, scenario, step line, step text |
| `.teshi/cdp-endpoint.json` | `mode` (`chrome` \| `embedded`), `ws_url`, `page_url`, optional `extension_connected` |
| `.teshi/pending-locator.json` | **You write** the proposal here (status must be `pending`) |

Do **not** write `.locators.md` directly. The user confirms in the Desktop Locator panel.

## Workflow

### 1. Load context

Read `.teshi/active-step.json` and `.teshi/cdp-endpoint.json`.

Extract:

- `step_text`, `step_keyword`, `step_line`, `feature_relative_path`
- `mode`, `ws_url`, `page_url`
- For `mode: "chrome"`, confirm `extension_connected` is true before snapshot; if false, ask the user to load the extension and retry Connect Chrome.

### 2. Inspect the page

Call the sidecar WebSocket at `ws_url` from `cdp-endpoint.json` (same commands for both modes).

Recommended inspection order:

1. `get_page_snapshot` (accessibility tree + interactive_elements)
2. Match elements to `step_text`
3. Stable selectors: `data-testid` > `[role=...][name=...]` > unique text > CSS path

```json
{"cmd":"get_page_snapshot","request_id":"snap-1"}
```

```json
{"cmd":"highlight_selector","request_id":"hl-1","selector":"[data-testid=\"login-btn\"]"}
```

In **chrome** mode, `navigate` is not supported — the user changes URL in Chrome manually.

### 3. Infer locators

Produce **1–3** candidates ranked by stability and match to `step_text`.

For click steps, set `"action": "click"`. Map keyword intent:

- **When** → interaction (click, fill, select)
- **Then** → assertion target (visible text, element state)

Each candidate:

```json
{
  "rank": 1,
  "strategy": "css",
  "value": "[data-testid=\"login-btn\"]",
  "action": "click",
  "confidence": 0.92,
  "rationale": "Step mentions login button; unique data-testid on page"
}
```

### 4. Highlight the primary candidate

Before writing the proposal, highlight rank **1** via sidecar:

```json
{"cmd":"highlight_selector","request_id":"hl-1","selector":"<rank-1 value>"}
```

If highlight fails (e.g. DevTools open on the tab in chrome mode), lower confidence and explain in `rationale`.

### 5. Write pending proposal

Write `.teshi/pending-locator.json`:

```json
{
  "step_ref": { "...copy entire active-step.json object..." },
  "candidates": [ ... ],
  "highlight": { "candidate_rank": 1, "applied": true },
  "status": "pending"
}
```

**Important:** set `step_ref` to a verbatim copy of `.teshi/active-step.json` (including `updated_at`) so Desktop can parse the proposal.

For **assertion** steps (Then / visibility / title checks), highlight may be impossible. Set `"applied": false` and explain in `rationale`.

Tell the user to review the highlighted element and confirm in the **Locator** bottom panel.

## Selector guidelines

- Prefer selectors that match **one** element.
- Avoid brittle positional selectors (`nth-child`) unless no alternative exists.
- Quote dynamic text with care; prefer partial text or role-based selectors.
- If the step is ambiguous, state assumptions in `rationale` and offer multiple candidates.

## Do not

- Edit `.feature` files or step definitions in this workflow.
- Write `{stem}.locators.md` — Desktop writes it after user confirmation.
- Overwrite `active-step.json` or `cdp-endpoint.json`.
- Start Embedded when the user needs a logged-in Chrome session (use Connect Chrome instead).

## Example agent summary to user

> I read the selected step "When I click the login button", inspected the page at `/login`, and highlighted `[data-testid="login-btn"]`. Three candidates are in `.teshi/pending-locator.json`. Please confirm in the Locator panel.
