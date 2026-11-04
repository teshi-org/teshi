# WinApp handshake stall: stderr backpressure

## Reproduction and cause

Session `01a16cba-c6d9-7ad1-9579-196d8cf462da` reported repeated WinApp
snapshot and health handshake timeouts while validating Enhook. The CLI was
`0.7.10-local.20261024.0941+0800 (78a683e)`, with a locally staged v5 runtime.
Its retained service scripts match the checkout. The retained environment uses
Python 3.13.11 and websockets 17.3.

An isolated service reproduced the failure without attaching a native window:
two CLI health calls succeeded and the third timed out during the handshake.
Draining the service's stderr immediately restored health; over 7 KB of
connection-handler diagnostics had accumulated.

The one-shot Rust client dropped TCP after receiving its terminal response
without sending a WebSocket Close frame. The service reported this abnormal
disconnect to stderr. The daemon piped stderr but only read it after a startup
failure. Once the finite OS pipe filled, synchronous logging blocked the Python
event loop, including new handshakes. Executor process isolation cannot prevent
this supervisor-side blockage.

## Fix

- Send a Close frame after receiving a terminal response. Failure to send this
  cleanup frame does not discard an already received operation result.
- Drain sidecar stderr from immediately after spawn until EOF. Retain only a
  16 KiB diagnostic tail for startup failures, with a bounded wait for EOF.
  Logging volume cannot consume unbounded memory or fill an unread pipe.
- Keep uncertain mutation handling unchanged; recovery never requires replaying
  a business command.

This fix changes the Rust client and launcher. The Python service and runtime
protocol remain v5.

## Validation

`resources/tests/test_winapp_transport_lifecycle.py` exercises the real CLI,
daemon and service in temporary projects, without native input:

1. Twenty consecutive one-shot health connections leave no connection-handler
   errors, even when the test intentionally leaves service stderr unread.
2. A managed service writes over 1 MiB to stderr before readiness and still
   starts successfully.
3. A managed service writes over 1 MiB to stderr on every request and remains
   responsive after readiness.

All three tests fail with the original installed CLI and pass with the fix.
The original executable is preserved with the local evidence. Additional
validation: 267 engine unit tests, 139 WinApp Python tests, and engine Clippy
with warnings denied passed. The Rust tests independently verify Close-frame
delivery and bounded diagnostic retention.

The fix is installed locally as
`0.7.10-local.20261024.1541+0800 (78a683e)`. Its SHA-256 is
`454b375a3c34e0709393123acb913ec58de880dc732bf723b31c8ece5d82b4b5`.
The three transport regressions also passed against that installed executable.
The commit identifier remains unchanged because the repair is uncommitted;
`identity.json` records the binaries and source hashes used for validation.

Local evidence is under `.teshi/artifacts/handshake-fix-20261024/`. It includes
the failing baseline, successful regression logs, Clippy and installation logs.
An initial custom-output build and the first Python run encountered Windows
file-sharing violations; those attempts are not passing evidence. A normal
build and subsequent full regression run succeeded.

These checks establish the reproduced transport fix. They do not replace the
remaining Enhook backup/restore UI acceptance scenarios.
