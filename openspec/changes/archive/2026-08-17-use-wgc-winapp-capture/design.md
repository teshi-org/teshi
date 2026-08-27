## Context

WinApp preview currently calls Pillow `ImageGrab` for the attached HWND's screen rectangle every 125 ms. Desktop and WASM clients already consume JPEG frames through a latest-frame WebSocket pipeline, while UI Automation independently supplies semantic element data. The capture backend can therefore change without replacing the transport or UIA planes.

## Goals / Non-Goals

**Goals:**
- Capture the composited target window by exact HWND even while it is occluded.
- Preserve the existing JPEG/Base64 frame contract and bounded 8 FPS delivery.
- Keep preview available through ImageGrab when WGC cannot start or stops unexpectedly.
- Make the active backend and fallback reason visible to clients.

**Non-Goals:**
- GPU texture sharing, H.264 transport, higher frame rates, audio, OCR, or visual-model recognition.
- Replacing UI Automation selectors/actions.
- Combining independent popup HWNDs into the main-window capture.

## Decisions

- Use the Python `windows-capture` 2.0.1 package. Its `window_hwnd` and free-threaded callback API fit the existing Python sidecar and avoid introducing a separately packaged helper executable. NumPy/OpenCV transitive dependencies are accepted.
- Add a capture-backend abstraction inside the sidecar. WGC owns a background capture control, an immutable latest-JPEG slot, a first-frame event, and terminal state protected by a thread lock; ImageGrab remains a synchronous fallback.
- Configure WGC with the exact attached HWND, a 125 ms minimum update interval, and cursor, capture border, and secondary-window capture disabled. Encode callback BGRA data to quality-70 JPEG immediately so no borrowed native frame survives the callback.
- Preserve broadcast timing. The asyncio loop reads the latest JPEG every 125 ms and may resend it when WGC emits no update for a static window; superseded callback frames are never queued.
- Fall back on import/initialization failure, a two-second first-frame timeout, or unexpected capture-thread termination while the HWND is still valid. A closed/invalid HWND produces `frame_error` instead of capturing unrelated screen pixels.
- Add optional `capture_backend` and `capture_fallback_reason` fields to frame and target metadata. Existing clients remain compatible; updated clients use the fields only for status text.

## Risks / Trade-offs

- [The selected Python package adds NumPy/OpenCV and an x64 native wheel] → Pin the version, install it only on Windows AMD64, and retain ImageGrab when import is unavailable.
- [WGC callbacks run outside asyncio] → Share only immutable JPEG bytes and small status values behind a lock; never call WebSocket APIs from the callback.
- [JPEG encoding can delay the capture callback] → Limit eligible WGC updates to 8 FPS and preserve latest-frame replacement rather than queueing work.
- [Capture sessions can outlive reattachment] → Stop and wait for the old capture control before replacing session state or shutting down.
- [WGC support varies by Windows/session/GPU] → Treat it as preferred, not required, and expose the exact fallback reason.

## Migration Plan

Install the conditional dependency, deploy the updated sidecar and clients together, and verify WGC on Windows 10 1903+ x64. Rollback consists of removing the dependency and reverting the sidecar; the wire protocol remains backward compatible throughout.

## Open Questions

None.
