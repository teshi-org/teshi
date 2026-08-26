# Teshi web-ui bindings

Use these rules when the SUT is teshi web (this repository's self-test) or when Chrome typed locator is unavailable (embedded mode).

## Embedded verification rounds

Complete **all** rounds before `teshi steps propose`. Never propose after a single snapshot.

| Round | Name | Required action | On failure |
|-------|------|-----------------|------------|
| R0 | Health | `browser doctor` (+ reconnect) | Stop; do not record |
| R1 | Context | Read `active-step.json` + `cdp-endpoint.json`; URL/tab matches step | Navigate / open_project / switch tab |
| R2 | Evidence | `browser snapshot` (slow: `--timeout-ms 90000`); cite role/name/testid | Do not guess selector |
| R3 | Uniqueness | Confirm **match count = 1** per candidate | Refine selector |
| R4 | Execute trial | Same action/value as propose: `browser execute` or `browser verify` | Do not propose |
| R5 | Highlight | `browser highlight` on rank-1; must succeed | confidence ≤ 0.5 or change selector |
| R6 | Preflight replay | If feature has prior bindings: `browser replay --until-line N-1 --non-interactive --yes` | Fix earlier steps first |

`TESHI_LOCATOR_STRICT=1` requires a matching entry in `.teshi/logs/locator-verify.jsonl` from `teshi browser verify`.

Rationale JSON for `--rationale`: `evidence`, `match_count`, `execute_ok`, `page_url`, optional `assumptions`.

Confidence: no R4 execute/verify → forbidden to propose. Highlight fail → confidence ≤ 0.5. match_count > 1 → forbidden unless the user confirms in writing.

Produce **2–3** candidates with match counts. Only rank-1 passing R3–R4 may be proposed.

## Open project

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

## Terminal input

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

## Then assert file visible

```bash
$TESHI browser execute --selector '[data-testid="FileTreeTab"]' --action click

$TESHI browser verify --step-line 15 \
  --selector '[data-testid="FileTreeNode-README.md"]' \
  --action assert_visible

$TESHI browser highlight '[data-testid="FileTreeNode-README.md"]'
```

## Selector guidelines

- Prefer `[data-testid="..."]` on SUT panels.
- **Forbidden:** `has-text` with `\` (Windows paths).
- **Forbidden:** `button:has-text("D:` path fragments.
- File tree assertion: click `FileTreeTab` if not on Files tab, then `assert_visible` `FileTreeNode-<name>`.
- One element per selector; avoid brittle `nth-child`.
