## 1. Shared Protocol and Compatibility Foundation

- [x] 1.1 Add typed protocol feature identifiers for P0 control, P1 observability/artifacts, and each P2 privileged capability in `teshi-engine`
- [x] 1.2 Extend extension registration and session discovery with independently negotiated feature availability and supported action metadata
- [x] 1.3 Add shared request envelopes for explicit target, lease token, timeout, page revision, caller label, and request ID
- [x] 1.4 Add stable wire errors for stale references, unsupported actions, unavailable capabilities, denied capabilities, wait timeout, artifact failure, and duplicate mutation
- [x] 1.5 Add protocol fixtures covering a P0-only extension, a P0/P1 extension, optional P2 permissions, and incompatible feature requests
- [x] 1.6 Add CLI JSON contract tests proving new success and error payloads contain no lease or capability-grant secrets

## 2. P0 Broker Bootstrap and Ownership

- [x] 2.1 Refactor Chrome broker startup into a standalone bundled entry point that accepts user-session and request-scoped project context
- [x] 2.2 Add a per-user startup lock and compatible-broker discovery so concurrent CLI processes start at most one broker
- [x] 2.3 Make `teshi browser sessions` start or reuse the Chrome broker when Desktop is closed
- [x] 2.4 Extend `.teshi/cdp-endpoint.json` compatibility data with broker PID/start identity, protocol, mode, and loopback endpoints
- [x] 2.5 Change Desktop Chrome mode to attach to the compatible user-session broker and avoid owning a second conflicting process
- [x] 2.6 Return an actionable incompatibility error without terminating an already-running broker
- [x] 2.7 Add process lifecycle tests for first start, concurrent start, stale endpoint, incompatible broker, Desktop coexistence, and clean shutdown
- [x] 2.8 Measure heartbeat command latency and implement or defer the negotiated direct command channel with recorded benchmark evidence

## 3. P0 Snapshot References and Structured Execution

- [x] 3.1 Define the revision-bound element-reference record and bounded per-session reference cache
- [x] 3.2 Emit deterministic compact `@eN` aliases in structured snapshots without changing opaque internal routing
- [x] 3.3 Validate reference target, snapshot, revision, frame/shadow context, age, and cache membership before use
- [x] 3.4 Clear reference caches on navigation, document replacement, disconnect, broker stop, and configured eviction limits
- [x] 3.5 Extend browser action operations to accept exactly one of reference, structured locator candidate, or CSS compatibility selector
- [x] 3.6 Preserve role, label, placeholder, test-id, frame, and shadow semantics while sending structured candidates to the extension or embedded backend
- [x] 3.7 Re-verify structured candidates immediately before mutation and return stale or ambiguous errors without acting
- [x] 3.8 Add cross-profile collision, stale navigation, frame, shadow-root, and candidate-fidelity tests

## 4. P0 Input Actions and Wait Conditions

- [x] 4.1 Define one advertised action enum and value-validation contract shared by CLI, broker, Chrome extension, embedded mode, replay, and docs
- [x] 4.2 Implement separate DOM activation and CDP pointer activation with verified hit-point and focus metadata
- [x] 4.3 Align fill, type, select, press-key, assert-visible, assert-text, and navigation behavior across Chrome and embedded backends
- [x] 4.4 Add typed wait conditions for URL, visible text, element state, page revision change, and bounded load completion
- [x] 4.5 Return action outcome and wait outcome separately and prevent wait timeout from retrying a successful mutation
- [x] 4.6 Add at-most-once dispatch protection across timeout, reconnect, heartbeat fallback, and duplicate request IDs
- [x] 4.7 Extend CLI parsing and help for references, candidate JSON, pointer action, typed waits, and explicit focus behavior
- [x] 4.8 Add real Chromium acceptance tests for DOM click, pointer click, reactive-page waits, wait timeout, fill/type, select, key press, and duplicate suppression

## 5. P0 Profile, Tab, Window, and Label Operations

- [x] 5.1 Add typed lookup operations for tab, extension instance, browser metadata, and profile label
- [x] 5.2 Add CLI commands to set and clear labels with live-session uniqueness validation while retaining opaque routing identity
- [x] 5.3 Add lease-scoped tab activation, tab creation, tab closure, window creation, and optional tab-group operations
- [x] 5.4 Return the complete new target identity after tab or window creation and never change another caller's implicit target
- [x] 5.5 Reject ambiguous tab IDs and duplicate labels before mutation with non-sensitive recovery metadata
- [x] 5.6 Add two-profile tests for lookup, labels, open, close, activate, new window, grouping, and cross-profile ID collisions

## 6. P0 Agent, BDD, and Release Integration

- [x] 6.1 Route legacy `browser_snapshot`, `browser_click`, `browser_type`, `browser_assert`, and `browser_go_back` tools through shared typed operations
- [x] 6.2 Preserve legacy implicit targeting only for one eligible target and add ambiguity regression tests
- [x] 6.3 Version step bindings for references and structured locators while retaining a reader for the previous binding format
- [x] 6.4 Route `browser replay`, `verify`, and `heal-execute` through the canonical action and wait contracts
- [x] 6.5 Keep locator MCP tools semantically identical to CLI and add an explicit disabled-by-default switch for safe mutation tools
- [x] 6.6 Update the browser-testing Skill, compatibility declaration, extension README, CLI docs, and browser-mode diagrams for P0
- [x] 6.7 Add package smoke tests proving installed CLI broker bootstrap and all referenced P0 resources work outside the source checkout
- [x] 6.8 Run the full native quality gates, extension protocol tests, broker tests, package smoke tests, and two-profile P0 acceptance gate before starting P1

## 7. P1 Artifact Storage, Screenshots, and PDF

- [x] 7.1 Add project/target/request-correlated artifact metadata and managed `.teshi/artifacts/browser` storage with bounded filenames and paths
- [x] 7.2 Implement viewport screenshot capture with PNG/JPEG options and file-based output
- [x] 7.3 Implement full-page screenshot capture with dimension and byte limits
- [x] 7.4 Implement element screenshot capture from a current reference or structured candidate with re-verification
- [x] 7.5 Implement PDF output with advertised backend support, paper, orientation, scale, and background options
- [x] 7.6 Return only artifact path, size, format, target, request, revision, and warnings in normal JSON output
- [x] 7.7 Add cleanup operations that remove managed artifacts only when explicitly requested and never delete caller-selected outputs implicitly
- [x] 7.8 Add screenshot/PDF tests for multiple profiles, unsupported backends, oversized output, invalid paths, stale elements, and cleanup

## 8. P1 Console and Network Observability

- [x] 8.1 Add bounded per-session console ring buffers with start, list, clear, stop, level filter, age, entry, and byte limits
- [x] 8.2 Add bounded per-session network capture with start, list, detail, clear, and stop operations
- [x] 8.3 Default network capture to metadata and require explicit bounded body retrieval with truncation and encoding markers
- [x] 8.4 Implement configurable redaction for authorization, Cookie, token, password, and sensitive header or field names
- [x] 8.5 Clear console and network buffers on stop, profile disconnect, and broker shutdown
- [x] 8.6 Expose P1 CLI commands using the complete target and lease and add capability-aware help text
- [x] 8.7 Add isolation, retention, truncation, redaction, disconnect, debugger-conflict, and two-profile concurrency tests

## 9. P1 Monitoring, Upload, and Workspace Organization

- [x] 9.1 Add optional bounded before/after page summaries and structured diffs to mutation operations
- [x] 9.2 Ensure monitoring observes one action result and never retries or duplicates the mutation
- [x] 9.3 Add file upload operations for explicit local paths with filesystem policy, existence, size, target, and actionability validation
- [x] 9.4 Ensure upload errors do not enumerate unrelated local files or expose full paths beyond authorized output
- [x] 9.5 Complete window focus and tab-group controls with explicit focus opt-in and non-fatal group-organization errors
- [x] 9.6 Update shared capability discovery, CLI help, Skill guidance, and compatibility metadata for every P1 operation
- [x] 9.7 Add real-browser tests for page diffs, file upload, focus behavior, groups, and simultaneous P1 work in two leased profiles
- [x] 9.8 Run the full native, extension, broker, package, artifact, redaction, and real-browser P1 gate before starting P2

## 10. P2 Grant, Policy, Permission, and Audit Foundation

- [x] 10.1 Define privileged capability names, grant records, expiry, broker-instance binding, project binding, caller label, and revocation semantics
- [x] 10.2 Add interactive grant, list, revoke, and expiry CLI commands that never print reusable secret material in discovery
- [x] 10.3 Add default-deny user/project policy loading and require explicit non-interactive acknowledgement for each allowed capability
- [x] 10.4 Add metadata-only privileged audit records with redacted argument summaries and bounded retention
- [x] 10.5 Add optional Chromium permission status and user-gesture request flows in the extension popup
- [x] 10.6 Make P2 MCP tools absent by default and validate any startup allowlist against effective policy
- [x] 10.7 Add tests for missing, expired, wrong-project, wrong-profile, wrong-broker, revoked, policy-denied, and permission-unavailable grants

## 11. P2 Arbitrary JavaScript and Raw CDP

- [x] 11.1 Implement target-scoped arbitrary JavaScript with `javascript` grant, lease, timeout, revision check, and result byte limit
- [x] 11.2 Add file-based script input without logging script source or unrestricted results by default
- [x] 11.3 Implement raw CDP with an effective domain/method allowlist derived from policy and grant scope
- [x] 11.4 Deny browser-level, target-escaping, download, filesystem, or other high-risk CDP methods unless separately authorized
- [x] 11.5 Correlate and quarantine late JavaScript/CDP responses so they cannot satisfy another request
- [x] 11.6 Add audits and tests for timeouts, oversized results, disallowed methods, stale revisions, target escape attempts, and two-profile isolation

## 12. P2 Cookies, Settings, and Extension Management

- [x] 12.1 Implement metadata-only Cookie listing under a `cookies` grant and approved optional browser permission
- [x] 12.2 Add a distinct Cookie value-access scope with origin/partition scoping, redaction defaults, and output limits
- [x] 12.3 Implement allowlisted content-setting operations under a separate grant and optional permission
- [x] 12.4 Implement allowlisted extension-management read or mutation operations under separate grants, with mutation disabled until browser-specific validation passes
- [x] 12.5 Update capability discovery immediately after optional permission grant, revocation, or browser rejection
- [x] 12.6 Add browser-specific tests for permission prompts, denial, revocation, Cookie partitioning, value redaction, setting scope, and extension-management allowlists

## 13. Final Distribution and Acceptance

- [x] 13.1 Update MSI, portable archives, standalone browser-testing package, extension bundle, and compatibility metadata with independent P0/P1/P2 feature declarations
- [x] 13.2 Document safe defaults, grant procedures, revocation, audit location, artifact retention, optional permissions, and MCP exclusions
- [x] 13.3 Add upgrade tests from the previous extension, endpoint, step-binding, and package formats plus rollback coverage for disabled feature bits
- [x] 13.4 Run strict OpenSpec validation and synchronize any implementation-discovered contract corrections before release
- [x] 13.5 Run formatting, native workspace check/test/clippy/doc gates, web smoke, Python broker suites, extension tests, and browser-agent package smoke tests
- [x] 13.6 Complete real Chromium acceptance with two Profiles and two concurrent agents across P0, P1, and explicitly granted P2 operations
- [x] 13.7 Verify a default installation exposes no active P2 grant, optional privileged permission, Cookie value, raw CDP tool, or arbitrary-JavaScript MCP tool

## 14. Review Corrections

- [x] 14.1 Authenticate loopback WebSocket command clients and reject untrusted browser origins before accepting leases, grants, or commands
- [x] 14.2 Bind policy, grant, artifact, cleanup, and upload behavior to each request's canonical project context
- [x] 14.3 Convert element screenshot viewport bounds to page coordinates after scrolling
- [x] 14.4 Prevent raw CDP from bypassing separately gated JavaScript, Cookie, and filesystem/upload capabilities
- [x] 14.5 Make bounded artifact and network-body payloads transportable without exceeding WebSocket message limits
- [x] 14.6 Align artifact failure wire codes across Rust, Python, the extension, fixtures, and regression tests
