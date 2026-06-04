---
name: winapp-locator
description: Infer UI Automation locators for the selected Gherkin step and propose them through teshi steps
---

# WinApp Locator Skill

Use this skill when the user is recording a BDD step binding for a WinUI3 or native Windows app in teshi Desktop/web.

## Prerequisites

1. A project is open in teshi Desktop/web.
2. **Connect WinUI3 App** is running in the Target panel.
3. `.teshi/cdp-endpoint.json` exists and has `"mode": "winapp"`.
4. The user selected a Gherkin **step** in the left panel, so `.teshi/active-step.json` exists.
5. You are working from the project root in the embedded terminal.
6. A compatible teshi CLI is available. Prefer `TESHI_CLI` when set; otherwise use `teshi` from PATH.
7. The target app window is attached. If it is not attached yet, list windows and attach explicitly.

If any prerequisite is missing, stop and tell the user what to do in the Desktop UI first.

## Context Files

| File | Purpose |
|------|---------|
| `.teshi/active-step.json` | Selected feature path, scenario, step line, step text |
| `.teshi/cdp-endpoint.json` | `mode: "winapp"` and `ws_url` for the sidecar |
| `.teshi/pending-locator.json` | Written by `teshi steps propose`; reviewed by Desktop/web |
| `.teshi/step-bindings/{feature}.json` | Written only after user confirmation; commit this file |

## Workflow

### 1. Check CLI and load context

Resolve the CLI command:

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI --version
$TESHI winapp snapshot --help
$TESHI steps propose --help
```

Read `.teshi/active-step.json` and `.teshi/cdp-endpoint.json`.

Extract:

- `step_text`, `step_keyword`, `step_line`, `feature_relative_path`
- `mode`, `ws_url`, `page_url`

If `mode` is not `winapp`, ask the user to click **Connect WinUI3 App**.

### 2. Attach the target window when needed

If snapshot says no window is attached, list windows:

```bash
$TESHI winapp list-windows
```

Attach using the most explicit target the user provided:

```bash
$TESHI winapp attach --hwnd 123456
$TESHI winapp attach --title 'My App'
$TESHI winapp attach --process-name MyApp.exe
```

Do not guess a destructive or unrelated app window. If multiple candidates are plausible, ask the user which one to attach.

### 3. Inspect the UIA tree

Use the stable CLI wrapper:

```bash
$TESHI winapp snapshot
```

Match elements to `step_text`. Prefer selector stability in this order:

1. `uia:automation_id=...`
2. `uia:control_type=...;name=...`
3. `uia:name=...`
4. `uia:path=...`

For ambiguous controls, include the bounding rectangle and rationale in your analysis before proposing.

### 4. Verify and highlight

Before proposing, highlight the primary candidate:

```bash
$TESHI winapp highlight 'uia:automation_id=LoginButton'
```

For non-destructive assertions, verify with execute:

```bash
$TESHI winapp execute --selector 'uia:automation_id=LoginButton' --action assert_visible
$TESHI winapp execute --selector 'uia:name=Welcome' --action assert_text --value-arg 'Welcome'
```

For actions that mutate app state (`click`, `fill`, `select`, `press_key`), only execute when the selected Gherkin step clearly describes that action.

### 5. Propose and wait

Write the pending proposal with `teshi steps propose`:

```bash
$TESHI steps propose \
  --strategy uia \
  --value 'uia:automation_id=LoginButton' \
  --action click \
  --confidence 0.92 \
  --rationale 'Step mentions the login button; AutomationId is unique in the UIA tree' \
  --highlight-applied
```

For `fill`, `assert_text`, `select`, and `press_key`, pass `--value-arg`. Sensitive values should be placeholders such as `${LOGIN_PW}`, not real secrets.

Tell the user to review the highlighted element and click **Confirm** or **Reject** in the Locator panel. Then wait:

```bash
$TESHI steps wait --until either --timeout 120
```

If the proposal is rejected, stop and tell the user the step was rejected. Do not automatically re-propose.

## Do Not

- Edit `.feature` files or step definitions in this workflow.
- Write `{stem}.locators.md`.
- Overwrite `active-step.json` or `cdp-endpoint.json`.
- Confirm on the user's behalf; visual confirmation belongs to Desktop/web.
- Use coordinate/path selectors when a stable `AutomationId` exists.
