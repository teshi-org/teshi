---
name: playwright-locator
description: Inspect and safely control explicit local Chromium Profiles through Teshi's P0 multi-session broker, including revision-bound refs, verified Playwright locators, typed actions/waits, and tab lifecycle. Use only when the requested test step supplies the intended action.
---

# Playwright Locator

Use Teshi's typed browser operations. Locator acquisition stays observational. Execute a P0 action only when the user or selected test step explicitly supplies that action; never invent one.

`teshi browser sessions` starts or reuses the per-user loopback broker even when Desktop is closed. A live incompatible broker is never terminated implicitly.

## Preflight

1. Run `teshi --version`. Require the CLI range in [references/compatibility.md](references/compatibility.md).
2. Run `teshi browser sessions`.
3. Stop before debugger attachment when no compatible `ready` session exists. Report the detected health/version and use the setup guidance in the reference.
4. If exactly one eligible session exists, select it. If several exist, match an explicit profile label supplied by the user or ask which opaque session to use. Never select by colliding browser-local tab IDs alone.
5. Run `teshi browser tabs --session <extension_instance_id>`. Select a tab by its session-scoped window ID and tab ID. Ask when URL/title evidence remains ambiguous.

Treat profile labels, titles, and URLs as display evidence only. Route exclusively with `extension_instance_id`, `window_id`, and `tab_id`.

## Acquire ownership

Acquire one bounded profile lease before reading page content or resolving a locator:

```bash
teshi browser lease acquire \
  --session <extension_instance_id> \
  --owner 'codex-playwright-locator' \
  --ttl 60
```

Keep the returned `lease_token` private. Renew it before expiry if the workflow takes longer than its TTL. A `browser_session_busy` response means another agent or UI owns the profile; do not retry in a tight loop or steal the session. Select a different dedicated profile or wait for its owner to release it.

## Resolve a locator

Describe only evidence present in the user's request, selected Gherkin step, or page snapshot. Do not infer an action. Supply one or more intent fields:

```bash
teshi browser locator \
  --session <extension_instance_id> \
  --window <window_id> \
  --tab <tab_id> \
  --lease-token <lease_token> \
  --purpose 'save changes control' \
  --role button \
  --text Save
```

Use `--element-ref` when a prior snapshot identifies the exact element, `--gherkin-step` when the user selected a step, and repeat `--test-id-attribute` only to override project configuration. The default project attribute is `data-testid`.

Accept a recommendation only when all of these hold:

- the response target exactly matches the selected session/window/tab;
- `verification` is `verified`, `match_count` is `1`, and the element is visible;
- the page-context revision is the one returned for this request;
- no warning contradicts the user's stability needs.

Prefer role/name, label, placeholder, configured test ID, and stable attributes. Treat positional selectors, generated classes, long DOM paths, coordinates, and ambiguous text as fragile even when returned as alternatives. Preserve frame and shadow-root context in the result.

## Re-verify or capture evidence

Re-verify a structured candidate when time has passed or the page may have changed:

```bash
teshi browser locator-verify \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token> \
  --page-revision <page_context_revision> \
  --candidate-json '<candidate JSON>'
```

Treat `stale_browser_target` or `stale_page_context` as a hard stop. Acquire a new snapshot and resolve again; never relabel a stale result as verified.

## Execute an explicit P0 action

Use exactly one of `--reference`, `--candidate-json`, or `--selector`. Compact refs such as `@e1` are valid only for their original Profile, tab, snapshot, and page revision. Structured candidates are re-verified immediately before mutation.

```bash
teshi browser execute \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token> \
  --reference @e1 --snapshot-id <snapshot_id> --page-revision <revision> \
  --action click --wait-text Saved
```

Use `pointer_click --focus` only when real pointer/focus behavior is required; ordinary `click` is DOM activation. Typed waits are `--wait-url`, `--wait-text`, `--wait-state`, `--wait-revision-change`, and `--wait-load`. Inspect `action_outcome` and `wait_outcome` separately: a wait timeout does not mean the action was not executed and must not trigger an automatic retry.

Use `teshi browser lookup`, `profile-label`, and `tab open|close|activate|new-window|group` for explicit Profile and tab lifecycle. Every tab/window mutation requires the Profile lease and returns complete target identity for created tabs. `tab activate` leaves the browser window unfocused unless `--focus-window` is explicit. Treat `organized: false` with `tab_group_unavailable` as a non-fatal grouping result; the tabs remain usable.

Capture a screenshot reference only when the user requests evidence or it materially helps disambiguation:

```bash
teshi browser evidence \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token> \
  --page-revision <page_context_revision>
```

Do not expose unrelated page content, URLs, titles, screenshot paths, or lease tokens in summaries.

For a mutation whose visible impact matters, add `--monitor`. This performs one action dispatch and observes bounded page summaries before and after it; a diff never authorizes retrying the mutation. For file inputs use `--action upload --file <project-relative-path>` with only caller-specified files. Do not discover or enumerate candidate files. A rejected file index or policy reason is enough to ask for a corrected explicit path.

## Capture bounded P1 diagnostics

Check the selected session's `capabilities.supported_operations` before using P1. Console and network capture require the same complete target and Profile lease as control operations. Start capture only when diagnostics are needed, apply tighter limits where practical, and always stop it before releasing the lease.

```bash
teshi browser console start \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token> --level info,error --max-entries 200
teshi browser console list \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token> --max-age-ms 60000
teshi browser console stop \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token>
```

Network capture stores metadata only by default. `network list` never returns headers or bodies. Use `network detail <request_id>` for redacted request/response headers. Add `--include-body` only when the user or test step needs the response body; it remains byte-bounded and reports encoding, truncation, original size, and returned size.

```bash
teshi browser network start \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token> --max-entries 500
teshi browser network list \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token>
teshi browser network detail <request_id> --include-body --max-body-bytes 65536 \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token>
teshi browser network stop \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token>
```

Authorization, Cookie, token, password, secret, and caller-configured fields are redacted by default. Do not treat redaction as permission to collect unrelated traffic. A debugger conflict means DevTools or another controller owns the target; stop and report it rather than retrying attachment.

## Keep privileged P2 access explicit

Privileged browser commands are disabled by default. Never request optional Chromium permissions silently or reuse a grant across projects, Profiles, callers, or broker restarts. For Cookie reads, start metadata-only; request the separate `cookie-values` grant only when values are explicitly necessary. Content settings must remain on the selected tab origin and fixed allowlist. Extension management is metadata-read-only; do not attempt enable, disable, or uninstall mutations. P2 MCP tools remain absent unless explicitly allowlisted by startup policy.

## Always release

Release ownership after success, failure, or cancellation:

```bash
teshi browser lease release \
  --session <extension_instance_id> \
  --lease-token <lease_token>
```

If release reports that the lease already expired, report that bounded recovery occurred and do not reuse the token.

## Return to the user

Return the recommended Playwright expression, structured arguments, frame/shadow context, match count, verification state, page revision, stability rationale, warnings, and useful alternatives. State why no locator is returned when the target is ambiguous, busy, disconnected, incompatible, timed out, or stale.

For MCP callers, observational tools are available by default. Start the server with `--allow-browser-mutations` to advertise the safe P0 `execute_browser_action` tool. Its target, validation, timeout, result, and error semantics are identical to the CLI operation.
