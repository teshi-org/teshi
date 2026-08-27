//! Python Playwright sidecar management.

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fd_lock::RwLock;
use serde::{Deserialize, Serialize};

use crate::{TeshiEngine, BROWSER_AGENT_SCHEMA_VERSION, BROWSER_BROKER_PROTOCOL_VERSION};

/// Browser session backend started by the sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserMode {
    /// Headless Playwright Chromium with JPEG stream.
    Embedded,
    /// User Chrome via teshi-bridge extension.
    Chrome,
    /// Native Windows apps via UI Automation and window capture.
    WinApp,
}

impl BrowserMode {
    fn as_str(self) -> &'static str {
        match self {
            BrowserMode::Embedded => "embedded",
            BrowserMode::Chrome => "chrome",
            BrowserMode::WinApp => "winapp",
        }
    }
}

/// Fixed HTTP discovery port for chrome mode (`GET /v1/bridge`).
pub const CHROME_DISCOVERY_PORT: u16 = 17373;

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

/// Public discovery record for the per-user Chrome broker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChromeBrokerEndpoint {
    pub schema_version: u16,
    pub protocol_version: u16,
    pub mode: String,
    pub ws_url: String,
    pub discovery_url: String,
    pub extension_frame_ws_url: String,
    pub broker_pid: u32,
    pub broker_start_id: String,
    #[serde(default)]
    pub broker_features: Vec<String>,
    #[serde(default)]
    pub project_root: String,
}

/// User-facing browser startup failure.
#[derive(Debug, Serialize)]
pub struct BrowserError {
    pub message: String,
    pub hint: Option<String>,
}

/// Sends a one-shot command to the browser sidecar WebSocket and waits for a response.
///
/// Uses a 10-second read deadline when `timeout` is omitted.
pub fn send_sidecar_command(
    ws_url: &str,
    command: serde_json::Value,
) -> Result<serde_json::Value, String> {
    send_sidecar_command_with_timeout(ws_url, command, std::time::Duration::from_secs(10))
}

/// Sends a one-shot command and waits up to `timeout` for a typed `response` message.
pub fn send_sidecar_command_with_timeout(
    ws_url: &str,
    command: serde_json::Value,
    timeout: std::time::Duration,
) -> Result<serde_json::Value, String> {
    use tungstenite::{connect, Message};

    let (mut socket, _) = connect(ws_url).map_err(|e| e.to_string())?;
    socket
        .send(Message::Text(command.to_string()))
        .map_err(|e| e.to_string())?;

    let deadline = std::time::Instant::now() + timeout;
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
    let secs = timeout.as_secs();
    Err(format!(
        "browser sidecar did not respond within {secs}s (CLI timeout; check extension heartbeat if using Connect Chrome)"
    ))
}

use crate::venv::{
    build_import_check_command, check_failure_detail, import_check_failed_message,
    resolve_project_venv, venv_python_failure_hint, ResolvedVenv,
};

/// Build a Python subprocess for the long-running sidecar (same env as preflight checks).
fn python_sidecar_command(venv: &ResolvedVenv) -> Command {
    build_import_check_command(venv)
}

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

/// Reads the versioned public discovery record from a loopback Chrome broker.
pub fn fetch_chrome_broker_endpoint(port: u16) -> Result<ChromeBrokerEndpoint, BrowserError> {
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
    if payload.get("mode").and_then(|v| v.as_str()) != Some("chrome") {
        return Err(BrowserError {
            message: "discovery port is not serving chrome bridge mode".into(),
            hint: None,
        });
    }
    let number = |name: &str| -> Result<u64, BrowserError> {
        payload
            .get(name)
            .and_then(|v| v.as_u64())
            .ok_or_else(|| BrowserError {
                message: format!("discovery response missing {name}"),
                hint: None,
            })
    };
    let string = |name: &str| -> Result<String, BrowserError> {
        payload
            .get(name)
            .and_then(|v| v.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| BrowserError {
                message: format!("discovery response missing {name}"),
                hint: None,
            })
    };
    Ok(ChromeBrokerEndpoint {
        schema_version: u16::try_from(number("schema_version")?).map_err(|_| BrowserError {
            message: "invalid broker schema_version".into(),
            hint: None,
        })?,
        protocol_version: u16::try_from(number("protocol_version")?).map_err(|_| BrowserError {
            message: "invalid broker protocol_version".into(),
            hint: None,
        })?,
        mode: "chrome".into(),
        ws_url: ws_url.to_string(),
        discovery_url: format!("http://127.0.0.1:{port}/v1/bridge"),
        extension_frame_ws_url: string("extension_frame_ws_url")?,
        broker_pid: u32::try_from(number("broker_pid")?).map_err(|_| BrowserError {
            message: "invalid broker_pid".into(),
            hint: None,
        })?,
        broker_start_id: string("broker_start_id")?,
        broker_features: payload
            .get("broker_features")
            .and_then(|value| value.as_array())
            .ok_or_else(|| BrowserError {
                message: "discovery response missing broker_features".into(),
                hint: None,
            })?
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect(),
        project_root: payload
            .get("project_root")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
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

/// Read the first line from the child's stdout and parse it as a port number.
/// Handles two formats:
///   - Plain port number: `54321\n` (embedded/chrome modes)
///   - JSON readiness object: `{"ready": true, "ws_url": "ws://127.0.0.1:54321", ...}` (winapp mode)
///
/// Uses a background thread so the main thread can poll for child exit and timeout.
fn read_port_from_child_stdout(child: &mut Child, timeout: Duration) -> Result<u16, BrowserError> {
    let mut stdout = child.stdout.take().expect("stdout must be piped");
    let deadline = Instant::now() + timeout;

    let handle = std::thread::spawn(move || -> Option<u16> {
        let mut line = String::new();
        BufReader::new(&mut stdout).read_line(&mut line).ok()?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        // 1) Plain port number (embedded/chrome modes)
        if let Ok(port) = trimmed.parse::<u16>() {
            return Some(port);
        }
        // 2) JSON readiness object (winapp mode): extract port from ws_url
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(ws_url) = v.get("ws_url").and_then(|u| u.as_str()) {
                if let Some(port) = ws_url
                    .rsplit(':')
                    .next()
                    .and_then(|p| p.parse::<u16>().ok())
                {
                    return Some(port);
                }
            }
        }
        None
    });

    loop {
        if handle.is_finished() {
            return match handle.join().unwrap() {
                Some(port) => Ok(port),
                None => Err(BrowserError {
                    message: "Browser sidecar printed invalid port.".into(),
                    hint: None,
                }),
            };
        }
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
                    "Check that Python dependencies (websockets, playwright) are installed.".into(),
                ),
            });
        }
        if Instant::now() >= deadline {
            return Err(BrowserError {
                message: "Timed out waiting for browser sidecar to report port.".into(),
                hint: None,
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn chrome_broker_state_dir() -> Result<PathBuf, BrowserError> {
    let base = dirs::data_local_dir()
        .or_else(dirs::cache_dir)
        .ok_or_else(|| BrowserError {
            message: "Cannot resolve a per-user data directory for the Chrome broker.".into(),
            hint: Some("Set LOCALAPPDATA (Windows) or XDG_DATA_HOME (Linux).".into()),
        })?;
    Ok(base.join("teshi").join("browser-broker"))
}

/// Location of the per-user broker compatibility record.
pub fn chrome_broker_endpoint_path() -> Result<PathBuf, BrowserError> {
    Ok(chrome_broker_state_dir()?.join("endpoint.json"))
}

fn validate_chrome_broker_compatibility(
    endpoint: ChromeBrokerEndpoint,
) -> Result<ChromeBrokerEndpoint, BrowserError> {
    if endpoint.schema_version != BROWSER_AGENT_SCHEMA_VERSION
        || endpoint.protocol_version != BROWSER_BROKER_PROTOCOL_VERSION
    {
        return Err(BrowserError {
            message: format!(
                "Incompatible Teshi Chrome broker is already running (schema {}, protocol {}); this CLI requires schema {}, protocol {}.",
                endpoint.schema_version,
                endpoint.protocol_version,
                BROWSER_AGENT_SCHEMA_VERSION,
                BROWSER_BROKER_PROTOCOL_VERSION
            ),
            hint: Some(
                "The running broker was left untouched. Use the matching Teshi CLI, or stop it explicitly before upgrading."
                    .into(),
            ),
        });
    }
    if !endpoint
        .broker_features
        .iter()
        .any(|feature| feature == "p0.control")
    {
        return Err(BrowserError {
            message: "Incompatible Teshi Chrome broker is already running; this CLI requires broker feature p0.control.".into(),
            hint: Some(
                "The running broker was left untouched. Stop it explicitly before upgrading Teshi."
                    .into(),
            ),
        });
    }
    Ok(endpoint)
}

fn discover_compatible_chrome_broker() -> Result<Option<ChromeBrokerEndpoint>, BrowserError> {
    if !port_is_open(CHROME_DISCOVERY_PORT) {
        return Ok(None);
    }
    let endpoint = fetch_chrome_broker_endpoint(CHROME_DISCOVERY_PORT).map_err(|error| {
        BrowserError {
            message: format!(
                "Port {CHROME_DISCOVERY_PORT} is occupied by a service that is not a compatible Teshi Chrome broker: {}",
                error.message
            ),
            hint: Some(
                "The listener was not terminated. Stop it explicitly or configure the matching Teshi version."
                    .into(),
            ),
        }
    })?;
    validate_chrome_broker_compatibility(endpoint).map(Some)
}

fn persist_user_broker_endpoint(endpoint: &ChromeBrokerEndpoint) -> Result<(), BrowserError> {
    let path = chrome_broker_endpoint_path()?;
    crate::fs_util::write_atomic(&path, endpoint).map_err(|error| BrowserError {
        message: format!(
            "Failed to write broker endpoint {}: {error}",
            path.display()
        ),
        hint: None,
    })
}

fn coordinate_broker_start<D, S>(
    state_dir: &Path,
    readiness_timeout: Duration,
    mut discover: D,
    mut start: S,
) -> Result<ChromeBrokerEndpoint, BrowserError>
where
    D: FnMut() -> Result<Option<ChromeBrokerEndpoint>, BrowserError>,
    S: FnMut() -> Result<(), BrowserError>,
{
    if let Some(endpoint) = discover()? {
        return Ok(endpoint);
    }
    std::fs::create_dir_all(state_dir).map_err(|error| BrowserError {
        message: format!("Failed to create broker state directory: {error}"),
        hint: None,
    })?;
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state_dir.join("startup.lock"))
        .map_err(|error| BrowserError {
            message: format!("Failed to open Chrome broker startup lock: {error}"),
            hint: None,
        })?;
    let mut lock = RwLock::new(lock_file);
    let _startup_guard = lock.write().map_err(|error| BrowserError {
        message: format!("Failed to acquire Chrome broker startup lock: {error}"),
        hint: None,
    })?;
    if let Some(endpoint) = discover()? {
        return Ok(endpoint);
    }
    start()?;
    let deadline = Instant::now() + readiness_timeout;
    while Instant::now() < deadline {
        if let Some(endpoint) = discover()? {
            return Ok(endpoint);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(BrowserError {
        message: "Timed out waiting for the user-session Chrome broker.".into(),
        hint: None,
    })
}

/// Starts or reuses the per-user Chrome broker under an inter-process startup lock.
pub fn ensure_user_chrome_broker(
    project_root: &Path,
    browser_service_script: &Path,
) -> Result<ChromeBrokerEndpoint, BrowserError> {
    let state_dir = chrome_broker_state_dir()?;
    let start = || {
        let venv = resolve_project_venv(project_root).ok_or_else(|| BrowserError {
            message: "Python virtual environment not found or not runnable.".into(),
            hint: Some("Create .venv and install websockets from python/requirements.txt.".into()),
        })?;
        let check = build_import_check_command(&venv)
            .args(["-c", "import websockets"])
            .output()
            .map_err(|error| BrowserError {
                message: format!("Failed to run Python: {error}"),
                hint: Some(format!(
                    "{} -m pip install websockets",
                    venv.python_exe.display()
                )),
            })?;
        if !check.status.success() {
            return Err(BrowserError {
                message: import_check_failed_message(&check, "websockets"),
                hint: Some(format!(
                    "{} -m pip install websockets",
                    venv.python_exe.display()
                )),
            });
        }
        if !browser_service_script.is_file() {
            return Err(BrowserError {
                message: format!(
                    "browser_service.py not found at {}",
                    browser_service_script.display()
                ),
                hint: Some("Reinstall Teshi so its bundled share resources are present.".into()),
            });
        }

        let stderr_path = state_dir.join("broker.stderr.log");
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_path)
            .map_err(|error| BrowserError {
                message: format!("Failed to open broker diagnostic log: {error}"),
                hint: None,
            })?;
        let mut cmd = python_sidecar_command(&venv);
        cmd.arg(browser_service_script).args([
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--mode",
            "chrome",
            "--user-session",
            "--project-root",
            &project_root.to_string_lossy(),
            "--discovery-port",
            &CHROME_DISCOVERY_PORT.to_string(),
        ]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        cmd.spawn().map_err(|error| BrowserError {
            message: format!("Failed to start the user-session Chrome broker: {error}"),
            hint: Some(format!("Inspect {}", stderr_path.display())),
        })?;
        Ok(())
    };
    let endpoint = coordinate_broker_start(
        &state_dir,
        Duration::from_secs(10),
        discover_compatible_chrome_broker,
        start,
    )?;
    persist_user_broker_endpoint(&endpoint)?;
    Ok(endpoint)
}

/// Starts the browser sidecar for the open project in the given mode.
pub async fn start_browser_sidecar(
    rt: Arc<TeshiEngine>,
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

    if mode == BrowserMode::Chrome {
        let endpoint = ensure_user_chrome_broker(&project_root, &rt.browser_service_script)?;
        *rt.sidecar.ws_url.lock().unwrap() = Some(endpoint.ws_url.clone());
        *rt.sidecar.mode.lock().unwrap() = Some(mode);
        *rt.project.browser_active.lock().unwrap() = true;
        rt.events.emit(
            "browser-started",
            serde_json::json!({
                "ws_url": endpoint.ws_url,
                "mode": mode.as_str(),
                "broker_pid": endpoint.broker_pid,
                "broker_start_id": endpoint.broker_start_id,
                "attached": true
            }),
        );
        return Ok(BrowserStartResult {
            ws_url: endpoint.ws_url,
            cdp_endpoint_path: project_root
                .join(".teshi")
                .join("cdp-endpoint.json")
                .to_string_lossy()
                .into_owned(),
            mode: mode.as_str().to_string(),
        });
    }

    let venv = resolve_project_venv(&project_root).ok_or_else(|| {
        let dot_venv = project_root.join(".venv");
        let hint = if dot_venv.is_dir() && crate::venv::is_uv_managed_venv(&dot_venv) {
            "uv managed .venv found but the base Python in pyvenv.cfg is missing. \
             Run `uv python install`, then `uv pip install websockets`."
                .into()
        } else {
            "Create .venv and run: pip install -r python/requirements.txt".into()
        };
        BrowserError {
            message: "Python virtual environment not found or not runnable.".into(),
            hint: Some(hint),
        }
    })?;

    let (import_snippet, import_label, pip_hint) = match mode {
        BrowserMode::Chrome => (
            "import websockets",
            "websockets",
            format!("{} -m pip install websockets", venv.python_exe.display()),
        ),
        BrowserMode::Embedded => (
            "import playwright, websockets",
            "Playwright/websockets",
            format!(
                "{} -m pip install -r python/requirements.txt",
                venv.python_exe.display()
            ),
        ),
        BrowserMode::WinApp => (
            "import websockets",
            "websockets",
            format!(
                "{} -m pip install -r python/requirements.txt",
                venv.python_exe.display()
            ),
        ),
    };

    let check = build_import_check_command(&venv)
        .args(["-c", import_snippet])
        .output()
        .map_err(|e| BrowserError {
            message: format!("Failed to run Python: {e}"),
            hint: Some(pip_hint.clone()),
        })?;
    if !check.status.success() {
        let detail = check_failure_detail(&check);
        return Err(BrowserError {
            message: import_check_failed_message(&check, import_label),
            hint: Some(venv_python_failure_hint(&detail, &pip_hint, &venv.root)),
        });
    }

    if mode == BrowserMode::Embedded {
        let chromium_check = build_import_check_command(&venv)
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

    let script = if mode == BrowserMode::WinApp {
        &rt.winapp_service_script
    } else {
        &rt.browser_service_script
    };
    if !script.is_file() {
        let script_name = if mode == BrowserMode::WinApp {
            "winapp_service.py"
        } else {
            "browser_service.py"
        };
        return Err(BrowserError {
            message: format!("{script_name} not found at {}", script.display()),
            hint: None,
        });
    }

    let cdp_port = if mode == BrowserMode::Embedded {
        pick_port().map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?
    } else {
        0
    };

    let mut cmd = python_sidecar_command(&venv);
    cmd.arg(script).args([
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--mode",
        mode.as_str(),
        "--project-root",
        &project_root.to_string_lossy(),
    ]);
    if mode == BrowserMode::Embedded {
        cmd.args(["--cdp-port", &cdp_port.to_string()]);
        if rt.embedded_no_preview_stream {
            cmd.arg("--no-preview-stream");
        }
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| BrowserError {
            message: format!("Failed to start browser sidecar: {e}"),
            hint: None,
        })?;

    let actual_port = read_port_from_child_stdout(&mut child, Duration::from_secs(10))?;
    let ready = wait_until_ready(&mut child, actual_port);
    if let Err(err) = ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }

    *rt.sidecar.child.lock().unwrap() = Some(child);
    let ws_url = format!("ws://127.0.0.1:{actual_port}");
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
pub async fn stop_browser_sidecar(rt: &TeshiEngine) -> Result<(), String> {
    rt.sidecar.stop().await.map_err(|e| e.to_string())?;
    *rt.project.browser_active.lock().unwrap() = false;
    Ok(())
}

/// Returns persisted recent project paths.
pub fn get_recent_projects() -> Result<Vec<String>, String> {
    crate::app_data::get_recent_projects().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn endpoint(start_id: &str) -> ChromeBrokerEndpoint {
        ChromeBrokerEndpoint {
            schema_version: BROWSER_AGENT_SCHEMA_VERSION,
            protocol_version: BROWSER_BROKER_PROTOCOL_VERSION,
            mode: "chrome".into(),
            ws_url: "ws://127.0.0.1:23456".into(),
            discovery_url: "http://127.0.0.1:17373/v1/bridge".into(),
            extension_frame_ws_url: "ws://127.0.0.1:23456/extension/frames".into(),
            broker_pid: 42,
            broker_start_id: start_id.into(),
            broker_features: vec!["p0.control".into()],
            project_root: "fixture".into(),
        }
    }

    #[test]
    fn first_start_and_existing_broker_reuse_share_start_identity() {
        let temp = tempfile::tempdir().unwrap();
        let started = AtomicBool::new(false);
        let starts = AtomicUsize::new(0);
        let discovered = || {
            Ok(started
                .load(Ordering::SeqCst)
                .then(|| endpoint("first-start")))
        };
        let result =
            coordinate_broker_start(temp.path(), Duration::from_secs(1), discovered, || {
                starts.fetch_add(1, Ordering::SeqCst);
                started.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        assert_eq!(result.broker_start_id, "first-start");
        assert_eq!(starts.load(Ordering::SeqCst), 1);

        let reused =
            coordinate_broker_start(temp.path(), Duration::from_secs(1), discovered, || {
                panic!("compatible broker must be reused")
            })
            .unwrap();
        assert_eq!(reused.broker_start_id, result.broker_start_id);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_start_is_serialized_by_per_user_lock() {
        let temp = tempfile::tempdir().unwrap();
        let state_dir = temp.path().to_path_buf();
        let started = Arc::new(AtomicBool::new(false));
        let starts = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..4 {
            let state_dir = state_dir.clone();
            let started = Arc::clone(&started);
            let starts = Arc::clone(&starts);
            threads.push(std::thread::spawn(move || {
                coordinate_broker_start(
                    &state_dir,
                    Duration::from_secs(2),
                    || {
                        Ok(started
                            .load(Ordering::SeqCst)
                            .then(|| endpoint("concurrent")))
                    },
                    || {
                        starts.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(40));
                        started.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
                .unwrap()
            }));
        }
        let identities: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap().broker_start_id)
            .collect();
        assert!(identities.iter().all(|identity| identity == "concurrent"));
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn incompatible_broker_is_reported_without_starting_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let starts = AtomicUsize::new(0);
        let mut incompatible = endpoint("older");
        incompatible.protocol_version = BROWSER_BROKER_PROTOCOL_VERSION + 1;
        let error = coordinate_broker_start(
            temp.path(),
            Duration::from_millis(10),
            || validate_chrome_broker_compatibility(incompatible.clone()).map(Some),
            || {
                starts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.message.contains("protocol 2"));
        assert!(error.message.contains("requires"));
        assert!(error.hint.unwrap().contains("left untouched"));
        assert_eq!(starts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn broker_without_required_feature_is_incompatible_with_p0_cli() {
        let mut discovered = endpoint("pre-p0-broker");
        discovered.broker_features.clear();
        let error = validate_chrome_broker_compatibility(discovered).unwrap_err();
        assert!(error.message.contains("p0.control"));
        assert!(error.hint.unwrap().contains("left untouched"));
    }

    #[tokio::test]
    async fn desktop_detach_does_not_stop_shared_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let state = SidecarState::new();
        *state.mode.lock().unwrap() = Some(BrowserMode::Chrome);
        state.stop().await.unwrap();
        assert!(std::net::TcpStream::connect(address).is_ok());
    }

    #[tokio::test]
    async fn owned_sidecar_clean_shutdown_clears_process_and_endpoint_state() {
        #[cfg(windows)]
        let child = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let child = Command::new("sh").args(["-c", "sleep 30"]).spawn().unwrap();
        let state = SidecarState::new();
        *state.child.lock().unwrap() = Some(child);
        *state.ws_url.lock().unwrap() = Some("ws://127.0.0.1:1".into());
        *state.mode.lock().unwrap() = Some(BrowserMode::Embedded);
        state.stop().await.unwrap();
        assert!(state.child.lock().unwrap().is_none());
        assert!(state.browser_ws_url().is_none());
        assert!(state.browser_mode().is_none());
    }
}
