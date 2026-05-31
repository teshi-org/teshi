//! PTY-backed terminal for Panel3.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use tauri::{AppHandle, Emitter, State};

use crate::project::ProjectState;

pub struct TerminalState {
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            writer: Mutex::new(None),
            child: Mutex::new(None),
        }
    }

    pub fn stop(&self) -> Result<()> {
        *self.writer.lock().unwrap() = None;
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn spawn_terminal(
    app: AppHandle,
    state: State<'_, ProjectState>,
    terminal: State<'_, TerminalState>,
) -> Result<(), String> {
    if terminal.writer.lock().unwrap().is_some() {
        return Ok(());
    }

    let project_root = state
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    terminal.stop().map_err(|e| e.to_string())?;

    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let shell = if cfg!(windows) {
        "powershell.exe"
    } else {
        "bash"
    };
    let mut cmd = CommandBuilder::new(shell);
    cmd.cwd(project_root.to_string_lossy().to_string());

    if let Some(venv) = find_venv(&project_root) {
        cmd.env("VIRTUAL_ENV", venv.to_string_lossy().to_string());
    }

    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    *terminal.writer.lock().unwrap() = Some(writer);
    *terminal.child.lock().unwrap() = Some(child);
    *state.terminal_active.lock().unwrap() = true;

    let app_handle = app.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = app_handle.emit("terminal-output", text);
                }
                Err(_) => break,
            }
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

fn find_venv(project_root: &PathBuf) -> Option<PathBuf> {
    for name in [".venv", "venv"] {
        let dir = project_root.join(name);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    None
}
