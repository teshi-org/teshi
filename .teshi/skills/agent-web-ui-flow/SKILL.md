---
name: agent-web-ui-flow
description: End-to-end external agent workflow to generate web-ui Gherkin features, record bindings with RVP verification, and replay against teshi web SUT
---

# Agent Web UI Flow

Use when an **external agent** (Cursor terminal, CI script) should **generate → record → replay** teshi web UI scenarios from **teshi desktop** as Host IDE.

## Related skills (read in order)

1. [web-ui-bootstrap](../web-ui-bootstrap/SKILL.md) — Host + SUT + Embedded setup
2. [bdd-feature-author](../bdd-feature-author/SKILL.md) — write `.feature` files
3. [bdd-locator](../bdd-locator/SKILL.md) — RVP verification + `steps propose`
4. [bdd-replay](../bdd-replay/SKILL.md) — validate bindings

## Phase 0 — Environment

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI --version   # must be >= 0.4.0
export TESHI_CLI="${TESHI:-teshi}"   # external terminals do not inherit Desktop TESHI_CLI
```

1. Open **teshi desktop** with the BDD project:
   - `teshi desktop --project . --start-embedded` — opens project and auto-starts embedded browser
   - Or `teshi web --project . --start-embedded` — same UI via browser, also auto-starts embedded browser
   - Or open manually: `teshi desktop --project .` then click **Start Embedded** in Browser panel.
2. Start **SUT** per web-ui-bootstrap (dev: `:1420` Vite + `:1421` API).
3. **Browser panel** connects automatically (with `--start-embedded`) or click **Start Embedded**.
4. Navigate SUT URL in the address bar (append `?e2e=1` for dev mode).
5. Health check:

```bash
$TESHI browser doctor || { $TESHI browser reconnect && $TESHI browser doctor; }
```

## Phase 1 — Generate feature

Follow **bdd-feature-author**. Then:

```bash
$TESHI steps list --feature tests/feature/web-ui/<name>.feature
$TESHI steps unbound --feature tests/feature/web-ui/<name>.feature
```

## Phase 2 — Record bindings (RVP required)

For each unbound step:

1. `teshi steps next-unbound --feature '<path>'`
2. Follow **bdd-locator** RVP rounds R0–R6.
3. Emit a **Verification Record** (markdown checklist) before propose:

```markdown
### Verification L<N>
- [x] R0 doctor ok
- [x] R1 page_url / tab context matches step
- [x] R2 snapshot evidence: ...
- [x] R3 match_count=1 for `[data-testid="..."]`
- [x] R4 browser verify ok
- [x] R5 highlight ok
- [x] R6 replay --until-line N-1 ok (if prior bindings exist)
```

4. `teshi steps propose ...` then `teshi steps wait --until confirmed --auto-confirm --timeout 120`

Optional strict gate: `TESHI_LOCATOR_STRICT=1` requires prior `teshi browser verify`.

**Do not** propose without R4 execute/verify success.

If SUT or sidecar restarts: `browser doctor` → `browser reconnect` → restart RVP from R1.

## Phase 3 — Replay validation

```bash
$TESHI browser doctor
$TESHI browser replay --feature tests/feature/web-ui/<name>.feature --non-interactive --yes
```

On failure: follow bdd-replay Failure Handling; **do not** silently edit bindings—re-record with bdd-locator.

## Phase 4 — CI (optional)

See web-ui-bootstrap §CI (`teshi run` + `serve-embedded`).

## Do not

- Skip RVP and guess selectors from step text alone.
- Use `has-text` with Windows paths (`D:\...`).
- Use `fill` on xterm; use `type` action instead.
- Confirm bindings without user/auto-confirm policy.
