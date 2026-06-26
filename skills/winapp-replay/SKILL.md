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
5. A compatible teshi CLI (**>= 0.4.0**). Prefer `TESHI_CLI` when set; otherwise use `teshi` from PATH.

## Preflight checklist

Before replay, verify:

1. **App is running** (or use `--launch`).
2. **Window attached** — replay fails fast when detached:

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI winapp list-windows
$TESHI winapp attach --title 'My App'
```

3. **Navigation bindings exist** in Background (e.g. open Library tab), not only visibility assertions.
4. **UI state** matches the scenario start (logged in, correct page visible).

Optional launch + replay:

```bash
$TESHI winapp replay --feature '<feature>' --launch 'C:\path\to\App.exe' --yes
```

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

- If replay reports **not attached**, run attach/launch preflight above.
- If there are no confirmed bindings, stop and run **winapp-locator** for missing steps.
- If replay fails at a line, report line, action, selector, and error; offer `teshi winapp snapshot` and re-bind that step (`steps unbind` + re-propose).
- Do not invent selectors during replay.

## Do not

- Use `teshi browser replay` for WinUI3 targets.
- Assume replay starts the app without `--launch` or a running process.
- Re-propose locators automatically after rejection.
