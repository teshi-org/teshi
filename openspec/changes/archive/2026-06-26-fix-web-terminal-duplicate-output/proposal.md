## Why

In `teshi web` (browser) mode, every terminal input and output character appears 6 times. The root cause is that `ensureEventsSocket()` does not reuse a WebSocket in `CONNECTING` state, creating multiple connections on the same page. Each connection subscribes to the server-side event bus independently, causing every `terminal-output` event to be dispatched and written to xterm multiple times.

## What Changes

1. **Fix `ensureEventsSocket()` in `web.ts`**: Accept `CONNECTING` as a reusable readyState, preventing duplicate WebSocket creation when `onEvent()` is called in quick succession
2. **Fix `onclose` handler in `web.ts`**: Use WebSocket object reference comparison instead of URL string comparison, preventing orphaned sockets from corrupting the active connection on close

## Capabilities

### New Capabilities

- `web-socket-connection`: WebSocket connection lifecycle management — ensure a single event connection per page, preventing duplicate connections caused by race conditions

## Impact

- `desktop/src/platform/web.ts` — modify `ensureEventsSocket()` function and the `onclose` closure's connection tracking
- Only affects `teshi web` (browser) mode; Tauri desktop mode is unaffected
