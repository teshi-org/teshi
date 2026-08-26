# Bug report template (for Agent → Gherkin)

Copy the block below into chat or a scratch file when starting a **winapp-regression** workflow in teshi. The Agent should turn this into one `.feature` **Scenario** with English keywords (`Given` / `When` / `Then`) and **Chinese step text** (or match your project language).

```text
Title: <short bug title>

Environment:
  - App build/version:
  - OS:
  - Account / role (if relevant):

Preconditions:
  - <state required before reproducing, one bullet per line>

Steps to reproduce:
  1. <user-visible action>
  2. <next action>

Expected:
  - <observable outcome the app should show>

Actual:
  - <what happened instead; include error text if any>

Notes:
  - <window title, timing, data fixtures, screenshots path, etc.>
```

## Agent rules when converting to Gherkin

1. Write **one Scenario** per bug unless the user asks for more.
2. Put **observable acceptance** in `Then` (compare Expected vs Actual).
3. Do **not** embed UIA selectors or `AutomationId` in the feature file.
4. Every Scenario needs at least one `Given` and one `Then`.
5. Keep steps **declarative** (what the user sees), not implementation detail.

## Next steps in teshi

1. Create or update the `.feature` file (TUI Agent tools or paste into teshi Desktop).
2. Connect WinUI3 and run the **winapp-regression** skill.
3. Confirm each binding in the Locator panel.
4. Run `teshi winapp replay`, then optionally `teshi export --target behave`.

See [winapp-modes.md](winapp-modes.md) and [winapp-regression](../skills/winapp-regression/SKILL.md).
