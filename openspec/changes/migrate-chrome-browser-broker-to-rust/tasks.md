## 1. Establish the Python and Extension Behavior Baseline

- [x] 1.1 Inventory every Chrome-only helper and every Python consumer shared with Embedded or WinApp; record the final removal boundary.
- [x] 1.2 Expand `browser_contract_fixtures.json` with deterministic discovery, heartbeat/reconnect, multi-target resolution, leases, request races, locator outcomes, evidence, authorization, and malformed/oversized cases.
- [x] 1.3 Add side-effect and state-transition assertions to Python broker tests for every fixture sequence; normalize generated IDs/timestamps only.
- [x] 1.4 Add the shared fixtures to a Rust protocol consumer and retain the current Python implementation as the temporary differential oracle.
- [x] 1.5 Run Python browser unit tests, extension Node protocol/network tests, existing real two-Profile acceptance where available, and the existing transport benchmark; record commands, results, and uncovered behavior.
- [x] 1.6 Add browser contract suites to CI without changing which broker is selected at runtime.

## 2. Implement Rust Protocol and Local Transport

- [x] 2.1 Add `teshi-browser-broker` to the Workspace and define strict typed protocol DTOs, stable errors, versions, feature negotiation, and fixture tests using existing Workspace dependency versions.
- [x] 2.2 Implement loopback-only fixed-port discovery and dynamic WebSocket listeners with bounded HTTP headers/bodies, frame sizes, connections, queues, and timeouts.
- [x] 2.3 Implement token validation, trusted-origin behavior, sanitized discovery, private per-user credential storage, endpoint PID/start identity checks, and negative tests; resolve and document the supported extension identity upgrade path.
- [x] 2.4 Add the hidden CLI broker-serve entry point and process launcher; verify concurrent startup, compatible reuse, incompatible listeners, crashes, and no process termination of unrelated listeners.
- [x] 2.5 Keep the Python broker as the runtime selection and compare Rust transport responses/frames against the shared fixtures and Python oracle.

## 3. Port Session, Target, Lease, and Request State

- [x] 3.1 Implement typed Profile/session records, heartbeat expiry, reconnect generations, compatibility health, and target-scoped frame/subscription state.
- [x] 3.2 Implement explicit and legacy target resolution; reject ambiguous and mismatched targets before extension dispatch.
- [x] 3.3 Implement project/caller-bound exclusive leases, renewal, release, expiry, and immediate pre-dispatch validation.
- [x] 3.4 Implement bounded command queues and pending request correlation, cancellation, disconnect cleanup, late-response quarantine, and exactly-once completion.
- [x] 3.5 Implement bounded page-revision element references and test stale, duplicate, cross-Profile, cross-project, and lease-expiry races.
- [ ] 3.6 Run deterministic Python/Rust differential state tests and real two-Profile isolation before moving routing to Rust.

## 4. Port Locator Policy and Browser Operation Coordination

- [ ] 4.1 Port snapshot normalization, candidate generation/ranking, structured intent scoring, and verification-result integration with typed Rust models.
- [ ] 4.2 Preserve role/name, label, placeholder, test ID, stable attribute, CSS fallback, iframe, and Shadow DOM semantics; compare fixture rankings with Python.
- [ ] 4.3 Route actions, waits, locator re-verification, page revision checks, and structured assertions through one command coordinator.
- [ ] 4.4 Add negative tests for ambiguous, missing, stale, hidden, disabled, timed-out, navigation-changed, and assertion-failed operations; verify none report success or silently retry.
- [ ] 4.5 Replay existing browser Features and step bindings against the Rust broker and compare successful side effects and stable failure codes.

## 5. Port Screenshot, Console, Network, and Artifact Handling

- [ ] 5.1 Preserve TSH1 metadata and route bounded binary preview frames only to subscribers of the complete matching target.
- [ ] 5.2 Port screenshot/PDF evidence coordination and managed artifact path validation, byte/dimension/pixel bounds, atomic writes, and safe cleanup.
- [ ] 5.3 Port target-scoped Console filtering, redaction, truncation, age/count/byte eviction, and termination diagnostics.
- [ ] 5.4 Port exact hostname-filtered Network capture, request/response correlation, capture IDs, monotonic sequences, contiguous ack barriers, deduplication, resend, and loss diagnostics.
- [ ] 5.5 Enforce body access grants, bounded memory/backpressure, no raw-body logging, and cleanup on target closure, capture stop, lease expiry, and broker restart.
- [ ] 5.6 Run real Chrome screenshot, Console, Network, reconnect, large-event, and backpressure scenarios; record memory and latency versus Python.

## 6. Port Privileged Authorization and Security Controls

- [ ] 6.1 Port project policy loading, typed capability grants, expiry/revocation, OS-user/broker/project/caller/Profile binding, and optional Chrome permission checks.
- [ ] 6.2 Port raw CDP method allowlists, JavaScript/Cookie/content-setting/extension metadata gates, upload scope, artifact access, and redacted audit records.
- [ ] 6.3 Add negative integration tests for hostile origins, stale tokens, path traversal/symlink escape, malformed targets, oversized frames, unknown fields/operations, grant revocation, and post-disconnect reuse.
- [ ] 6.4 Verify the existing extension permissions remain explicit and no protocol-v0 compatibility path bypasses current authorization.
- [ ] 6.5 Complete security review of discovery, endpoint permissions, logs, request scoping, and actual server-side checks before production routing changes.

## 7. Integrate All Teshi Entry Points

- [ ] 7.1 Switch Chrome startup in `teshi-engine` from Python venv resolution/import checks to the shared Rust broker launcher; keep Embedded and WinApp branches unchanged.
- [ ] 7.2 Update endpoint creation/health/reconnect so every project points to the same broker generation without exposing project paths or credentials publicly.
- [ ] 7.3 Verify CLI, Agent, MCP, Daemon, GPUI Desktop, and daemon-backed GPUI WASM Web requests use the same typed state and broker identity.
- [ ] 7.4 Test multiple projects, multiple terminals, client exits, daemon restarts, broker crashes, lease ownership, and recovery while preserving other clients' sessions.
- [ ] 7.5 Run Embedded Playwright and WinApp regression suites to prove their existing runtimes and error guidance remain intact.
- [ ] 7.6 Enable Rust as the test-only Chrome implementation behind an explicit development selector; do not auto-fallback between implementations.

## 8. Complete Acceptance, Packaging, and Chrome Python Removal

- [ ] 8.1 Run full shared protocol, Rust workspace, Python Embedded/WinApp, extension Node, CLI/MCP/Daemon/UI, and security suites; resolve every unexplained failure.
- [ ] 8.2 Run real Chrome end-to-end tests with two Profiles and at least two project roots for connection, navigation, snapshot, locator, click/fill/assert, screenshot, Console, Network, and reconnect.
- [ ] 8.3 Run Chrome acceptance with Python executables absent from PATH and no project venv; prove Embedded still reports its documented Python requirement and remains functional when configured.
- [ ] 8.4 Build and inspect Windows MSI/EXE, Linux and macOS archives, nightly artifacts, and browser-testing package contents; retain only Embedded/WinApp Python resources that have live consumers.
- [ ] 8.5 Measure broker startup, locator acquisition, command execution, screenshot transport, steady/peak memory, and high-volume Network behavior against the recorded Python baseline.
- [ ] 8.6 Update browser docs, skills, extension upgrade instructions, user errors, endpoint fixture markers, CI/release/nightly contract tests, and supported protocol metadata.
- [ ] 8.7 After every acceptance gate passes, remove the production Chrome Python selection and dead Chrome-only Python broker code; retain differential fixtures and any Python code still imported by Embedded.
- [ ] 8.8 Run final release-equivalent validation and publish a stage report with exact commands/results, behavior invariants, failure recovery, performance measurements, and any platform boundary not exercised.
