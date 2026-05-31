//! PTY-backed terminal for Panel3.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::project::ProjectState;

/// Shared PTY session state for the embedded terminal panel.
pub struct TerminalState {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl TerminalState {
    /// Creates an empty terminal session holder.
    pub fn new() -> Self {
        Self {
            master: Mutex::new(None),
            writer: Mutex::new(None),
            child: Mutex::new(None),
        }
    }

    /// Kills the shell and drops PTY handles so a fresh session can be spawned.
    pub fn stop(&self) -> Result<()> {
        *self.master.lock().unwrap() = None;
        *self.writer.lock().unwrap() = None;
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
        }
        Ok(())
    }

    fn clear_after_exit(&self) {
        *self.master.lock().unwrap() = None;
        *self.writer.lock().unwrap() = None;
        *self.child.lock().unwrap() = None;
    }
}

/// Stops the PTY session and clears the project busy flag.
#[tauri::command]
pub fn stop_terminal(
    state: State<'_, ProjectState>,
    terminal: State<'_, TerminalState>,
) -> Result<(), String> {
    terminal.stop().map_err(|e| e.to_string())?;
    *state.terminal_active.lock().unwrap() = false;
    Ok(())
}

/// Resizes the PTY to match the xterm viewport.
#[tauri::command]
pub fn resize_terminal(
    cols: u16,
    rows: u16,
    terminal: State<'_, TerminalState>,
) -> Result<(), String> {
    if cols == 0 || rows == 0 {
        return Ok(());
    }
    let master = terminal.master.lock().unwrap();
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

/// Spawns an interactive shell in the opened project directory.
#[tauri::command]
pub async fn spawn_terminal(
    app: AppHandle,
    state: State<'_, ProjectState>,
    terminal: State<'_, TerminalState>,
) -> Result<(), String> {
    let project_root = state
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    // Always rebuild so a dead reader thread cannot leave a zombie "running" session.
    terminal.stop().map_err(|e| e.to_string())?;
    *state.terminal_active.lock().unwrap() = false;

    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let cwd = shell_cwd(&project_root);
    let mut last_err = String::from("no shell available");
    let mut child = None;
    for mut cmd in shell_commands() {
        cmd.cwd(cwd.to_string_lossy().to_string());
        apply_terminal_env(&mut cmd);
        if let Some(venv) = find_venv(&project_root) {
            cmd.env("VIRTUAL_ENV", venv.to_string_lossy().to_string());
        }
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

    *terminal.master.lock().unwrap() = Some(pair.master);
    *terminal.writer.lock().unwrap() = Some(writer);
    *terminal.child.lock().unwrap() = Some(child);
    *state.terminal_active.lock().unwrap() = true;

    // Nudge interactive shells to emit an initial prompt in ConPTY.
    if let Some(writer) = terminal.writer.lock().unwrap().as_mut() {
        let _ = writer.write_all(b"\r");
        let _ = writer.flush();
    }

    let app_handle = app.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Base64 keeps PTY bytes intact through Tauri JSON IPC (truecolor SGR, etc.).
                    let payload = BASE64.encode(&buf[..n]);
                    let _ = app_handle.emit("terminal-output", payload);
                }
                Err(_) => break,
            }
        }
        let _ = app_handle.emit("terminal-exit", ());
        if let Some(terminal_state) = app_handle.try_state::<TerminalState>() {
            terminal_state.clear_after_exit();
        }
        if let Some(project_state) = app_handle.try_state::<ProjectState>() {
            *project_state.terminal_active.lock().unwrap() = false;
        }
    });

    Ok(())
}

#[tauri::command]
pub fn write_terminal(data: String, terminal: State<'_, TerminalState>) -> Result<(), String> {
    if let Some(writer) = terminal.writer.lock().unwrap().as_mut() {
        writer
            .write_all(data.as_bytes())
            .map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Strips Windows extended-path prefixes so CreateProcess/shell cwd is reliable.
fn shell_cwd(project_root: &Path) -> PathBuf {
    dunce::simplified(project_root).to_path_buf()
}

fn shell_commands() -> Vec<CommandBuilder> {
    if cfg!(windows) {
        vec![
            shell_command("pwsh.exe", &["-NoLogo"]),
            shell_command("powershell.exe", &["-NoLogo"]),
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

/// Env vars that tell shells and CLI tools to emit ANSI color over ConPTY/xterm.
fn apply_terminal_env(cmd: &mut CommandBuilder) {
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("CLICOLOR", "1");
    cmd.env("CLICOLOR_FORCE", "1");
    cmd.env("FORCE_COLOR", "1");
    cmd.env_remove("NO_COLOR");
    cmd.env_remove("TERM_PROGRAM");
    if cfg!(windows) {
        // .NET console apps (including PowerShell) may suppress ANSI without this hint.
        cmd.env("DOTNET_SYSTEM_CONSOLE_ALLOW_ANSI_COLOR_REDIRECTION", "1");
    }
}

fn find_venv(project_root: &Path) -> Option<PathBuf> {
    for name in [".venv", "venv"] {
        let dir = project_root.join(name);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::PtySystem;
    use std::io::Read;
    use std::time::{Duration, Instant};

    /// Probes whether a TUI child emits SGR color sequences through our PTY setup.
    #[test]
    fn pty_child_emits_sgr_color_sequences() {
        let chrys = std::env::var("CHRYS_BIN")
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/bin/chrys.exe")))
            .filter(|p| p.is_file());

        let Some(chrys) = chrys else {
            eprintln!("skip: chrys binary not found");
            return;
        };

        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let mut cmd = CommandBuilder::new(chrys.to_string_lossy().to_string());
        apply_terminal_env(&mut cmd);
        let child = pair.slave.spawn_command(cmd).expect("spawn chrys");
        drop(child);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let n = reader.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() > 64 * 1024 {
                break;
            }
        }

        let text = String::from_utf8_lossy(&buf);
        let counts = [
            ("38;2", text.matches("\x1b[38;2;").count()),
            ("38;5", text.matches("\x1b[38;5;").count()),
            ("48;2", text.matches("\x1b[48;2;").count()),
            ("48;5", text.matches("\x1b[48;5;").count()),
            ("16-fg", text.matches("\x1b[3").count()),
        ];
        eprintln!("captured {} bytes, color counts: {counts:?}", buf.len());
        if let Some(idx) = text.find("\x1b[38;") {
            eprintln!("color sample: {:?}", &text[idx..idx.saturating_add(40)]);
        }
        let has_color = counts[0].1 + counts[1].1 + counts[2].1 + counts[3].1 > 0;
        assert!(has_color, "expected chrys to emit ANSI color SGR sequences");
    }
}
