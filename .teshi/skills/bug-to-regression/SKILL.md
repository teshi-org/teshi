---
name: bug-to-regression
description: Turn a human bug report into a Gherkin scenario, bind each step with WinUI UIA locators, replay to verify, and optionally export behave tests
---

# Bug to Regression Skill

Use when the user describes a WinUI3 bug and wants a closed loop: **feature → bindings → replay → (optional) export**.

## Prerequisites

1. Project open in teshi Desktop; **Connect WinUI3 App** active.
2. `.teshi/cdp-endpoint.json` has `"mode": "winapp"`.
3. teshi CLI available (`TESHI_CLI` or `teshi` on PATH).
4. User provides a bug report using [doc/bug-report-template.md](../../doc/bug-report-template.md).

## Phase 1 — Feature (acceptance spec)

1. Read the bug report; draft **one Scenario** with English keywords and project-language step text.
2. Ensure a strong **`Then`** from Expected vs Actual.
3. Create the feature:
   - **TUI**: use `create_feature_file` / `insert_scenario` (user approves with Y).
   - **Desktop terminal only**: output the Gherkin block for the user to save, or edit in the Gherkin panel.
4. Run `teshi steps list --feature '<path>'` after the file exists.

## Phase 2 — Bind each step

For **each** step that is not `confirmed`:

```bash
TESHI=${TESHI_CLI:-teshi}
# Optional: focus Desktop on the step without clicking
$TESHI steps select --feature '<feature>' --line <step_line>

# Follow winapp-locator skill
$TESHI winapp snapshot
# ... highlight, steps propose, user Confirm in Locator panel ...
$TESHI steps wait --until either --timeout 120
```

Shortcut helpers:

```bash
# List steps still needing bindings
$TESHI steps unbound --feature '<feature>'

# Select next unbound step (JSON includes step_line)
$TESHI steps next-unbound --feature '<feature>'
```

Repeat until `steps unbound` returns an empty list.

## Phase 3 — Verify

```bash
$TESHI winapp replay --feature '<feature>' --yes
```

If the user wants step-by-step review, omit `--yes` (see **winapp-replay** skill).

## Phase 4 — Export (optional, CI without teshi)

When all bindings are confirmed:

```bash
$TESHI export --target behave --feature '<feature>' --out ./tests-e2e
```

Follow **behave-export-guide** for `.env` and `behave` commands.

## Do not

- Put selectors in the `.feature` file.
- Skip user Confirm in the Locator panel.
- Run AI locator inference on every CI run (bindings are the source of truth).

## Related skills

- [winapp-locator](../winapp-locator/SKILL.md)
- [winapp-replay](../winapp-replay/SKILL.md)
- [behave-export-guide](../behave-export-guide/SKILL.md)
