//! Standalone terminal sidecar with VTE screen grid for CLI automation.
//!
//! Binds TCP on `127.0.0.1:0`, writes `.teshi/cdp-endpoint.json`, and
//! serves exactly one WebSocket client connection at a time.
//! Each connection can issue JSON commands (snapshot, status, send, exec,
//! resize, kill) against a persistent PTY shell session.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde_json::{json, Value};
use teshi_runtime::ProcessState;
use teshi_runtime::ScreenGrid;
use tokio::net::TcpListener as TokioTcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

// ---------------------------------------------------------------------------
// PTY lifecycle
// ---------------------------------------------------------------------------

/// Wraps a running PTY shell, its ScreenGrid, and a background reader thread.
struct PtySession {
    screen: Arc<ScreenGrid>,
    writer: Box<dyn Write + Send>,
    /// The master PTY handle — taking it out and dropping it causes the reader
    /// thread to exit.
    master: Option<Box<dyn MasterPty + Send>>,
    /// Set to false to signal the reader thread to stop.
    running: Arc<AtomicBool>,
    /// The background reader thread handle.
    reader_thread: Option<std::thread::JoinHandle<()>>,
}

impl PtySession {
    /// Stop the reader thread and release PTY resources.
    fn shutdown(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        // Drop the master PTY handle – closes ConPTY so the reader's
        // `Read::read` call fails / returns EOF and the thread exits.
        drop(self.master.take());
        // Join the reader thread (best-effort).
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Spawn a shell in a PTY and start feeding its output into a ScreenGrid.
fn start_pty() -> Result<PtySession> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open PTY")?;

    // Find a working shell
    let mut last_err = String::new();
    let mut child: Option<Box<dyn portable_pty::Child + Send + Sync>> = None;
    for cmd in shell_commands() {
        match pair.slave.spawn_command(cmd) {
            Ok(c) => {
                child = Some(c);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let _child = child.ok_or_else(|| anyhow::anyhow!("no shell available: {}", last_err))?;

    let mut reader = pair.master.try_clone_reader().context("clone PTY reader")?;
    let writer = pair.master.take_writer().context("take PTY writer")?;

    let screen = Arc::new(ScreenGrid::new(24, 80));
    let running = Arc::new(AtomicBool::new(true));
    let screen_clone = Arc::clone(&screen);
    let running_clone = Arc::clone(&running);

    let reader_thread = std::thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut buf = [0u8; 8192];
            while running_clone.load(Ordering::SeqCst) {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        screen_clone.feed(&buf[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        })
        .context("spawn PTY reader thread")?;

    Ok(PtySession {
        screen,
        writer,
        master: Some(pair.master),
        running,
        reader_thread: Some(reader_thread),
    })
}

/// Shell candidates (tried in order).
fn shell_commands() -> Vec<CommandBuilder> {
    if cfg!(windows) {
        vec![
            {
                let mut cmd = CommandBuilder::new("pwsh.exe");
                cmd.args(["-NoLogo", "-NoExit"]);
                cmd
            },
            {
                let mut cmd = CommandBuilder::new("powershell.exe");
                cmd.args(["-NoLogo", "-NoExit"]);
                cmd
            },
            {
                let cmd = CommandBuilder::new("cmd.exe");
                cmd
            },
        ]
    } else {
        vec![{
            let cmd = CommandBuilder::new("bash");
            cmd
        }]
    }
}

// ---------------------------------------------------------------------------
// CDP endpoint helpers
// ---------------------------------------------------------------------------

/// Walk up from CWD until a `.teshi/` directory is found.
fn find_project_root() -> Option<std::path::PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    if let Ok(canonical) = current.canonicalize() {
        current = canonical;
    }
    loop {
        if current.join(".teshi").is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Write `.teshi/cdp-endpoint.json` with the sidecar WebSocket URL.
fn write_cdp_endpoint(port: u16) -> Result<std::path::PathBuf> {
    let project_root = find_project_root()
        .context("no `.teshi` directory found in parent chain (not inside a teshi project)")?;
    let teshi_dir = project_root.join(".teshi");
    std::fs::create_dir_all(&teshi_dir).context("create .teshi directory")?;

    let endpoint = json!({
        "ws_url": format!("ws://127.0.0.1:{}", port),
        "mode": "terminal",
    });

    let path = teshi_dir.join("cdp-endpoint.json");
    let json_str =
        serde_json::to_string_pretty(&endpoint).context("serialize cdp-endpoint.json")?;
    std::fs::write(&path, &json_str).context("write cdp-endpoint.json")?;
    Ok(path)
}

/// Remove the cdp-endpoint.json written by this sidecar.
#[allow(dead_code)]
fn remove_cdp_endpoint() {
    if let Some(project_root) = find_project_root() {
        let path = project_root.join(".teshi").join("cdp-endpoint.json");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&path.with_extension("lock"));
    }
}

// ---------------------------------------------------------------------------
// JSON command dispatcher
// ---------------------------------------------------------------------------

fn handle_command(pty: &mut Option<PtySession>, cmd: &Value) -> Value {
    let request_id = cmd.get("request_id").cloned();
    let cmd_name = cmd.get("cmd").and_then(|v| v.as_str()).unwrap_or("");

    let response = match cmd_name {
        "snapshot" => handle_snapshot(pty, cmd),
        "status" => handle_status(pty),
        "send" => handle_send(pty, cmd),
        "exec" => handle_exec(pty, cmd),
        "resize" => handle_resize(pty, cmd),
        "kill" => handle_kill(pty),
        _ => json!({"ok": false, "error": format!("unknown command: {}", cmd_name)}),
    };

    let mut result = json!({
        "type": "response",
    });
    if let Some(id) = request_id {
        result["request_id"] = id;
    }
    // Flatten response fields into result
    if let Some(obj) = response.as_object() {
        for (k, v) in obj {
            result[k] = v.clone();
        }
    }
    result
}

fn require_pty<'a>(pty: &'a mut Option<PtySession>) -> Result<&'a mut PtySession, Value> {
    pty.as_mut()
        .ok_or_else(|| json!({"ok": false, "error": "no pty session; start one first"}))
}

fn handle_snapshot(pty: &mut Option<PtySession>, cmd: &Value) -> Value {
    let pty = match require_pty(pty) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let full = cmd.get("full").and_then(|v| v.as_bool()).unwrap_or(true);
    let snap = pty.screen.snapshot(full);
    json!({"ok": true, "snapshot": snap})
}

fn handle_status(pty: &mut Option<PtySession>) -> Value {
    match pty.as_ref() {
        Some(session) => {
            let st = session.screen.status();
            json!({"ok": true, "status": st})
        }
        None => {
            // Return a "dead" status instead of error so callers can detect
            // and auto-recover
            json!({"ok": true, "status": {
                "state": "exited",
                "rows": 0, "cols": 0,
                "cursor": {"row": 0, "col": 0},
                "scrollback_len": 0, "dirty_count": 0
            }})
        }
    }
}

fn handle_send(pty: &mut Option<PtySession>, cmd: &Value) -> Value {
    let pty = match require_pty(pty) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let data = cmd.get("data").and_then(|v| v.as_str()).unwrap_or("");
    let newline = cmd
        .get("newline")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut payload: Vec<u8> = data.as_bytes().to_vec();
    if newline {
        payload.push(b'\n');
    }
    if let Err(e) = pty.writer.write_all(&payload) {
        return json!({"ok": false, "error": format!("write error: {}", e)});
    }
    if let Err(e) = pty.writer.flush() {
        return json!({"ok": false, "error": format!("flush error: {}", e)});
    }
    json!({"ok": true})
}

fn handle_exec(pty: &mut Option<PtySession>, cmd: &Value) -> Value {
    // Auto-spawn if no session
    if pty.is_none() {
        match start_pty() {
            Ok(session) => {
                *pty = Some(session);
                // Brief pause for shell init
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => return json!({"ok": false, "error": format!("auto-spawn failed: {}", e)}),
        }
    }
    let pty = match require_pty(pty) {
        Ok(p) => p,
        Err(e) => return e,
    };

    // Accept both "command" (from CLI) and "data" (backward compat)
    let data = cmd
        .get("command")
        .or_else(|| cmd.get("data"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Accept both "timeout_ms" (from CLI) and "timeout" (backward compat)
    let timeout_s = cmd
        .get("timeout_ms")
        .and_then(|v| v.as_f64())
        .map(|ms| ms / 1000.0)
        .or_else(|| cmd.get("timeout").and_then(|v| v.as_f64()))
        .unwrap_or(10.0);

    // Write command + newline
    let mut payload: Vec<u8> = data.as_bytes().to_vec();
    payload.push(b'\n');
    if let Err(e) = pty.writer.write_all(&payload) {
        return json!({"ok": false, "error": format!("write error: {}", e)});
    }
    if let Err(e) = pty.writer.flush() {
        return json!({"ok": false, "error": format!("flush error: {}", e)});
    }

    // Poll until state != Running or timeout
    let deadline = Instant::now() + Duration::from_secs_f64(timeout_s);
    pty.screen.clear_dirty();

    // Wait for new content to appear (command output)
    // We loop checking both process state and dirty content
    loop {
        let state = pty.screen.process_state();

        // If state transitioned to idle/waiting/exited, command likely finished
        if matches!(
            state,
            ProcessState::Idle | ProcessState::WaitingForInput | ProcessState::Exited(_)
        ) {
            break;
        }
        // If still running, wait for it to finish
        if state == ProcessState::Running {
            // Keep polling
        }
        // If spawned (no output yet), keep waiting for first output
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(80));
    }

    // Final settle: wait a bit more for any trailing output
    std::thread::sleep(Duration::from_millis(200));

    let snap = pty.screen.snapshot(true);
    pty.screen.clear_dirty();
    let st = pty.screen.status();
    json!({"ok": true, "snapshot": snap, "status": st})
}

fn handle_resize(pty: &mut Option<PtySession>, cmd: &Value) -> Value {
    let pty = match require_pty(pty) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let rows = cmd.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
    let cols = cmd.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;

    if let Some(ref master) = pty.master {
        if let Err(e) = master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            return json!({"ok": false, "error": format!("PTY resize error: {}", e)});
        }
    }
    pty.screen.resize(rows, cols);
    json!({"ok": true})
}

fn handle_kill(pty: &mut Option<PtySession>) -> Value {
    match pty.take() {
        Some(mut session) => {
            session.shutdown();
            json!({"ok": true})
        }
        None => json!({"ok": false, "error": "no pty session to kill"}),
    }
}

// ---------------------------------------------------------------------------
// WebSocket handler (one connection at a time)
// ---------------------------------------------------------------------------

async fn handle_connection(
    stream: tokio::net::TcpStream,
    pty: &mut Option<PtySession>,
) -> Result<()> {
    let ws_stream = accept_async(stream).await?;
    let (mut write, mut read) = ws_stream.split();

    loop {
        let msg = match read.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                eprintln!("WebSocket read error: {}", e);
                break;
            }
            None => break,
        };

        match msg {
            Message::Text(text) => {
                let cmd: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        let resp = json!({
                            "type": "response",
                            "ok": false,
                            "error": format!("invalid JSON: {}", e),
                        });
                        let _ = write.send(Message::Text(resp.to_string())).await;
                        continue;
                    }
                };

                let response = handle_command(pty, &cmd);
                let _ = write.send(Message::Text(response.to_string())).await;
            }
            Message::Close(_) => break,
            Message::Ping(data) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Message::Pong(_) => {}
            Message::Binary(_) => {}
            Message::Frame(_) => {}
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    // ── Bind on a random loopback port ────────────────────────────────────
    let std_listener =
        std::net::TcpListener::bind("127.0.0.1:0").context("bind TCP listener on 127.0.0.1:0")?;
    let port = std_listener.local_addr()?.port();
    std_listener
        .set_nonblocking(true)
        .context("set TCP listener to non-blocking")?;
    let tokio_listener =
        TokioTcpListener::from_std(std_listener).context("convert to tokio TCP listener")?;

    // ── Write CDP endpoint ───────────────────────────────────────────────
    let endpoint_path = write_cdp_endpoint(port)?;
    eprintln!("terminal sidecar ready on ws://127.0.0.1:{}", port);
    eprintln!("cdp-endpoint: {}", endpoint_path.display());

    // ── Ctrl+C cleanup ────────────────────────────────────────────────────
    let _cleanup_handle = {
        let ep = endpoint_path.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("terminal sidecar shutting down...");
            let _ = std::fs::remove_file(&ep);
            let _ = std::fs::remove_file(&ep.with_extension("lock"));
            std::process::exit(0);
        })
    };

    // ── Start PTY session ────────────────────────────────────────────────
    let mut pty: Option<PtySession> = Some(start_pty().context("start PTY session")?);
    eprintln!("PTY session started");

    loop {
        match tokio_listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!("connection from {}", addr);
                if let Err(e) = handle_connection(stream, &mut pty).await {
                    eprintln!("connection error: {}", e);
                }
                eprintln!("connection closed");
            }
            Err(e) => {
                eprintln!("accept error: {}", e);
                break;
            }
        }
    }

    // ── Cleanup ───────────────────────────────────────────────────────────
    if let Some(ref mut session) = pty {
        session.shutdown();
    }
    let _ = std::fs::remove_file(&endpoint_path);
    Ok(())
}
