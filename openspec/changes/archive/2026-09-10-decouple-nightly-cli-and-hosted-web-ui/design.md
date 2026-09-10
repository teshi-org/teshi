## Context

This change crosses the Teshi CLI, daemon, WASM shell, release packaging, and the separate `teshi-org.github.io` Pages repository. The architecture is already selected: the latest full GPUI WASM UI runs at `https://teshi-org.github.io/app/`, with `https://teshi.org/app/` retained as a compatible alternate entrypoint, while `teshi web` starts a loopback-only local daemon and opens the hosted application with ephemeral launch data in the URL fragment.

### Current architecture facts

1. `apps/teshi-cli/src/main.rs` forwards `teshi web` into `teshi_daemon::run_client`. `run_client` currently requires a local `apps/teshi-web/dist` (or installed `share/web`), starts or reuses a project daemon, and opens `http://127.0.0.1:<port>`.
2. `WebOptions` says the port is auto-picked, but `ensure_daemon` actually uses fixed port `20253` when `--port` is absent. The daemon manifest is project-scoped and a live daemon is reused. No launch session is minted by `teshi web`.
3. The daemon currently serves `dist` as its HTTP fallback. Most Web UI operations are synchronous same-origin XHR calls to `/api/v1/*`.
4. Existing browser-facing WebSockets are `/api/v1/events` for server events and `/api/v1/browser/stream` for preview relay. The latter uses a latest-frame watch buffer plus a bounded control queue and keeps the underlying browser/WinApp sidecar URL private.
5. Browser routes use permissive CORS at the outer layer but explicit `same_origin_only` middleware on protected/session/preview routes. Hosted WebSocket upgrades use an exact allowlist containing the two supported hosted origins.
6. HTTP authentication reads `X-Teshi-Token`. A supplied invalid token fails closed, but a tokenless request whose TCP peer is loopback currently receives implicit `Admin`; browser WebSocket clients also cannot set this custom header with the standard WebSocket API.
7. `SessionStore` holds random UUID-based tokens in memory and supports `Admin`, `AgentRecorder`, and `BatchRunner`; it does not define a hosted-Web-UI scope or a connection-level first-message handshake.
8. `scripts/build-teshi-web.sh` and its PowerShell counterpart build `apps/teshi-web` with nightly Rust and `wasm-bindgen`. The reusable `release.yml` installs that toolchain, builds the WASM distribution for every Windows matrix build, stages it into `share/web`, and uploads a web-dist artifact. `nightly.yml` calls this workflow in `windows-installer` mode, so ordinary nightly CLI updates still build and package the UI.
9. Product `BuildIdentity` already contains SemVer, channel, full Git SHA, build timestamp, and monotonic CI `build_sequence`. No hosted UI manifest currently declares a minimum compatible identity.
10. `teshi-org.github.io` currently deploys a Hugo marketing site only. Its Pages workflow checks out that repository, runs Hugo, uploads `public`, and has no Teshi source checkout, Rust/WASM build, `/app/` shell, or compatibility manifest.

### Gap to the target architecture

| Boundary | Current | Required |
| --- | --- | --- |
| UI ownership | daemon serves bundled `dist` | Hugo Pages publishes the complete latest UI at `/app/` |
| CLI launch | fixed default port, no session, localhost URL | available loopback port, ephemeral session, hosted fragment URL |
| Business transport | same-origin REST plus event WS | authenticated `/ws/control` RPC and events |
| Preview transport | same-origin `/api/v1/browser/stream` | independently authenticated `/ws/preview` |
| Browser trust | same-origin and loopback implicit Admin | exact trusted hosted Origin plus explicit scoped session |
| Compatibility | release assets are co-built | hosted manifest and negotiated minimum CLI identity |
| Nightly delivery | Windows nightly builds/packages WASM | nightly contains no complete Web UI and does not build WASM |
| Migration | REST is the active backend | REST remains during phase one while WS reaches coverage |

## Goals / Non-Goals

**Goals:**

- Establish `teshi.org/app/` as the single hosted, latest-only GPUI WASM Web UI.
- Make `teshi web` a loopback daemon/session launcher rather than a static-site host.
- Provide authenticated, versioned, connection-oriented control and preview protocols.
- Preserve control responsiveness when preview frames are large or consumers are slow.
- Reject incompatible local CLIs before any business request is accepted and provide explicit nightly-upgrade guidance.
- Remove GPUI WASM work and payloads from nightly CLI publication without changing unrelated release contents.
- Migrate incrementally while keeping existing `/api/v1/*` routes operational in phase one.

**Non-Goals:**

- Deleting or globally deprecating `/api/v1/*` REST routes.
- Hosting historical UI versions or selecting an older UI for an older CLI.
- Relaying daemon traffic through `teshi.org`, exposing the daemon to a LAN, or adding a cloud control plane.
- Granting arbitrary web origins, arbitrary loopback pages, or loopback TCP peers Admin privileges.
- Replacing the browser/WinApp sidecars or changing their private loopback protocols.
- Redesigning GPUI views, Terminal/Agent/Project/BDD domain behavior, or the native desktop transport.
- Changing stable-channel installer policy beyond shared-workflow refactoring strictly required to make the nightly path skip Web UI build/package steps.

## Decisions

### 1. The hosted page owns UI assets; the daemon owns local state and execution

The canonical application URL is `https://teshi-org.github.io/app/`; `https://teshi.org/app/` is a compatible alternate entrypoint. The Pages artifact contains the loader, JavaScript glue, WASM, static resources, and `ui-manifest.json`. The daemon no longer needs a Web UI distribution to satisfy `teshi web`; it owns only local APIs, runtime state, sidecars, sessions, and WebSocket endpoints.

The `apps/teshi-web` source remains in the Teshi repository so it can share Rust UI crates. The Hugo Pages workflow checks out an immutable Teshi source SHA, performs the GPUI WASM build itself, places the result under Hugo's `/app/` output, emits the manifest, then deploys one Pages artifact. A dispatch/manual input is resolved to and recorded as a full source SHA. Rebuilding the UI is a Web UI deployment action, not a side effect of each CLI nightly.

Alternatives such as continuing to serve local assets, uploading the UI as a nightly release artifact, or maintaining versioned UI directories are excluded by the selected architecture.

### 2. `teshi web` creates an ephemeral loopback launch session

When no explicit diagnostic port is requested, the launcher binds or reserves port `0` and uses the OS-selected available port; it must not merely retry the fixed legacy port. The production daemon binds `127.0.0.1` only. The launcher creates a cryptographically random, unguessable token in the daemon's in-memory session store before opening the browser.

The launch URL is:

```text
https://teshi-org.github.io/app/#port=<port>&token=<session-token>
```

The hosted page copies the values from `location.hash` into memory and immediately removes the sensitive fragment from browser history with `history.replaceState`. URL fragments are not part of the HTTP request to GitHub Pages or `teshi.org`; the `/app/` page must also avoid third-party scripts and set a restrictive policy so hosted dependencies cannot read the token.

“One-time session” means one ephemeral launcher-created session, scoped to the daemon process and this hosted-UI launch. It is never persisted and becomes invalid on explicit teardown, daemon exit, or daemon restart. The token may reauthenticate the same browser session after a transient socket reconnect, but concurrent control ownership is rejected or replaces only the same authenticated session according to a deterministic single-control-connection rule.

The existing `--port`/`--no-open` flags may remain diagnostic controls. `--dist` and local static serving are not part of the production `teshi web` path; any retained developer-only local route must be explicit and must not be packaged as the nightly UI.

### 3. WebSocket upgrade trust and first-message authentication are separate gates

Both `/ws/control` and `/ws/preview` accept browser upgrades only when the `Origin` header exactly matches one of the configured production origins `https://teshi.org` or `https://teshi-org.github.io` (with an explicit development allowlist available only in development/test configuration). Missing, malformed, `null`, HTTP, subdomain, lookalike, or unrelated origins are rejected before upgrade. WebSocket Origin validation is used instead of CORS, which does not authorize WebSocket upgrades.

After upgrade, the first application message must be a versioned `client_hello` carrying the launch token, channel (`control` or `preview`), and client protocol/UI compatibility identity. Until it succeeds, the connection has no business authorization. A timeout, malformed first message, invalid/expired token, wrong channel, or incompatible protocol closes the socket with a stable machine-readable error and performs no domain operation.

The daemon replies with `server_hello` containing its `BuildIdentity`, selected protocol versions, session identifier, and granted capabilities. The hosted UI uses a dedicated `HostedWebUi` role/capability set that enumerates the operations required by the full UI. It is not `Admin`, and `peer.ip().is_loopback()` is never sufficient to elevate a hosted WebSocket.

### 4. `/ws/control` is a multiplexed RPC/event protocol

After the handshake, `/ws/control` carries JSON messages with stable envelopes:

```json
{"type":"request","id":"opaque-client-id","method":"project.open","params":{}}
{"type":"response","id":"opaque-client-id","ok":true,"result":{}}
{"type":"event","event":"terminal.output","payload":{}}
{"type":"error","code":"incompatible_cli","message":"...","details":{}}
```

Every request has a client-generated correlation id and exactly one terminal response. Method names are namespaced and versioned by the negotiated control protocol. Phase-one mapping covers every operation the hosted UI actually uses across LLM configuration, browser sessions, Project, filesystem, Gherkin/BDD, runs/exchanges, locator/steps, Terminal, Agent, and daemon lifecycle. Existing engine functions remain the domain implementation; the WS dispatcher is an adapter, not a second business layer.

Events share the authenticated control socket and preserve ordering within one connection. Queues are bounded. Terminal output and other bursty events must have explicit backpressure/coalescing behavior and must not silently create additional control sockets. Reconnect always repeats `client_hello`, restores only safe subscriptions/state, and never replays non-idempotent requests automatically.

The old `/api/v1/events` and REST calls stay available during migration and tests, but the hosted production UI is accepted only when its business operations no longer depend on them.

### 5. `/ws/preview` is isolated from control traffic

Browser and WinApp preview frames use `/ws/preview`, never `/ws/control`. Its first message authenticates the same ephemeral session and negotiates the preview protocol. A preview connection is accepted only while its authenticated control session is valid (or within a short deterministic reconnect grace period), preventing an independently stolen preview token from becoming a durable channel.

The daemon still connects to private loopback sidecars and chooses the supported stream; the page never learns their URLs. The existing newest-frame-wins buffer is retained, non-frame control/error messages remain separately bounded, and closing or lagging preview must not block control RPC/events. Client-to-sidecar arbitrary command forwarding remains forbidden.

### 6. Compatibility is declared by the latest hosted UI and enforced before business traffic

The Pages build emits `/app/ui-manifest.json` with a versioned schema containing:

- immutable UI source identity and Pages deployment identity;
- `control_protocol` and `preview_protocol` versions;
- `minimum_cli`, represented by a complete expected build identity plus its monotonic minimum `build_sequence`;
- the supported release channel and a user-facing nightly upgrade URL/action.

The same compatibility constants are compiled into the WASM bundle so a stale or substituted manifest cannot weaken the handshake. During `client_hello`/`server_hello`, the UI and daemon compare protocol versions and the daemon's compiled `BuildIdentity`. No request/event subscription/preview attach occurs until compatibility succeeds. If the local nightly build sequence is below the minimum, or a required protocol does not overlap, the UI closes/refuses the business session and displays an explicit “upgrade nightly CLI” state. It does not fetch or fall back to historical UI assets.

Pages publishes only the current `/app/` entrypoint and content-addressed current assets. A new deployment replaces the prior Pages artifact; no compatibility archive or historical routing table is generated. The entrypoint/manifest are fetched with cache-busting behavior, while hashed WASM/JS assets may be immutable.

### 7. Nightly and Web UI workflows are independent

For `nightly.yml`'s `windows-installer` invocation, `release.yml` skips nightly Rust WASM target installation, `wasm-bindgen` installation/cache, GPUI WASM build, web-dist validation, `share/web` staging, generated installer components for Web assets, and web-dist upload. Bundle/update manifests must likewise stop requiring or inventorying `share/web` for this path.

The Hugo repository's workflow installs the pinned Rust nightly/target/`wasm-bindgen` toolchain required by `scripts/build-teshi-web.sh` (or a CI-safe equivalent), builds from the selected Teshi SHA, integrates the output at `/app/`, writes compatibility metadata, runs Web smoke/manifest checks, and deploys Hugo plus the UI as one Pages artifact.

The shared stable release path is not silently changed: conditional logic must prove that nightly omits Web UI while any retained stable packaging behavior remains as separately configured until a later explicit decision.

### 8. REST removal is a later, separately reviewed change

Phase one adds WebSocket adapters and migrates the hosted WASM client without deleting `/api/v1/*`. Coverage is demonstrated by an inventory mapping each hosted UI operation to a control RPC/event or preview message plus automated and browser-level acceptance tests. Only after that evidence exists may another OpenSpec change deprecate or remove REST routes.

This staged boundary provides rollback: redeploy the previous hosted UI and/or ship a corrective CLI while the old local REST surface remains intact. It avoids coupling transport replacement to domain behavior changes.

## Acceptance Criteria

- A production `teshi web` launch opens the exact hosted `/app/#port=...&token=...` form, uses an OS-selected loopback port, requires no installed `share/web`, and does not write the token to disk or logs.
- Network capture of the initial `https://teshi.org/app/` request contains neither port nor token; the page removes the fragment after parsing it.
- Both WS endpoints reject all non-allowlisted Origins, and both reject/close connections that send business data before a valid first-message handshake.
- A valid hosted session receives only the `HostedWebUi` capability set; a tokenless loopback hosted connection cannot invoke any business method and never becomes Admin.
- The full hosted UI completes its inventoried workflows over one control connection, while preview uses a distinct connection and a slow preview consumer does not delay a control round trip beyond the agreed test threshold.
- An older nightly CLI below `minimum_cli.build_sequence` receives no business request and the UI shows actionable nightly-upgrade guidance without loading an older UI.
- The nightly workflow contains no GPUI WASM toolchain/build step and its installer/archive manifests contain no complete Web UI or `share/web` payload.
- The Hugo Pages workflow builds and deploys Hugo plus current GPUI WASM assets and a validated compatibility manifest from a recorded immutable Teshi source SHA.
- Existing `/api/v1/*` routes and their current regression tests remain present and passing throughout phase one.

## Risks / Trade-offs

- [Browser security policy blocks `https` page to `ws://127.0.0.1`] → Validate the exact production scheme in supported Chromium browsers before broad migration; keep the architecture gated on a real-browser smoke test and surface a diagnostic failure rather than weakening bind/origin policy.
- [Token in the fragment is readable by all `/app/` JavaScript] → Ship no third-party script on `/app/`, use a restrictive CSP/referrer policy, erase the fragment immediately, keep the session in memory, and never log message bodies containing credentials.
- [Trusted origin is compromised] → Limit the session to a dedicated capability set, process lifetime, and local project; keep dangerous administrative operations outside that scope and retain server-side input/path validation.
- [Latest UI advances faster than installed CLI] → Fail before business connection with the manifest-defined minimum and a direct nightly upgrade action; do not mask mismatch with partial behavior.
- [Build sequence is not meaningful for local/dev builds] → Treat incomplete identities as development-only and require an explicit development compatibility override; never let identity `0` satisfy a production minimum.
- [Cross-repository deployment is not reproducible] → Resolve inputs to a full Teshi SHA, record it in the manifest, pin build tooling, and validate the generated Pages artifact before deploy.
- [Control multiplexing allows burst traffic to starve RPC] → Use bounded queues and separate priority/backpressure classes; keep preview entirely separate and test latency under Terminal/event load.
- [Temporary duplicate REST and WS adapters drift] → Keep a checked operation inventory and shared domain functions; do not duplicate business rules in transport code.
- [Removing bundled UI reduces offline startup] → Accept this as a consequence of the selected hosted-latest architecture and provide a clear network/startup diagnostic, not an undocumented local fallback.

## Migration Plan

1. Define shared protocol envelopes, compatibility manifest schema, operation inventory, hosted-UI role, and security tests without removing REST.
2. Add daemon session pre-minting plus `/ws/control` handshake/RPC/events, then migrate the WASM backend operation groups incrementally.
3. Add `/ws/preview`, reuse the existing bounded sidecar relay, and migrate browser/WinApp preview.
4. Add hosted launch behavior and prove fragment secrecy, exact Origin rejection, non-Admin authorization, reconnect, and incompatibility refusal in a real browser.
5. Update the Hugo repository to build the selected Teshi SHA, publish `/app/` and `ui-manifest.json`, and run Web smoke tests before Pages deployment.
6. Make nightly release packaging skip every Web UI build/artifact/payload path and verify installer contents and update manifests.
7. Run an end-to-end migration audit showing the hosted UI uses only the two new WebSockets for business traffic while all legacy REST endpoints still exist.

Rollback does not require a historical UI service: before removing REST, the Pages deployment can be reverted to the immediately previous known-good site commit and a corrective nightly CLI can be published. Compatibility refusal remains fail-closed during a mismatch.

## Open Questions

None at the architecture boundary. Exact RPC method payloads and the numeric latency/reconnect thresholds must be recorded in the protocol inventory/tests during implementation, without changing the fixed hosting, transport, trust, compatibility, or staged-REST decisions above.
