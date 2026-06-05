//! CDP endpoint discovery, sidecar health checks, and embedded reconnect helpers.

use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use teshi_runtime::send_sidecar_command_with_timeout;

/// Parsed `.teshi/cdp-endpoint.json` payload used by browser CLI commands.
#[derive(Debug, Clone)]
pub struct CdpEndpoint {
    /// Project root directory containing `.teshi/cdp-endpoint.json`.
    pub project_root: PathBuf,
    /// Absolute path to the endpoint JSON file.
    pub endpoint_path: PathBuf,
    pub mode: String,
    pub ws_url: String,
    pub page_url: Option<String>,
}

/// Locates a project root by walking upward from `start` until `.teshi/cdp-endpoint.json` exists.
pub fn resolve_browser_project_root(start: &Path) -> Result<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| start.to_path_buf())
    };
    loop {
        if dir.join(".teshi").join("cdp-endpoint.json").is_file() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    Err(anyhow!(
        "no .teshi/cdp-endpoint.json found from {}; run Start Embedded in desktop or `teshi browser serve-embedded`",
        start.display()
    ))
}

/// Reads and parses the CDP endpoint file under `project_root`.
pub fn read_cdp_endpoint(project_root: &Path) -> Result<CdpEndpoint> {
    let endpoint_path = project_root.join(".teshi").join("cdp-endpoint.json");
    let text = fs::read_to_string(&endpoint_path)
        .with_context(|| format!("read {}", endpoint_path.display()))?;
    let payload: Value = serde_json::from_str(&text).context("parse cdp-endpoint.json")?;
    let mode = payload
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let ws_url = payload
        .get("ws_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("cdp-endpoint.json missing ws_url"))?
        .to_string();
    let page_url = payload
        .get("page_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok(CdpEndpoint {
        project_root: project_root.to_path_buf(),
        endpoint_path,
        mode,
        ws_url,
        page_url,
    })
}

/// Result of a sidecar health probe suitable for JSON CLI output.
#[derive(Debug, serde::Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub mode: String,
    pub ws_url: String,
    pub page_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tcp_reachable: bool,
    pub snapshot_ok: bool,
}

/// Probes TCP reachability and issues a short `get_page_snapshot` over the sidecar WebSocket.
pub fn doctor_endpoint(project_root: &Path) -> Result<DoctorReport> {
    let endpoint = read_cdp_endpoint(project_root)?;
    let tcp_reachable = tcp_probe_ws_url(&endpoint.ws_url);
    let mut snapshot_ok = false;
    let mut error = None;

    if !tcp_reachable {
        error = Some(format!(
            "TCP unreachable on {}; embedded sidecar may be stale — run `teshi browser reconnect`",
            endpoint.ws_url
        ));
    } else {
        match send_sidecar_command_with_timeout(
            &endpoint.ws_url,
            json!({
                "cmd": "get_page_snapshot",
                "request_id": "browser-doctor"
            }),
            Duration::from_secs(8),
        ) {
            Ok(response) => {
                snapshot_ok = response.get("ok").and_then(|v| v.as_bool()) == Some(true);
                if !snapshot_ok {
                    error = response
                        .get("error")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or_else(|| Some("get_page_snapshot returned ok=false".into()));
                }
            }
            Err(err) => {
                error = Some(format!(
                    "{err}; embedded stale → try `teshi browser reconnect`"
                ));
            }
        }
    }

    let ok = tcp_reachable && snapshot_ok;
    Ok(DoctorReport {
        ok,
        mode: endpoint.mode,
        ws_url: endpoint.ws_url,
        page_url: endpoint.page_url,
        error,
        tcp_reachable,
        snapshot_ok,
    })
}

fn tcp_probe_ws_url(ws_url: &str) -> bool {
    let Ok(parsed) = url_to_socket_addr(ws_url) else {
        return false;
    };
    TcpStream::connect_timeout(&parsed, Duration::from_secs(2)).is_ok()
}

fn url_to_socket_addr(ws_url: &str) -> Result<SocketAddr> {
    let stripped = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))
        .ok_or_else(|| anyhow!("unsupported ws_url scheme: {ws_url}"))?;
    let (host, port) = stripped
        .split_once(':')
        .ok_or_else(|| anyhow!("ws_url missing port: {ws_url}"))?;
    let port: u16 = port
        .split('/')
        .next()
        .unwrap_or(port)
        .parse()
        .with_context(|| format!("invalid port in ws_url: {ws_url}"))?;
    Ok(format!("{host}:{port}").parse()?)
}

/// Spawns a detached `teshi browser serve-embedded` child and waits for a fresh endpoint file.
pub fn reconnect_embedded(
    project_root: &Path,
    navigate: Option<&str>,
    wait_secs: u64,
) -> Result<CdpEndpoint> {
    let before = read_cdp_endpoint(project_root).ok();
    let teshi_exe = std::env::current_exe().context("resolve current teshi binary")?;
    let mut cmd = Command::new(&teshi_exe);
    cmd.arg("browser")
        .arg("serve-embedded")
        .arg("--project")
        .arg(project_root);
    if let Some(url) = navigate {
        cmd.arg("--navigate").arg(url);
    } else if let Some(ref prev) = before
        && let Some(ref page_url) = prev.page_url
        && page_url.starts_with("http")
    {
        cmd.arg("--navigate").arg(page_url);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    cmd.spawn()
        .with_context(|| format!("spawn detached {}", teshi_exe.display()))?;

    let deadline = Instant::now() + Duration::from_secs(wait_secs.max(5));
    loop {
        if Instant::now() >= deadline {
            break;
        }
        if let Ok(current) = read_cdp_endpoint(project_root) {
            let changed = before
                .as_ref()
                .is_none_or(|prev| prev.ws_url != current.ws_url);
            if changed {
                return Ok(current);
            }
            if doctor_endpoint(project_root).is_ok_and(|r| r.ok) {
                return Ok(current);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(anyhow!(
        "timed out waiting for embedded sidecar after reconnect; check Python venv and Playwright"
    ))
}

/// When enabled (default), runs doctor and one reconnect attempt before sidecar commands.
pub fn auto_reconnect_enabled() -> bool {
    !matches!(
        std::env::var_os("TESHI_BROWSER_AUTO_RECONNECT").as_deref(),
        Some(v) if v == "0" || v == "false"
    )
}

/// Ensures the sidecar responds; attempts embedded reconnect once when doctor fails.
pub fn ensure_sidecar_healthy(project_root: &Path) -> Result<CdpEndpoint> {
    if doctor_endpoint(project_root).is_ok_and(|r| r.ok) {
        return read_cdp_endpoint(project_root);
    }
    if !auto_reconnect_enabled() {
        return Err(anyhow!(
            "browser sidecar unhealthy; run `teshi browser doctor` and `teshi browser reconnect`"
        ));
    }
    let endpoint = read_cdp_endpoint(project_root).ok();
    if endpoint.as_ref().is_some_and(|e| e.mode == "embedded") {
        reconnect_embedded(project_root, None, 45)?;
        if doctor_endpoint(project_root).is_ok_and(|r| r.ok) {
            return read_cdp_endpoint(project_root);
        }
    }
    Err(anyhow!(
        "browser sidecar still unhealthy after reconnect; run `teshi browser doctor` for details"
    ))
}
