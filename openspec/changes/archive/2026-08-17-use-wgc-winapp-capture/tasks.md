## 1. Capture Backend

- [x] 1.1 Add the conditional pinned Windows capture dependency and optional import handling
- [x] 1.2 Implement WGC latest-frame capture, JPEG encoding, lifecycle cleanup, and ImageGrab fallback
- [x] 1.3 Route preview broadcasting and screenshot commands through the active backend with compatible metadata

## 2. Preview Clients and Documentation

- [x] 2.1 Surface WGC and fallback state in native and WASM GPUI preview adapters and the shared view
- [x] 2.2 Update WinApp documentation for WGC behavior, requirements, and fallback limitations

## 3. Verification

- [x] 3.1 Add Python tests for WGC selection, latest-frame replacement, cleanup, and fallback behavior
- [x] 3.2 Add Rust tests for backend metadata handling and preview status
- [x] 3.3 Run OpenSpec validation and repository quality gates
