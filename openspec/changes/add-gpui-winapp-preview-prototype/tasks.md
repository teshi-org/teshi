## 1. Shared GPUI Preview

- [x] 1.1 Add a shared `WinAppPreview` entity with connection/error states and latest-JPEG replacement
- [x] 1.2 Render the preview in `AppShell` with contained aspect ratio and visible status text
- [x] 1.3 Add focused unit tests for state transitions and last-frame retention

## 2. Native Desktop Adapter

- [x] 2.1 Start WinApp mode for the current project and request attachment to the configured target process
- [x] 2.2 Stream sidecar messages on a worker thread through a bounded latest-event slot into GPUI

## 3. WASM Adapter

- [x] 3.1 Start WinApp mode through the daemon API and connect using the browser WebSocket API
- [x] 3.2 Attach to the target application and forward browser WebSocket frames and errors into the shared preview entity

## 4. Verification

- [x] 4.1 Format the workspace and run native checks/tests for affected crates
- [x] 4.2 Build or check the wasm32 GPUI shell and document any environment-only blockers
