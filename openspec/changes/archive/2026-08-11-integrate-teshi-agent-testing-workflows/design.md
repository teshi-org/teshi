## Context

Teshi currently has a Manifest V3 Chromium extension, a Python browser bridge on `127.0.0.1:17373`, browser CLI commands, and a structured page snapshot used for locator recording. The extension polls the bridge with metadata for the current browser window and receives one queued command. Agents discover one WebSocket endpoint through `.teshi/cdp-endpoint.json`.

This works for one recording profile, but the state model is global: the bridge has one current extension, one active tab, one command queue, and one endpoint file. Two Chrome profiles running the same extension can send overlapping heartbeats and collide. Browser tab identifiers are meaningful only within their extension/browser instance, so `tab_id` alone is not a safe target key.

The first external-agent use case is narrower than full test execution. A coding agent needs to inspect a real browser profile with existing login state and ask Teshi for Playwright locator candidates while authoring a script. Later, multiple agents must be able to use separate browser profiles concurrently.

## Goals / Non-Goals

**Goals:**

- Let an external agent discover an installed Teshi browser extension and obtain useful Playwright locators from a real browser profile.
- Prefer semantic Playwright locators and return enough evidence for an agent to judge and use them safely.
- Support multiple extension instances and profiles through one local broker with explicit target routing.
- Prevent one agent's commands, responses, or screenshots from leaking into another browser session.
- Provide exclusive session ownership for mutating locator workflows while allowing safe discovery.
- Keep CLI, MCP, and Skills on one typed operation contract.
- Preserve a low-friction single-profile compatibility path.

**Non-Goals:**

- Generating complete Playwright test suites or choosing test assertions for the agent.
- General-purpose remote browser access or exposing the bridge beyond loopback by default.
- Synchronizing cookies or browser data between profiles.
- Running deterministic CI replay, WinApp automation, Behave export, or consumer onboarding in this change.
- Allowing multiple agents to mutate the same tab concurrently.
- Supporting Firefox or Safari in the first implementation; the protocol may remain browser-neutral where inexpensive.

## Architecture

```text
Agent A -- CLI/MCP --+                         +-- teshi-bridge / Profile A
                    |    local Teshi broker    |
Agent B -- CLI/MCP --+--> 127.0.0.1:17373 -----+-- teshi-bridge / Profile B
                    |                          |
Agent C -- CLI/MCP --+                         +-- teshi-bridge / Profile C

Target = extension_instance_id + window_id + tab_id
Mutation ownership = lease(extension_instance_id)
```

The broker is the rendezvous and routing layer. Agents connect to Teshi, not directly to individual extension ports. Extension instances register independently, and all operation messages carry an explicit target and request identifier.

## Decisions

### 1. Use one loopback broker with an instance-indexed registry

The existing fixed loopback address remains the extension discovery point. The broker stores a record per live extension instance rather than allowing the last heartbeat to become global state. Each record contains instance identity, user-facing label, browser metadata, windows, tabs, health, last heartbeat, command queue, and active lease.

Assigning one port per profile was considered, but extension configuration and agent discovery become harder, port ownership is fragile, and agents would still need a registry. A single broker provides one stable local integration point and can multiplex sessions.

### 2. Give each extension installation a stable opaque identity

On first run, the extension generates a random `extension_instance_id` and persists it in profile-local extension storage. It may also store a user-editable `profile_label`; Teshi does not infer or expose the browser's filesystem profile path. Heartbeats include the identity, label, extension version, browser/version metadata, window inventory, and tab inventory.

The opaque identity is stable across service-worker restarts and browser restarts but changes if extension storage is cleared. Human labels are not unique and are never used as routing keys.

### 3. Make browser targets composite and explicit

The canonical target is `(extension_instance_id, window_id, tab_id)`. Commands, responses, frames, diagnostics, and locator results include that target plus a unique `request_id`. The broker rejects stale or mismatched responses rather than delivering them to another caller.

When exactly one eligible session and tab exist, legacy commands may resolve them implicitly. When more than one eligible target exists, implicit commands fail with an `ambiguous_browser_target` error and list non-sensitive candidates.

### 4. Use leases to coordinate parallel agents

Session listing and health inspection do not require a lease. Locator acquisition, navigation, highlighting, clicking, and typing require an exclusive lease on the extension instance because they depend on mutable tab and debugger state. A lease has an opaque token, owner label, acquisition time, expiry, and renewable TTL.

Commands must present the lease token. Disconnect or expiry releases the session after in-flight work is cancelled or bounded. An agent can hold more than one session, and different agents can hold different sessions concurrently. The first version does not allow tab-level shared mutation inside one profile.

### 5. Return a structured Playwright locator result

Locator acquisition begins with a page snapshot and returns ranked candidates rather than a bare CSS selector. Each candidate contains:

- locator kind and structured arguments;
- a rendered Playwright expression;
- page, frame, and shadow-root context where applicable;
- match count, visibility, enabled state, and verification status;
- a stability rationale and warnings;
- optional element evidence suitable for diagnostics.

The initial ranking favors `getByRole`, `getByLabel`, `getByPlaceholder`, and configured test-id attributes before stable attribute/CSS fallbacks. Positional, generated-class, long DOM-path, and coordinate selectors are marked fragile. Text supplied by the user or agent can narrow the intended element, but Teshi does not invent test behavior.

### 6. Verify candidates in the browser before recommending them

The recommended candidate must be evaluated against the targeted tab and frame. Teshi records its match count and whether the intended element is actionable or observable for the requested purpose. A locator that is ambiguous, stale, or unsupported is returned as rejected or lower-ranked instead of being presented as verified.

A snapshot revision or equivalent page-context token accompanies the result so agents can detect navigation or document changes between inspection and use.

### 7. Share typed operations across CLI and MCP

Reusable operations live outside UI shells and cover session listing, lease management, tab listing, snapshot acquisition, locator resolution, locator verification, and optional evidence capture. CLI JSON output and the local STDIO MCP adapter render the same result and error types.

The MCP server is local and inherits the caller's OS permissions. It does not expose the loopback browser broker as a remote desktop service.

### 8. Package extension setup with the agent workflow

Release artifacts that advertise browser locator support include the compatible extension bundle, installation instructions, focused Skills, MCP metadata, and CLI compatibility range. A Skill checks broker health, extension compatibility, session ambiguity, and lease acquisition before asking for locators.

Chrome Web Store publication may be added later; unpacked and installer-provided extension installation remain supported. Update-channel choice does not change the protocol.

### 9. Migrate the single-session surface incrementally

Existing `teshi browser snapshot`, `execute`, `verify`, and related commands continue to resolve a default target when unambiguous. New `--session` and `--tab` selectors, or equivalent structured arguments, take precedence. `.teshi/cdp-endpoint.json` remains readable during migration but is not the source of truth for the multi-session registry.

### 10. Use the shared GPUI shell as the only supported web product surface

`teshi web` serves the WASM output built from `apps/teshi-web`. Browser-session presentation and selection live in `crates/teshi-ui` so the native desktop and web shells share the same fail-closed selection model. The WASM adapter calls same-origin daemon endpoints; it never reaches the fixed loopback broker directly, which keeps the page usable through a LAN-bound daemon without widening broker exposure.

When exactly one healthy session exists, the view may select it for single-profile compatibility. With multiple eligible sessions, the view starts unselected and requires an explicit profile choice. A selected session that disappears remains visibly unavailable and is not silently replaced by another profile. Session cards expose only display label, shortened opaque identity, health, browser metadata, non-sensitive lease status, and tab inventory.

The older React application is removed from the repository together with its package metadata, tests, and build configuration. It is not built, tested, served, shipped, or retained as an alternate `teshi web` implementation.

## Safety and Privacy

- The broker binds to loopback by default and rejects cross-origin/non-local control unless a future authenticated mode explicitly enables it.
- Session inventory exposes only the metadata needed for selection; URLs and titles may be sensitive and should support redaction in logs.
- Profile filesystem paths, cookies, storage values, form secrets, and page HTML are not returned unless explicitly part of an authorized snapshot contract.
- Mutating operations require a valid lease and explicit target.
- Extension version and protocol compatibility are checked before debugger attachment or page mutation.

## Risks / Trade-offs

- [A crashed agent leaves a session locked] -> Use renewable TTL leases and bounded cancellation.
- [Chrome tab IDs collide across profiles] -> Scope every tab and response by extension instance identity.
- [Page changes after snapshot] -> Return a revision/context token and verify before recommending or executing.
- [Semantic locators are not unique] -> Report match count, rank alternatives, and allow explicit disambiguation rather than silently adding positional selectors.
- [Multiple extension heartbeats overload the broker] -> Keep metadata incremental, bound queues, and expire stale instances.
- [The `debugger` permission conflicts with DevTools or another automation tool] -> Surface attachment ownership and actionable conflict diagnostics.
- [User-facing profile labels collide] -> Treat labels as display-only and route only by opaque identity.
- [MCP and CLI drift] -> Run both adapters against shared operation contract tests.
- [A WASM page cannot reach the host loopback broker when served remotely] -> Proxy the narrow discovery and tab-activation operations through same-origin daemon routes while keeping the broker loopback-only.
- [Two UI implementations drift or select different profiles] -> Keep selection policy and rendering model in shared GPUI code and remove the React implementation.

## Migration Plan

1. Define session identity, target, lease, snapshot, and locator result schemas with compatibility fixtures for existing commands.
2. Add stable identity and version metadata to the extension heartbeat while accepting legacy heartbeats during a transition window.
3. Convert the bridge to a multi-instance registry with per-instance queues, request correlation, and stale-session expiry.
4. Add explicit session/tab selection and lease operations to the reusable Rust operation layer and CLI.
5. Add ranked Playwright locator generation and in-browser verification over the targeted page/frame context.
6. Add the local STDIO MCP adapter and focused Agent Skills after the typed operations stabilize.
7. Package and smoke-test the extension plus Skills outside the source checkout.
8. Switch `teshi web`, installers, and release archives to the GPUI WASM distribution and delete the retired React application.
9. Exercise two or more real Chromium profiles and concurrent mock agents through the GPUI WASM surface before removing legacy single-session assumptions.

Rollback keeps legacy single-session commands and extension heartbeats available behind the compatibility adapter while disabling multi-session discovery and MCP publication.

## Open Questions

- What default lease TTL best balances interactive agent work with quick crash recovery?
- Should read-only snapshots require a lease for consistency, or permit an explicit best-effort observation mode?
- Which test-id attribute names should be configured globally versus per project?
- Should the first packaged release use only unpacked/installer installation or also target Chrome Web Store publication?
