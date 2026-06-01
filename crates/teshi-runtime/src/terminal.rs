//! PTY-backed terminal for the file tree / terminal panel.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};

use crate::TeshiRuntime;

/// Shared PTY session state for the embedded terminal panel.
pub struct TerminalState {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
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
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    rt.terminal.stop().map_err(|e| e.to_string())?;
    *rt.project.terminal_active.lock().unwrap() = false;

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
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let payload = BASE64.encode(&buf[..n]);
                    rt_reader.events.emit("terminal-output", payload);
                }
                Err(_) => break,
            }
        }
        rt_reader.events.emit("terminal-exit", ());
        rt_reader.terminal.clear_after_exit();
        *rt_reader.project.terminal_active.lock().unwrap() = false;
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

/// Disable PSReadLine history predictions in the embedded panel; ListView/Inline both
/// clutter the small xterm viewport and Inline misaligns in web-based emulators.
/// Re-apply after `$PROFILE` so user dotfiles cannot re-enable predictions.
const PSREADLINE_EMBEDDED_INIT: &str = concat!(
    "try { Set-PSReadLineOption -PredictionSource None } catch { }; ",
    "if (Test-Path $PROFILE) { . $PROFILE }; ",
    "try { Set-PSReadLineOption -PredictionSource None } catch { }"
);

fn shell_commands() -> Vec<CommandBuilder> {
    if cfg!(windows) {
        vec![
            shell_command(
                "pwsh.exe",
                &["-NoLogo", "-NoExit", "-Command", PSREADLINE_EMBEDDED_INIT],
            ),
            shell_command(
                "powershell.exe",
                &["-NoLogo", "-NoExit", "-Command", PSREADLINE_EMBEDDED_INIT],
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
