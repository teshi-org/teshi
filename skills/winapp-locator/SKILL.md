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
4. The user or CLI selected a Gherkin **step** (`teshi steps select` / `next-unbound` writes `.teshi/active-step.json`).
5. You are working from the project root in the embedded terminal.
6. A compatible teshi CLI (**>= 0.4.0**). Prefer `TESHI_CLI` when set; otherwise use `teshi` from PATH.
7. The target app window is attached. If it is not attached yet, list windows and attach explicitly.

If any prerequisite is missing, stop and tell the user what to do in the Desktop UI first.

## Context Files

| File | Purpose |
|------|---------|
| `.teshi/active-step.json` | Selected feature path, scenario, step line, step text (CLI or Desktop) |
| `.teshi/settings.json` | `locator_auto_confirm_sec` (default 60; 0 = manual only) |
| `.teshi/cdp-endpoint.json` | `mode: "winapp"` and `ws_url` for the sidecar |
| `.teshi/pending-locator.json` | Written by `teshi steps propose`; reviewed by Desktop/web |
| `.teshi/step-bindings/{feature}.json` | Written only after confirmation; commit this file |

## Workflow

### 1. Check CLI and load context

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

```bash
$TESHI winapp list-windows
$TESHI winapp attach --hwnd 123456
$TESHI winapp attach --title 'My App'
$TESHI winapp attach --process-name MyApp.exe
```

Do not guess a destructive or unrelated app window. If multiple candidates are plausible, ask the user which one to attach.

### 3. Inspect the UIA tree

```bash
$TESHI winapp snapshot
```

Match elements to `step_text`. Prefer selector stability in this order:

1. `uia:automation_id=...`
2. `uia:control_type=...;name=...`
3. `uia:name=...`
4. `uia:path=...` (last resort; document risk)

For list items without AutomationId, see [doc/winui-automation-ids.md](../../doc/winui-automation-ids.md). Prefer app-side `AutomationId` (e.g. `LibraryGameItem_{id}`) over fragile name/path selectors.

For ambiguous controls, include the bounding rectangle and rationale in your analysis before proposing.

### 4. Verify and highlight

```bash
$TESHI winapp highlight 'uia:automation_id=LoginButton'
$TESHI winapp execute --selector 'uia:automation_id=LoginButton' --action assert_visible
$TESHI winapp execute --selector 'uia:name=Welcome' --action assert_text --value-arg 'Welcome'
```

For actions that mutate app state (`click`, `fill`, `select`, `press_key`), only execute when the selected Gherkin step clearly describes that action.

For system/process checks, use `--action exec` with `--value-arg` (PowerShell one-liner) instead of fake UIA elements.

### 5. Propose and wait

```bash
$TESHI steps propose \
  --line <step_line> \
  --strategy uia \
  --value 'uia:automation_id=LoginButton' \
  --action click \
  --confidence 0.92 \
  --rationale 'Step mentions the login button; AutomationId is unique in the UIA tree' \
  --highlight-applied
```

`--line` must match `active-step.json`; mismatch exits with code 1.

For `fill`, `assert_text`, `select`, `press_key`, and `exec`, pass `--value-arg`. Sensitive values should be placeholders such as `${LOGIN_PW}`, not real secrets.

Default wait (auto-confirm after 60s unless user Rejects):

```bash
$TESHI steps wait --until confirmed --timeout 60 --auto-confirm
```

On mismatch between active step and pending proposal, auto-confirm **rejects** and exits 2. User can still Confirm manually when aligned.

For manual-only review, omit `--auto-confirm`.

If the proposal is rejected, stop and tell the user the step was rejected. Do not automatically re-propose.

## Do Not

- Edit `.feature` files or step definitions in this workflow.
- Write `{stem}.locators.md`.
- Confirm on the user's behalf when `--auto-confirm` is off and they asked to review visually.
- Use coordinate/path selectors when a stable `AutomationId` exists.
