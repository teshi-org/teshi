---
name: bdd-replay
description: Replay confirmed teshi step-bindings through the browser bridge before recording or debugging later BDD steps
---

# BDD Replay Skill

Use this skill when the user wants confirmed BDD steps replayed as setup for the currently selected step, or when validating recorded step-bindings.

## Prerequisites

1. A project is open in teshi Desktop/web.
2. The Browser panel is connected to the intended page.
3. `.teshi/cdp-endpoint.json` exists.
4. `.teshi/step-bindings/{feature}.json` contains confirmed bindings.
5. You are working from the project root.
6. A compatible teshi CLI is available. Prefer `TESHI_CLI` when set; otherwise use `teshi` from PATH.

## Workflow

0. Resolve the CLI command:

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI --version
$TESHI browser replay --help
$TESHI steps resolve --help
```

If any command fails, stop and ask the user to install a newer teshi MSI, fix PATH, or set `TESHI_CLI` to a compatible development binary.

1. Resolve the target sequence:

```bash
$TESHI steps resolve
```

Use `--until-line N` when replaying setup only up to a selected step.

2. Default to interactive replay. Explain the next action before it runs and wait for the user to agree in chat or terminal. `navigate` bindings are replayed as first-class setup actions:

```bash
$TESHI browser replay --until-line <line>
```

3. Use non-interactive replay only when the user asks for CI-style execution:

```bash
$TESHI browser replay --non-interactive --until-line <line>
```

`--yes` is an alias for `--non-interactive`.

4. Use dry run to inspect the planned sequence:

```bash
$TESHI browser replay --dry-run --until-line <line>
```

## Failure Handling

- If a step is unbound or pending, stop and tell the user to record/confirm it first.
- If a `navigate` binding fails, stop and report the URL and error.
- If `execute_locator` fails, stop at that step and report the line, action, selector, and error.
- In interactive use, you may take a fresh `teshi browser snapshot` and ask the user whether to re-record the failing step with `bdd-locator`.
- In non-interactive use, do not infer or modify bindings.

## Do Not

- Read or write deprecated `.locators.md` files.
- Confirm new bindings on behalf of the user.
- Start or attach another browser tool to the dedicated Chrome recording profile.
