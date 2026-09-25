//! Python Playwright sidecar management.

#[cfg(windows)]
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fd_lock::RwLock;
use serde::{Deserialize, Serialize};
use teshi_browser_broker::{
    BrokerIdentityChallenge, BrokerIdentityProof, EndpointRecord, PrivateCredentialStore,
    BROWSER_BROKER_IDENTITY_CHALLENGE_PATH, MAX_TRUSTED_EXTENSION_ORIGINS,
};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_TIMEOUT};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};

use crate::{
    ensure_winapp_runtime, TeshiEngine, BROWSER_AGENT_SCHEMA_VERSION,
    BROWSER_BROKER_PROTOCOL_VERSION,
};

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
const MAX_DISCOVERY_HEADER_BYTES: usize = 16 * 1024;
const MAX_DISCOVERY_BODY_BYTES: usize = 64 * 1024;
const MAX_DISCOVERY_RESPONSE_BYTES: usize = MAX_DISCOVERY_HEADER_BYTES + MAX_DISCOVERY_BODY_BYTES;
const MAX_DISCOVERY_FEATURES: usize = 64;

#[cfg(windows)]
const ELEVATION_CANCELLED_ERROR: u32 = 1223;

/// Handle returned by `ShellExecuteExW` for an elevated sidecar process.
///
/// A standard `std::process::Child` cannot represent a process started through
/// the Windows `runas` shell verb. Keeping the process handle in the daemon
/// lets the daemon stop the elevated executor with the rest of the session.
#[cfg(windows)]
struct ElevatedProcess {
    // Store the opaque Windows handle as an integer so SidecarState remains
    // Send + Sync when shared with Teshi's watcher and terminal threads.
    handle: isize,
}

#[cfg(not(windows))]
struct ElevatedProcess;

#[cfg(windows)]
impl ElevatedProcess {
    fn is_running(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle as HANDLE, 0) == WAIT_TIMEOUT }
    }

    fn terminate_if_running(&self) {
        if self.is_running() {
            unsafe {
                let _ = TerminateProcess(self.handle as HANDLE, 1);
                let _ = WaitForSingleObject(self.handle as HANDLE, 3_000);
            }
        }
    }

    fn stop(self) {
        self.terminate_if_running();
    }
}

#[cfg(windows)]
impl Drop for ElevatedProcess {
    fn drop(&mut self) {
        self.terminate_if_running();
        unsafe {
            let _ = CloseHandle(self.handle as HANDLE);
        }
    }
}

#[cfg(not(windows))]
impl ElevatedProcess {
    fn is_running(&self) -> bool {
        false
    }

    fn stop(self) {}
}

/// Holds the browser sidecar child process and WebSocket URL.
pub struct SidecarState {
    child: Mutex<Option<Child>>,
    elevated_process: Mutex<Option<ElevatedProcess>>,
    ws_url: Mutex<Option<String>>,
    mode: Mutex<Option<BrowserMode>>,
    elevated: Mutex<bool>,
    lifecycle_lock: tokio::sync::Mutex<()>,
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
            elevated_process: Mutex::new(None),
            ws_url: Mutex::new(None),
            mode: Mutex::new(None),
            elevated: Mutex::new(false),
            lifecycle_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Stops the sidecar process if running.
    pub async fn stop(&self) -> Result<()> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;
        self.stop_inner()
    }

    fn stop_inner(&self) -> Result<()> {
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

        if let Some(process) = self.elevated_process.lock().unwrap().take() {
            process.stop();
        }

        *self.ws_url.lock().unwrap() = None;
        *self.mode.lock().unwrap() = None;
        *self.elevated.lock().unwrap() = false;
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

    /// Returns true when the active WinApp executor was started elevated.
    pub fn is_elevated(&self) -> bool {
        *self.elevated.lock().unwrap()
    }

    /// Returns true when the owned sidecar child still appears to be running.
    pub fn child_is_running(&self) -> bool {
        let child_running = self
            .child
            .lock()
            .unwrap()
            .as_mut()
            .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_none()));
        let elevated_running = self
            .elevated_process
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(ElevatedProcess::is_running);
        child_running || elevated_running
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

fn ws_url_to_addr(ws_url: &str) -> Result<std::net::SocketAddr> {
    let stripped = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))
        .ok_or_else(|| anyhow::anyhow!("unsupported ws_url scheme"))?;
    let (host, rest) = stripped
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("ws_url missing port"))?;
    let port: u16 = rest
        .split('/')
        .next()
        .unwrap_or(rest)
        .parse()
        .context("parse ws_url port")?;
    Ok(format!("{host}:{port}").parse()?)
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
        .set_read_timeout(Some(Duration::from_millis(750)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_millis(750))))
        .map_err(|error| BrowserError {
            message: format!("cannot bound broker discovery I/O: {error}"),
            hint: None,
        })?;
    stream
        .write_all(
            format!(
                "GET /v1/bridge HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?;
    let mut buf = Vec::new();
    stream
        .take((MAX_DISCOVERY_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut buf)
        .map_err(|e| BrowserError {
            message: format!("failed to read bounded broker discovery response: {e}"),
            hint: None,
        })?;
    if buf.len() > MAX_DISCOVERY_RESPONSE_BYTES {
        return Err(BrowserError {
            message: "discovery response exceeds its configured size limit".into(),
            hint: None,
        });
    }
    let header_end = buf
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| BrowserError {
            message: "discovery response has no complete HTTP header block".into(),
            hint: None,
        })?;
    if header_end > MAX_DISCOVERY_HEADER_BYTES {
        return Err(BrowserError {
            message: "discovery response headers exceed their configured limit".into(),
            hint: None,
        });
    }
    let headers = std::str::from_utf8(&buf[..header_end]).map_err(|_| BrowserError {
        message: "discovery response headers are not valid HTTP text".into(),
        hint: None,
    })?;
    let mut lines = headers.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut status_parts = status_line.split_ascii_whitespace();
    if !matches!(status_parts.next(), Some("HTTP/1.0" | "HTTP/1.1"))
        || status_parts.next() != Some("200")
    {
        return Err(BrowserError {
            message: "discovery endpoint did not return HTTP 200".into(),
            hint: None,
        });
    }
    let mut content_length = None;
    let mut content_type = None;
    let mut transfer_encoding = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(BrowserError {
                message: "discovery response contains a malformed HTTP header".into(),
                hint: None,
            });
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err(BrowserError {
                        message: "discovery response contains duplicate Content-Length headers"
                            .into(),
                        hint: None,
                    });
                }
                content_length = Some(value.trim().parse::<usize>().map_err(|_| BrowserError {
                    message: "discovery response has an invalid Content-Length".into(),
                    hint: None,
                })?);
            }
            "content-type" => content_type = Some(value.trim().to_ascii_lowercase()),
            "transfer-encoding" => transfer_encoding = Some(value.trim().to_ascii_lowercase()),
            _ => {}
        }
    }
    let body_start = header_end + 4;
    let body = &buf[body_start..];
    if body.len() > MAX_DISCOVERY_BODY_BYTES
        || content_length.is_some_and(|length| length > MAX_DISCOVERY_BODY_BYTES)
    {
        return Err(BrowserError {
            message: "discovery response body exceeds its configured size limit".into(),
            hint: None,
        });
    }
    if content_length.is_some_and(|length| length != body.len())
        || transfer_encoding.is_some()
        || content_type
            .as_deref()
            .is_none_or(|value| !value.starts_with("application/json"))
    {
        return Err(BrowserError {
            message: "discovery response body or content headers are invalid".into(),
            hint: None,
        });
    }
    let payload: serde_json::Value = serde_json::from_slice(body).map_err(|e| BrowserError {
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
    let extension_frame_ws_url = string("extension_frame_ws_url")?;
    validate_discovery_ws_urls(ws_url, &extension_frame_ws_url)?;
    let schema_version = u16::try_from(number("schema_version")?).map_err(|_| BrowserError {
        message: "invalid broker schema_version".into(),
        hint: None,
    })?;
    let protocol_version =
        u16::try_from(number("protocol_version")?).map_err(|_| BrowserError {
            message: "invalid broker protocol_version".into(),
            hint: None,
        })?;
    let broker_pid = u32::try_from(number("broker_pid")?).map_err(|_| BrowserError {
        message: "invalid broker_pid".into(),
        hint: None,
    })?;
    if schema_version == 0 || protocol_version == 0 || broker_pid == 0 {
        return Err(BrowserError {
            message: "discovery returned an invalid broker identity".into(),
            hint: None,
        });
    }
    Ok(ChromeBrokerEndpoint {
        schema_version,
        protocol_version,
        mode: "chrome".into(),
        ws_url: ws_url.to_string(),
        discovery_url: format!("http://127.0.0.1:{port}/v1/bridge"),
        extension_frame_ws_url,
        broker_pid,
        broker_start_id: {
            let start_id = string("broker_start_id")?;
            if start_id.len() > 128 {
                return Err(BrowserError {
                    message: "discovery broker_start_id exceeds its size limit".into(),
                    hint: None,
                });
            }
            start_id
        },
        broker_features: {
            let features = payload
                .get("broker_features")
                .and_then(|value| value.as_array())
                .ok_or_else(|| BrowserError {
                    message: "discovery response missing broker_features".into(),
                    hint: None,
                })?;
            if features.len() > MAX_DISCOVERY_FEATURES {
                return Err(BrowserError {
                    message: "discovery broker_features exceeds its count limit".into(),
                    hint: None,
                });
            }
            features
                .iter()
                .map(|value| {
                    let feature = value.as_str().ok_or_else(|| BrowserError {
                        message: "discovery broker_features contains a non-string value".into(),
                        hint: None,
                    })?;
                    if feature.is_empty() || feature.len() > 128 {
                        return Err(BrowserError {
                            message: "discovery broker feature name is invalid".into(),
                            hint: None,
                        });
                    }
                    Ok(feature.to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?
        },
        project_root: payload
            .get("project_root")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

fn validate_discovery_ws_urls(
    ws_url: &str,
    extension_frame_ws_url: &str,
) -> Result<(), BrowserError> {
    let invalid_endpoint = || BrowserError {
        message: "discovery returned a non-loopback or malformed WebSocket endpoint".into(),
        hint: None,
    };
    let command = url::Url::parse(ws_url).map_err(|_| invalid_endpoint())?;
    let frames = url::Url::parse(extension_frame_ws_url).map_err(|_| invalid_endpoint())?;
    let is_local_ws = |url: &url::Url| {
        url.scheme() == "ws"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    };
    if !is_local_ws(&command)
        || !is_local_ws(&frames)
        || command.path() != "/"
        || frames.path() != "/extension/frames"
        || command.host_str() != frames.host_str()
        || command.port() != frames.port()
        || command.query() != frames.query()
        || !valid_broker_ws_query(command.query())
    {
        return Err(invalid_endpoint());
    }
    Ok(())
}

fn valid_broker_ws_query(query: Option<&str>) -> bool {
    let Some(query) = query else {
        return true;
    };
    let Some(token) = query.strip_prefix("token=") else {
        return false;
    };
    token.len() >= 32
        && token.len() <= 512
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
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
                return ws_url_to_addr(ws_url).ok().map(|address| address.port());
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

/// Return the sidecar WebSocket coordinate without query credentials for
/// runtime events. The full authenticated URL remains available only through
/// the direct start response and the private project endpoint file.
fn public_event_ws_url(ws_url: &str) -> String {
    ws_url.split(['?', '#']).next().unwrap_or(ws_url).to_owned()
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

/// Confirm that a Rust listener matches both the private process generation and
/// the bearer secret written by that exact process. Public metadata alone is not
/// sufficient to reuse a listener on the fixed discovery port.
fn validate_rust_transport_broker(
    endpoint: &ChromeBrokerEndpoint,
    credential_store: &PrivateCredentialStore,
    expected_extension_origins: Option<&[String]>,
) -> Result<(), BrowserError> {
    if !endpoint
        .broker_features
        .iter()
        .any(|feature| feature == "transport.v1")
    {
        return Err(BrowserError {
            message: "Rust Chrome broker does not advertise the compatible transport.v1 feature."
                .into(),
            hint: Some("The running listener was left untouched.".into()),
        });
    }
    let public_record = EndpointRecord {
        schema_version: endpoint.schema_version,
        protocol_version: endpoint.protocol_version,
        mode: endpoint.mode.clone(),
        ws_url: endpoint.ws_url.clone(),
        discovery_url: endpoint.discovery_url.clone(),
        extension_frame_ws_url: endpoint.extension_frame_ws_url.clone(),
        broker_pid: endpoint.broker_pid,
        broker_start_id: endpoint.broker_start_id.clone(),
        broker_features: endpoint.broker_features.clone(),
        bridge: "rust".into(),
    };
    let credential = credential_store
        .read_for_endpoint(&public_record)
        .map_err(|error| BrowserError {
            message: format!(
                "Rust Chrome broker identity did not match its private credential: {}",
                error.message
            ),
            hint: Some("The listener was left untouched; do not reuse stale broker state.".into()),
        })?;
    if let Some(expected) = expected_extension_origins {
        let mut expected = expected.to_vec();
        expected.sort();
        if credential.trusted_extension_origins() != expected {
            return Err(BrowserError {
                message: "Rust Chrome broker is paired with a different extension identity set."
                    .into(),
                hint: Some(
                    "The listener was left untouched; review the configured extension IDs.".into(),
                ),
            });
        }
    }
    verify_rust_broker_identity(endpoint, &credential, &public_record)
}

fn verify_rust_broker_identity(
    endpoint: &ChromeBrokerEndpoint,
    credential: &teshi_browser_broker::PrivateBrokerCredential,
    public_record: &EndpointRecord,
) -> Result<(), BrowserError> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use uuid::Uuid;

    validate_discovery_ws_urls(&endpoint.ws_url, &endpoint.extension_frame_ws_url)?;
    if url::Url::parse(&endpoint.ws_url).is_ok_and(|url| url.query().is_some())
        || url::Url::parse(&endpoint.extension_frame_ws_url).is_ok_and(|url| url.query().is_some())
    {
        return Err(BrowserError {
            message: "Rust Chrome broker public endpoint must not contain a token".into(),
            hint: None,
        });
    }
    let mut identity_url = url::Url::parse(&endpoint.discovery_url).map_err(|_| BrowserError {
        message: "Rust Chrome broker discovery URL is malformed".into(),
        hint: None,
    })?;
    if identity_url.scheme() != "http"
        || identity_url.host_str() != Some("127.0.0.1")
        || identity_url.path() != "/v1/bridge"
        || identity_url.query().is_some()
        || identity_url.fragment().is_some()
        || !identity_url.username().is_empty()
        || identity_url.password().is_some()
    {
        return Err(BrowserError {
            message: "Rust Chrome broker identity endpoint is not a plain loopback URL".into(),
            hint: None,
        });
    }
    identity_url.set_path(BROWSER_BROKER_IDENTITY_CHALLENGE_PATH);
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    nonce_bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let challenge = BrokerIdentityChallenge {
        nonce: URL_SAFE_NO_PAD.encode(nonce_bytes),
    };
    let port = identity_url.port().ok_or_else(|| BrowserError {
        message: "Rust Chrome broker discovery URL is missing its listener port".into(),
        hint: None,
    })?;
    let address: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_millis(500)).map_err(|_| {
            BrowserError {
                message: "Rust Chrome broker did not answer its identity challenge".into(),
                hint: Some("The listener was left untouched.".into()),
            }
        })?;
    stream
        .set_read_timeout(Some(Duration::from_millis(750)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_millis(750))))
        .map_err(|_| BrowserError {
            message: "cannot bound Rust broker identity verification I/O".into(),
            hint: None,
        })?;
    let request_body = serde_json::to_vec(&challenge).map_err(|_| BrowserError {
        message: "cannot serialize Rust broker identity challenge".into(),
        hint: None,
    })?;
    write!(
        stream,
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        identity_url.path(),
        request_body.len()
    )
    .and_then(|()| stream.write_all(&request_body))
    .map_err(|_| BrowserError {
        message: "cannot send bounded Rust broker identity challenge".into(),
        hint: Some("The listener was left untouched.".into()),
    })?;
    let mut response = Vec::new();
    stream
        .take((MAX_DISCOVERY_HEADER_BYTES + 4 * 1024 + 1) as u64)
        .read_to_end(&mut response)
        .map_err(|_| BrowserError {
            message: "cannot read bounded Rust Chrome broker identity response".into(),
            hint: Some("The listener was left untouched.".into()),
        })?;
    if response.len() > MAX_DISCOVERY_HEADER_BYTES + 4 * 1024 {
        return Err(BrowserError {
            message: "Rust Chrome broker identity response exceeds its size limit".into(),
            hint: Some("The listener was left untouched.".into()),
        });
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .filter(|header_end| *header_end <= MAX_DISCOVERY_HEADER_BYTES)
        .ok_or_else(|| BrowserError {
            message: "Rust Chrome broker identity response has malformed headers".into(),
            hint: Some("The listener was left untouched.".into()),
        })?;
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| BrowserError {
        message: "Rust Chrome broker identity response headers are malformed".into(),
        hint: Some("The listener was left untouched.".into()),
    })?;
    let mut lines = headers.split("\r\n");
    if lines.next().is_none_or(|line| {
        let mut parts = line.split_ascii_whitespace();
        !matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1")) || parts.next() != Some("200")
    }) {
        return Err(BrowserError {
            message: "Rust Chrome broker identity response is not successful".into(),
            hint: Some("The listener was left untouched.".into()),
        });
    }
    let mut content_length = None;
    let mut content_type = None;
    let mut transfer_encoding = false;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(|| BrowserError {
            message: "Rust Chrome broker identity response has malformed headers".into(),
            hint: Some("The listener was left untouched.".into()),
        })?;
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err(BrowserError {
                        message: "Rust Chrome broker identity response has duplicate lengths"
                            .into(),
                        hint: Some("The listener was left untouched.".into()),
                    });
                }
                content_length = Some(value.trim().parse::<usize>().map_err(|_| BrowserError {
                    message: "Rust Chrome broker identity response length is invalid".into(),
                    hint: Some("The listener was left untouched.".into()),
                })?);
            }
            "content-type" => content_type = Some(value.trim().to_ascii_lowercase()),
            "transfer-encoding" => transfer_encoding = true,
            _ => {}
        }
    }
    let body = &response[header_end + 4..];
    if body.len() > 4 * 1024
        || content_length != Some(body.len())
        || transfer_encoding
        || content_type
            .as_deref()
            .is_none_or(|value| !value.starts_with("application/json"))
    {
        return Err(BrowserError {
            message: "Rust Chrome broker identity response is invalid".into(),
            hint: Some("The listener was left untouched.".into()),
        });
    }
    let proof: BrokerIdentityProof = serde_json::from_slice(body).map_err(|_| BrowserError {
        message: "Rust Chrome broker identity response is malformed".into(),
        hint: Some("The listener was left untouched.".into()),
    })?;
    credential
        .verify_identity_proof(&challenge, &proof, public_record)
        .map_err(|_| BrowserError {
            message: "Rust Chrome broker did not prove possession of its private credential".into(),
            hint: Some("The listener was left untouched.".into()),
        })
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
    if endpoint
        .broker_features
        .iter()
        .any(|feature| feature == "transport.v1")
    {
        let credential_store = PrivateCredentialStore::new(chrome_broker_state_dir()?);
        validate_rust_transport_broker(&endpoint, &credential_store, None)?;
    }
    validate_chrome_broker_compatibility(endpoint).map(Some)
}

fn discover_rust_transport_broker_at(
    port: u16,
    state_dir: &Path,
    trusted_extension_origins: &[String],
) -> Result<Option<ChromeBrokerEndpoint>, BrowserError> {
    if !port_is_open(port) {
        return Ok(None);
    }
    let endpoint = fetch_chrome_broker_endpoint(port).map_err(|error| BrowserError {
        message: format!(
            "Port {port} is occupied by a listener that is not a verifiable Rust Chrome transport broker: {}",
            error.message
        ),
        hint: Some("The listener was left untouched.".into()),
    })?;
    if endpoint.schema_version != BROWSER_AGENT_SCHEMA_VERSION
        || endpoint.protocol_version != BROWSER_BROKER_PROTOCOL_VERSION
    {
        return Err(BrowserError {
            message: "The Rust Chrome transport broker protocol is incompatible.".into(),
            hint: Some("The existing listener was left untouched.".into()),
        });
    }
    let credential_store = PrivateCredentialStore::new(state_dir);
    validate_rust_transport_broker(
        &endpoint,
        &credential_store,
        Some(trusted_extension_origins),
    )?;
    Ok(Some(endpoint))
}

fn prepare_private_broker_state_dir(path: &Path) -> Result<(), BrowserError> {
    std::fs::create_dir_all(path).map_err(|error| BrowserError {
        message: format!("cannot create Rust broker state directory: {error}"),
        hint: None,
    })?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| BrowserError {
        message: format!("cannot inspect Rust broker state directory: {error}"),
        hint: None,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BrowserError {
            message: "Rust broker state path must be a real directory.".into(),
            hint: Some("The existing path was left untouched.".into()),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| BrowserError {
                message: format!("cannot restrict Rust broker state directory: {error}"),
                hint: None,
            },
        )?;
    }
    Ok(())
}

fn open_private_broker_diagnostic_log(path: &Path) -> Result<std::fs::File, BrowserError> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(BrowserError {
                message: "Rust broker diagnostic path must be a regular file.".into(),
                hint: Some("The existing path was left untouched.".into()),
            });
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|error| BrowserError {
        message: format!("cannot open Rust broker diagnostic log: {error}"),
        hint: None,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| BrowserError {
                message: format!("cannot restrict Rust broker diagnostic log: {error}"),
                hint: None,
            })?;
    }
    Ok(file)
}

/// Start or reuse the internal Rust transport process without changing the
/// production Chrome runtime selector. The caller must supply exact paired
/// extension origins; no wildcard or arbitrary-ID fallback is provided.
#[doc(hidden)]
fn ensure_rust_transport_broker_at(
    state_dir: &Path,
    teshi_executable: &Path,
    trusted_extension_origins: &[String],
    discovery_port: u16,
    readiness_timeout: Duration,
) -> Result<ChromeBrokerEndpoint, BrowserError> {
    if discovery_port == 0
        || trusted_extension_origins.is_empty()
        || trusted_extension_origins.len() > MAX_TRUSTED_EXTENSION_ORIGINS
        || trusted_extension_origins
            .iter()
            .any(|origin| !valid_chrome_extension_origin(origin))
        || trusted_extension_origins
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != trusted_extension_origins.len()
    {
        return Err(BrowserError {
            message: "Rust transport startup requires a fixed port and a bounded set of unique exact Chrome extension origins.".into(),
            hint: None,
        });
    }
    if !teshi_executable.is_file() {
        return Err(BrowserError {
            message: format!(
                "Teshi CLI executable is unavailable at {}",
                teshi_executable.display()
            ),
            hint: Some(
                "Install teshi beside the application that starts Chrome automation.".into(),
            ),
        });
    }

    let state_dir = state_dir.to_path_buf();
    prepare_private_broker_state_dir(&state_dir)?;
    let executable = teshi_executable.to_path_buf();
    let stderr_path = state_dir.join("broker.stderr.log");
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let mut discover = {
        let child_slot = Arc::clone(&child_slot);
        let state_dir = state_dir.clone();
        let stderr_path = stderr_path.clone();
        move || {
            let child_status = {
                let mut slot = child_slot.lock().unwrap();
                match slot.as_mut() {
                    Some(child) => child.try_wait().map_err(|error| BrowserError {
                        message: format!("cannot inspect the Rust broker child process: {error}"),
                        hint: Some("The listener was not terminated.".into()),
                    })?,
                    None => None,
                }
            };
            if let Some(status) = child_status {
                return Err(BrowserError {
                    message: format!("Rust browser broker exited during startup ({status})."),
                    hint: Some(format!(
                        "Inspect {} for startup diagnostics.",
                        stderr_path.display()
                    )),
                });
            }
            discover_rust_transport_broker_at(discovery_port, &state_dir, trusted_extension_origins)
        }
    };
    let mut start = {
        let child_slot = Arc::clone(&child_slot);
        let state_dir = state_dir.clone();
        let executable = executable.clone();
        let stderr_path = stderr_path.clone();
        move || {
            let stderr = open_private_broker_diagnostic_log(&stderr_path)?;
            let mut command = Command::new(&executable);
            command
                .arg("--browser-broker-internal")
                .arg("--state-dir")
                .arg(&state_dir)
                .arg("--discovery-port")
                .arg(discovery_port.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::from(stderr));
            for origin in trusted_extension_origins {
                command.arg("--trusted-extension-origin").arg(origin);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const DETACHED_PROCESS: u32 = 0x00000008;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            }
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            let child = command.spawn().map_err(|error| BrowserError {
                message: format!("cannot start the internal Rust Chrome broker: {error}"),
                hint: Some(format!(
                    "Inspect {} for startup diagnostics.",
                    stderr_path.display()
                )),
            })?;
            *child_slot.lock().unwrap() = Some(child);
            Ok(())
        }
    };
    let endpoint =
        coordinate_broker_start(&state_dir, readiness_timeout, &mut discover, &mut start)?;
    persist_user_broker_endpoint(&endpoint)?;
    Ok(endpoint)
}

fn valid_chrome_extension_origin(origin: &str) -> bool {
    let Some(id) = origin.strip_prefix("chrome-extension://") else {
        return false;
    };
    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
}

fn resolve_teshi_cli_executable(current_executable: &Path) -> Result<PathBuf, BrowserError> {
    let is_teshi = current_executable
        .file_stem()
        .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case("teshi"));
    if is_teshi && current_executable.is_file() {
        return Ok(current_executable.to_path_buf());
    }
    let sibling_name = if cfg!(windows) { "teshi.exe" } else { "teshi" };
    let sibling = current_executable.with_file_name(sibling_name);
    if sibling.is_file() {
        return Ok(sibling);
    }
    Err(BrowserError {
        message: format!(
            "cannot locate the Teshi CLI broker host beside {}",
            current_executable.display()
        ),
        hint: Some(format!("Install the Teshi CLI at {}.", sibling.display())),
    })
}

/// Starts or reuses the transport-only Rust broker for the current OS user.
///
/// This is an internal migration bootstrap, not the production Chrome runtime
/// selector: the current implementation intentionally advertises no `p0.control`
/// feature, so normal Chrome automation continues to use the existing backend.
#[doc(hidden)]
pub fn ensure_user_rust_transport_broker_transport_only(
    trusted_extension_origins: &[String],
) -> Result<ChromeBrokerEndpoint, BrowserError> {
    let current_executable = std::env::current_exe().map_err(|error| BrowserError {
        message: format!("cannot locate the Teshi application executable: {error}"),
        hint: None,
    })?;
    let teshi_executable = resolve_teshi_cli_executable(&current_executable)?;
    ensure_rust_transport_broker_at(
        &chrome_broker_state_dir()?,
        &teshi_executable,
        trusted_extension_origins,
        CHROME_DISCOVERY_PORT,
        Duration::from_secs(10),
    )
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
    start_browser_sidecar_with_options(rt, mode, false).await
}

/// Starts a browser sidecar, optionally giving only the WinApp executor a
/// high-integrity token through the Windows `runas` shell verb.
///
/// The daemon, CLI, and all other sidecar modes remain at their existing
/// integrity level. `elevated` is meaningful only for `BrowserMode::WinApp`.
pub async fn start_browser_sidecar_with_options(
    rt: Arc<TeshiEngine>,
    mode: BrowserMode,
    elevated: bool,
) -> Result<BrowserStartResult, BrowserError> {
    if elevated && mode != BrowserMode::WinApp {
        return Err(BrowserError {
            message: "elevation is only supported for the WinApp executor.".into(),
            hint: None,
        });
    }
    let _lifecycle_guard = rt.sidecar.lifecycle_lock.lock().await;

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

    if mode != BrowserMode::Chrome
        && rt.sidecar.browser_mode() == Some(mode)
        && rt.sidecar.is_elevated() == elevated
        && rt.sidecar.child_is_running()
    {
        if let Some(ws_url) = rt.sidecar.browser_ws_url() {
            if let Ok(addr) = ws_url_to_addr(&ws_url) {
                if port_is_open(addr.port()) {
                    return Ok(BrowserStartResult {
                        ws_url,
                        cdp_endpoint_path: project_root
                            .join(".teshi")
                            .join("cdp-endpoint.json")
                            .to_string_lossy()
                            .into_owned(),
                        mode: mode.as_str().to_string(),
                    });
                }
            }
        }
    }

    rt.sidecar.stop_inner().ok();
    *rt.project.browser_active.lock().unwrap() = false;

    if mode == BrowserMode::Chrome {
        let endpoint = ensure_user_chrome_broker(&project_root, &rt.browser_service_script)?;
        *rt.sidecar.ws_url.lock().unwrap() = Some(endpoint.ws_url.clone());
        *rt.sidecar.mode.lock().unwrap() = Some(mode);
        *rt.project.browser_active.lock().unwrap() = true;
        rt.events.emit(
            "browser-started",
            serde_json::json!({
                "ws_url": public_event_ws_url(&endpoint.ws_url),
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

    let managed_winapp = if mode == BrowserMode::WinApp {
        Some(ensure_winapp_runtime().map_err(|error| BrowserError {
            message: "Failed to prepare Teshi WinApp runtime.".into(),
            hint: Some(error.to_string()),
        })?)
    } else {
        None
    };

    let venv = if managed_winapp.is_none() {
        Some(resolve_project_venv(&project_root).ok_or_else(|| {
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
        })?)
    } else {
        None
    };

    if let Some(venv) = &venv {
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
            BrowserMode::WinApp => unreachable!("WinApp uses Teshi managed runtime"),
        };

        let check = build_import_check_command(venv)
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
            let chromium_check = build_import_check_command(venv)
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
    }

    let script = managed_winapp
        .as_ref()
        .map(|runtime| &runtime.service_script)
        .unwrap_or(&rt.browser_service_script);
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

    let sidecar_port = if elevated {
        pick_port().map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?
    } else {
        0
    };
    let auth_token =
        (mode == BrowserMode::WinApp).then(|| format!("tk_{}", uuid::Uuid::new_v4().simple()));
    let cdp_port = if mode == BrowserMode::Embedded {
        pick_port().map_err(|e| BrowserError {
            message: e.to_string(),
            hint: None,
        })?
    } else {
        0
    };

    let mut cmd = if let Some(runtime) = &managed_winapp {
        Command::new(&runtime.python_exe)
    } else {
        python_sidecar_command(venv.as_ref().expect("venv for non-WinApp sidecar"))
    };
    let sidecar_port_arg = sidecar_port.to_string();
    cmd.arg(script).args([
        "--host",
        "127.0.0.1",
        "--port",
        &sidecar_port_arg,
        "--mode",
        mode.as_str(),
        "--project-root",
        &project_root.to_string_lossy(),
    ]);
    if let Some(auth_token) = auth_token.as_deref() {
        cmd.args(["--auth-token", auth_token]);
    }
    if mode == BrowserMode::Embedded {
        cmd.args(["--cdp-port", &cdp_port.to_string()]);
        if rt.embedded_no_preview_stream {
            cmd.arg("--no-preview-stream");
        }
    }

    let (ws_url, child, elevated_process) = if elevated {
        #[cfg(windows)]
        {
            let process = spawn_elevated_winapp_service(
                managed_winapp
                    .as_ref()
                    .expect("elevated WinApp uses managed runtime"),
                script,
                &project_root,
                sidecar_port,
                auth_token.as_deref().ok_or_else(|| BrowserError {
                    message: "WinApp sidecar authentication token was not created.".into(),
                    hint: None,
                })?,
            )?;
            if let Err(error) = wait_until_elevated_ready(&process, sidecar_port) {
                process.stop();
                return Err(error);
            }
            (
                format!(
                    "ws://127.0.0.1:{sidecar_port}/?token={}",
                    auth_token.as_deref().ok_or_else(|| BrowserError {
                        message: "WinApp sidecar authentication token was not created.".into(),
                        hint: None,
                    })?
                ),
                None,
                Some(process),
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (&managed_winapp, script, &project_root, sidecar_port);
            return Err(BrowserError {
                message: "elevated WinApp sessions are only supported on Windows.".into(),
                hint: None,
            });
        }
    } else {
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| BrowserError {
                message: format!("Failed to start browser sidecar: {e}"),
                hint: None,
            })?;

        let actual_port = match read_port_from_child_stdout(&mut child, Duration::from_secs(10)) {
            Ok(port) => port,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        if let Err(error) = wait_until_ready(&mut child, actual_port) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        (
            match auth_token.as_deref() {
                Some(token) => format!("ws://127.0.0.1:{actual_port}/?token={token}"),
                None => format!("ws://127.0.0.1:{actual_port}"),
            },
            Some(child),
            None,
        )
    };

    *rt.sidecar.child.lock().unwrap() = child;
    *rt.sidecar.elevated_process.lock().unwrap() = elevated_process;
    *rt.sidecar.ws_url.lock().unwrap() = Some(ws_url.clone());
    *rt.sidecar.mode.lock().unwrap() = Some(mode);
    *rt.sidecar.elevated.lock().unwrap() = elevated;
    *rt.project.browser_active.lock().unwrap() = true;

    let mut event = serde_json::json!({
        "ws_url": public_event_ws_url(&ws_url),
        "mode": mode.as_str(),
    });
    if elevated {
        event["elevated"] = serde_json::json!(true);
    }
    rt.events.emit("browser-started", event);

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

#[cfg(windows)]
fn elevation_error(error_code: u32) -> BrowserError {
    if error_code == ELEVATION_CANCELLED_ERROR {
        return BrowserError {
            message: "WinApp elevation was cancelled or denied by the user.".into(),
            hint: Some(
                "Confirm the Windows UAC prompt to create an elevated WinApp session.".into(),
            ),
        };
    }
    BrowserError {
        message: format!(
            "Windows could not start the elevated WinApp executor (error {error_code})."
        ),
        hint: Some(
            "The WinApp session was not elevated; retry and confirm the Windows UAC prompt.".into(),
        ),
    }
}

#[cfg(windows)]
fn quote_windows_arg(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn spawn_elevated_winapp_service(
    runtime: &crate::ManagedRuntime,
    script: &Path,
    project_root: &Path,
    port: u16,
    auth_token: &str,
) -> Result<ElevatedProcess, BrowserError> {
    let arguments = [
        script.to_string_lossy().into_owned(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--mode".into(),
        "winapp".into(),
        "--project-root".into(),
        project_root.to_string_lossy().into_owned(),
        "--auth-token".into(),
        auth_token.into(),
    ];
    let parameters = arguments
        .iter()
        .map(|argument| quote_windows_arg(OsStr::new(argument)))
        .collect::<Vec<_>>()
        .join(" ");
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = runtime
        .python_exe
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let parameters: Vec<u16> = parameters
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let directory: Vec<u16> = project_root
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut execute_info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    execute_info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    execute_info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    execute_info.lpVerb = verb.as_ptr();
    execute_info.lpFile = file.as_ptr();
    execute_info.lpParameters = parameters.as_ptr();
    execute_info.lpDirectory = directory.as_ptr();
    // Keep the executor's console hidden. The UAC consent UI is owned by
    // Windows and remains visible to the user.
    execute_info.nShow = 0;

    let started = unsafe { ShellExecuteExW(&mut execute_info) };
    if started == 0 {
        let error_code = unsafe { GetLastError() };
        return Err(elevation_error(error_code));
    }
    if execute_info.hProcess.is_null() {
        return Err(BrowserError {
            message: "Windows started the elevated WinApp executor without a process handle."
                .into(),
            hint: Some("The elevated session cannot be lifecycle-managed safely.".into()),
        });
    }
    Ok(ElevatedProcess {
        handle: execute_info.hProcess as isize,
    })
}

#[cfg(windows)]
fn wait_until_elevated_ready(process: &ElevatedProcess, port: u16) -> Result<(), BrowserError> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if !process.is_running() {
            return Err(BrowserError {
                message: "Elevated WinApp executor exited before it became ready.".into(),
                hint: Some(
                    "Check the WinApp runtime installation and its diagnostic output.".into(),
                ),
            });
        }
        if port_is_open(port) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(BrowserError {
                message: "Elevated WinApp executor did not become ready in time.".into(),
                hint: Some("The elevated WinApp service failed to open its WebSocket port.".into()),
            });
        }
        std::thread::sleep(Duration::from_millis(200));
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
    use teshi_browser_broker::{BrokerRuntime, BrokerServerConfig};

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
    fn discovery_websocket_urls_must_remain_local_and_correlated() {
        let local = "ws://127.0.0.1:43123/";
        let frames = "ws://127.0.0.1:43123/extension/frames";
        assert!(validate_discovery_ws_urls(local, frames).is_ok());
        assert!(validate_discovery_ws_urls(
            "ws://127.0.0.1:43123/?token=abcdefghijklmnopqrstuvwxyz0123456789",
            "ws://127.0.0.1:43123/extension/frames?token=abcdefghijklmnopqrstuvwxyz0123456789"
        )
        .is_ok());
        assert!(validate_discovery_ws_urls("ws://evil.example:43123/", frames).is_err());
        assert!(
            validate_discovery_ws_urls(local, "ws://127.0.0.1:43124/extension/frames").is_err()
        );
        assert!(validate_discovery_ws_urls(
            "ws://127.0.0.1:43123/?token=abcdefghijklmnopqrstuvwxyz0123456789",
            frames
        )
        .is_err());
        assert!(validate_discovery_ws_urls(
            "ws://127.0.0.1:43123/?token=bad&token=duplicate",
            "ws://127.0.0.1:43123/extension/frames?token=bad&token=duplicate"
        )
        .is_err());
    }

    #[test]
    fn rust_broker_state_and_diagnostic_log_are_private_and_reject_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let state_dir = temp.path().join("user-state");
        prepare_private_broker_state_dir(&state_dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&state_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }

        let log_path = state_dir.join("broker.stderr.log");
        drop(open_private_broker_diagnostic_log(&log_path).unwrap());
        assert!(std::fs::metadata(&log_path).unwrap().is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&log_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = temp.path().join("external-log-target");
            std::fs::write(&target, b"keep").unwrap();
            let symlink_path = state_dir.join("symlink.log");
            symlink(&target, &symlink_path).unwrap();
            assert!(open_private_broker_diagnostic_log(&symlink_path).is_err());
            assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn engine_discovery_client_interoperates_with_rust_broker_listener() {
        let mut config =
            BrokerServerConfig::new("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        config.broker_features = vec!["transport.v1".into()];
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let temp = tempfile::tempdir().unwrap();
        let credentials = PrivateCredentialStore::new(temp.path().join("private-state"));
        runtime.persist_private_credential(&credentials).unwrap();
        let public = runtime.endpoint_record();
        let port = public
            .discovery_url
            .strip_prefix("http://127.0.0.1:")
            .and_then(|value| value.split('/').next())
            .unwrap()
            .parse::<u16>()
            .unwrap();
        let received = tokio::task::spawn_blocking(move || fetch_chrome_broker_endpoint(port))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.broker_pid, std::process::id());
        assert_eq!(received.broker_start_id, public.broker_start_id);
        assert_eq!(received.ws_url, public.ws_url);
        assert_eq!(
            received.extension_frame_ws_url,
            public.extension_frame_ws_url
        );
        assert_eq!(received.broker_features, vec!["transport.v1"]);
        let trusted_origins =
            vec!["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()];
        validate_rust_transport_broker(&received, &credentials, Some(&trusted_origins)).unwrap();
        let wrong_extension = validate_rust_transport_broker(
            &received,
            &credentials,
            Some(&["chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned()]),
        );
        assert!(wrong_extension
            .unwrap_err()
            .message
            .contains("different extension identity set"));

        let mut stale = received.clone();
        stale.broker_start_id.push_str("-stale");
        assert!(validate_rust_transport_broker(&stale, &credentials, None).is_err());
        runtime.shutdown().await;
    }

    #[test]
    fn forged_listener_cannot_reuse_broker_credential_or_observe_token() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        use std::io::{Read, Write};
        use std::net::TcpListener as StdTcpListener;

        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut forged_endpoint = endpoint("forged-generation");
        forged_endpoint.discovery_url = format!("http://127.0.0.1:{port}/v1/bridge");
        forged_endpoint.ws_url = format!("ws://127.0.0.1:{port}/");
        forged_endpoint.extension_frame_ws_url = format!("ws://127.0.0.1:{port}/extension/frames");
        forged_endpoint.broker_features = vec!["transport.v1".into()];

        let temp = tempfile::tempdir().unwrap();
        let credentials = PrivateCredentialStore::new(temp.path().join("private-state"));
        let token = "Q".repeat(43);
        let credential = teshi_browser_broker::PrivateBrokerCredential::for_endpoint(
            &EndpointRecord {
                schema_version: forged_endpoint.schema_version,
                protocol_version: forged_endpoint.protocol_version,
                mode: forged_endpoint.mode.clone(),
                ws_url: forged_endpoint.ws_url.clone(),
                discovery_url: forged_endpoint.discovery_url.clone(),
                extension_frame_ws_url: forged_endpoint.extension_frame_ws_url.clone(),
                broker_pid: forged_endpoint.broker_pid,
                broker_start_id: forged_endpoint.broker_start_id.clone(),
                broker_features: forged_endpoint.broker_features.clone(),
                bridge: "rust".into(),
            },
            token.clone(),
            vec!["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()],
        )
        .unwrap();
        credentials.write(&credential).unwrap();

        let fake_identity = forged_endpoint.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            let (body_start, body_len) = loop {
                let read = stream.read(&mut chunk).unwrap();
                assert_ne!(read, 0, "client closed before its challenge was complete");
                request.extend_from_slice(&chunk[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    assert!(request.len() < 8 * 1024);
                    continue;
                };
                let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let body_len = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                let body_start = header_end + 4;
                if request.len() >= body_start + body_len {
                    break (body_start, body_len);
                }
                assert!(request.len() < 8 * 1024);
            };
            let request_text = String::from_utf8_lossy(&request);
            assert!(request_text.starts_with("POST /v1/bridge/identity "));
            assert!(!request_text.contains(&token));
            assert!(!request_text.contains("token="));
            let challenge: BrokerIdentityChallenge =
                serde_json::from_slice(&request[body_start..body_start + body_len]).unwrap();
            let forged_proof = BrokerIdentityProof {
                schema_version: fake_identity.schema_version,
                protocol_version: fake_identity.protocol_version,
                broker_pid: fake_identity.broker_pid,
                broker_start_id: fake_identity.broker_start_id,
                nonce: challenge.nonce,
                proof: URL_SAFE_NO_PAD.encode([0u8; 32]),
            };
            let body = serde_json::to_vec(&forged_proof).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });

        let error =
            validate_rust_transport_broker(&forged_endpoint, &credentials, None).unwrap_err();
        assert!(error.message.contains("did not prove possession"));
        server.join().unwrap();
    }

    #[test]
    fn discovery_response_body_is_bounded() {
        use std::io::Write;
        use std::net::TcpListener as StdTcpListener;

        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let body = vec![b' '; MAX_DISCOVERY_BODY_BYTES + 1];
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        let error = fetch_chrome_broker_endpoint(port).unwrap_err();
        assert!(error.message.contains("size limit"));
        server.join().unwrap();
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

    #[test]
    #[cfg(windows)]
    fn elevation_cancelled_is_not_reported_as_generic_launch_failure() {
        let error = elevation_error(ELEVATION_CANCELLED_ERROR);
        assert!(error.message.contains("cancelled or denied"));
        assert!(error
            .hint
            .as_deref()
            .is_some_and(|hint| hint.contains("UAC")));
    }

    #[test]
    fn authenticated_sidecar_url_keeps_port_discoverable() {
        let address = ws_url_to_addr("ws://127.0.0.1:43123/?token=tk_secret").unwrap();
        assert_eq!(address.port(), 43123);
    }

    #[test]
    fn runtime_event_ws_url_drops_query_credentials() {
        assert_eq!(
            public_event_ws_url("ws://127.0.0.1:43123/?token=tk_secret"),
            "ws://127.0.0.1:43123/"
        );
        assert_eq!(
            public_event_ws_url("ws://127.0.0.1:43123/winapp"),
            "ws://127.0.0.1:43123/winapp"
        );
    }

    #[tokio::test]
    async fn sidecar_stop_clears_elevated_session_state() {
        let state = SidecarState::new();
        *state.elevated.lock().unwrap() = true;
        state.stop().await.unwrap();
        assert!(!state.is_elevated());
        assert!(!state.child_is_running());
    }
}
