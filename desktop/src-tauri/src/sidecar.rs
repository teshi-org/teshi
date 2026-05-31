//! Python Playwright sidecar management.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::project::ProjectState;

pub struct SidecarState {
    child: Mutex<Option<Child>>,
    ws_url: Mutex<Option<String>>,
}

impl SidecarState {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
            ws_url: Mutex::new(None),
        }
    }

    pub async fn stop(&self) -> Result<()> {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.ws_url.lock().unwrap() = None;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct BrowserStartResult {
    pub ws_url: String,
}

#[derive(Debug, Serialize)]
pub struct BrowserError {
    pub message: String,
    pub hint: Option<String>,
}

fn find_python(project_root: &Path) -> Option<PathBuf> {
    for venv in [".venv", "venv"] {
        let python = project_root.join(venv).join(if cfg!(windows) {
            "Scripts/python.exe"
        } else {
            "bin/python"
        });
        if python.exists() {
            return Some(python);
        }
    }
    None
}

fn pick_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

fn browser_service_script(app: &AppHandle) -> Result<PathBuf> {
    app.path()
        .resolve(
            "resources/browser_service.py",
            tauri::path::BaseDirectory::Resource,
        )
        .context("resolve browser_service.py")
}

#[tauri::command]
pub async fn start_browser_sidecar(
    app: AppHandle,
    state: State<'_, ProjectState>,
    sidecar: State<'_, SidecarState>,
) -> Result<BrowserStartResult, BrowserError> {
    sidecar.stop().await.ok();

    let project_root = state
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| BrowserError {
            message: "Open a project before starting the browser.".into(),
            hint: None,
        })?;

    let python = find_python(&project_root).ok_or_else(|| BrowserError {
        message: "Python virtual environment not found in project root.".into(),
        hint: Some("Create .venv and run: pip install -r python/requirements.txt".into()),
    })?;

    let check = Command::new(&python)
        .args(["-c", "import playwright"])
        .output()
        .map_err(|e| BrowserError {
            message: format!("Failed to run Python: {e}"),
            hint: Some(format!(
                "{} -m pip install playwright websockets",
                python.display()
            )),
        })?;
    if !check.status.success() {
        return Err(BrowserError {
            message: "Playwright is not installed in the project venv.".into(),
            hint: Some(format!(
                "{} -m pip install -r python/requirements.txt",
                python.display()
            )),
        });
    }

    let chromium_check = Command::new(&python)
        .args(["-c", "from playwright.sync_api import sync_playwright; p=sync_playwright().start(); b=p.chromium.launch(headless=True); b.close(); p.stop()"])
        .output();
    if chromium_check.is_err() || !chromium_check.as_ref().unwrap().status.success() {
        return Err(BrowserError {
            message: "Chromium browser is not installed for Playwright.".into(),
            hint: Some(format!(
                "{} -m playwright install chromium",
                python.display()
            )),
        });
    }

    let script = browser_service_script(&app).map_err(|e| BrowserError {
        message: e.to_string(),
        hint: None,
    })?;
    let port = pick_port().map_err(|e| BrowserError {
        message: e.to_string(),
        hint: None,
    })?;

    let child = Command::new(&python)
        .arg(&script)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| BrowserError {
            message: format!("Failed to start browser sidecar: {e}"),
            hint: None,
        })?;

    *sidecar.child.lock().unwrap() = Some(child);
    let ws_url = format!("ws://127.0.0.1:{port}");
    *sidecar.ws_url.lock().unwrap() = Some(ws_url.clone());
    *state.browser_active.lock().unwrap() = true;

    app.emit("browser-started", ws_url.clone())
        .map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?;

    Ok(BrowserStartResult { ws_url })
}

#[tauri::command]
pub async fn stop_browser_sidecar(
    sidecar: State<'_, SidecarState>,
    state: State<'_, ProjectState>,
) -> Result<(), String> {
    sidecar.stop().await.map_err(|e| e.to_string())?;
    *state.browser_active.lock().unwrap() = false;
    Ok(())
}

/// Opens the native folder picker and returns the selected path.
#[tauri::command]
pub async fn open_project_dir(app: AppHandle) -> Result<Option<String>, String> {
    use crate::app_data::open_dialog_default_dir;

    let default = open_dialog_default_dir();
    let picked = app
        .dialog()
        .file()
        .set_title("Open Project")
        .set_directory(default.unwrap_or_else(|| PathBuf::from(".")))
        .blocking_pick_folder();

    Ok(picked.map(|p| p.to_string()))
}

/// Confirms destructive switch/exit when runtime is active.
#[tauri::command]
pub async fn confirm_teardown(
    app: AppHandle,
    state: State<'_, ProjectState>,
) -> Result<bool, String> {
    if !state.is_busy() {
        return Ok(true);
    }
    let answer = app
        .dialog()
        .message("Browser/Terminal is running. Continuing will stop them.")
        .title("Confirm")
        .buttons(MessageDialogButtons::OkCancel)
        .kind(MessageDialogKind::Warning)
        .blocking_show();

    Ok(answer)
}

#[tauri::command]
pub fn get_recent_projects_cmd() -> Result<Vec<String>, String> {
    crate::app_data::get_recent_projects().map_err(|e| e.to_string())
}
