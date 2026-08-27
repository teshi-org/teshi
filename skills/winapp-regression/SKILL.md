---
name: winapp-regression
description: Turn a WinUI3 bug report into a Gherkin scenario, bind UIA locators, replay to verify, and optionally export behave tests. Use whenever connecting a WinUI3 or native Windows app in teshi, recording UIA step-bindings, replaying them, or exporting CI tests. Do not use for Chrome or embedded browser locators.
---

# WinApp Regression

Closed loop for WinUI3 / native Windows: **feature → bind → replay → (optional) export**. Stay in this skill for locator, replay, and export. Browser work belongs to **playwright-locator**. Gherkin conventions live in **bdd-feature**.

## Prerequisites

1. Project open in teshi Desktop/web; **Connect WinUI3 App** active.
2. `.teshi/cdp-endpoint.json` has `"mode": "winapp"`.
3. Working from project root. Prefer the Desktop embedded terminal (`TESHI_CLI` is set).
4. teshi CLI **>= 0.4.0**. The 0.3.0 MSI lacks `winapp` / `steps` / `export` — do not guess commands.

```bash
TESHI="${TESHI_CLI:-teshi}"
$TESHI --version
$TESHI winapp --help
$TESHI steps --help
```

External PowerShell if Desktop did not inject the CLI:

```powershell
$env:TESHI_CLI = 'D:\Dev\Rust\teshi\target\debug\teshi.exe'
```

If `mode` is not `winapp`, stop and ask the user to click **Connect WinUI3 App**.

## Context files

| File | Purpose |
|------|---------|
| `.teshi/active-step.json` | Selected feature path, scenario, step line, step text |
| `.teshi/settings.json` | `locator_auto_confirm_sec` (default 60; 0 = manual only) |
| `.teshi/cdp-endpoint.json` | `mode: "winapp"` and `ws_url` |
| `.teshi/pending-locator.json` | Written by `teshi steps propose` |
| `.teshi/step-bindings/{feature}.json` | Written only after confirmation; commit this file |

Do not write `{stem}.locators.md`.

## Phase 1 — Feature

When the user describes a bug, use [doc/bug-report-template.md](../../doc/bug-report-template.md). Draft **one Scenario** with English keywords and project-language step text. Put a strong **`Then`** from Expected vs Actual. Apply **bdd-feature** conventions; do not embed UIA selectors in the Feature.

Create the file:

- **TUI**: `create_feature_file` / `insert_scenario` (user approves with Y).
- **Desktop terminal**: output the Gherkin block for the user to save, or edit in the Gherkin panel.

Then:

```bash
$TESHI steps list --feature '<path>'
```

## Phase 2 — Bind each step

Attach the target window before inspecting. Do not guess a destructive or unrelated window. If several candidates are plausible, ask.

```bash
$TESHI winapp list-windows
$TESHI winapp attach --hwnd 123456
$TESHI winapp attach --title 'My App'
$TESHI winapp attach --process-name MyApp.exe
```

Inspect the UIA tree and match `step_text`. Prefer selector stability in this order:

1. `uia:automation_id=...`
2. `uia:control_type=...;name=...`
3. `uia:name=...`
4. `uia:path=...` (last resort; document risk)

For list items without AutomationId, see [doc/winui-automation-ids.md](../../doc/winui-automation-ids.md). Prefer app-side IDs (e.g. `LibraryGameItem_{id}`) over fragile name/path selectors.

Verify before proposing:

```bash
$TESHI winapp snapshot
$TESHI winapp highlight 'uia:automation_id=LoginButton'
$TESHI winapp execute --selector 'uia:automation_id=LoginButton' --action assert_visible
$TESHI winapp execute --selector 'uia:name=Welcome' --action assert_text --value-arg 'Welcome'
```

For actions that mutate app state (`click`, `fill`, `select`, `press_key`), execute only when the selected Gherkin step clearly describes that action.

For process checks (e.g. Steam running), keep declarative Gherkin and bind with `--action exec`:

```bash
$TESHI steps propose --action exec --value 'system-check' \
  --value-arg 'Get-Process steam -ErrorAction SilentlyContinue | Select-Object -First 1' \
  --strategy script --confidence 1.0 --rationale 'Steam must be running'
```

Default unattended binding loop (`steps select`, `next-unbound`, and `unbind` require teshi 0.4.0+). Confirm `.teshi/active-step.json` matches the intended `step_line` and `feature_relative_path` before each propose.

```bash
TESHI="${TESHI_CLI:-teshi}"
while true; do
  NEXT=$($TESHI steps next-unbound --feature '<feature>' 2>/dev/null) || break
  echo "$NEXT"

  $TESHI winapp snapshot
  $TESHI steps propose \
    --line <step_line> \
    --strategy uia \
    --value 'uia:automation_id=...' \
    --action click \
    --confidence 0.9 \
    --rationale '...' \
    --highlight-applied

  $TESHI steps wait --until confirmed --timeout 60 --auto-confirm || exit 2
done
```

`--line` must match `active-step.json`; mismatch exits with code 1. For `fill`, `assert_text`, `select`, `press_key`, and `exec`, pass `--value-arg`. Use placeholders such as `${LOGIN_PW}`, not real secrets.

On mismatch between active step and pending proposal, auto-confirm **rejects** and exits 2. For visual review, omit `--auto-confirm`. If rejected, stop — do not auto re-propose.

Remove a wrong binding:

```bash
$TESHI steps unbind --feature '<feature>' --line <step_line>
```

## Phase 3 — Replay

Preflight:

1. App is running (or use `--launch`).
2. Window attached — replay fails fast when detached.
3. Background includes **navigation** bindings (e.g. open Library), not only `assert_visible`.
4. UI state matches the scenario start.

```bash
$TESHI winapp list-windows
$TESHI winapp attach --title 'My App'
$TESHI steps resolve --feature '<feature-relative-path>'
```

Interactive (default):

```bash
$TESHI winapp replay --feature '<feature-relative-path>' --until-line <line>
```

Non-interactive:

```bash
$TESHI winapp replay --feature '<feature-relative-path>' --yes
$TESHI winapp replay --feature '<feature>' --launch 'C:\path\to\App.exe' --yes
```

Dry run:

```bash
$TESHI winapp replay --feature '<feature-relative-path>' --dry-run
```

If replay reports **not attached**, run attach/launch first. If a line fails, report line, action, selector, and error; snapshot and re-bind that step (`steps unbind` + propose). Do not invent selectors during replay. Do not use `teshi browser replay` for WinUI3 targets.

## Phase 4 — Export (optional)

When all bindings are confirmed:

```bash
$TESHI export --target behave --feature '<feature>' --out ./tests-e2e
```

Then read [references/behave-export.md](references/behave-export.md) for `.env`, venv, `behave --dry-run`, and CI.

## Do not

- Put selectors in the `.feature` file.
- Use teshi 0.3.0 MSI commands that do not exist.
- Run AI locator inference on every CI run (bindings are the source of truth).
- Confirm on the user's behalf when `--auto-confirm` is off and they asked to review visually.
- Use coordinate/path selectors when a stable `AutomationId` exists.
- Assume replay starts the app without `--launch` or a running process.
