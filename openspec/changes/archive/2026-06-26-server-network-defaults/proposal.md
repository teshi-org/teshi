## Why

The `teshi web` daemon server currently hardcodes `127.0.0.1` as the bind address and uses a random free port by default. This prevents accessing the web UI from other devices on the local network (e.g., via Tailscale) and makes the port unpredictable across restarts.

## What Changes

- Change default bind address from `127.0.0.1` to `0.0.0.0` (listen on all interfaces)
- Add `--host` CLI argument to `teshi web` to allow overriding the bind address
- Change default port from auto-pick to `20253`
- Keep existing `--port` CLI argument for overriding
- Update Vite dev proxy target from `1421` to `20253`
- Update `bootstrap_dev.py` default `api_port` from `1421` to `20253`
- Update test files that hardcode `1421` to `20253`
- The `--daemon-internal` subcommand also receives the host parameter so the background process binds to the correct address

## Capabilities

### New Capabilities

None — this is a configuration change, not a new capability.

### Modified Capabilities

None — no spec-level behavior changes.

## Impact

- `crates/teshi-daemon/src/lib.rs` — Add `--host` to `WebOptions` and `DaemonInternalOptions`; change default port to `20253`; thread host through `ensure_daemon` and `spawn_daemon_background`
- `crates/teshi-daemon/src/daemon.rs` — Pass `--host` to background process CLI
- `desktop/vite.config.ts` — Update proxy target port from `1421` to `20253`
- `scripts/bootstrap_dev.py` — Update default `api_port` from `1421` to `20253`
- `tests/regression_terminal_dup.py` — Update hardcoded port from `1421` to `20253`
- `test_ws.py` — Update hardcoded port from `1421` to `20253`
- CSP / extension manifest permissions — Unchanged (already allow `0.0.0.0` via wildcard patterns)
