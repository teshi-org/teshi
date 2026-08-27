## Context

Teshi currently has two partially overlapping browser-control models. The older exploration tools use snapshot references and high-level click/type operations, while the new external-agent path uses an explicit `extension_instance_id + window_id + tab_id` target, exclusive profile leases, correlated requests, page-context revisions, verified Playwright candidates, and a local STDIO MCP adapter. Chrome control still depends on a Desktop-started Python sidecar, extension commands are narrower than the documented action set, and the CLI lacks the operational surface expected from a standalone browser agent.

The change spans Rust CLI and operation models, the Python broker/sidecar, the Chromium extension, packaging, compatibility metadata, and BDD replay. P0 and P1 must remain safe for normal local use. P2 deliberately crosses a privilege boundary because arbitrary script, CDP, cookies, and browser settings can expose or alter data outside the selected test step.

## Goals / Non-Goals

**Goals:**

- Provide a standalone, auto-starting Chrome broker path usable without opening Teshi Desktop.
- Unify legacy browser refs/actions and the multi-profile broker behind one typed target, lease, request, revision, and error model.
- Make P0 sufficient for dependable multi-profile browser operation: inspect, target, execute, wait, and manage tabs.
- Add bounded P1 observability and artifact workflows without requiring unrestricted script or Cookie access.
- Make P2 capabilities explicit, short-lived, auditable, and disabled by default.
- Preserve verified Playwright locator generation, BDD step bindings, replay, and same-host isolation.

**Non-Goals:**

- Replacing Playwright for full cross-browser test execution or implementing WebDriver compatibility.
- Providing remote unauthenticated control of a user's browser.
- Allowing two owners to mutate one browser profile concurrently.
- Enabling P2 capabilities automatically when the extension or agent package is installed.
- Guaranteeing automation of `chrome://`, extension, browser-privileged, DRM, or other non-debuggable pages.
- Copying another browser CLI's wire protocol or command syntax verbatim.

## Decisions

### 1. One shared typed operation layer

All new commands will become `teshi-engine` browser operations. CLI, Desktop, replay, and any MCP exposure will adapt to those operations rather than constructing extension JSON independently. Every stateful operation carries a request ID, explicit target, lease token, timeout, and optional page-context revision.

This extends the existing locator architecture and prevents CLI, MCP, and UI semantics from drifting. Direct one-off JSON commands remain only inside the bounded legacy compatibility adapter.

Alternative considered: implement P0/P1 only in the CLI. That is faster initially but would duplicate target validation and reproduce the old single-session assumptions.

### 2. A user-session broker with CLI bootstrap

Chrome mode will use one loopback-only broker per OS user session. `teshi browser` commands that require Chrome will discover the broker, acquire a process-start lock, start it when absent, wait for readiness, and then execute the requested operation. Project root becomes canonicalized request context rather than broker ownership; policy, grants, uploads, artifacts, and cleanup use that request root instead of the root that happened to start the broker. Desktop attaches to the same broker instead of owning an incompatible second process.

The endpoint record will include broker PID/start identity, protocol/version, authenticated loopback URLs, mode, and project context needed by compatibility clients. A random broker-start token authenticates WebSocket connections and every mutating HTTP fallback request; browser-page origins and unauthenticated requests are rejected. Startup is idempotent; a live compatible broker is reused, while an incompatible broker produces an actionable version error instead of being killed implicitly.

Alternative considered: one broker per project. The fixed extension port and multiple Profiles make that model ambiguous and prevent two project CLIs from coexisting reliably.

### 3. Snapshot references are revision-bound aliases

P0 introduces compact references such as `@e1`, but they are presentation aliases for broker records containing target identity, snapshot ID, page-context revision, frame/shadow context, and an element handle or re-resolution recipe. Reference caches are isolated per extension instance and bounded by count and age.

Using a reference with another target, after navigation/document replacement, or after eviction fails with `stale_element_reference`; Teshi never silently resolves the same alias against a newer page.

### 4. Structured locators are first-class execution targets

Control operations accept exactly one of: a snapshot reference, a structured verified locator candidate plus its page revision, or a CSS selector compatibility input. Structured candidates are re-verified immediately before mutation and preserve role/label/test-id, frame, and shadow context. They are not converted to CSS merely to reach the extension.

Locator acquisition remains observational. The caller must issue a separate action request using the candidate and lease.

### 5. P0 uses typed input and typed waits

P0 adds DOM activation and CDP-backed pointer activation as distinct actions. Pointer activation dispatches a bounded mouse event sequence at the verified element center. Keyboard and text actions use explicit action types and consistent value validation across Chrome and embedded modes.

Post-action waits are typed conditions such as URL match/change, text visible, element visible/hidden/enabled, page revision change, and bounded load completion. Arbitrary JavaScript is not a P0 wait condition. Results contain action status and wait status separately so a successful click followed by a timeout is not misreported as an unexecuted click.

### 6. P0 tab lifecycle remains lease-scoped

The CLI can list/lookup sessions, assign a unique display label, open a tab, close a tab, activate a tab, and optionally create a window or group. Profile labels remain aliases and never replace opaque routing identity. Mutating tab/window operations require the selected Profile lease. Newly created targets are returned with their complete composite identity.

### 7. P1 artifacts and diagnostics are bounded per session

Screenshots, PDFs, console events, network metadata/bodies, uploads, and operation diffs are correlated to request ID and target. Binary data is written to files, not emitted as base64 in normal JSON output. Artifact ingress and the WebSocket/HTTP transports that carry it share explicit compatible size bounds, and element screenshot clips are converted from viewport to page coordinates before CDP capture. Default storage is a project-scoped `.teshi/artifacts/browser/` area with configurable output paths constrained by normal filesystem authorization.

Console/network capture uses bounded ring buffers with entry, age, and byte limits. Network bodies are omitted by default, size-limited when requested, and marked as truncated/base64. Authorization, Cookie, and common secret fields are redacted from summaries and audit output. File upload requires an explicit local path and never enumerates unrelated files.

### 8. P2 uses two independent gates

Each privileged operation requires both:

1. a valid profile lease and an explicit short-lived Teshi capability grant (`javascript`, `raw-cdp`, `cookies`, `content-settings`, or `extension-management`); and
2. any Chromium optional permission approved through an extension user gesture when the browser API requires it.

Grants are bound to the local user, broker instance, project, extension instance, capability, and expiry. They are not reusable across broker restarts and are not returned by discovery. Interactive grants require confirmation; non-interactive grants require an explicit policy entry and command-line acknowledgement. MCP does not expose P2 tools unless the server is started with an allowlist that is no broader than the active policy.

Every P2 call writes a metadata-only audit event containing time, capability, caller label, target, request ID, outcome, and redacted argument summary. Script source, Cookie values, response bodies, and CDP payload bodies are excluded unless a separately documented debug mode is enabled.

### 9. Extension permissions remain least-privilege

P0 uses the existing `debugger`, `tabs`, `activeTab`, `alarms`, and `storage` permissions. P1 adds only permissions proven necessary by its final implementation. Cookie, content-setting, and extension-management permissions are declared optional and requested through the extension popup; failure or revocation produces `browser_capability_unavailable` without weakening other operations.

### 10. Phase gates are independently releasable

P0 is a prerequisite for P1, and P1 is a prerequisite for P2 implementation work, but P2 remains runtime-disabled by default. Each phase has its own protocol feature bits and compatibility checks so a P0 CLI can explain that a connected extension lacks P1 instead of treating the entire session as unusable.

## Risks / Trade-offs

- [A user-session broker changes current Desktop process ownership] -> Introduce compatibility endpoint fields, a startup lock, attach/detach tests, and a rollback path that restores Desktop-owned startup while retaining operation schemas.
- [Heartbeat command delivery makes interactive control feel slow] -> Measure P0 latency and allow a protocol-versioned bidirectional command channel while retaining heartbeats for liveness and fallback.
- [References become stale frequently on reactive pages] -> Return precise revision metadata and recovery instructions; never retarget silently.
- [CDP pointer input can focus or scroll the browser] -> Keep focus behavior explicit, report it in results, and retain DOM activation as a separate lower-impact action.
- [Network/console buffers expose sensitive data] -> Default to metadata, redact known secret fields, bound retention, clear on disconnect/stop, and require explicit body capture.
- [P2 turns Teshi into a high-privilege local controller] -> Use capability grants, optional browser permissions, same-host enforcement, audit metadata, lease checks, and default-deny MCP exposure.
- [Broad command growth destabilizes BDD replay] -> Route replay through the same typed operations and preserve existing step-binding versions with explicit migration.
- [Chrome and embedded Playwright semantics differ] -> Publish per-backend capability discovery and conformance tests rather than claiming unsupported parity.

## Migration Plan

1. Add protocol feature discovery and shared operation types without changing existing command behavior.
2. Introduce CLI broker bootstrap and migrate Desktop to attach to the compatible user-session broker.
3. Implement P0 refs, structured execution, input, waits, lookup/labels, and tab lifecycle behind feature bits; retain legacy single-target commands.
4. Update step bindings and replay to the canonical target/action model, with readers for the previous binding version.
5. Ship P0 compatibility metadata and real multi-Profile acceptance tests.
6. Add P1 artifacts and diagnostics with bounded storage and redaction; keep each subsystem disabled until explicitly started or requested.
7. Add P2 grant/policy/audit infrastructure first, then privileged capabilities individually. No release enables them by default.
8. Rollback by disabling the relevant feature bit and using legacy commands; persisted binding and policy formats remain readable by the immediately preceding release.

## Open Questions

- Whether the P0 direct command channel should replace heartbeat polling immediately or first ship as an optional negotiated transport requires a latency benchmark during implementation.
- Chromium support for optional `management` and `contentSettings` permissions must be validated per supported browser before those individual P2 capabilities can leave experimental status.
