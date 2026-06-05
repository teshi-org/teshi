---
name: bdd-locator
description: Infer DOM locators for the selected Gherkin step with RVP verification, propose through teshi steps, and wait for Desktop/web confirmation
---

# BDD Locator Skill

Use when recording a BDD step binding in **teshi Desktop** or **teshi web** (browser panel).

## Prerequisites

1. Project open in teshi Desktop/web.
2. Browser session in the Browser panel (**Connect Chrome** or **Start Embedded**).
3. `.teshi/cdp-endpoint.json` exists.
4. User selected a Gherkin **step** (writes `.teshi/active-step.json`).
5. Working from project root in the embedded terminal.
6. Compatible CLI: `export TESHI_CLI=...` in **external** terminals (Desktop sets it automatically).

**Before any step:** run health check (RVP R0):

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI browser doctor || { $TESHI browser reconnect && $TESHI browser doctor; }
```

**Hard rule:** never propose after a single snapshot without completing RVP R3–R4.

## Context files

| File | Purpose |
|------|---------|
| `.teshi/active-step.json` | Selected feature, scenario, step line, step text |
| `.teshi/cdp-endpoint.json` | `mode`, `ws_url`, `page_url` |
| `.teshi/pending-locator.json` | Written by `teshi steps propose` |
| `.teshi/step-bindings/{feature}.json` | Confirmed bindings (commit) |
| `.teshi/logs/locator-verify.jsonl` | R4–R5 audit log (strict gate) |

Do **not** write `.locators.md`.

## §2.5 — Open project (web-ui SUT)

**Forbidden:** `button:has-text("D:\...")` or any `has-text` with Windows path backslashes.

Preferred — API binding:

```bash
$TESHI browser verify --step-line 11 --selector '' --action open_project \
  --value-arg 'D:/Dev/Rust/teshi'

$TESHI steps propose --strategy api --action open_project \
  --value-arg 'D:/Dev/Rust/teshi' --confidence 1.0 \
  --rationale '{"evidence":"SUT projects/open API","match_count":1,"execute_ok":true}'
```

Fallback — stable testid (only if `open_project` unavailable):

```bash
$TESHI steps propose --strategy testid \
  --value '[data-testid="WelcomeRecent-D__Dev_Rust_teshi"]' \
  --action click ...
```

## §2.6 — Terminal input

xterm steps **must** use `type` (fill + Enter). **Never** propose `fill` on `.xterm-helper-textarea`.

```bash
$TESHI browser verify --step-line 13 --selector '.xterm-helper-textarea' \
  --action type --value-arg 'touch README.md'

$TESHI steps propose --strategy css --value '.xterm-helper-textarea' \
  --action type --value-arg 'touch README.md' --confidence 0.95 \
  --rationale '{"evidence":"xterm helper textarea","match_count":1,"execute_ok":true}' \
  --highlight-applied
```

Alternative: split into two Gherkin steps (`fill` + `press_key Enter`) — second choice.

## §3 — Locator Verification Protocol (RVP)

Complete **all** rounds before `teshi steps propose`.

| Round | Name | Required action | On failure |
|-------|------|-----------------|------------|
| R0 | Health | `browser doctor` (+ reconnect) | Stop; do not record |
| R1 | Context | Read `active-step.json` + `cdp-endpoint.json`; URL/tab matches step | Navigate / open_project / switch tab |
| R2 | Evidence | `browser snapshot` (slow: `--timeout-ms 90000`); cite role/name/testid in reply | Do not guess selector |
| R3 | Uniqueness | Confirm **match count = 1** per candidate (execute or snapshot tree) | Refine selector |
| R4 | Execute trial | Same action/value as propose: `browser execute` or `browser verify` | Do not propose |
| R5 | Highlight | `browser highlight` on rank-1; must succeed | confidence ≤ 0.5 or change selector |
| R6 | Preflight replay | If feature has prior bindings: `browser replay --until-line N-1 --non-interactive --yes` | Fix earlier steps first |

### R4 example (Then assert file visible)

```bash
# Ensure Files tab first if needed
$TESHI browser execute --selector '[data-testid="FileTreeTab"]' --action click

$TESHI browser verify --step-line 15 \
  --selector '[data-testid="FileTreeNode-README.md"]' \
  --action assert_visible

$TESHI browser highlight '[data-testid="FileTreeNode-README.md"]'
```

### R6 example

```bash
$TESHI browser replay --until-line 14 --non-interactive --yes
```

### Rationale JSON (for `--rationale`)

Include: `evidence`, `match_count`, `execute_ok`, `page_url`, optional `assumptions`.

### Confidence rules

- No R4 execute/verify → **forbidden** to propose
- Highlight fail → confidence ≤ 0.5
- match_count > 1 → forbidden unless user confirms in writing

### Strict gate

`TESHI_LOCATOR_STRICT=1` requires a matching entry in `.teshi/logs/locator-verify.jsonl` from `teshi browser verify`.

## Explicit URL navigation steps

If step text contains an explicit `http(s)` URL:

```bash
$TESHI browser navigate '<url>'
$TESHI steps propose --strategy url --action navigate --value-arg '<url>' \
  --confidence 1.0 --rationale 'Step explicitly opens this URL'
$TESHI steps wait --until either --timeout 120
```

## Infer locators (after R2)

Produce **2–3** candidates with match counts. Only rank-1 passing R3–R4 may be proposed.

Map keyword intent:

- **When/And** → click, type, fill, select, press_key
- **Then/And** → assert_visible, assert_text

## Propose and wait

```bash
$TESHI steps propose \
  --strategy testid \
  --value '[data-testid="FileTreeNode-README.md"]' \
  --action assert_visible \
  --confidence 0.95 \
  --rationale '{"evidence":"file tree node README.md","match_count":1,"execute_ok":true}' \
  --highlight-applied

$TESHI steps wait --until either --timeout 120
```

If rejected, stop — do not auto re-propose.

## Selector guidelines

- Prefer `[data-testid="..."]` on SUT panels.
- **Forbidden:** `has-text` with `\` (Windows paths).
- **Forbidden:** `button:has-text("D:` path fragments.
- File tree assertion: click `FileTreeTab` if not on Files tab, then `assert_visible` `FileTreeNode-<name>`.
- One element per selector; avoid brittle `nth-child`.

## Do not

- Edit `.feature` files (use **bdd-feature-author**).
- Propose without R4 execute matching the binding action.
- Use `click` in execute but `assert_visible` in propose (action mismatch).
- Skip R6 when ≥1 confirmed binding exists.
- Confirm on the user's behalf.

## Example summary to user

> Verification L15: R0 ok, snapshot shows README.md node, verify assert_visible ok, highlight ok, replay through L14 ok. Proposed `[data-testid="FileTreeNode-README.md"]`. Please confirm in Locator panel.
