## Context

The `teshi web` daemon server (Axum HTTP server) is spawned as a background process via PowerShell `Start-Process`. The bind address is hardcoded to `[127, 0, 0, 1]` in `lib.rs:182` and the port defaults to auto-pick from a free system port. The `--port` CLI flag is the only way to set the port; there is no `--host` flag.

The daemon's client-side code (`run_client`) spawns the background process via `ensure_daemon` → `spawn_daemon_background`, passing `--port` as a CLI argument. The background process receives these as `DaemonInternalOptions`.

Downstream consumers (Vite proxy, bootstrap script, test files) hardcode port `1421` — a historic convention that is now disconnected from the daemon's actual auto-pick behavior.

## Goals / Non-Goals

**Goals:**
- Change default bind address from `127.0.0.1` to `0.0.0.0`
- Add `--host` CLI argument to both `WebOptions` (user-facing) and `DaemonInternalOptions` (internal)
- Change default port from auto-pick to `20253`
- Thread `--host` through the full daemon spawn chain so the background process binds correctly
- Update all downstream hardcoded port references (`1421` → `20253`)

**Non-Goals:**
- Do not change CORS configuration (already open with `.allow_origin(Any)`)
- Do not modify Web UI client code (uses relative URLs + `window.location.host`)
- Do not modify Chrome bridge or its bind address (stays `127.0.0.1:17373`)
- Do not change CSP or extension manifest (already allow the relevant patterns)

## Decisions

### 1. Default bind address: `0.0.0.0` with `--host` override

- `--host` added to `WebOptions` with `default_value = "0.0.0.0"`
- `--host` added to `DaemonInternalOptions` with `default_value = "0.0.0.0"`
- `lib.rs:182` changed from `SocketAddr::from(([127, 0, 0, 1], opts.port))` to `format!("{}:{}", opts.host, opts.port).parse()`
- Host string is parsed as `SocketAddr` via standard library — supports both IP addresses (`0.0.0.0`, `127.0.0.1`) and hostnames

### 2. Default port: `20253` with `--port` override

- Port selection in `ensure_daemon` (`lib.rs:223-227`) changed from `pick_free_port()?` to `20253` when no `--port` is specified
- `--port` still overrides the default, preserving the existing behavior exactly

### 3. Daemon spawn chain: thread `--host` through

Current chain:
```
WebOptions → run_client → ensure_daemon(spawn_daemon_background) → background process → DaemonInternalOptions → run_daemon_internal → bind
```

Changes:
- `ensure_daemon` accepts `host: &str` parameter
- `spawn_daemon_background` adds `"--host", host.to_string()` to the CLI args
- `DaemonInternalOptions` gains `--host` field
- The browser-open URL in `run_client` remains `127.0.0.1` (it opens the browser on the local machine)

### 4. Downstream port references

- `desktop/vite.config.ts`: proxy target `http://127.0.0.1:1421` → `http://127.0.0.1:20253`
- `scripts/bootstrap_dev.py`: `api_port=1421` → `api_port=20253`
- `tests/regression_terminal_dup.py`: `http://127.0.0.1:1421` → `http://127.0.0.1:20253`
- `test_ws.py`: `http://127.0.0.1:1421` → `http://127.0.0.1:20253`

## Risks / Trade-offs

- [Port conflict] → `20253` is an unregistered ephemeral-range port. If already in use, users can override with `--port`. Low risk.
- [Breaking existing daemon.json] → Previously recorded port in `~/.teshi/daemon.json` will mismatch. The daemon manifest is rewritten on every spawn, so this self-heals. No migration needed.
- [Security: 0.0.0.0 exposes to LAN] → User explicitly requested this. Tailscale network boundary provides isolation. If a user wants loopback-only, they can `teshi web --host 127.0.0.1`.
- [CLI URL still uses 127.0.0.1] → The browser-open URL and health-check URLs in CLI code remain `127.0.0.1`. This is correct — they run on the local machine and connect to the daemon via loopback regardless of the bind address.
