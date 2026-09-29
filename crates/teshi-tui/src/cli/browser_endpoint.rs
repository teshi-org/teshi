//! CDP endpoint discovery, sidecar health checks, and embedded reconnect helpers.

use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use teshi_engine::{
    CHROME_DISCOVERY_PORT, ChromeBrokerEndpoint, default_browser_service_script,
    ensure_user_chrome_broker, fetch_chrome_broker_endpoint, send_sidecar_command_with_timeout,
    write_atomic,
};

const ENDPOINT_READ_ATTEMPTS: usize = 20;
const ENDPOINT_READ_RETRY_DELAY: Duration = Duration::from_millis(5);

/// Parsed `.teshi/cdp-endpoint.json` payload used by browser CLI commands.
#[derive(Debug, Clone)]
pub struct CdpEndpoint {
    /// Project root directory containing `.teshi/cdp-endpoint.json`.
    pub project_root: PathBuf,
    /// Absolute path to the endpoint JSON file.
    pub endpoint_path: PathBuf,
    pub mode: String,
    pub bridge: String,
    pub ws_url: String,
    pub page_url: Option<String>,
    pub broker_pid: Option<u32>,
    pub broker_start_id: Option<String>,
    pub protocol_version: Option<u16>,
    pub broker_features: Vec<String>,
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
    let mut payload = None;
    let mut last_error = None;
    for attempt in 0..ENDPOINT_READ_ATTEMPTS {
        match fs::read_to_string(&endpoint_path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(value) => {
                    payload = Some(value);
                    break;
                }
                Err(error) => {
                    last_error = Some(anyhow!(error).context("parse cdp-endpoint.json"));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                last_error =
                    Some(anyhow!(error).context(format!("read {}", endpoint_path.display())));
            }
            Err(error) => {
                return Err(anyhow!(error).context(format!("read {}", endpoint_path.display())));
            }
        }
        if attempt + 1 < ENDPOINT_READ_ATTEMPTS {
            std::thread::sleep(ENDPOINT_READ_RETRY_DELAY);
        }
    }
    let payload = payload
        .ok_or_else(|| last_error.unwrap_or_else(|| anyhow!("read cdp-endpoint.json failed")))?;
    let mode = payload
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let bridge = payload
        .get("bridge")
        .and_then(|value| value.as_str())
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
    let broker_pid = payload
        .get("broker_pid")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok());
    let broker_start_id = payload
        .get("broker_start_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let protocol_version = payload
        .get("protocol_version")
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok());
    let broker_features = payload
        .get("broker_features")
        .and_then(|value| value.as_array())
        .map(|features| {
            features
                .iter()
                .filter_map(|feature| feature.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(CdpEndpoint {
        project_root: project_root.to_path_buf(),
        endpoint_path,
        mode,
        bridge,
        ws_url,
        page_url,
        broker_pid,
        broker_start_id,
        protocol_version,
        broker_features,
    })
}

/// Writes project-scoped compatibility data for an attached user-session broker.
pub fn write_chrome_broker_endpoint(
    project_root: &Path,
    endpoint: &ChromeBrokerEndpoint,
) -> Result<()> {
    teshi_engine::write_chrome_broker_endpoint(project_root, endpoint)
        .map_err(|error| anyhow!("{}", error.message))
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
    pub broker_generation_match: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_start_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u16>,
}

/// Probes TCP reachability and issues a short `get_page_snapshot` over the sidecar WebSocket.
pub fn doctor_endpoint(project_root: &Path) -> Result<DoctorReport> {
    let endpoint = read_cdp_endpoint(project_root)?;
    let mut broker_generation_match = true;
    let mut generation_error = None;
    if endpoint.mode == "chrome" && endpoint.bridge == "rust" {
        match fetch_chrome_broker_endpoint(CHROME_DISCOVERY_PORT) {
            Ok(current) => {
                broker_generation_match = endpoint.ws_url == current.ws_url
                    && endpoint.broker_pid == Some(current.broker_pid)
                    && endpoint.broker_start_id.as_deref()
                        == Some(current.broker_start_id.as_str())
                    && endpoint.protocol_version == Some(current.protocol_version)
                    && endpoint.broker_features == current.broker_features;
                if !broker_generation_match {
                    generation_error = Some(
                        "project endpoint points to a stale Rust broker generation; refreshing it"
                            .to_owned(),
                    );
                }
            }
            Err(error) => {
                broker_generation_match = false;
                generation_error = Some(error.message);
            }
        }
    }
    let tcp_reachable = broker_generation_match && tcp_probe_ws_url(&endpoint.ws_url);
    let mut snapshot_ok = false;
    let mut error = generation_error;

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

    let ok = broker_generation_match && tcp_reachable && snapshot_ok;
    Ok(DoctorReport {
        ok,
        mode: endpoint.mode,
        ws_url: endpoint.ws_url,
        page_url: endpoint.page_url,
        error,
        tcp_reachable,
        snapshot_ok,
        broker_generation_match,
        broker_pid: endpoint.broker_pid,
        broker_start_id: endpoint.broker_start_id,
        protocol_version: endpoint.protocol_version,
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

/// Refresh a Chrome project pointer from the shared user broker. This path is
/// generation-aware and never launches a second broker for a stale project.
pub fn reconnect_chrome_broker(project_root: &Path) -> Result<CdpEndpoint> {
    let current = read_cdp_endpoint(project_root)?;
    if current.mode != "chrome" {
        return Err(anyhow!("project endpoint is not a Chrome broker endpoint"));
    }
    if current.bridge == "rust"
        && std::env::var("TESHI_BROWSER_BROKER_IMPLEMENTATION").as_deref() != Ok("rust")
    {
        return Err(anyhow!(
            "Rust Chrome endpoint requires TESHI_BROWSER_BROKER_IMPLEMENTATION=rust for reconnect; Teshi will not fall back to Python"
        ));
    }
    let broker = ensure_user_chrome_broker(project_root, &default_browser_service_script())
        .map_err(|error| match error.hint {
            Some(hint) => anyhow!("{} ({hint})", error.message),
            None => anyhow!(error.message),
        })?;
    write_chrome_broker_endpoint(project_root, &broker)?;
    read_cdp_endpoint(project_root)
}

/// Writes `cdp-endpoint.json` from the Rust side with the actual ws_url.
/// Used after starting the sidecar to ensure the file exists before any command reads it.
pub fn write_cdp_endpoint_from_rust(
    project_root: &Path,
    ws_url: &str,
    mode: &str,
    page_url: &str,
) -> Result<()> {
    let path = project_root.join(".teshi").join("cdp-endpoint.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let payload = json!({
        "mode": mode,
        "ws_url": ws_url,
        "page_url": page_url,
        "bridge": "python",
        "extension_connected": false,
        "viewport": {"width": 1920, "height": 1080},
    });
    write_atomic(&path, &payload).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// When enabled (default), runs doctor and one reconnect attempt before sidecar commands.
pub fn auto_reconnect_enabled() -> bool {
    !matches!(
        std::env::var_os("TESHI_BROWSER_AUTO_RECONNECT").as_deref(),
        Some(v) if v == "0" || v == "false"
    )
}

/// Ensures the sidecar responds; attempts one mode-preserving reconnect when doctor fails.
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
    if endpoint
        .as_ref()
        .is_some_and(|e| e.mode == "chrome" && e.bridge == "rust")
    {
        if std::env::var("TESHI_BROWSER_BROKER_IMPLEMENTATION").as_deref() != Ok("rust") {
            return Err(anyhow!(
                "Rust Chrome endpoint requires TESHI_BROWSER_BROKER_IMPLEMENTATION=rust for reconnect; Teshi will not fall back to Python"
            ));
        }
        let broker = ensure_user_chrome_broker(project_root, &default_browser_service_script())
            .map_err(|error| match error.hint {
                Some(hint) => anyhow!("{} ({hint})", error.message),
                None => anyhow!(error.message),
            })?;
        write_chrome_broker_endpoint(project_root, &broker)?;
        if doctor_endpoint(project_root).is_ok_and(|r| r.ok) {
            return read_cdp_endpoint(project_root);
        }
    }
    Err(anyhow!(
        "browser sidecar still unhealthy after reconnect; run `teshi browser doctor` for details"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broker_endpoint() -> ChromeBrokerEndpoint {
        ChromeBrokerEndpoint {
            schema_version: 1,
            protocol_version: 1,
            mode: "chrome".into(),
            ws_url: "ws://127.0.0.1:24567".into(),
            discovery_url: "http://127.0.0.1:17373/v1/bridge".into(),
            extension_frame_ws_url: "ws://127.0.0.1:24567/extension/frames".into(),
            broker_pid: 1234,
            broker_start_id: "broker-start-a".into(),
            broker_features: vec!["p0.control".into()],
        }
    }

    #[test]
    fn stale_project_endpoint_is_replaced_with_live_broker_identity() {
        let temp = tempfile::tempdir().unwrap();
        let teshi = temp.path().join(".teshi");
        fs::create_dir_all(&teshi).unwrap();
        fs::write(
            teshi.join("cdp-endpoint.json"),
            r#"{"mode":"chrome","ws_url":"ws://127.0.0.1:1","broker_start_id":"stale"}"#,
        )
        .unwrap();
        write_chrome_broker_endpoint(temp.path(), &broker_endpoint()).unwrap();
        let current = read_cdp_endpoint(temp.path()).unwrap();
        assert_eq!(current.ws_url, "ws://127.0.0.1:24567");
        assert_eq!(current.broker_start_id.as_deref(), Some("broker-start-a"));
        assert_eq!(current.broker_pid, Some(1234));
    }

    #[test]
    fn cli_and_desktop_compatibility_writes_preserve_shared_broker_identity() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let endpoint = broker_endpoint();
        write_chrome_broker_endpoint(first.path(), &endpoint).unwrap();
        write_chrome_broker_endpoint(second.path(), &endpoint).unwrap();
        let cli = read_cdp_endpoint(first.path()).unwrap();
        let desktop = read_cdp_endpoint(second.path()).unwrap();
        assert_eq!(cli.broker_start_id, desktop.broker_start_id);
        assert_eq!(cli.ws_url, desktop.ws_url);
        assert_eq!(cli.bridge, "python");
        let first_text = fs::read_to_string(first.path().join(".teshi/cdp-endpoint.json")).unwrap();
        let second_text =
            fs::read_to_string(second.path().join(".teshi/cdp-endpoint.json")).unwrap();
        for text in [first_text, second_text] {
            let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert!(payload.get("project_root").is_none());
            assert!(payload.get("broker_project_root").is_none());
            assert!(payload.get("token").is_none());
            assert!(payload.get("secret").is_none());
        }
    }

    #[test]
    fn rust_transport_endpoint_is_marked_without_persisting_a_credential() {
        let temp = tempfile::tempdir().unwrap();
        let mut endpoint = broker_endpoint();
        endpoint.broker_features.insert(0, "transport.v1".into());
        write_chrome_broker_endpoint(temp.path(), &endpoint).unwrap();
        let current = read_cdp_endpoint(temp.path()).unwrap();
        assert_eq!(current.bridge, "rust");
        let text = fs::read_to_string(current.endpoint_path).unwrap();
        assert!(!text.contains("token"));
        assert!(!text.contains("project_root"));
        assert!(!text.contains("broker_project_root"));
    }

    #[test]
    fn two_projects_repair_to_one_current_broker_generation_regardless_of_start_order() {
        let project_a = tempfile::tempdir().unwrap();
        let project_b = tempfile::tempdir().unwrap();
        let first = broker_endpoint();
        let mut restarted = first.clone();
        restarted.ws_url = "ws://127.0.0.1:24568".into();
        restarted.extension_frame_ws_url = "ws://127.0.0.1:24568/extension/frames".into();
        restarted.broker_pid = 4321;
        restarted.broker_start_id = "broker-start-b".into();
        restarted
            .broker_features
            .push("p1.observability_artifacts".into());

        // B can attach before A; both pointers still represent the same user
        // broker generation rather than spawning project-scoped brokers.
        write_chrome_broker_endpoint(project_b.path(), &first).unwrap();
        write_chrome_broker_endpoint(project_a.path(), &first).unwrap();
        write_chrome_broker_endpoint(project_a.path(), &restarted).unwrap();
        write_chrome_broker_endpoint(project_b.path(), &restarted).unwrap();
        let a = read_cdp_endpoint(project_a.path()).unwrap();
        let b = read_cdp_endpoint(project_b.path()).unwrap();
        assert_eq!(a.ws_url, b.ws_url);
        assert_eq!(a.broker_pid, b.broker_pid);
        assert_eq!(a.broker_start_id, b.broker_start_id);
        assert_eq!(a.protocol_version, b.protocol_version);
        assert_eq!(a.broker_features, b.broker_features);
    }
}
