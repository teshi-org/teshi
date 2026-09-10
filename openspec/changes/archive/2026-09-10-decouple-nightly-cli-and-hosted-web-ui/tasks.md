## 1. Freeze Protocol and Migration Contracts

- [x] 1.1 Define shared serialized types and schema versions for `client_hello`, `server_hello`, request, response, event, and error envelopes, including stable close/error codes.
- [x] 1.2 Define and test the `ui-manifest.json` schema, compiled UI compatibility constants, minimum CLI `BuildIdentity`, and monotonic build-sequence comparison rules.
- [x] 1.3 Inventory every `/api/v1/*` operation and event used by `apps/teshi-web`, then map the hosted UI's LLM, Browser, Project, filesystem, Gherkin/BDD, run/exchange, locator/steps, Terminal, Agent, and daemon-lifecycle needs to namespaced control methods/events or preview messages.
- [x] 1.4 Define the explicit `HostedWebUi` capability matrix and verify that it excludes administrative operations not required by the UI.

## 2. Add Ephemeral Hosted Sessions and Secure WebSocket Handshakes

- [x] 2.1 Extend daemon session state to mint cryptographically random, process-memory-only hosted launch sessions with teardown/restart invalidation and safe redaction in diagnostics.
- [x] 2.2 Add exact production Origin validation for `https://teshi.org` and `https://teshi-org.github.io` on `/ws/control` and `/ws/preview`, with explicit development/test configuration rather than a production wildcard.
- [x] 2.3 Implement the first-message timeout and token/channel/protocol/build handshake so no business handler or sidecar connection runs before authentication and negotiation succeed.
- [x] 2.4 Enforce `HostedWebUi` authorization independently of TCP peer address and add negative tests for tokenless loopback, invalid/expired tokens, wrong channels, malformed hellos, untrusted Origins, and forbidden methods.
- [x] 2.5 Implement deterministic single-control-connection ownership and authenticated reconnect behavior without automatic replay of non-idempotent requests.

## 3. Implement the Control Protocol

- [x] 3.1 Add `/ws/control` routing and a bounded dispatcher that correlates each accepted request with exactly one response and isolates individual request failures.
- [x] 3.2 Adapt the inventoried Project, filesystem, Gherkin/BDD, run/exchange, locator/steps, and daemon-lifecycle operations to shared domain functions behind namespaced control methods.
- [x] 3.3 Adapt LLM configuration, browser-session, Terminal, and Agent operations to the same control dispatcher without duplicating domain authorization or validation rules.
- [x] 3.4 Move hosted runtime subscriptions onto the authenticated control socket with ordered event envelopes and documented bounded overflow behavior for Terminal/Agent bursts.
- [x] 3.5 Add protocol tests for correlation, authorization, concurrent requests, event ordering, reconnect, bounded queues, overflow behavior, and connection cleanup.

## 4. Implement the Independent Preview Protocol

- [x] 4.1 Add `/ws/preview` with preview-channel handshake and a dependency on the same live hosted control session, including a bounded reconnect grace rule.
- [x] 4.2 Reuse the existing private sidecar relay and newest-frame-wins buffering for browser and WinApp streams while keeping non-frame messages independently bounded.
- [x] 4.3 Prevent arbitrary client-to-sidecar command forwarding and verify that daemon responses/UI state never expose private sidecar URLs.
- [x] 4.4 Add load tests proving a stalled preview consumer neither grows memory without bound nor delays control RPC/events beyond the recorded acceptance threshold.

## 5. Migrate the GPUI WASM Client

- [x] 5.1 Add launch-state parsing for `location.hash`, validate the loopback port/token pair, copy it only to memory, and remove the fragment with `history.replaceState` before normal UI startup.
- [x] 5.2 Replace the synchronous same-origin XHR backend with one unique negotiated `/ws/control` client supporting correlated asynchronous RPC, events, reconnect, and user-visible transport failures.
- [x] 5.3 Migrate each operation group in the checked inventory and add UI/integration coverage before marking that group complete.
- [x] 5.4 Move browser and WinApp preview to `ws://127.0.0.1:<port>/ws/preview`, authenticate its first message, and retain reconnect/latest-frame behavior independently of control.
- [x] 5.5 Add the compatibility gate and explicit upgrade-nightly state so an incompatible or unverifiable CLI issues no business request and never selects historical UI assets.
- [x] 5.6 Apply an `/app/` security policy with no third-party scripts, restrictive script/connect sources and referrer behavior, then test that the token is absent from storage, logs, HTTP requests, and referrers.

## 6. Change teshi web into a Hosted-UI Launcher

- [x] 6.1 Refactor daemon startup to use an OS-selected available port on `127.0.0.1` by default and return the actually bound port without a fixed-port race.
- [x] 6.2 Mint the hosted launch session before opening the browser and open exactly `https://teshi-org.github.io/app/#port=<port>&token=<session-token>` without logging the token; retain `teshi.org` as a compatible alternate entrypoint.
- [x] 6.3 Remove the production startup dependency on `apps/teshi-web/dist`, installed `share/web`, and `--dist`, while preserving only explicitly documented development diagnostics if still required.
- [x] 6.4 Verify `--no-open`, explicit diagnostic port behavior, project selection/reuse semantics, daemon idle shutdown, session teardown, and stale-manifest recovery under the new launch lifecycle.

## 7. Move Web UI Delivery to Hugo Pages

- [x] 7.1 Update the `teshi-org/teshi-org.github.io` workflow to resolve and check out an immutable Teshi source SHA and install pinned Rust nightly, `wasm32-unknown-unknown`, and matching `wasm-bindgen` tooling.
- [x] 7.2 Build `apps/teshi-web` in the Hugo workflow, stage the complete current distribution under `/app/`, and generate `ui-manifest.json` from the resolved source/deployment identity and minimum CLI contract.
- [x] 7.3 Integrate Hugo and Web UI into one Pages artifact with content-addressed assets, a cache-busted entrypoint/manifest path, and no historical UI catalog.
- [x] 7.4 Gate Pages deployment on manifest validation plus the Web UI smoke suite, and verify a failed build leaves the active Pages deployment unchanged.
- [x] 7.5 Record and test the cross-repository trigger/manual input path so Web UI deployment is deliberate and is not performed by each ordinary CLI nightly build.

## 8. Decouple Nightly Packaging

- [x] 8.1 Update the nightly invocation and reusable release conditions so `windows-installer` nightly jobs skip Rust WASM target installation, `wasm-bindgen` cache/install, and GPUI WASM compilation.
- [x] 8.2 Remove nightly `share/web` staging, Web-file validation, generated installer components, web-dist upload/download, and Web entries from bundle/update manifests.
- [x] 8.3 Add artifact-content tests proving the nightly installer and published assets contain CLI/daemon/runtime support but no complete Web UI payload.
- [x] 8.4 Run the shared release workflow contract tests to prove separately configured stable packaging behavior was not silently changed.

## 9. End-to-End Acceptance and Staged Migration Evidence

- [x] 9.1 Run a supported Chromium end-to-end test from the real `https://teshi.org/app/` secure context to both `ws://127.0.0.1` endpoints, including browser mixed-content/private-network behavior and actionable failure diagnostics.
- [x] 9.2 Capture the initial hosted-page request and prove the fragment port/token never reaches the hosting service, referrer, logs, or browser persistence.
- [x] 9.3 Verify valid launch, exact-Origin rejection, pre-handshake rejection, non-Admin authorization, session expiry/restart, reconnection, and old-nightly upgrade refusal.
- [x] 9.4 Exercise every checked UI operation over `/ws/control` and browser/WinApp streams over `/ws/preview`, and prove normal hosted workflows make no `/api/v1/*` request.
- [x] 9.5 Run daemon, WASM smoke, release-manifest, installer-content, and repository quality gates while confirming all existing `/api/v1/*` routes and REST regressions remain present.
- [x] 9.6 Publish a migration coverage report and defer every REST deletion/deprecation item to a separate future OpenSpec change.
