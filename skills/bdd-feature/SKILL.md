---
name: bdd-feature
description: Write, extend, review, or split Gherkin .feature files for teshi (web-ui self-test and general BDD). Use whenever creating scenarios, auditing Feature granularity, or keeping selectors out of step text. Do not use this skill to record locators or replay bindings.
---

# BDD Feature

Use this skill for `.feature` authoring and review. Continue browser binding and replay through the **teshi** skill's binding workflow, with **playwright-locator** for browser operations; use **winapp-regression** for native Windows targets.

When reviewing PRs, splitting scenarios, or unsure about granularity, read [references/convention.md](references/convention.md).

## Output location (teshi web-ui self-test)

- Default: `features/en-US/<name>.feature` (English) and `features/zh-CN/<name>.feature` (Chinese)
- Tag Teshi Web scenarios `@web-ui`; add runner-specific tags only when the selected runner requires them.

CLI control-plane E2E lives under `tests/feature/en-US/` and `tests/feature/zh-CN/`. For a WinUI3 bug regression in another project, put the Feature in that project’s usual `features/` directory and continue with **winapp-regression**.

## Language rules

- Gherkin **keywords** in English (`Feature`, `Background`, `Given`, `When`, `Then`, `And`).
- Step text in the project language (Chinese or English) describing user-visible behavior.
- **Never** embed CSS selectors, testids, AutomationIds, or URLs in step text unless the step explicitly mentions a URL.

## Background template (dev SUT with automation flags)

```gherkin
# language: en

@web-ui
Feature: <short title>
  <one-line description>

  Background:
    Given teshi web is running at http://127.0.0.1:20253/?e2e=1
```

Use the actual configured daemon address when it differs. The supported frontend is GPUI WASM. When working in the Teshi source checkout, consult `doc/web-ui-self-test.md` for build and validation guidance; installed consumers need not have that document. Do not assume legacy React/Vite DOM selectors or ports apply.

## Step granularity (important for replay)

| Intent | Step pattern | Binding style |
|--------|--------------|---------------|
| Establish project context | Separate `Given` | setup supported by the actual SUT/runner |
| Switch a supported panel | Separate `When`/`And` | verified panel control |
| Run terminal command in a SUT that provides a terminal | One command per step | verified terminal input |
| Item appears in a SUT list | Separate `Then` | verified item assertion |
| Navigate to URL | Background or explicit Given | `navigate` action |

Do **not** combine "switch tab + assert file" in one step if replay needs intermediate state.

## After writing

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI steps list --feature features/en-US/<name>.feature
$TESHI steps unbound --feature features/en-US/<name>.feature
```

Hand unbound browser steps to **teshi** for binding and **playwright-locator** for browser verification. Hand unbound WinUI3 steps to **winapp-regression**. If a specialist is unavailable, inspect the relevant CLI help rather than assuming a repository path exists.

## Do not

- Write selectors into `.feature` files.
- Use `.locators.md` (deprecated).
- Record or replay bindings in this skill.
