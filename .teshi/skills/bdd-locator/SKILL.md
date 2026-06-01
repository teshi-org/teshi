---
name: bdd-locator
description: Infer DOM locators for the selected Gherkin step, propose them through teshi steps, and wait for Desktop/web confirmation
---

# BDD Locator Skill

Use this skill when the user is recording a new BDD step binding in **teshi Desktop** or **teshi web**.

## Prerequisites

1. A project is open in teshi Desktop/web.
2. A browser session is running in the Browser panel:
   - **Connect Chrome** for logged-in apps using the dedicated recording Chrome profile with `extension/teshi-bridge` loaded.
   - **Start Embedded** only for local/staging pages that do not require a real login profile.
3. `.teshi/cdp-endpoint.json` exists. For non-navigation steps, `page_url` should already match the page under test; explicit URL navigation steps may correct it with `teshi browser navigate`.
4. The user selected a Gherkin **step** in the left panel (the app writes `.teshi/active-step.json`).
5. You are working from the project root in the embedded terminal.
6. A compatible teshi CLI is available. Prefer `TESHI_CLI` when set; otherwise use `teshi` from PATH.

If any prerequisite is missing, stop and tell the user what to do in the Desktop UI first (see [doc/browser-modes.md](../../doc/browser-modes.md)).

## Context Files

| File | Purpose |
|------|---------|
| `.teshi/active-step.json` | Selected feature path, scenario, step line, step text |
| `.teshi/cdp-endpoint.json` | `mode` (`chrome` \| `embedded`), `ws_url`, `page_url`, optional `extension_connected` |
| `.teshi/pending-locator.json` | Written by `teshi steps propose`; reviewed by Desktop/web |
| `.teshi/step-bindings/{feature}.json` | Written only after user confirmation; commit this file |

Do **not** write `.locators.md`. It is deprecated and no longer part of the workflow.

## Workflow

### 1. Check CLI and load context

Resolve the CLI command:

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI --version
$TESHI browser navigate --help
$TESHI steps propose --help
```

If any command fails, stop and ask the user to install a newer teshi MSI, fix PATH, or set `TESHI_CLI` to a compatible development binary.

Read `.teshi/active-step.json` and `.teshi/cdp-endpoint.json`.

Extract:

- `step_text`, `step_keyword`, `step_line`, `feature_relative_path`
- `mode`, `ws_url`, `page_url`
- For `mode: "chrome"`, confirm `extension_connected` is true before browser commands; if false, ask the user to load the extension and retry Connect Chrome.

### 2. Handle explicit URL navigation steps

If the selected step clearly means opening a URL and contains an explicit `http(s)` or `file` URL, navigate first:

```bash
$TESHI browser navigate '<url>'
```

Then propose the navigation binding:

```bash
$TESHI steps propose \
  --strategy url \
  --action navigate \
  --value-arg '<url>' \
  --confidence 1.0 \
  --rationale 'Step explicitly opens this URL'
```

Tell the user to confirm or reject the navigation binding in the Locator panel, then wait:

```bash
$TESHI steps wait --until either --timeout 120
```

If the proposal is rejected, stop. Do not automatically re-propose. If the step is not an explicit URL navigation step, continue below.

### 3. Inspect the page

Use the stable CLI wrapper:

Recommended inspection order:

1. `$TESHI browser snapshot`
2. Match elements to `step_text`
3. Stable selectors: `data-testid` > `[role=...][name=...]` > unique text > CSS path

```bash
$TESHI browser snapshot
$TESHI browser highlight '[data-testid="login-btn"]'
```

Verify selectors before proposing (use `--value-arg` for fill/assert_text/select/press_key, same as `steps propose`):

```bash
$TESHI browser execute --selector '[data-testid="login-btn"]' --action click
$TESHI browser execute --selector 'input[name=email]' --action fill --value-arg 'demo@example.com'
$TESHI browser execute --selector 'h1' --action assert_text --value-arg 'Welcome'
```

On slow pages, allow a longer snapshot wait: `$TESHI browser snapshot --timeout-ms 90000`.

### 4. Infer locators

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
  "value_arg": null,
  "confidence": 0.92,
  "rationale": "Step mentions login button; unique data-testid on page"
}
```

### 5. Highlight the primary candidate

Before writing the proposal, highlight rank **1**:

```bash
$TESHI browser highlight '<rank-1 selector>'
```

If highlight fails (e.g. DevTools open on the tab in chrome mode), lower confidence and explain in `rationale`.

### 6. Propose and wait

Write the pending proposal with `teshi steps propose`:

```bash
$TESHI steps propose \
  --strategy css \
  --value '[data-testid="login-btn"]' \
  --action click \
  --confidence 0.92 \
  --rationale 'Step mentions login button; unique data-testid on page' \
  --highlight-applied
```

For `fill`, `assert_text`, `select`, and `press_key`, pass `--value-arg`. Sensitive values should be placeholders such as `${LOGIN_PW}`, not real secrets.

Tell the user to review the highlighted element and click **Confirm** or **Reject** in the Locator panel. Then wait:

```bash
$TESHI steps wait --until either --timeout 120
```

If wait exits rejected/non-zero, stop and tell the user the step was rejected. Do not automatically re-propose.

## Selector guidelines

- Prefer selectors that match **one** element.
- Avoid brittle positional selectors (`nth-child`) unless no alternative exists.
- Quote dynamic text with care; prefer partial text or role-based selectors.
- If the step is ambiguous, state assumptions in `rationale` and offer multiple candidates.

## Do not

- Edit `.feature` files or step definitions in this workflow.
- Write `{stem}.locators.md`.
- Overwrite `active-step.json` or `cdp-endpoint.json`.
- Start Embedded when the user needs a logged-in Chrome session (use Connect Chrome instead).
- Confirm on the user's behalf; visual confirmation belongs to Desktop/web.

## Example agent summary to user

> I read the selected step "When I click the login button", inspected the page at `/login`, highlighted `[data-testid="login-btn"]`, and proposed it through `teshi steps`. Please confirm or reject it in the Locator panel.
