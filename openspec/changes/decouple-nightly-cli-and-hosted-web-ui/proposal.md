## Why

`teshi web` and the nightly installer currently treat the GPUI WASM distribution as a local daemon asset, coupling ordinary CLI releases to a nightly-only WASM build and to same-origin HTTP APIs. The product boundary is now fixed as a latest-only UI hosted at `https://teshi.org/app/` that connects directly to a loopback daemon, so its delivery, authentication, transport, and compatibility contracts must be specified before implementation.

## What Changes

- Make the Hugo Pages deployment for `teshi.org` build and publish the complete GPUI WASM application at `/app/`; only the latest deployed UI is retained.
- Change `teshi web` to start a daemon on `127.0.0.1` using an available dynamic port, mint a random launch session token, and open `https://teshi.org/app/#port=<port>&token=<session-token>`.
- Keep port and token in the URL fragment so the initial request to `teshi.org` does not disclose them to the hosting service.
- Add `/ws/control` as the authenticated RPC/event channel for Terminal, Agent, Project, BDD, and other UI control traffic.
- Add `/ws/preview` as a separately backpressured channel for browser and WinApp preview traffic so large frames cannot block control messages.
- Require an exact trusted `Origin` on both WebSocket upgrades and require the first client message to complete session-token and protocol negotiation before any business operation is accepted.
- Introduce a dedicated hosted-Web-UI authorization scope. A loopback TCP peer alone never grants Admin access to the hosted client.
- Publish a UI manifest containing the UI identity, control/preview protocol versions, and minimum compatible CLI build identity/build sequence; an incompatible latest UI refuses business connection and tells the user to upgrade the nightly CLI.
- Stop nightly release jobs from building, uploading, or packaging the GPUI WASM distribution. The local nightly payload remains CLI/daemon/runtime and related native/runtime support, while the Hugo repository owns Web UI builds.
- Preserve all existing `/api/v1/*` REST routes during this phase. Migrate and verify the UI over WebSocket first; REST removal is a separate future change.

## Capabilities

### New Capabilities

- `hosted-web-ui-delivery`: Build and deploy the complete latest GPUI WASM UI through the `teshi-org.github.io` Hugo Pages workflow while excluding it from nightly CLI artifacts.
- `hosted-web-session-bootstrap`: Define loopback dynamic-port startup, fragment-based launch data, trusted-Origin enforcement, one-time launch session authentication, and non-Admin hosted-UI authorization.
- `web-control-protocol`: Define the negotiated `/ws/control` RPC/event transport and its separation from the `/ws/preview` high-volume stream.
- `hosted-ui-compatibility`: Define the latest-only UI manifest, CLI build identity negotiation, incompatible-CLI refusal, and upgrade guidance.

### Modified Capabilities

- `gpui-wasm-web-shell`: Move the official GPUI WASM shell from daemon-served same-origin assets to the hosted `/app/` application and route its daemon operations through negotiated WebSockets.
- `web-socket-connection`: Replace the legacy event-only connection contract with one unique authenticated control connection and explicit reconnect/reauthentication behavior.
- `winapp-preview-proxy`: Move the browser-facing preview endpoint from `/api/v1/browser/stream` to the authenticated cross-origin `/ws/preview` channel while retaining bounded latest-frame delivery and private loopback sidecars.

## Impact

- Teshi repository: `apps/teshi-cli`, `apps/teshi-daemon`, `apps/teshi-web`, shared UI backend adapters, build scripts, daemon/session tests, and nightly/release packaging logic.
- Website repository: `teshi-org/teshi-org.github.io` Hugo layout/static assets and Pages workflow.
- Public local interface: two new WebSocket endpoints and a versioned handshake; existing REST APIs remain available in phase one.
- Security boundary: `https://teshi.org` becomes an explicitly allowlisted browser Origin, but receives only the permissions of the minted hosted-UI session and never implicit loopback Admin.
- Delivery boundary: nightly CLI publication and latest Web UI deployment become independent pipelines, with compatibility enforced at connection time.
