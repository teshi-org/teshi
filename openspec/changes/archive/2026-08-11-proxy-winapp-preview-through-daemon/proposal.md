## Why

The GPUI web client currently receives the sidecar's loopback WebSocket URL, so a browser on another LAN machine tries to connect to its own `127.0.0.1` and never receives the WinUI preview. The daemon must relay that stream through its own origin while keeping the capture sidecar private to the Teshi host.

## What Changes

- Add a daemon WebSocket endpoint that connects to the active WinApp sidecar and relays preview protocol messages.
- Restrict the endpoint to same-origin browser requests and keep the sidecar bound to loopback.
- Change the GPUI WASM client to derive the preview WebSocket URL from the page origin instead of consuming the sidecar's private URL.
- Preserve bounded/latest-frame behavior so a slow remote client does not stall capture indefinitely.
- Document LAN and secure-context constraints for the preview prototype.

## Capabilities

### New Capabilities

- `winapp-preview-proxy`: Same-origin daemon transport for remotely viewing the loopback WinApp capture stream.

### Modified Capabilities

None.

## Impact

- Affects `apps/teshi-daemon`, `apps/teshi-web`, and WinApp mode documentation.
- Adds a daemon dependency on the Tokio WebSocket client implementation.
- Adds `/api/v1/browser/stream` as a same-origin WebSocket API; the existing sidecar start/stop APIs remain unchanged.
