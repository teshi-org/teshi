## Why

The WinApp preview currently captures the target's screen rectangle with Pillow, so occluding windows appear in the stream and the target must remain visible. Windows Graphics Capture can capture the DWM-composited window surface by HWND while preserving the existing JPEG preview contract.

## What Changes

- Prefer Windows Graphics Capture for attached WinApp windows on supported Windows x64 hosts.
- Preserve the 8 FPS JPEG/Base64 preview protocol and bounded latest-frame delivery.
- Automatically fall back to Pillow ImageGrab when WGC is unavailable or fails, and report the active backend and fallback reason.
- Surface the active capture backend in native and WASM GPUI preview status.
- Keep UI Automation inspection and action behavior unchanged.

## Capabilities

### New Capabilities
- `winapp-capture-backend`: HWND-scoped WGC capture lifecycle, JPEG production, and ImageGrab fallback behavior.

### Modified Capabilities
- `gpui-winapp-preview`: Preview frames and target metadata expose the active capture backend, and the UI distinguishes WGC from the visibility-sensitive fallback.

## Impact

- `resources/winapp_service.py` gains a threaded WGC capture backend and fallback orchestration.
- Windows Python environments gain the pinned `windows-capture` package and its NumPy/OpenCV dependencies.
- Native/WASM adapters and the shared GPUI preview consume optional non-breaking frame metadata.
- WinApp documentation and tests are updated; daemon transport remains opaque and protocol-compatible.
