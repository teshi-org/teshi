---
name: playwright-locator
description: Acquire and verify stable Playwright locators from a user's local Chromium profile through Teshi's multi-session browser broker. Use when an agent needs to inspect a live page, distinguish several connected browser profiles or tabs, generate Playwright locator expressions from an element description or Gherkin step, re-verify a candidate, or capture optional request-scoped evidence. Do not use this workflow to invent or execute test actions.
---

# Playwright Locator

Use Teshi's typed browser operations. Keep the workflow observational: identify an element and return verified locator candidates without clicking, filling, navigating, or inventing a test step.

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

Capture a screenshot reference only when the user requests evidence or it materially helps disambiguation:

```bash
teshi browser evidence \
  --session <extension_instance_id> --window <window_id> --tab <tab_id> \
  --lease-token <lease_token> \
  --page-revision <page_context_revision>
```

Do not expose unrelated page content, URLs, titles, screenshot paths, or lease tokens in summaries.

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

For MCP callers, use the equivalent `list_browser_sessions`, `list_browser_tabs`, lease, snapshot, locator, verification, and evidence tools. Their target, validation, timeout, result, and error semantics are identical to the CLI operations.
