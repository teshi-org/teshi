---
name: bdd-feature
description: Write, extend, review, or split Gherkin .feature files for teshi (web-ui self-test and general BDD). Use whenever creating scenarios, auditing Feature granularity, or keeping selectors out of step text. Do not use this skill to record locators or replay bindings.
---

# BDD Feature

Create or change `.feature` files only. Locator recording and replay belong to **playwright-locator** (browser) or **winapp-regression** (WinUI3).

When reviewing PRs, splitting scenarios, or unsure about granularity, read [references/convention.md](references/convention.md).

## Output location (teshi web-ui self-test)

- Default: `tests/feature/web-ui/<name>.feature`
- Tags: `@web-ui @embedded`

For a WinUI3 bug regression, put the Feature in the project’s usual features directory and continue with **winapp-regression**.

## Language rules

- Gherkin **keywords** in English (`Feature`, `Background`, `Given`, `When`, `Then`, `And`).
- Step text in the project language (Chinese or English) describing user-visible behavior.
- **Never** embed CSS selectors, testids, AutomationIds, or URLs in step text unless the step explicitly mentions a URL.

## Background template (dev SUT with automation flags)

```gherkin
# language: en

@web-ui @embedded
Feature: <short title>
  <one-line description>

  Background:
    Given teshi web is running at http://127.0.0.1:1420/?e2e=1
```

For CI/stable dist-only runs, use port `1421` without Vite.

## Step granularity (important for replay)

| Intent | Step pattern | Binding style |
|--------|--------------|---------------|
| Open project | Separate `Given` | `open_project` API action |
| Switch Files tab | Separate `When`/`And` | click `FileTreeTab` |
| Run terminal command | One command per step | `type` on `.xterm-helper-textarea` |
| File appears in tree | Separate `Then` | `assert_visible` on `FileTreeNode-<file>` |
| Navigate to URL | Background or explicit Given | `navigate` action |

Do **not** combine "switch tab + assert file" in one step if replay needs intermediate state.

## After writing

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI steps list --feature tests/feature/web-ui/<name>.feature
$TESHI steps unbound --feature tests/feature/web-ui/<name>.feature
```

Hand unbound browser steps to **playwright-locator**. Hand unbound WinUI3 steps to **winapp-regression**.

## Do not

- Write selectors into `.feature` files.
- Use `.locators.md` (deprecated).
- Record or replay bindings in this skill.
