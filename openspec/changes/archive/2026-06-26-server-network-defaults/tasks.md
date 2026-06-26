## 1. Add `--host` to CLI options and thread through daemon spawn chain

- [x] 1.1 Add `--host` field to `WebOptions` in `crates/teshi-daemon/src/lib.rs` with `default_value = "0.0.0.0"`
- [x] 1.2 Add `--host` field to `DaemonInternalOptions` in `crates/teshi-daemon/src/lib.rs` with `default_value = "0.0.0.0"`
- [x] 1.3 Add `host` parameter to `ensure_daemon()` and pass it to `spawn_daemon_background()`
- [x] 1.4 Pass `--host` in `spawn_daemon_background()` CLI args in `crates/teshi-runtime/src/daemon.rs`
- [x] 1.5 Change bind address construction in `run_daemon_internal()` (`lib.rs:182`) from `SocketAddr::from(([127, 0, 0, 1], port))` to parse from `format!("{}:{}", opts.host, opts.port)`

## 2. Change default port from auto-pick to `20253`

- [x] 2.1 In `ensure_daemon()` (`lib.rs:223-227`), change fallback from `pick_free_port()?` to `20253` when no `--port` is specified
- [x] 2.2 Verify `--port` override still works when explicitly provided

## 3. Update downstream port references

- [x] 3.1 Update `desktop/vite.config.ts` proxy target from `1421` to `20253`
- [x] 3.2 Update `scripts/bootstrap_dev.py` default `api_port` from `1421` to `20253`
- [x] 3.3 Update `tests/regression_terminal_dup.py` hardcoded port from `1421` to `20253`
- [x] 3.4 Update `test_ws.py` hardcoded port from `1421` to `20253`

## 4. Verify and test

- [x] 4.1 Run `cargo check` to ensure compilation passes
- [x] 4.2 Run `cargo test` to ensure existing tests pass
- [x] 4.3 Verify `teshi web` starts on `0.0.0.0:20253` by default
- [x] 4.4 Verify `teshi web --host 127.0.0.1` binds to loopback only
- [x] 4.5 Verify `teshi web --port 3000` uses custom port
