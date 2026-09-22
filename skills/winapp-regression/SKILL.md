---
name: winapp-regression
description: Create and verify a native Windows or WinUI3 BDD regression with Teshi UIA locators, confirmed step bindings, replay, and optional behave export. Use for native Windows targets; Chromium control belongs to playwright-locator.
---

# WinApp Regression

Complete the requested native regression: feature → verified binding → confirmation → replay → optional export. Use `bdd-feature` for scenario conventions when available. Examples use `teshi` as shorthand for the executable from `TESHI_CLI`, otherwise PATH.

## Establish the environment

Work from the intended project root. Check `teshi --version`, `teshi winapp --help`, and `teshi steps --help`; do not rely on a minimum historical version as proof of current capabilities.

WinApp commands require a live native Windows sidecar and project `.teshi/cdp-endpoint.json` with `mode: "winapp"`. `winapp attach` selects a window through that sidecar; it does not start the sidecar. If the endpoint is absent, stale, or in another mode, report the missing connection and use setup supported by the installed application. Do not fabricate the endpoint or assume current GPUI exposes a legacy **Connect WinUI3 App** button. The current WinApp GPUI surface is a screenshot stream, not an embedded terminal or Gherkin editor.

## Write and select a regression

Use the bug's preconditions, reproduction action, and expected visible result to write a focused Scenario in the project's feature directory. Match the project's language and runner conventions. Edit the feature directly when authorized or use the TUI; keep UIA selectors out of step text.

```text
teshi steps list --feature features/regression.feature
teshi steps unbound --feature features/regression.feature
teshi steps select --feature features/regression.feature --line 12
```

Substitute the real path and step line. Check `.teshi/active-step.json` matches that feature and step text. `steps next-unbound --feature <path>` also selects the next step; inspect its result and distinguish exhaustion from errors.

## Attach and verify

```text
teshi winapp list-windows
teshi winapp attach --hwnd 123456
teshi winapp snapshot
```

Choose the actual window from discovery. Alternatives are `attach --pid`, `--title`, or `--process-name`; title/process fragments can be ambiguous. If launching the application is requested, use `winapp launch <executable-path>` with its supported arguments. Do not attach an unrelated window.

Prefer a stable `uia:automation_id=...`, then `uia:control_type=...;name=...`, then a unique `uia:name=...`. Use `uia:path=...` only when necessary and report its fragility. Verify the target against the current UIA tree; do not invent AutomationIds.

For a step asserting that a welcome message is visible, an example verification is:

```text
teshi winapp execute --selector "uia:automation_id=WelcomeMessage" --action assert_visible
```

For a negative existence assertion, use the same verified selector with
`--action assert_not_exists`. It succeeds only when no UIA control matches;
hidden controls still count as existing.

Use only the actual discovered selector and the action/value supplied by the step. Mutating verification changes application state; restore the relevant scenario setup before replay or inspecting later steps when necessary. A highlight alone is not action verification. Do not retry failed mutations until you understand their outcome.

## Choose the click action deliberately

- `click` prefers non-intrusive UIA activation (`InvokePattern`, then other
  supported UIA patterns). It never silently becomes a real system-pointer
  click. Use it for ordinary stable automation.
- `pointer_click` moves the real system pointer to the unique visible
  interactive element and sends a foreground left-button click. Use it for
  hover, pressed state, WinUI3 custom title bars, caption islands, non-client
  area interaction, and controls that depend on real mouse messages.
- `pointer_click` is allowed in the default `--mode auto` because the action
  itself explicitly requests physical input. It is also allowed with
  `--mode foreground`, and is rejected by `--mode background`. It must not be
  downgraded to background `PostMessage` input.

Do not use `pointer_click` merely because `click` failed once. First inspect
the control and determine whether it actually requires physical pointer
semantics. A missing or broken UIA pattern, ambiguous selector, integrity
mismatch, stale attachment, or transient provider error is not evidence that
real pointer input is required.

For example:

```text
teshi winapp execute --selector "uia:control_type=ButtonControl;name=Close" --action pointer_click
```

## Propose and confirm

After successful verification of that same assertion:

```text
teshi steps propose --line 12 --strategy uia --value "uia:automation_id=WelcomeMessage" --action assert_visible --confidence 0.95 --rationale "Unique message verified visible in the attached window"
```

For actions such as `fill`, `assert_text`, `select`, or `press_key`, pass the verified `--value-arg`. Use supported secret placeholders instead of persisting passwords. Set `--highlight-applied` only after successful highlighting.

Follow the user's confirmation mode. For visual review, show evidence and wait without automatic confirmation (`steps wait --until confirmed --timeout 60`). If agent confirmation is authorized, use `steps confirm --rank 1`; `steps wait --auto-confirm` attempts confirmation on timeout. Rejection or context mismatch requires inspection, not automatic re-proposal. There is no need to ask again when the user already authorized confirmation.

Pending proposals are in `.teshi/pending-locator.json`; confirmed bindings are in `.teshi/step-bindings/`. Manage them through CLI. When repair is requested, remove the wrong binding with `steps unbind --feature <path> --line <N>` before re-recording. Do not conceal a product regression by changing an assertion to match the failure.

## Replay

Ensure the correct app is attached and its state matches scenario preconditions. Include the setup/navigation steps required by the scenario, not only assertions.

```text
teshi steps resolve --feature features/regression.feature
teshi winapp replay --feature features/regression.feature --dry-run
teshi winapp replay --feature features/regression.feature --non-interactive
```

Use `--until-line <N>` for a bounded replay. `--launch <executable-path>` is available when launching is in scope; check help for its behavior. Use `winapp replay`, not browser replay. Inspect each step result and final status; dry-run does not execute the test. On failure, report the feature/line, action, error, and potentially changed state before deciding whether repair is appropriate.

## Export when requested

```text
teshi export --target behave --feature features/regression.feature --out tests-e2e
```

Export confirmed bindings, then read [references/behave-export.md](references/behave-export.md). Check existing output before overwriting generated artifacts. Report separately whether export, dry-run, and actual replay passed. Committing or publishing is a separate action when requested by the user.

The behave exporter currently does not support the native `pointer_click`
action. A feature using that action must use native `winapp replay`, or provide
a custom behave step definition; export otherwise fails with
`unsupported export action: pointer_click`.
