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
            // Bound wait so window close does not hang on a stuck Playwright process.
            use std::time::{Duration, Instant};
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => break,
                }
            }
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

    // sidecar 运行时同时依赖 playwright 与 websockets，启动前一并检测以避免子进程秒退。
    let check = Command::new(&python)
        .args(["-c", "import playwright, websockets"])
        .output()
        .map_err(|e| BrowserError {
            message: format!("Failed to run Python: {e}"),
            hint: Some(format!(
                "{} -m pip install -r python/requirements.txt",
                python.display()
            )),
        })?;
    if !check.status.success() {
        return Err(BrowserError {
            message: "Playwright/websockets are not installed in the project venv.".into(),
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

    let mut child = Command::new(&python)
        .arg(&script)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| BrowserError {
            message: format!("Failed to start browser sidecar: {e}"),
            hint: None,
        })?;

    // 轮询 WebSocket 端口直到就绪；若子进程提前退出或超时，kill 后返回错误，
    // 避免在 sidecar 实际不可用时仍把 browser_active 置为 true。
    if let Err(err) = wait_until_ready(&mut child, port) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }

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

/// 在超时时间内轮询 sidecar 的监听端口，确认其已就绪。
fn wait_until_ready(child: &mut Child, port: u16) -> Result<(), BrowserError> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::{Duration, Instant};

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let deadline = Instant::now() + Duration::from_secs(20);

    loop {
        // 子进程已退出说明启动失败（依赖缺失、Chromium 未安装等）。
        if let Ok(Some(status)) = child.try_wait() {
            return Err(BrowserError {
                message: format!("Browser sidecar exited during startup (status: {status})."),
                hint: Some(
                    "Check that Playwright Chromium is installed and the venv is valid.".into(),
                ),
            });
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(BrowserError {
                message: "Browser sidecar did not become ready in time.".into(),
                hint: Some("The Playwright service failed to open its WebSocket port.".into()),
            });
        }
        std::thread::sleep(Duration::from_millis(200));
    }
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
