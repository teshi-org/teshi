## Context

`resources/winapp_service.py` already captures an attached HWND with Pillow at 8 FPS and broadcasts JSON messages containing Base64 JPEG data. The React browser panel consumes this protocol, but the shared `teshi-ui` GPUI shell currently renders only a placeholder and settings. Native GPUI can use a blocking Tungstenite client on a worker thread; GPUI WASM must use the browser `WebSocket` API and receive callbacks on the browser main thread.

The shared UI crate must remain independent of `teshi-engine` and target-specific networking. The prototype also needs a practical way to acquire the random sidecar endpoint: native desktop can start the sidecar against the current project, while the daemon-hosted WASM shell can call the existing `/api/v1/browser/start` API.

## Goals / Non-Goals

**Goals:**

- Render the latest WinApp JPEG frame in the shared GPUI main surface on native Windows and wasm32.
- Start/reuse the existing WinApp sidecar and automatically request attachment to a configurable target process.
- Keep connection state and errors visible in the shared UI.
- Bound memory by replacing the latest frame instead of queuing frames.
- Keep target-specific side effects in the two application entry crates.

**Non-Goals:**

- Reliable capture of minimized or occluded windows.
- Windows Graphics Capture, shared GPU textures, audio, remote streaming, or production frame-rate tuning.
- Mouse/keyboard forwarding through the preview.
- A general window picker; this prototype targets a configured application by process name.

## Decisions

### Shared view owns presentation state only

Add a `WinAppPreview` entity to `teshi-ui` with public methods for connecting, waiting, frame, and error transitions. `AppShell` owns and renders that entity on its main surface. Platform entry crates create the entity and deliver events, preserving the existing module boundary.

Alternative: define a cross-platform networking trait in `teshi-ui`. This was rejected for the prototype because native worker threads and browser callback lifetimes require substantially different subscription ownership models.

### Reuse the current JSON/Base64 JPEG protocol

Both adapters parse `frame`, `frame_error`, and `response` messages. A frame becomes `gpui::Image::from_bytes(ImageFormat::Jpeg, bytes)` and replaces the prior image. GPUI performs normal image decoding/rendering and `ObjectFit::Contain` preserves the captured aspect ratio.

Alternative: add a binary protocol or raw BGRA frames. This would improve efficiency but changes the sidecar and is unnecessary for an 8 FPS feasibility prototype.

### Automatically start and attach for the prototype

Native desktop constructs a `TeshiEngine`, opens the current directory, starts `BrowserMode::WinApp`, and sends `attach_window` for `TESHI_WINAPP_PROCESS` (default `TargetApp.exe`). The worker then owns a persistent Tungstenite connection.

The daemon-hosted WASM shell synchronously starts WinApp mode through the existing same-origin API, opens a browser WebSocket to the returned URL, and sends the same attach command. This matches the shell's existing synchronous XHR bootstrap style.

Alternative: require users to manually supply a WebSocket URL. This is retained as a possible debugging fallback but is not the primary prototype experience because the random port makes it cumbersome.

### Latest-frame delivery is bounded

The native worker writes events into a single shared latest-value slot; a GPUI task polls and drains that slot. Browser callbacks update the entity directly through the retained single-threaded GPUI `AppCell`. Neither target creates an unbounded frame queue.

## Risks / Trade-offs

- [Screen-rectangle capture records occluders and fails when minimized] → Label the preview as a prototype and require the target application to remain visible; replace with Windows Graphics Capture later.
- [An HTTPS page may block `ws://127.0.0.1` mixed content or local-network access] → Prefer daemon hosting over loopback HTTP during development and surface the browser WebSocket error verbatim.
- [Synchronous WASM startup XHR can briefly block UI] → Keep it consistent with the existing WASM configuration backend for the prototype; migrate to async bootstrap later.
- [Automatic process matching can select an unintended window] → Use a neutral default target and allow `TESHI_WINAPP_PROCESS` override; a window picker is future work.
- [Per-frame JPEG image identities may churn GPU/image caches] → Store only the latest `Arc<Image>` in the view and verify behavior during the prototype; introduce an explicit bounded cache or decoded texture path if profiling shows retention.

## Migration Plan

No persisted data migration is required. The feature is additive. Rollback consists of removing the preview entity and platform adapters; the existing sidecar and React client remain protocol-compatible.

## Open Questions

- Whether browser security policy permits loopback WebSocket access in every intended deployment environment.
- Whether the pinned GPUI WASM renderer releases replaced JPEG resources promptly enough for long sessions.
- Whether a follow-up should prioritize a window picker or Windows Graphics Capture.
