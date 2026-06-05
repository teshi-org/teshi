---
name: bdd-replay
description: Replay confirmed teshi step-bindings through the browser bridge with health checks before recording or CI validation
---

# BDD Replay Skill

Use when replaying confirmed BDD steps as setup for the selected step, or validating recorded bindings end-to-end.

## Prerequisites

1. Project open in teshi Desktop/web.
2. Browser panel connected to the intended SUT page.
3. `.teshi/cdp-endpoint.json` exists.
4. `.teshi/step-bindings/{feature}.json` has confirmed bindings.
5. Project root as cwd.
6. Compatible CLI (`TESHI_CLI` in external terminals).

## Workflow

### Step 0 — Health check (required)

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI --version
$TESHI browser doctor || $TESHI browser reconnect
$TESHI browser doctor   # must succeed before replay
```

If the second doctor fails, stop — do not replay on a stale sidecar.

### Step 1 — Resolve sequence

```bash
$TESHI steps resolve
# or
$TESHI steps resolve --until-line <N>
```

### Step 2 — Interactive replay (default)

Explain the next action; wait for user agreement:

```bash
$TESHI browser replay --until-line <line>
```

`navigate` and `open_project` bindings replay as first-class setup actions.

### Step 3 — Non-interactive (CI / agent validation)

```bash
$TESHI browser replay --non-interactive --yes --until-line <line>
# full feature:
$TESHI browser replay --feature tests/feature/web-ui/<name>.feature --non-interactive --yes
```

### Step 4 — Dry run

```bash
$TESHI browser replay --dry-run --until-line <line>
```

## Failure handling

| Symptom | Agent action |
|---------|--------------|
| snapshot/replay hang or timeout | doctor → reconnect → retry **once**; still fail → report stale `ws_url` |
| `Locator.wait_for` timeout | Report line/action/selector; do not edit binding silently — re-record with bdd-locator |
| navigate / open_project fail | Report URL/path and error; check SUT is running |
| SPA state wrong after navigate | Background should use `?e2e=1`; open project via `open_project` not recent-path click |
| assert_visible on hidden element | Click `FileTreeTab` first; confirm prior step left Terminal tab |
| unbound or pending step | Stop; record/confirm with bdd-locator |

In interactive mode, you may take a fresh snapshot and ask whether to re-record. In non-interactive mode, **do not** infer or modify bindings.

## Do not

- Replay indefinitely when doctor fails.
- Assume killing/restarting SUT leaves old `cdp-endpoint.json` valid.
- Read or write deprecated `.locators.md`.
- Confirm new bindings on behalf of the user.
- Attach another automation tool to the dedicated Chrome recording profile.
