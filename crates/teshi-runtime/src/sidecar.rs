//! Python Playwright sidecar management.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::TeshiRuntime;

/// Holds the browser sidecar child process and WebSocket URL.
pub struct SidecarState {
    child: Mutex<Option<Child>>,
    ws_url: Mutex<Option<String>>,
}

impl Default for SidecarState {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarState {
    /// Creates an empty sidecar holder.
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
            ws_url: Mutex::new(None),
        }
    }

    /// Stops the sidecar process if running.
    pub async fn stop(&self) -> Result<()> {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
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

    /// Returns the browser sidecar WebSocket URL when the sidecar is running.
    pub fn browser_ws_url(&self) -> Option<String> {
        self.ws_url.lock().unwrap().clone()
    }
}

/// Result of starting the Playwright browser sidecar.
#[derive(Debug, Serialize)]
pub struct BrowserStartResult {
    pub ws_url: String,
    pub cdp_endpoint_path: String,
}

/// User-facing browser startup failure.
#[derive(Debug, Serialize)]
pub struct BrowserError {
    pub message: String,
    pub hint: Option<String>,
}

/// Sends a one-shot command to the browser sidecar WebSocket and waits for a response.
pub fn send_sidecar_command(
    ws_url: &str,
    command: serde_json::Value,
) -> Result<serde_json::Value, String> {
    use tungstenite::{connect, Message};

    let (mut socket, _) = connect(ws_url).map_err(|e| e.to_string())?;
    socket
        .send(Message::Text(command.to_string()))
        .map_err(|e| e.to_string())?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let message = socket.read().map_err(|e| e.to_string())?;
        if let Message::Text(text) = message {
            let payload: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if payload.get("type") == Some(&serde_json::Value::String("response".into())) {
                return Ok(payload);
            }
        }
    }
    Err("browser sidecar did not respond in time".into())
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

/// Starts the Playwright browser sidecar for the open project.
pub async fn start_browser_sidecar(
    rt: Arc<TeshiRuntime>,
) -> Result<BrowserStartResult, BrowserError> {
    rt.sidecar.stop().await.ok();

    let project_root = rt
        .project
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

    let script = &rt.browser_service_script;
    if !script.is_file() {
        return Err(BrowserError {
            message: format!("browser_service.py not found at {}", script.display()),
            hint: None,
        });
    }

    let port = pick_port().map_err(|e| BrowserError {
        message: e.to_string(),
        hint: None,
    })?;
    let cdp_port = pick_port().map_err(|e| BrowserError {
        message: e.to_string(),
        hint: None,
    })?;

    let mut child = Command::new(&python)
        .arg(script)
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--cdp-port",
            &cdp_port.to_string(),
            "--project-root",
            &project_root.to_string_lossy(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| BrowserError {
            message: format!("Failed to start browser sidecar: {e}"),
            hint: None,
        })?;

    if let Err(err) = wait_until_ready(&mut child, port) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }

    *rt.sidecar.child.lock().unwrap() = Some(child);
    let ws_url = format!("ws://127.0.0.1:{port}");
    *rt.sidecar.ws_url.lock().unwrap() = Some(ws_url.clone());
    *rt.project.browser_active.lock().unwrap() = true;

    rt.events.emit("browser-started", ws_url.clone());

    let cdp_endpoint_path = project_root
        .join(".teshi")
        .join("cdp-endpoint.json")
        .to_string_lossy()
        .into_owned();

    Ok(BrowserStartResult {
        ws_url,
        cdp_endpoint_path,
    })
}

fn wait_until_ready(child: &mut Child, port: u16) -> Result<(), BrowserError> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::{Duration, Instant};

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let deadline = Instant::now() + Duration::from_secs(20);

    loop {
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

/// Stops the browser sidecar and clears the busy flag.
pub async fn stop_browser_sidecar(rt: &TeshiRuntime) -> Result<(), String> {
    rt.sidecar.stop().await.map_err(|e| e.to_string())?;
    *rt.project.browser_active.lock().unwrap() = false;
    Ok(())
}

/// Returns persisted recent project paths.
pub fn get_recent_projects() -> Result<Vec<String>, String> {
    crate::app_data::get_recent_projects().map_err(|e| e.to_string())
}
