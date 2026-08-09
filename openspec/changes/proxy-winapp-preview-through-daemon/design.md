## Context

WinApp capture runs in a Python sidecar that deliberately listens on a random loopback port. The native desktop can connect to that URL, but returning it to a LAN browser is incorrect because loopback resolves on the browser's machine. The daemon already owns the sidecar lifecycle and serves the WASM application, so it is the natural transport boundary.

## Goals / Non-Goals

**Goals:**

- Relay the active WinApp preview through the daemon's HTTP origin.
- Keep the Python sidecar inaccessible from the LAN.
- Avoid applying backpressure from a slow browser directly to the capture producer.
- Let the WASM client work for localhost, LAN HTTP development, and HTTPS deployments without hard-coded hosts.

**Non-Goals:**

- Solving WebGPU's HTTPS secure-context requirement.
- Providing Internet-grade authentication or TURN/WebRTC transport.
- Changing native desktop capture transport.
- General-purpose forwarding of arbitrary sidecar commands.

## Decisions

1. The daemon exposes `/api/v1/browser/stream` as an Axum WebSocket endpoint. It resolves the active sidecar URL internally and connects with `tokio-tungstenite`; the URL is never required by the remote browser.
2. The daemon initiates `attach_window` for `TESHI_WINAPP_PROCESS` (default `TargetApp.exe`) after opening the upstream socket. The browser side is receive-only apart from WebSocket control/close frames, limiting command exposure.
3. The route enforces the existing same-origin check. This is a prototype boundary, not a replacement for future authenticated WebSocket session design.
4. Frame messages use a `watch` channel so unread frames are replaced by the newest frame. Non-frame protocol messages use a small bounded channel so errors and attach responses remain observable.
5. The WASM client constructs `ws://<page-host>/api/v1/browser/stream` or `wss://...` based on `window.location.protocol`; an explicit `winapp_ws` query override remains available for diagnostics.

## Risks / Trade-offs

- [Each browser creates one upstream sidecar connection] → Accept for the prototype; introduce daemon-level fan-out if concurrent viewers become common.
- [Same-origin does not authenticate every user on a LAN-hosted daemon] → Keep the route narrow and receive-only; address complete daemon authentication separately.
- [JPEG/base64 frames are bandwidth-heavy] → Latest-frame buffering prevents unbounded memory growth; a later production transport can use binary frames or WebRTC.
- [HTTP LAN pages may not initialize WebGPU] → Document HTTPS or the Chromium development-origin flag as a separate prerequisite.

## Migration Plan

1. Add the proxy route without removing the start/stop API.
2. Switch the WASM default endpoint to the same-origin route.
3. Rebuild and restart the daemon; older clients remain compatible with start/stop responses.
4. Roll back by reverting the WASM endpoint and daemon route; the sidecar protocol is unchanged.

## Open Questions

- Whether production deployments should multiplex all viewers through one upstream connection.
- Whether the preview endpoint should later be folded into session-token authentication or use a short-lived WebSocket ticket.
