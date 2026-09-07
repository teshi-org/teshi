# Gherkin bindings and replay

Use this workflow to bind an existing Gherkin step, then replay confirmed bindings. Write or revise scenarios with `bdd-feature`; use `playwright-locator` for Chromium inspection and actions, or `winapp-regression` for native Windows UIA. Read only the specialist relevant to the target.

## Select the step through CLI

Run from the intended project root using the executable resolved in [CLI workflows](cli.md). Check `steps --help` and the target family's help in the installed version.

```text
teshi steps list --feature features/en-US/example.feature
teshi steps unbound --feature features/en-US/example.feature
teshi steps select --feature features/en-US/example.feature --line 12
```

Replace the example path and line with an actual step. `steps next-unbound --feature <path>` also selects a step; it changes recording context and is not just a read-only query. Check its output and exit status rather than hiding errors in a loop.

Verify `.teshi/active-step.json` identifies the intended feature, line, and step text before proposing. This works without a GUI step-selection panel. Re-select after edits that move source lines.

## Establish replay state and verify

1. Establish the correct target and liveness. For Chromium, discover session/window/tab and acquire the exclusive lease using `playwright-locator`. Carry target and token through supported calls and release the lease after success or failure. For embedded mode, check `browser doctor`; reconnect only the intended embedded sidecar when needed and recheck health. For Windows, attach the intended native window.
2. When prior confirmed steps supply necessary setup, restore the scenario start and replay through the previous step before inspecting a later one. Replay can mutate the application; keep it within the requested test scope. Do not replay unrelated scenarios or repeatedly submit an already completed action.
3. Inspect the live target. Use evidence to select a stable locator with a unique match and the required state. Refresh stale page references; never copy a snapshot-local `@e1` reference into a durable selector.
4. Verify the same action and input value intended for the binding. Assertions need assertion evidence, not merely evidence that clicking worked. For embedded strict verification (`TESHI_LOCATOR_STRICT=1`), use `browser verify` with the selected source line to produce the required verification record. Check its installed help for target arguments.
5. Highlight when a visual review needs it and the target supports it. Report `--highlight-applied` only after actual successful highlighting. Do not add arbitrary candidate counts or confidence values to substitute for evidence.

`And`/`But` inherit the preceding keyword's intent; neither implies a click. An explicit navigation step uses its stated URL. Do not add navigation, panel changes, or shell commands absent from the requested scenario just to make an assertion pass.

## Propose and confirm

Example for an already verified CSS assertion (replace the selector, line, and rationale with actual evidence):

```text
teshi steps propose --line 12 --strategy css --value "[data-testid='status']" --action assert_visible --confidence 0.95 --rationale "Unique visible status element; assert_visible succeeded"
```

For UIA, use `--strategy uia` and a verified `uia:...` value. For actions taking input, supply the same `--value-arg` used during verification. A structured browser locator candidate must be represented by a supported durable binding strategy; do not stringify arbitrary candidate JSON as a CSS selector. If no supported equivalent exists, report that limitation.

The proposal is pending, not confirmed. Choose the confirmation path according to the user's request:

- For a requested visual review, present the evidence and leave confirmation to the reviewer. `steps wait --until confirmed --timeout 60` observes the result without automatic confirmation. Do not promise a Locator panel in a GUI version that lacks one.
- When the user has authorized agent confirmation, `steps confirm --rank 1` confirms explicitly. `steps wait --until confirmed --timeout 60 --auto-confirm` is another supported path: it attempts confirmation on timeout, so timeout is not a read-only wait in that mode.
- On rejection, active-step mismatch, or timeout without confirmation, inspect and report the result. Do not silently re-propose or force confirmation.

Pending state lives in `.teshi/pending-locator.json`; confirmed bindings live in `.teshi/step-bindings/`. Use CLI operations to manage them instead of fabricating these files. `steps unbind --feature <path> --line <line>` removes a binding when the task calls for repair.

## Replay and inspect outcomes

```text
teshi steps resolve --feature features/en-US/example.feature
teshi browser replay --feature features/en-US/example.feature --dry-run
teshi browser replay --feature features/en-US/example.feature --non-interactive
```

For Chromium, append `--session <id> --window <id> --tab <id> --lease-token <token>` to replay calls. For native Windows, use `winapp replay` instead. `--until-line <N>` bounds execution; dry-run prints planned actions and does not establish that they passed.

Inspect final step outcomes, not just the process launch or accepted proposal. On failure, report the feature/line, action, relevant error, and whether state may already have changed. Re-record only when repair is requested; an assertion failure may be a product defect. A wait timeout after an action must not automatically repeat that action.

Teshi Web itself uses GPUI WASM. Old `FileTreeTab`, xterm, recent-project selectors, and `open_project` examples are not evidence that those controls/actions exist in the current SUT. Inspect the actual surface and supported test bridge described in [GUI workflows](gui.md).
