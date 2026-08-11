## 1. Daemon Proxy

- [x] 1.1 Add the daemon WebSocket client dependency and same-origin preview route
- [x] 1.2 Implement receive-only sidecar relay with latest-frame buffering and bounded control messages
- [x] 1.3 Add focused tests for message classification and route security behavior

## 2. GPUI Web Client

- [x] 2.1 Derive the default preview WebSocket URL from the page origin
- [x] 2.2 Remove the browser-to-sidecar attach command while retaining the diagnostic URL override

## 3. Documentation and Verification

- [x] 3.1 Document the same-origin transport and the separate WebGPU secure-context requirement
- [x] 3.2 Format and run targeted Rust checks/tests, including the wasm target when available
- [x] 3.3 Restart the LAN daemon and verify an end-to-end preview message through the proxy
