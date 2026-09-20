---
name: playwright-locator
description: Acquire verified Playwright locators in Chrome or embedded browser, bind them to the selected Gherkin step with teshi steps propose, and replay confirmed step-bindings. Use whenever recording, verifying, or replaying browser BDD steps in teshi Desktop/web. Do not use for WinUI3 or native Windows UIA targets.
---

# Playwright Locator

Browser BDD in teshi is one loop: **inspect → verify → propose → confirm → replay**. Do not switch skills between those phases.

WinUI3/native Windows work belongs to **winapp-regression**. Feature text belongs to **bdd-feature**.

For typed Chrome P0/P1 operations (sessions, lease, locator, locator-verify, revision-bound execute, console/network capture, P2 grants), follow the packaged skill first:

`agent-packages/teshi-browser-testing/skills/playwright-locator/SKILL.md`

This workspace skill adds teshi Gherkin bindings and replay. Teshi-specific SUT rules live in [references/teshi-bindings.md](references/teshi-bindings.md).

## Prerequisites

1. Project open in teshi Desktop/web.
2. Browser panel connected (**Connect Chrome** or **Start Embedded**).
3. `.teshi/cdp-endpoint.json` exists.
4. Working from project root. Prefer the Desktop embedded terminal (`TESHI_CLI` is set). External shells must export `TESHI_CLI` or use teshi ≥ 0.4.0 from PATH.
5. For binding: a Gherkin **step** is selected (`.teshi/active-step.json`).
6. For replay: `.teshi/step-bindings/{feature}.json` has confirmed bindings.

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI --version
```

Do **not** write `.locators.md`.

## Route by browser mode

Read `mode` from `.teshi/cdp-endpoint.json`.

### Chrome (`mode` is chrome / broker)

1. Read the packaged skill and [references/compatibility.md](../../agent-packages/teshi-browser-testing/skills/playwright-locator/references/compatibility.md).
2. `teshi browser sessions` → pick `extension_instance_id`.
3. `teshi browser tabs --session <id>` → pick `window_id` + `tab_id`.
4. Acquire a lease; keep `lease_token` private; release it when done.
5. Resolve a locator with `teshi browser locator` (observational). Accept only `verification=verified`, `match_count=1`, current page revision.
6. Execute the **step's** action with `teshi browser execute` only when the selected Gherkin step supplies that action.
7. Continue at **Propose and wait**.

Never invent an action. Never select a profile by colliding browser-local tab IDs alone.

### Embedded (`mode` is `embedded`)

```bash
$TESHI browser doctor || { $TESHI browser reconnect && $TESHI browser doctor; }
```

If the second doctor fails, stop. Then follow the verification rounds in [references/teshi-bindings.md](references/teshi-bindings.md) before proposing.

## Context files

| File | Purpose |
|------|---------|
| `.teshi/active-step.json` | Selected feature, scenario, step line, step text |
| `.teshi/cdp-endpoint.json` | `mode`, `ws_url`, `page_url` |
| `.teshi/pending-locator.json` | Written by `teshi steps propose` |
| `.teshi/step-bindings/{feature}.json` | Confirmed bindings (commit) |
| `.teshi/logs/locator-verify.jsonl` | Strict-gate audit log from `browser verify` |

## Propose and wait

Map keyword intent:

- **When/And** → click, type, fill, select, press_key
- **Then/And** → assert_visible, assert_text

Propose only the rank-1 candidate that already executed successfully with the **same** action and value as the binding.

```bash
$TESHI steps propose \
  --strategy testid \
  --value '[data-testid="FileTreeNode-README.md"]' \
  --action assert_visible \
  --confidence 0.95 \
  --rationale '{"evidence":"file tree node README.md","match_count":1,"execute_ok":true}' \
  --highlight-applied

$TESHI steps wait --until confirmed --auto-confirm --timeout 120
```

If rejected, stop — do not auto re-propose. Do not confirm on the user's behalf when they asked to review visually.

If the step text contains an explicit `http(s)` URL:

```bash
$TESHI browser navigate '<url>'
$TESHI steps propose --strategy url --action navigate --value-arg '<url>' \
  --confidence 1.0 --rationale 'Step explicitly opens this URL'
$TESHI steps wait --until confirmed --auto-confirm --timeout 120
```

Call `teshi browser navigate` only when that URL is in the step text.

## Replay

Health first. If doctor fails twice, stop — do not replay on a stale sidecar.

```bash
$TESHI browser doctor || $TESHI browser reconnect
$TESHI browser doctor
$TESHI steps resolve
# or: $TESHI steps resolve --until-line <N>
```

Interactive (default): explain the next action, then:

```bash
$TESHI browser replay --until-line <line>
```

Non-interactive (CI / agent validation):

```bash
$TESHI browser replay --non-interactive --yes --until-line <line>
$TESHI browser replay --feature tests/feature/web-ui/<name>.feature --non-interactive --yes
```

Dry run:

```bash
$TESHI browser replay --dry-run --until-line <line>
```

`navigate` and `open_project` replay as first-class setup actions.

Before recording a later step, if the feature already has confirmed bindings, replay through the previous line (`--until-line N-1 --non-interactive --yes`).

## Failure handling

| Symptom | Agent action |
|---------|--------------|
| snapshot/replay hang or timeout | doctor → reconnect → retry **once**; still fail → report stale `ws_url` |
| `Locator.wait_for` timeout | Report line/action/selector; re-record that step (`steps unbind` + propose). Do not edit the binding silently |
| navigate / open_project fail | Report URL/path and error; check SUT is running |
| SPA state wrong after navigate | Background should use `?e2e=1`; open project via `open_project`, not recent-path click |
| assert_visible on hidden element | Click `FileTreeTab` first; confirm prior step did not leave Terminal tab |
| unbound or pending step | Stop; bind/confirm that step before continuing replay |
| `browser_session_busy` | Do not steal the lease; use another dedicated Chrome profile or wait |

In interactive mode, take a fresh snapshot and ask whether to re-record. In non-interactive mode, **do not** infer or modify bindings.

## Always release (Chrome lease)

After success, failure, or cancellation, release the profile lease from the packaged-skill workflow. Do not reuse an expired token.

## Do not

- Edit `.feature` files (use **bdd-feature**).
- Use `teshi winapp` commands.
- Propose without an execute/verify that matches the binding action.
- Use `click` in execute but `assert_visible` in propose (action mismatch).
- Skip preflight replay when ≥1 confirmed binding exists.
- Replay indefinitely when doctor fails.
- Attach another automation tool to the dedicated Chrome recording profile.
- Confirm new bindings on the user's behalf when they asked to review visually.

## Example summary to user

> Verification L15: session/lease ok, locator verified match_count=1, execute assert_visible ok, replay through L14 ok. Proposed `[data-testid="FileTreeNode-README.md"]`. Please confirm in Locator panel.
