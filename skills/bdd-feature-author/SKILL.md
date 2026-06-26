---
name: bdd-feature-author
description: Write Gherkin feature files for teshi web-ui self-test scenarios (no selectors in steps)
---

# BDD Feature Author Skill

Use when an **external agent** must **create or extend** `.feature` files for teshi web UI testing. Binding/locator work belongs to **bdd-locator**, not this skill.

## Output location

- Default: `tests/feature/web-ui/<name>.feature`
- Tags: `@web-ui @embedded`

## Language rules

- Gherkin **keywords** in English (`Feature`, `Background`, `Given`, `When`, `Then`, `And`).
- Step text in the project language (Chinese or English) describing user-visible behavior.
- **Never** embed CSS selectors, testids, or URLs in step text unless the step explicitly mentions a URL.

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

Hand off unbound steps to **bdd-locator** via **agent-web-ui-flow**.

## Do not

- Write selectors into `.feature` files.
- Use `.locators.md` (deprecated).
- Record bindings in this skill.
