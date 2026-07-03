## Context

teshi already has full PTY lifecycle management in `crates/teshi-runtime/src/terminal.rs` (based on `portable-pty`), exposed to desktop and web frontends via `TeshiRuntime`. The current terminal is GUI-only: xterm.js renders raw ANSI output, `TerminalState` handles PTY forwarding and rate limiting.

The existing `teshi browser` and `teshi winapp` use the same architectural pattern: an **independent sidecar process** (Python) exposes a WebSocket service; the CLI connects directly to the sidecar to send JSON commands and receive JSON responses. Port discovery is done via `.teshi/cdp-endpoint.json`.

Terminal automation should follow the same pattern:

- CLI ↔ sidecar via **WebSocket**, reusing `send_sidecar_command_with_timeout`
- Port discovery via **cdp-endpoint.json**, `mode: "terminal"`
- Sidecar is a **Rust binary** (not Python), leveraging `vte` crate for ANSI parsing

## Goals / Non-Goals

**Goals:**
- New `ScreenGrid` module using `vte` crate to parse raw PTY output into structured row/col grids
- Process state detection: Running / Idle / WaitingForInput / Exited
- New standalone Rust sidecar binary `crates/teshi-terminal-sidecar/`, embedding `portable-pty` + `vte` + WebSocket server
- New `teshi terminal serve-embedded` subcommand to start the sidecar (analogous to `teshi browser serve-embedded`)
- New `teshi terminal {snapshot|status|exec|send|resize|kill}` CLI subcommands, communicating with sidecar via WebSocket, outputting JSON to stdout
- Reuse `send_sidecar_command_with_timeout` as the WebSocket transport layer
- Reuse `read_cdp_endpoint` for `.teshi/cdp-endpoint.json` discovery
- Sidecar writes `.teshi/cdp-endpoint.json` (`mode: "terminal"`)

**Non-Goals:**
- Do not modify the existing embedded terminal panel (xterm.js / Tauri) — existing panel continues with the old base64 event path
- Do not introduce MCP protocol — CLI outputs JSON for other agents to parse, not MCP JSON-RPC
- No multi-terminal sessions (one shell at a time)
- No frontend rendering (no canvas terminal renderer)
- No daemon dependency — sidecar runs independently

## Decisions

### D1: VTE integration point → Insert VTE parsing into the forwarder thread

Current `terminal.rs` dual-thread model:
```
Reader Thread → raw bytes → mpsc Channel → Forwarder Thread → BASE64 → events.emit
```

VTE parsing is inserted into the Forwarder Thread: after receiving raw bytes, feed them to `vte::Parser`; `GridPerformer` (implementing `vte::Perform` trait) updates the `ScreenGrid` state. The forwarder still emits `"terminal-output"` (keeping the existing xterm.js panel uninterrupted), while `ScreenGrid` provides snapshot/status query interfaces.

```
Forwarder Thread:
  raw bytes → vte::Parser → GridPerformer → ScreenGrid
     │                                         │
     │  (existing path)                        │ (new query interface)
     ▼                                         ▼
  BASE64 → events.emit("terminal-output")    snapshot() / status()
```

**Alternative considered**: Adding a third thread for VTE parsing. Rejected because it adds thread synchronization complexity, and the forwarder already consumes the byte stream — adding parsing inline is the simplest path.

### D2: CLI ↔ Sidecar protocol → WebSocket (consistent with browser/winapp)

CLI connects to the sidecar WebSocket via `send_sidecar_command_with_timeout`, sends JSON command frames, receives JSON response frames. Fully reuses the existing implementation:

```
CLI                          Sidecar (Rust)
 │                              │
 │── WS: { "cmd": "snapshot",   │
 │         "request_id": "x" }  │
 │                              │──→ ScreenGrid::snapshot()
 │                              │
 │← WS: { "type": "response",  │
 │         "request_id": "x",   │
 │         "ok": true,          │
 │         "rows": 24, ... }    │
 │                              │
```

**Command list**:

| cmd | request_id | Parameters | Description |
|-----|------------|------|------|
| `snapshot` | `terminal-snapshot` | `full: bool` | Read screen grid |
| `status` | `terminal-status` | none | Query process state |
| `exec` | `terminal-exec` | `command`, `timeout_ms` | Write command and wait for completion |
| `send` | `terminal-send` | `data`, `newline` | Write text |
| `resize` | `terminal-resize` | `cols`, `rows` | Resize |
| `kill` | `terminal-kill` | none | Kill shell, reset session |

Existing sidecar WebSocket protocol (`send_sidecar_command_with_timeout` contract):
- Request frame: JSON object with `cmd` and `request_id` fields
- Response frame: JSON object with `type: "response"` and `request_id` fields
- Timeout: handled by `send_sidecar_command_with_timeout`

### D3: Process state detection → timeout + prompt matching

| State | Detection logic |
|------|----------|
| `Running` | PTY output received within the last 500ms |
| `Idle` | No output for 500ms+, last line is not a prompt |
| `WaitingForInput` | No output for 500ms+, last line matches prompt pattern (`$ `, `> `, `# `, `❯ `, etc.) |
| `Exited(i32)` | PTY EOF, record exit code |

Prompt detection uses regex matching against common shell prompt patterns. Not configurable — initial coverage of mainstream shells (pwsh, bash, zsh, cmd, fish) is sufficient.

### D4: Sidecar initialization → automatic spawn

The sidecar spawns a shell immediately on startup, before accepting connections.

### D5: New `screen.rs` module in `teshi-runtime`

`ScreenGrid` is a standalone module holding `vte::Parser` + `GridPerformer` + row/col grid. The forwarder thread shares it via `Arc<Mutex<ScreenGrid>>` for snapshot/status queries.

`ScreenGrid` is not coupled to `TerminalState`'s lock structure. The sidecar builds its own `TerminalState` instance (does not depend on daemon's `TeshiRuntime`).

### D6: Sidecar architecture

```
teshi-terminal-sidecar binary
│
├── WebSocket Server (tokio-tungstenite)
│   ├── Listens on 127.0.0.1:0 (random port)
│   ├── Accepts CLI connection → parses JSON command → dispatches
│   └── Command handlers: snapshot / status / exec / send / resize / kill
│
├── TerminalState (portable-pty)
│   ├── spawn_terminal → starts shell
│   ├── write_terminal → writes stdin
│   ├── resize_terminal → resizes PTY
│   └── stop_terminal → kills shell
│
├── ScreenGrid (vte)
│   ├── VTE parses raw PTY output
│   ├── Dirty-row tracking / incremental reads
│   └── Process state detection
│
└── cdp-endpoint.json writer
    ├── ws_url: "ws://127.0.0.1:<random_port>"
    └── mode: "terminal"
```

### D7: CLI command design

```
teshi terminal serve-embedded          → Start sidecar (foreground, Ctrl+C to stop)
teshi terminal snapshot                → Full grid JSON
teshi terminal status                  → State JSON (low cost)
teshi terminal exec <command>          → Execute command, return final grid
teshi terminal exec <command> --timeout 30000 → Custom timeout
teshi terminal send <text>             → Write text, immediate return
teshi terminal send <text> --newline   → Write text + append newline
teshi terminal resize <cols> <rows>    → Resize PTY
teshi terminal kill                    → Kill current shell
```

All commands output JSON to stdout, non-zero exit code on failure. Consistent with `teshi browser`/`teshi winapp` `print_json_response` pattern.

### D8: Endpoint discovery

The sidecar writes `.teshi/cdp-endpoint.json` on startup:
```json
{
  "ws_url": "ws://127.0.0.1:54321",
  "mode": "terminal"
}
```

CLI reads via `read_cdp_endpoint(project_root)`, extracts `ws_url`, and passes directly to `send_sidecar_command_with_timeout`. Identical discovery mechanism to browser/winapp.

`mode: "terminal"` is distinct from `"browser"`/`"winapp"` — sidecars have independent ports, no conflict.

### D9: Does the sidecar reuse existing `TerminalState`?

Not directly. The sidecar implements its own PTY lifecycle (`start_pty()` in main.rs) rather than reusing `terminal.rs` functions (which are tied to `TeshiRuntime`). The core PTY logic (`portable-pty` calls) is rewritten in the sidecar crate.

`ScreenGrid` from `teshi-runtime` is shared via a workspace dependency.

### D10: New crate dependencies

`crates/teshi-terminal-sidecar/Cargo.toml`:
- `portable-pty = "0.8"` — PTY management
- `vte = "0.15"` — ANSI parsing
- `tokio` — async runtime
- `tokio-tungstenite` — WebSocket server
- `serde` / `serde_json` — JSON serialization
- `anyhow` — error handling
- `teshi-runtime` — ScreenGrid module only

## Risks / Trade-offs

- **[Risk] VTE parsing accuracy**: `vte::Perform` trait must handle all CSI/OSC/ESC sequences correctly. Full terminal emulation is complex. → **Mitigation**: Initial coverage of common/popular TUIs (npm, git log, basic vim operations, etc.), not pursuing 100% accuracy. On parse failure, degrade to raw text (strip ANSI control codes).
- **[Risk] exec blocking time**: `exec` in the sidecar must wait for command completion. → **Mitigation**: sidecar WebSocket handler is async, using `tokio::time::timeout` + polling `ScreenGrid.state` + max default timeout 60s.
- **[Risk] Prompt detection false negatives**: Non-standard prompts or complex PS1 may not match. → **Mitigation**: Introduce `SPAWNED` as initial state, transition to `RUNNING` on any output, transition to `IDLE` on timeout with no output. `WAITING_FOR_INPUT` is only an optimization hint, not a critical decision.
- **[Risk] New crate vs reusing teshi-runtime**: Sidecar not depending on teshi-runtime would mean maintaining two PTY codebases. → **Mitigation**: ScreenGrid module stays in `teshi-runtime`; sidecar depends on `teshi-runtime` only for `screen.rs`. PTY low-level code (portable-pty calls) is written directly in the sidecar crate, independent from daemon's terminal.rs.
- **[Risk] Concurrent access**: Multiple CLI connections may read inconsistent state. → **Mitigation**: `ScreenGrid` uses `Mutex` (PTY itself is single-session), snapshot returns atomic copy.

## Open Questions

- exec "wait for completion" polling interval: recommended 100ms, configurable.
- Snapshot incremental mode: control via optional `full: bool` parameter. Full snapshot sufficient for initial release.
- WebSocket Debug Viewer needed? Skip for now, add later if needed.
- Handle non-PTY scenarios (e.g., stdin/stdout pipe instead of PTY)? Not supported initially, PTY only.
