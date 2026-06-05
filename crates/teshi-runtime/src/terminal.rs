//! PTY-backed terminal for the file tree / terminal panel.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use tokio::sync::Mutex as AsyncMutex;

use crate::TeshiRuntime;

/// Shared PTY session state for the embedded terminal panel.
pub struct TerminalState {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
    /// Serializes spawn/restart so concurrent frontend requests cannot leak shells.
    spawn_lock: AsyncMutex<()>,
    /// Monotonic session id; output from stale reader threads is dropped after respawn.
    session_id: AtomicU64,
    /// Join handle for the PTY output forwarder (kept so stop can wait briefly).
    output_forwarder: Mutex<Option<JoinHandle<()>>>,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalState {
    /// Creates an empty terminal session holder.
    pub fn new() -> Self {
        Self {
            master: Mutex::new(None),
            writer: Mutex::new(None),
            child: Mutex::new(None),
            spawn_lock: AsyncMutex::new(()),
            session_id: AtomicU64::new(0),
            output_forwarder: Mutex::new(None),
        }
    }

    /// Kills the shell and drops PTY handles so a fresh session can be spawned.
    pub fn stop(&self) -> Result<()> {
        // Invalidate any in-flight reader/forwarder threads before dropping handles.
        self.session_id.fetch_add(1, Ordering::SeqCst);
        *self.master.lock().unwrap() = None;
        *self.writer.lock().unwrap() = None;
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
        }
        let _ = self.output_forwarder.lock().unwrap().take();
        Ok(())
    }

    pub(crate) fn clear_after_exit(&self) {
        *self.master.lock().unwrap() = None;
        *self.writer.lock().unwrap() = None;
        *self.child.lock().unwrap() = None;
    }
}

/// Stops the PTY session and clears the project busy flag.
pub fn stop_terminal(rt: &TeshiRuntime) -> Result<(), String> {
    rt.terminal.stop().map_err(|e| e.to_string())?;
    *rt.project.terminal_active.lock().unwrap() = false;
    Ok(())
}

/// Resizes the PTY to match the xterm viewport.
pub fn resize_terminal(rt: &TeshiRuntime, cols: u16, rows: u16) -> Result<(), String> {
    if cols == 0 || rows == 0 {
        return Ok(());
    }
    let master = rt.terminal.master.lock().unwrap();
    if let Some(master) = master.as_ref() {
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Normalizes viewport dimensions from the frontend (xterm FitAddon).
fn normalized_pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols: cols.max(2),
        rows: rows.max(2),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Spawns an interactive shell in the opened project directory.
///
/// `cols` and `rows` must match the xterm viewport so ConPTY/PSReadLine render
/// the prompt at the correct width on first paint.
pub async fn spawn_terminal(rt: Arc<TeshiRuntime>, cols: u16, rows: u16) -> Result<(), String> {
    let _spawn_guard = rt.terminal.spawn_lock.lock().await;

    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    rt.terminal.stop().map_err(|e| e.to_string())?;
    *rt.project.terminal_active.lock().unwrap() = false;
    let session_id = rt.terminal.session_id.load(Ordering::SeqCst);
    tracing::debug!(
        session_id,
        cols,
        rows,
        "spawn_terminal: starting PTY session"
    );

    let pty_size = normalized_pty_size(cols, rows);
    let pty_system = NativePtySystem::default();
    let pair = pty_system.openpty(pty_size).map_err(|e| e.to_string())?;

    let cwd = shell_cwd(&project_root);
    let mut last_err = String::from("no shell available");
    let mut child = None;
    for mut cmd in shell_commands() {
        cmd.cwd(cwd.to_string_lossy().to_string());
        apply_terminal_env(&mut cmd, rt.embedded_terminal_teshi_cli());
        // Do not inject VIRTUAL_ENV here: it breaks `uv pip` on Windows (os error 448) when uv
        // inspects `.venv\Scripts\python.exe`. Users activate the venv in the shell if needed.
        match pair.slave.spawn_command(cmd) {
            Ok(spawned) => {
                child = Some(spawned);
                break;
            }
            Err(err) => {
                last_err = err.to_string();
            }
        }
    }
    let child = child.ok_or(last_err)?;

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    *rt.terminal.master.lock().unwrap() = Some(pair.master);
    *rt.terminal.writer.lock().unwrap() = Some(writer);
    *rt.terminal.child.lock().unwrap() = Some(child);
    *rt.project.terminal_active.lock().unwrap() = true;

    let rt_reader = Arc::clone(&rt);
    let rt_forwarder = Arc::clone(&rt);
    let (output_tx, output_rx) = std::sync::mpsc::channel::<Vec<u8>>();

    // Drain the PTY on a dedicated thread so a slow host emit (Tauri/webview) cannot
    // block ConPTY reads and freeze external TUI agents (e.g. Chrys).
    // Coalesce rapid chunks (e.g. PSReadLine per-char redraws) into batches.
    // Flush on idle (no new data for 20ms) or when batch exceeds 4KB.
    let forwarder = std::thread::spawn(move || {
        let mut batch: Vec<u8> = Vec::with_capacity(8192);
        let idle_ms = std::time::Duration::from_millis(20);
        loop {
            let chunk = match output_rx.recv_timeout(idle_ms) {
                Ok(chunk) => chunk,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Idle timeout — flush accumulated batch
                    if !batch.is_empty() {
                        let payload = BASE64.encode(&batch);
                        rt_forwarder.events.emit("terminal-output", payload);
                        batch.clear();
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    if !batch.is_empty() {
                        let payload = BASE64.encode(&batch);
                        rt_forwarder.events.emit("terminal-output", payload);
                    }
                    break;
                }
            };
            if rt_forwarder.terminal.session_id.load(Ordering::SeqCst) != session_id {
                break;
            }
            batch.extend_from_slice(&chunk);
            if batch.len() >= 4096 {
                let payload = BASE64.encode(&batch);
                rt_forwarder.events.emit("terminal-output", payload);
                batch.clear();
            }
        }
    });
    *rt.terminal.output_forwarder.lock().unwrap() = Some(forwarder);

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            if rt_reader.terminal.session_id.load(Ordering::SeqCst) != session_id {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if output_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        drop(output_tx);
        if rt_reader.terminal.session_id.load(Ordering::SeqCst) == session_id {
            rt_reader.events.emit("terminal-exit", ());
            rt_reader.terminal.clear_after_exit();
            *rt_reader.project.terminal_active.lock().unwrap() = false;
            tracing::debug!(session_id, "spawn_terminal: PTY session exited");
        }
    });

    Ok(())
}

/// Writes bytes to the PTY stdin.
pub fn write_terminal(rt: &TeshiRuntime, data: String) -> Result<(), String> {
    let mut guard = rt.terminal.writer.lock().unwrap();
    let Some(writer) = guard.as_mut() else {
        return Err(
            "terminal shell is not running; switch to the Terminal tab or click inside it to start the shell".into(),
        );
    };
    writer
        .write_all(data.as_bytes())
        .map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn shell_cwd(project_root: &Path) -> PathBuf {
    dunce::simplified(project_root).to_path_buf()
}

/// Embedded panel init: drop PSReadLine (per-keystroke VT redraws break browser xterm),
/// then optionally load profile aliases. Profile is loaded last so Remove-Module runs again.
const EMBEDDED_SHELL_INIT: &str = concat!(
    "Remove-Module PSReadLine -ErrorAction SilentlyContinue -Force; ",
    "if (Test-Path $PROFILE) { . $PROFILE }; ",
    "Remove-Module PSReadLine -ErrorAction SilentlyContinue -Force"
);

fn shell_commands() -> Vec<CommandBuilder> {
    if cfg!(windows) {
        vec![
            shell_command(
                "pwsh.exe",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NoExit",
                    "-Command",
                    EMBEDDED_SHELL_INIT,
                ],
            ),
            shell_command(
                "powershell.exe",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NoExit",
                    "-Command",
                    EMBEDDED_SHELL_INIT,
                ],
            ),
            shell_command("cmd.exe", &[]),
        ]
    } else {
        vec![shell_command("bash", &[])]
    }
}

fn shell_command(program: &str, args: &[&str]) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(program);
    for arg in args {
        cmd.arg(*arg);
    }
    cmd
}

fn apply_terminal_env(cmd: &mut CommandBuilder, teshi_cli: Option<&Path>) {
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("CLICOLOR", "1");
    cmd.env("CLICOLOR_FORCE", "1");
    cmd.env("FORCE_COLOR", "1");
    cmd.env("TESHI_EMBEDDED_TERMINAL", "1");
    cmd.env_remove("NO_COLOR");
    cmd.env_remove("TERM_PROGRAM");
    if let Some(path) = teshi_cli {
        cmd.env("TESHI_CLI", path.to_string_lossy().into_owned());
    }
    if cfg!(windows) {
        cmd.env("DOTNET_SYSTEM_CONSOLE_ALLOW_ANSI_COLOR_REDIRECTION", "1");
    }
}
