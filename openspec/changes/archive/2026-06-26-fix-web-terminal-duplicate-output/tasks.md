## 1. Fix `ensureEventsSocket` Connection Reuse Check

- [x] 1.1 Modify `ensureEventsSocket()` readyState check to accept CONNECTING state
- [x] 1.2 Fix `onclose` handler to use WebSocket object reference comparison instead of URL string comparison
- [x] 1.3 Verify: only one WebSocket is created under consecutive `onEvent()` calls

## 2. Verification & Testing

- [x] 2.1 Start `teshi web` and verify terminal input characters no longer duplicate
- [x] 2.2 Verify `open-project-cli`, `recent-loaded`, `project-changed` and other events still work
- [x] 2.3 Verify WebSocket auto-reconnects on disconnect (idle timeout or daemon restart)
- [x] 2.4 Verify desktop (`teshi desktop`) terminal is unaffected
