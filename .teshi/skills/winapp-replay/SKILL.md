---
name: winapp-replay
description: Replay confirmed WinUI3 UIA step-bindings through winapp replay before debugging or closing a bug regression
---

# WinApp Replay Skill

Use this skill when the user wants confirmed WinUI3/native bindings replayed as setup for the selected step, or when validating a full bug-regression Scenario.

## Prerequisites

1. A project is open in teshi Desktop/web.
2. **Connect WinUI3 App** is running in the Target panel.
3. `.teshi/cdp-endpoint.json` exists with `"mode": "winapp"`.
4. `.teshi/step-bindings/{feature}.json` contains confirmed bindings for the target feature.
5. The target app window is attached.
6. A compatible teshi CLI is available. Prefer `TESHI_CLI` when set; otherwise use `teshi` from PATH.

## Workflow

0. Resolve the CLI:

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI --version
$TESHI winapp replay --help
$TESHI steps resolve --help
```

1. Resolve the binding sequence:

```bash
$TESHI steps resolve --feature '<feature-relative-path>'
```

Use `--until-line N` when replaying only up to the selected step.

2. Default to **interactive** replay unless the user asks for CI-style execution:

```bash
$TESHI winapp replay --feature '<feature-relative-path>' --until-line <line>
```

3. Non-interactive (CI-style on a dev machine with sidecar running):

```bash
$TESHI winapp replay --feature '<feature-relative-path>' --yes
```

4. Dry run to inspect the plan:

```bash
$TESHI winapp replay --feature '<feature-relative-path>' --dry-run
```

## Failure handling

- If there are no confirmed bindings, stop and run **winapp-locator** for missing steps.
- If replay fails at a line, report line, action, selector, and error; offer `teshi winapp snapshot` and re-bind that step.
- Do not invent selectors during replay.

## Do not

- Use `teshi browser replay` for WinUI3 targets.
- Confirm new bindings on behalf of the user.
- Re-propose locators automatically after rejection.
