//! Python Playwright sidecar management.

use std::io::Read;
use std::net::TcpListener;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::TeshiRuntime;

/// Browser session backend started by the sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserMode {
    /// Headless Playwright Chromium with JPEG stream.
    Embedded,
    /// User Chrome via teshi-bridge extension.
    Chrome,
}

impl BrowserMode {
    fn as_str(self) -> &'static str {
        match self {
            BrowserMode::Embedded => "embedded",
            BrowserMode::Chrome => "chrome",
        }
    }
}

/// Fixed HTTP discovery port for chrome mode (`GET /v1/bridge`).
pub const CHROME_DISCOVERY_PORT: u16 = 17373;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Holds the browser sidecar child process and WebSocket URL.
pub struct SidecarState {
    child: Mutex<Option<Child>>,
    ws_url: Mutex<Option<String>>,
    mode: Mutex<Option<BrowserMode>>,
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
            mode: Mutex::new(None),
        }
    }

    /// Stops the sidecar process if running.
    pub async fn stop(&self) -> Result<()> {
        let mode = *self.mode.lock().unwrap();
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => break,
                }
            }
        }

        // Release orphaned chrome discovery listeners after crashes or untracked sidecars.
        if mode == Some(BrowserMode::Chrome) || port_is_open(CHROME_DISCOVERY_PORT) {
            let _ = kill_listener_on_port(CHROME_DISCOVERY_PORT);
            let _ = wait_for_port_release(CHROME_DISCOVERY_PORT, Duration::from_secs(2));
        }

        *self.ws_url.lock().unwrap() = None;
        *self.mode.lock().unwrap() = None;
        Ok(())
    }

    /// Returns the browser sidecar WebSocket URL when the sidecar is running.
    pub fn browser_ws_url(&self) -> Option<String> {
        self.ws_url.lock().unwrap().clone()
    }

    /// Returns the active browser backend mode, if any.
    pub fn browser_mode(&self) -> Option<BrowserMode> {
        *self.mode.lock().unwrap()
    }
}

/// Result of starting the Playwright browser sidecar.
#[derive(Debug, Serialize)]
pub struct BrowserStartResult {
    pub ws_url: String,
    pub cdp_endpoint_path: String,
    pub mode: String,
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

/// Resolved project virtual environment paths for browser sidecar subprocesses.
struct ProjectVenv {
    root: PathBuf,
    python_exe: PathBuf,
}

fn resolve_project_venv(project_root: &Path) -> Option<ProjectVenv> {
    for name in [".venv", "venv"] {
        let root = project_root.join(name);
        let python_exe = root.join(if cfg!(windows) {
            "Scripts/python.exe"
        } else {
            "bin/python"
        });
        if !python_exe.is_file() {
            continue;
        }
        return Some(ProjectVenv { root, python_exe });
    }
    None
}

/// Build a Python subprocess for preflight checks (`python.exe` so stdout/stderr pipes work).
fn python_check_command(venv: &ProjectVenv) -> Command {
    let mut cmd = Command::new(&venv.python_exe);
    apply_windows_no_window(&mut cmd);
    apply_venv_isolation(&mut cmd, venv);
    cmd
}

/// Build a Python subprocess for the long-running sidecar (`python.exe`; `CREATE_NO_WINDOW` avoids a console flash).
fn python_sidecar_command(venv: &ProjectVenv) -> Command {
    python_check_command(venv)
}

fn check_failure_detail(check: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&check.stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = String::from_utf8_lossy(&check.stdout);
    let stdout = stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    format!("exit code {}", check.status.code().unwrap_or(-1))
}

fn import_check_failed_message(check: &std::process::Output, packages: &str) -> String {
    format!(
        "{packages} are not installed in the project venv ({}).",
        check_failure_detail(check)
    )
}

/// Keep browser sidecar imports and Playwright on the project venv, not global Python.
fn apply_venv_isolation(cmd: &mut Command, venv: &ProjectVenv) {
    cmd.env("VIRTUAL_ENV", &venv.root);
    cmd.env_remove("PYTHONHOME");
    cmd.env_remove("PYTHONPATH");
    cmd.env_remove("PYTHONUSERBASE");

    let scripts = venv
        .root
        .join(if cfg!(windows) { "Scripts" } else { "bin" });
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", prepend_path_entry(&scripts, &path));
    } else {
        cmd.env("PATH", &scripts);
    }
}

fn prepend_path_entry(prefix: &Path, existing: &std::ffi::OsStr) -> std::ffi::OsString {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut path = prefix.as_os_str().to_os_string();
    path.push(sep);
    path.push(existing);
    path
}

#[cfg(windows)]
fn apply_windows_no_window(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_windows_no_window(_cmd: &mut Command) {}

fn pick_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("bind ephemeral port")?;
    Ok(listener.local_addr()?.port())
}

/// True when something accepts TCP connections on the loopback port.
fn port_is_open(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

fn wait_for_port_release(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !port_is_open(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !port_is_open(port)
}

/// PID of the process listening on `127.0.0.1:port`, if discoverable.
fn find_listener_pid(port: u16) -> Option<u32> {
    #[cfg(windows)]
    {
        let output = Command::new("netstat").args(["-ano"]).output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let needle = format!("127.0.0.1:{port}");
        for line in text.lines() {
            if line.contains(&needle) && line.contains("LISTENING") {
                if let Some(pid) = line.split_whitespace().last() {
                    if let Ok(pid) = pid.parse::<u32>() {
                        return Some(pid);
                    }
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("lsof")
            .args(["-ti", &format!("tcp:{port}")])
            .output()
            .ok()?;
        let pid = String::from_utf8_lossy(&output.stdout).trim();
        pid.parse().ok()
    }
}

/// Stops the process bound to the discovery port (orphaned `browser_service.py`).
fn kill_listener_on_port(port: u16) -> Result<(), String> {
    let Some(pid) = find_listener_pid(port) else {
        return Ok(());
    };
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("taskkill failed for PID {pid} (port {port})"))
        }
    }
    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("kill failed for PID {pid} (port {port})"))
        }
    }
}

fn normalize_project_root(path: &Path) -> PathBuf {
    dunce::simplified(path)
        .canonicalize()
        .unwrap_or_else(|_| dunce::simplified(path).to_path_buf())
}

fn project_roots_match(expected: &Path, got: &str) -> bool {
    if got.trim().is_empty() {
        return false;
    }
    let got_path = Path::new(got);
    let a = normalize_project_root(expected);
    let b = normalize_project_root(got_path);
    if a == b {
        return true;
    }
    #[cfg(windows)]
    return a
        .to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy());
    #[cfg(not(windows))]
    false
}

/// Parsed `GET /v1/bridge` payload for chrome mode reuse.
struct ChromeBridgeDiscovery {
    ws_url: String,
    project_root: String,
}

fn fetch_chrome_bridge_discovery(port: u16) -> Result<ChromeBridgeDiscovery, BrowserError> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let mut stream =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500))
            .map_err(|e| BrowserError {
                message: format!("discovery port {port} is not reachable: {e}"),
                hint: None,
            })?;
    stream
        .write_all(b"GET /v1/bridge HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| BrowserError {
        message: e.to_string(),
        hint: None,
    })?;
    let text = String::from_utf8_lossy(&buf);
    if !text.contains("200 OK") {
        return Err(BrowserError {
            message: "discovery endpoint did not return bridge info".into(),
            hint: None,
        });
    }
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .or_else(|| text.split("\n\n").nth(1))
        .unwrap_or("")
        .trim();
    let payload: serde_json::Value = serde_json::from_str(body).map_err(|e| BrowserError {
        message: format!("invalid discovery JSON: {e}"),
        hint: None,
    })?;
    let ws_url = payload
        .get("ws_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| BrowserError {
            message: "discovery response missing ws_url".into(),
            hint: None,
        })?;
    let project_root = payload
        .get("project_root")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if payload.get("mode").and_then(|v| v.as_str()) != Some("chrome") {
        return Err(BrowserError {
            message: "discovery port is not serving chrome bridge mode".into(),
            hint: None,
        });
    }
    Ok(ChromeBridgeDiscovery {
        ws_url: ws_url.to_string(),
        project_root,
    })
}

fn read_child_stderr(child: &mut Child) -> String {
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = Vec::new();
        if stderr.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
            return String::from_utf8_lossy(&buf).trim().to_string();
        }
    }
    String::new()
}

/// Frees port 17373 or reuses an existing bridge for the same project.
fn prepare_chrome_discovery(project_root: &Path) -> Result<Option<String>, BrowserError> {
    if !port_is_open(CHROME_DISCOVERY_PORT) {
        return Ok(None);
    }

    if let Ok(bridge) = fetch_chrome_bridge_discovery(CHROME_DISCOVERY_PORT) {
        if project_roots_match(project_root, &bridge.project_root) {
            return Ok(Some(bridge.ws_url));
        }
    }

    kill_listener_on_port(CHROME_DISCOVERY_PORT).map_err(|e| BrowserError {
        message: format!("Port {CHROME_DISCOVERY_PORT} is in use and could not be released: {e}"),
        hint: Some("Close other teshi instances or run: netstat -ano | findstr :17373".into()),
    })?;
    if !wait_for_port_release(CHROME_DISCOVERY_PORT, Duration::from_secs(3)) {
        return Err(BrowserError {
            message: format!("Port {CHROME_DISCOVERY_PORT} is still in use after cleanup."),
            hint: Some(
                "End the orphaned python.exe on port 17373, then click Connect Chrome again."
                    .into(),
            ),
        });
    }
    Ok(None)
}

/// Starts the browser sidecar for the open project in the given mode.
pub async fn start_browser_sidecar(
    rt: Arc<TeshiRuntime>,
    mode: BrowserMode,
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

    let venv = resolve_project_venv(&project_root).ok_or_else(|| BrowserError {
        message: "Python virtual environment not found in project root.".into(),
        hint: Some("Create .venv and run: pip install -r python/requirements.txt".into()),
    })?;

    let import_snippet = if mode == BrowserMode::Chrome {
        "import websockets"
    } else {
        "import playwright, websockets"
    };
    let import_label = if mode == BrowserMode::Chrome {
        "websockets"
    } else {
        "Playwright/websockets"
    };

    let pip_hint = if mode == BrowserMode::Chrome {
        format!("{} -m pip install websockets", venv.python_exe.display())
    } else {
        format!(
            "{} -m pip install -r python/requirements.txt",
            venv.python_exe.display()
        )
    };

    let check = python_check_command(&venv)
        .args(["-c", import_snippet])
        .output()
        .map_err(|e| BrowserError {
            message: format!("Failed to run Python: {e}"),
            hint: Some(pip_hint.clone()),
        })?;
    if !check.status.success() {
        return Err(BrowserError {
            message: import_check_failed_message(&check, import_label),
            hint: Some(pip_hint),
        });
    }

    if mode == BrowserMode::Embedded {
        let chromium_check = python_check_command(&venv)
            .args(["-c", "from playwright.sync_api import sync_playwright; p=sync_playwright().start(); b=p.chromium.launch(headless=True); b.close(); p.stop()"])
            .output();
        if chromium_check.is_err() || !chromium_check.as_ref().unwrap().status.success() {
            let message = match &chromium_check {
                Ok(output) => format!(
                    "Chromium browser is not installed for Playwright ({}).",
                    check_failure_detail(output)
                ),
                Err(e) => format!("Failed to run Chromium check: {e}"),
            };
            return Err(BrowserError {
                message,
                hint: Some(format!(
                    "{} -m playwright install chromium",
                    venv.python_exe.display()
                )),
            });
        }
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
    let cdp_port = if mode == BrowserMode::Embedded {
        pick_port().map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?
    } else {
        0
    };

    if mode == BrowserMode::Chrome {
        if let Some(ws_url) = prepare_chrome_discovery(&project_root)? {
            *rt.sidecar.ws_url.lock().unwrap() = Some(ws_url.clone());
            *rt.sidecar.mode.lock().unwrap() = Some(mode);
            *rt.project.browser_active.lock().unwrap() = true;
            rt.events.emit(
                "browser-started",
                serde_json::json!({ "ws_url": ws_url, "mode": mode.as_str() }),
            );
            let cdp_endpoint_path = project_root
                .join(".teshi")
                .join("cdp-endpoint.json")
                .to_string_lossy()
                .into_owned();
            return Ok(BrowserStartResult {
                ws_url,
                cdp_endpoint_path,
                mode: mode.as_str().to_string(),
            });
        }
    }

    let mut cmd = python_sidecar_command(&venv);
    cmd.arg(script).args([
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "--mode",
        mode.as_str(),
        "--project-root",
        &project_root.to_string_lossy(),
    ]);
    if mode == BrowserMode::Embedded {
        cmd.args(["--cdp-port", &cdp_port.to_string()]);
    } else {
        cmd.args(["--discovery-port", &CHROME_DISCOVERY_PORT.to_string()]);
    }
    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| BrowserError {
            message: format!("Failed to start browser sidecar: {e}"),
            hint: None,
        })?;

    let ready = if mode == BrowserMode::Chrome {
        wait_until_chrome_ready(&mut child, port, CHROME_DISCOVERY_PORT)
    } else {
        wait_until_ready(&mut child, port)
    };
    if let Err(err) = ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }

    *rt.sidecar.child.lock().unwrap() = Some(child);
    let ws_url = format!("ws://127.0.0.1:{port}");
    *rt.sidecar.ws_url.lock().unwrap() = Some(ws_url.clone());
    *rt.sidecar.mode.lock().unwrap() = Some(mode);
    *rt.project.browser_active.lock().unwrap() = true;

    rt.events.emit(
        "browser-started",
        serde_json::json!({ "ws_url": ws_url, "mode": mode.as_str() }),
    );

    let cdp_endpoint_path = project_root
        .join(".teshi")
        .join("cdp-endpoint.json")
        .to_string_lossy()
        .into_owned();

    Ok(BrowserStartResult {
        ws_url,
        cdp_endpoint_path,
        mode: mode.as_str().to_string(),
    })
}

fn wait_until_chrome_ready(
    child: &mut Child,
    ws_port: u16,
    discovery_port: u16,
) -> Result<(), BrowserError> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::{Duration, Instant};

    let ws_addr: SocketAddr = ([127, 0, 0, 1], ws_port).into();
    let deadline = Instant::now() + Duration::from_secs(20);

    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let detail = read_child_stderr(child);
            let message = if detail.is_empty() {
                format!("Browser sidecar exited during startup (status: {status}).")
            } else {
                format!("Browser sidecar exited during startup (status: {status}): {detail}")
            };
            return Err(BrowserError {
                message,
                hint: Some(
                    "If port 17373 is in use, click Disconnect then Connect Chrome again.".into(),
                ),
            });
        }
        let ws_up = TcpStream::connect_timeout(&ws_addr, Duration::from_millis(400)).is_ok();
        let discovery_ok = fetch_discovery_bridge(discovery_port).is_ok();
        if ws_up && discovery_ok {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(BrowserError {
                message: "Chrome bridge did not become ready in time.".into(),
                hint: Some(
                    "Load unpacked extension from C:\\Program Files\\teshi\\share\\teshi-bridge in Chrome, activate your target tab, then retry."
                        .into(),
                ),
            });
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn fetch_discovery_bridge(discovery_port: u16) -> Result<(), BrowserError> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let mut stream = TcpStream::connect_timeout(
        &([127, 0, 0, 1], discovery_port).into(),
        Duration::from_millis(500),
    )
    .map_err(|e| BrowserError {
        message: e.to_string(),
        hint: None,
    })?;
    stream
        .write_all(b"GET /v1/bridge HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?;
    let mut buf = [0u8; 512];
    let n = stream.read(&mut buf).map_err(|e| BrowserError {
        message: e.to_string(),
        hint: None,
    })?;
    let text = String::from_utf8_lossy(&buf[..n]);
    if text.contains("200 OK") && text.contains("ws_url") {
        Ok(())
    } else {
        Err(BrowserError {
            message: "discovery endpoint did not return bridge info".into(),
            hint: None,
        })
    }
}

fn wait_until_ready(child: &mut Child, port: u16) -> Result<(), BrowserError> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::{Duration, Instant};

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let deadline = Instant::now() + Duration::from_secs(20);

    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let detail = read_child_stderr(child);
            let message = if detail.is_empty() {
                format!("Browser sidecar exited during startup (status: {status}).")
            } else {
                format!("Browser sidecar exited during startup (status: {status}): {detail}")
            };
            return Err(BrowserError {
                message,
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
