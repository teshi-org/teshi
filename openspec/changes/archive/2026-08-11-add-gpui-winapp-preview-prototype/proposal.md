## Why

Teshi already emits live JPEG frames for attached Windows applications, but the shared GPUI shell cannot display them. A small cross-target prototype is needed to validate that the same preview experience works in both the native desktop shell and the GPUI WASM shell before investing in a production-grade Windows capture pipeline.

## What Changes

- Add a shared GPUI preview surface that displays the latest JPEG frame while preserving its aspect ratio.
- Add platform adapters that subscribe to the existing WinApp WebSocket stream from native desktop and browser WASM environments.
- Provide connection, waiting, streaming, and error states without buffering stale frames.
- Expose a configurable preview endpoint so the prototype can attach to a running WinApp sidecar.
- Keep the existing 8 FPS JPEG/Base64 protocol and visible-window `ImageGrab` capture limitations for this prototype.

## Capabilities

### New Capabilities

- `gpui-winapp-preview`: Live WinApp JPEG preview behavior shared by native and WASM GPUI shells, including endpoint configuration and connection states.

### Modified Capabilities

- `gpui-shell`: The shared GPUI root shell gains a WinApp preview surface backed by platform-provided frame streaming.

## Impact

- Affects `crates/teshi-ui`, `apps/teshi-desktop`, and `apps/teshi-web`.
- Reuses `resources/winapp_service.py` and its existing WebSocket frame schema without changing the sidecar protocol.
- Adds target-specific WebSocket plumbing and in-memory JPEG decoding; no changes to the target application are required.
- The WASM path requires browser access to the loopback WinApp sidecar and therefore remains subject to browser mixed-content and local-network policy.
