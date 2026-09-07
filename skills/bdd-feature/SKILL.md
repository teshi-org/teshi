---
name: bdd-feature
description: Write, extend, review, or split Gherkin .feature files for teshi (web-ui self-test and general BDD). Use whenever creating scenarios, auditing Feature granularity, or keeping selectors out of step text. Do not use this skill to record locators or replay bindings.
---

# BDD Feature

Use this skill for `.feature` authoring and review. Continue browser binding and replay through the **teshi** skill's binding workflow, with **playwright-locator** for browser operations; use **winapp-regression** for native Windows targets.

When reviewing PRs, splitting scenarios, or unsure about granularity, read [references/convention.md](references/convention.md).

## Output location (Teshi product self-test)

- Default: `features/en-US/<english-name>.feature` and `features/zh-CN/<中文名>.feature`
- English filenames are snake_case capability names (`requirement_listing.feature`). Chinese filenames match the `功能` title (`需求列表.feature`). Sidecar `*.bindings.json` files use the same stem
- Tag Teshi Web scenarios `@web-ui`
- Tag requirement CLI control-plane scenarios `@cli` only
- CLI E2E is executed with `teshi-requirement-cli-runner` against `@cli` files in those language directories. Run those files (or a glob of them), not the whole language directory — it also contains `@web-ui` scenarios
- Add further tags only when the selected runner requires them

For a WinUI3 bug regression in another project, put the Feature in that project’s usual `features/` directory and continue with **winapp-regression**.

## Language rules

Teshi self-test keeps paired locales. Keywords must match the Gherkin language header; step text uses that locale.

- `features/en-US/`: English snake_case filenames; `# language: en` with English keywords (`Feature`, `Background`, `Scenario`, `Given`, `When`, `Then`, `And`) and English step text
- `features/zh-CN/`: Chinese filenames matching the `功能` title; `# language: zh-CN` with Chinese keywords (`功能`, `背景`, `场景`, `假如`, `当`, `那么`, `并且`) and Chinese step text
- Do not mix English keywords under `# language: zh-CN`, or translated keywords under `# language: en`
- **Never** embed CSS selectors, testids, AutomationIds, or URLs in step text unless the step explicitly mentions a URL

## Background template (dev SUT with automation flags)

```gherkin
# language: en

@web-ui
Feature: <short title>
  <one-line description>

  Background:
    Given teshi web is running at http://127.0.0.1:20253/?e2e=1
```

Chinese pair:

```gherkin
# language: zh-CN

@web-ui
功能: <短标题>
  <一句话描述>

  背景:
    假如 teshi web 运行在 http://127.0.0.1:20253/?e2e=1
```

Use the actual configured daemon address when it differs. The supported frontend is GPUI WASM. When working in the Teshi source checkout, consult `doc/web-ui-self-test.md` for build and validation guidance; installed consumers need not have that document. Do not assume legacy React/Vite DOM selectors or ports apply.

Requirement CLI control-plane files use `@cli` and a store Background, not the web URL. Pair English snake_case names with Chinese titles:

```gherkin
# language: en

@cli
Feature: Requirement listing
  <one-line description>

  Background:
    Given an isolated requirement library with sample login and checkout documents
```

```gherkin
# language: zh-CN

@cli
功能: 需求列表
  <一句话描述>

  背景:
    假如 已有一份包含登录和结账样例文档的隔离需求库
```

Save those as `features/en-US/requirement_listing.feature` and `features/zh-CN/需求列表.feature`.

## Step granularity (important for replay)

| Intent | Step pattern | Binding style |
|--------|--------------|---------------|
| Establish project context | Separate `Given` | setup supported by the actual SUT/runner |
| Switch a supported panel | Separate `When`/`And` | verified panel control |
| Run terminal command in a SUT that provides a terminal | One command per step | verified terminal input |
| Item appears in a SUT list | Separate `Then` | verified item assertion |
| Navigate to URL | Background or explicit Given | `navigate` action |

Do **not** combine "switch tab + assert file" in one step if replay needs intermediate state.

CLI JSON contract steps must say JSON in the When that produces JSON. Do not assert a JSON error `code` after a When that never requested `--json`. Include an explicit success or failure Then before field assertions.

## After writing

For `@web-ui` files:

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI steps list --feature features/en-US/<name>.feature
$TESHI steps unbound --feature features/en-US/<name>.feature
```

Hand unbound browser steps to **teshi** for binding and **playwright-locator** for browser verification. Hand unbound WinUI3 steps to **winapp-regression**. If a specialist is unavailable, inspect the relevant CLI help rather than assuming a repository path exists.

For `@cli` requirement files, implement or extend steps in `tests/steps/requirement-cli` and run them with `teshi run --runner-cmd teshi-requirement-cli-runner`.

## Do not

- Write selectors into `.feature` files.
- Use `.locators.md` (deprecated).
- Record or replay bindings in this skill.
