## Context

In `teshi web` mode, the `ensureEventsSocket()` function in `desktop/src/platform/web.ts` is responsible for maintaining a single event WebSocket connection. The current implementation only checks `readyState === WebSocket.OPEN`. When `onEvent()` is called multiple times in a synchronous loop (e.g., 6 calls from `App.tsx`), each call creates a new WebSocket because the previous one is still in `CONNECTING` state.

Result: multiple WebSocket connections exist on the same page, each subscribing to the Rust event bus independently. Every `terminal-output` event is received and dispatched by all connections, causing `term.write()` to be called multiple times and characters to appear duplicated.

Additionally, the `onclose` handler uses `eventsSocket?.url === url` to determine whether the closing socket is the current one. When multiple connections coexist, an orphaned connection's close event can corrupt the active connection (since all share the same URL), nullifying the tracked socket and triggering unnecessary reconnections.

## Goals / Non-Goals

**Goals:**
- Ensure `ensureEventsSocket()` maintains at most one WebSocket connection per page
- Fix the `onclose` handler to accurately identify the current connection
- Eliminate the terminal output duplication bug

**Non-Goals:**
- Do not modify `App.tsx`'s `onEvent` call pattern (fire-and-forget is harmless after the fix)
- Do not modify desktop (Tauri) code — unaffected by this issue
- Do not modify Rust backend

## Decisions

### Decision 1: Relax readyState check in `ensureEventsSocket()`

**Before:** Only accepts `readyState === WebSocket.OPEN`.

**After:** Accept both `OPEN` and `CONNECTING`. Create a new connection only when `CLOSING` or `CLOSED`.

```typescript
// before
if (eventsSocket && eventsSocket.readyState === WebSocket.OPEN) {
    return eventsSocket;
}

// after
if (eventsSocket && (eventsSocket.readyState === WebSocket.OPEN || eventsSocket.readyState === WebSocket.CONNECTING)) {
    return eventsSocket;
}
```

**Rationale:** A WebSocket in `CONNECTING` state is establishing and will be available shortly. Reusing it avoids unnecessary duplicate connections.

### Decision 2: Fix `onclose` handler with reference comparison

**Before:** Uses `eventsSocket?.url === url` in the onclose closure.

**After:** Captures the WebSocket reference in the closure and compares by reference.

```typescript
// before
eventsSocket.onclose = () => {
    if (eventsSocket?.url === url) {
        eventsSocket = null;
    }
};

// after
const ws = new WebSocket(url);
ws.onclose = () => {
    if (eventsSocket === ws) {  // reference comparison
        eventsSocket = null;
    }
};
eventsSocket = ws;
```

**Rationale:** When multiple WebSockets exist, all share the same URL. String comparison cannot distinguish between different socket instances, but reference comparison can.

## Risks / Trade-offs

- **Risk**: WebSocket disconnects after fix cannot reconnect → **Mitigation**: when `onclose` or `onerror` fires (and it is the current connection), `eventsSocket` is set to null; the next `ensureEventsSocket()` call will create a new connection
- **Risk**: A `CLOSING` socket is reused → **Mitigation**: only accept `OPEN` and `CONNECTING`; `CLOSING` is treated as unusable. In practice `CLOSING` is transient and the socket will be unavailable shortly
