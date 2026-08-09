---
name: bug-to-regression
description: Turn a human bug report into a Gherkin scenario, bind each step with WinUI UIA locators, replay to verify, and optionally export behave tests
---

# Bug to Regression Skill

Use when the user describes a WinUI3 bug and wants a closed loop: **feature → bindings → replay → (optional) export**.

## Prerequisites

1. Project open in teshi Desktop; **Connect WinUI3 App** active.
2. `.teshi/cdp-endpoint.json` has `"mode": "winapp"`.
3. User provides a bug report using [doc/bug-report-template.md](../../doc/bug-report-template.md).

### CLI self-check (run first)

External terminals do **not** inherit Desktop `TESHI_CLI`. Always verify the CLI before `winapp` / `steps` / `export`:

```bash
TESHI="${TESHI_CLI:-teshi}"
$TESHI --version          # must be >= 0.4.0
$TESHI winapp --help      # fail → stop; use Desktop embedded terminal or set TESHI_CLI to dev build
$TESHI steps --help       # 0.3.0 MSI lacks steps/winapp/export — do not guess commands
```

Prefer the Desktop embedded terminal (sets `TESHI_CLI` to the dev build) or export:

```powershell
$env:TESHI_CLI = 'D:\Dev\Rust\teshi\target\debug\teshi.exe'
```

## Phase 1 — Feature (acceptance spec)

1. Read the bug report; draft **one Scenario** with English keywords and project-language step text.
2. Ensure a strong **`Then`** from Expected vs Actual.
3. Create the feature:
   - **TUI**: use `create_feature_file` / `insert_scenario` (user approves with Y).
   - **Desktop terminal only**: output the Gherkin block for the user to save, or edit in the Gherkin panel.
4. Run `$TESHI steps list --feature '<path>'` after the file exists.

## Phase 2 — Bind each step (unattended loop)

`steps select`, `steps next-unbound`, and `steps unbind` require **teshi 0.4.0+**.

Before each propose, confirm `.teshi/active-step.json` matches the step you intend (`step_line`, `feature_relative_path`).

Default unattended binding loop:

```bash
TESHI="${TESHI_CLI:-teshi}"
while true; do
  NEXT=$($TESHI steps next-unbound --feature '<feature>' 2>/dev/null) || break
  echo "$NEXT"   # JSON includes step_line — verify against active-step.json

  $TESHI winapp snapshot
  # highlight primary candidate, then propose (optional --line guard):
  $TESHI steps propose \
    --line <step_line> \
    --strategy uia \
    --value 'uia:automation_id=...' \
    --action click \
    --confidence 0.9 \
    --rationale '...' \
    --highlight-applied

  # Auto-confirm after 60s unless user rejects in Locator panel (locator_auto_confirm_sec in .teshi/settings.json)
  $TESHI steps wait --until confirmed --timeout 60 --auto-confirm || exit 2
done
```

Remove a wrong binding:

```bash
$TESHI steps unbind --feature '<feature>' --line <step_line>
```

For manual review, omit `--auto-confirm` on `steps wait` and ask the user to Confirm in the Locator panel.

Follow **winapp-locator** for snapshot/highlight/propose details.

## System / process preconditions (non-UI steps)

Keep declarative Gherkin steps. For process checks (e.g. Steam running), bind with:

```bash
$TESHI steps propose --action exec --value 'system-check' \
  --value-arg 'Get-Process steam -ErrorAction SilentlyContinue | Select-Object -First 1' \
  --strategy script --confidence 1.0 --rationale 'Steam must be running'
```

Or use `assert_visible` with a placeholder selector only when no better option exists; patch generated hooks after export if needed.

## Phase 3 — Verify (preflight + replay)

Before replay:

```bash
$TESHI winapp list-windows
$TESHI winapp attach --title 'Target App'   # or --hwnd / --process-name
# optional one-shot:
$TESHI winapp replay --feature '<feature>' --launch 'C:\path\to\App.exe' --yes
```

**Background** must include **navigation** bindings (e.g. click Library), not only `assert_visible`.

```bash
$TESHI winapp replay --feature '<feature>' --yes
```

If the user wants step-by-step review, omit `--yes` (see **winapp-replay** skill).

## Phase 4 — Export (optional, CI without teshi)

When all bindings are confirmed:

```bash
$TESHI export --target behave --feature '<feature>' --out ./tests-e2e
```

After export:

```powershell
Get-ChildItem -Recurse tests-e2e\__pycache__ | Remove-Item -Recurse -Force
cd tests-e2e
behave --dry-run
```

Follow **behave-export-guide** for `.env` and full `behave` commands.

## Do not

- Put selectors in the `.feature` file.
- Use teshi 0.3.0 MSI commands that do not exist (`winapp`, `steps`, `export`).
- Run AI locator inference on every CI run (bindings are the source of truth).

## Related skills

- [winapp-locator](../winapp-locator/SKILL.md)
- [winapp-replay](../winapp-replay/SKILL.md)
- [behave-export-guide](../behave-export-guide/SKILL.md)
