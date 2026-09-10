//! Daemon HTTP server and client auto-spawn for the teshi web UI.

mod server;
pub mod session;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use teshi_engine::{
    default_browser_service_script, default_winapp_service_script, find_project_root, open_project,
    remove_daemon_manifest, spawn_daemon_background, DaemonManifest, DaemonManifestExt,
    RuntimeConfig, TeshiEngine,
};
use tracing::info;

pub use server::{run_server, run_server_with_listener};

fn hosted_launch_url(port: u16, token: &str) -> String {
    format!("https://teshi.org/app/#port={port}&token={token}")
}

fn resolve_project_root(explicit: Option<&std::path::Path>) -> PathBuf {
    if let Some(project) = explicit {
        let root = project.to_path_buf();
        // Keep the existing CLI behavior: an explicit project is made ready
        // for the daemon manifest before the child process is started.
        std::fs::create_dir_all(root.join(".teshi")).ok();
        return root;
    }
    if let Some(root) = find_project_root(None) {
        return root;
    }

    // No project — use a user-level daemon directory so the hosted UI can
    // start without a project (welcome screen → user picks a project later).
    let fallback =
        teshi_engine::app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("teshi"));
    let root = fallback.join("daemon");
    std::fs::create_dir_all(root.join(".teshi")).ok();
    root
}

fn requested_daemon_port(requested: Option<u16>) -> u16 {
    requested.unwrap_or(0)
}

fn should_open_hosted_ui(no_open: bool) -> bool {
    !no_open
}

fn existing_daemon_port(project_root: &std::path::Path) -> Option<u16> {
    let manifest = DaemonManifest::load_manifest(project_root)?;
    if manifest.is_daemon_alive() {
        Some(manifest.port)
    } else {
        // A stale manifest must never be reused as a launch target.
        remove_daemon_manifest(project_root);
        None
    }
}

fn loopback_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        // Session/bootstrap traffic is always directed at the local daemon.
        // Never send it through a corporate or system HTTP proxy: proxies can
        // return misleading gateway errors for an otherwise healthy daemon.
        .no_proxy()
        .build()
        .context("create loopback daemon HTTP client")
}

async fn mint_hosted_session(port: u16) -> Result<String> {
    let client = loopback_http_client()?;
    let session_url = format!("http://127.0.0.1:{port}/api/v1/sessions");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let last_error = match client
            .post(&session_url)
            .json(&serde_json::json!({ "role": "hosted_web_ui" }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let session: serde_json::Value = response
                    .json()
                    .await
                    .context("decode hosted Web UI session")?;
                return session
                    .get("token")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .context("daemon returned an empty hosted Web UI session token");
            }
            Ok(response) if !response.status().is_server_error() => {
                anyhow::bail!(
                    "daemon rejected hosted Web UI session ({})",
                    response.status()
                );
            }
            Ok(response) => {
                format!("daemon returned {}", response.status())
            }
            Err(error) => error.to_string(),
        };
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("daemon did not become ready within 15s: {}", last_error);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

// ---- User-facing CLI options ----

/// CLI options for `teshi web`.
#[derive(Debug, Parser)]
pub struct WebOptions {
    /// Project directory to open on startup.
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// TCP port for the local server (default: OS-selected loopback port).
    #[arg(long)]
    pub port: Option<u16>,
    /// Host address to bind the local server (default: 127.0.0.1).
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Do not open the system browser automatically.
    #[arg(long)]
    pub no_open: bool,
    /// Optional local Web distribution for development diagnostics only.
    #[arg(long)]
    pub dist: Option<PathBuf>,
    /// Auto-start embedded browser after server starts.
    #[arg(long)]
    pub start_embedded: bool,
}

// ---- Internal daemon options (hidden flag, not user-facing) ----

/// Options for the `--daemon-internal` fork mode.
#[derive(Debug, Parser)]
pub struct DaemonInternalOptions {
    /// Hidden flag used to detect fork mode (must be first arg).
    #[arg(long, hide = true)]
    pub daemon_internal: bool,
    #[arg(long)]
    pub project_root: PathBuf,
    /// Port to bind. Zero asks the OS to select an available port.
    #[arg(long)]
    pub port: u16,
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long)]
    pub dist: Option<PathBuf>,
}

// ---- Client mode (teshi web) ----

/// Client mode: ensure daemon is running, then open browser.
pub async fn run_client(opts: WebOptions) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

    // Resolve project root
    let project_root = resolve_project_root(opts.project.as_deref());

    // Ensure daemon is running
    let port = ensure_daemon(&project_root, opts.dist.clone(), opts.port, &opts.host).await?;

    let token = mint_hosted_session(port)
        .await
        .context("mint hosted Web UI session")?;

    let url = hosted_launch_url(port, &token);

    if should_open_hosted_ui(opts.no_open) {
        webbrowser::open(&url).context("open browser")?;
    }

    if opts.start_embedded {
        // Use reqwest to trigger embedded browser start via daemon API
        let client = loopback_http_client()?;
        let api_url = format!("http://127.0.0.1:{port}/api/v1/browser/start");
        let session_token = token.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Err(e) = client
                .post(&api_url)
                .header("x-teshi-token", session_token)
                .json(&serde_json::json!({"mode": "embedded"}))
                .send()
                .await
            {
                tracing::error!("auto-start embedded browser via daemon: {e:#?}");
            }
        });
    }

    info!("teshi web → https://teshi.org/app/#port={port}&token=<redacted>");
    Ok(())
}

// ---- Daemon internal mode (forked process) ----

/// Internal daemon entry point (called from forked process with `--daemon-internal`).
pub async fn run_daemon_internal(opts: DaemonInternalOptions) -> Result<()> {
    // Redirect stdout/stderr to daemon log file
    let log_dir = opts.project_root.join(".teshi").join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("daemon.log"))
        .ok();

    if let Some(file) = log_file {
        let _ = tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_env_filter("info")
            .try_init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter("info")
            .try_init()
            .ok();
    }

    // Create TeshiEngine
    let script = default_browser_service_script();
    let winapp_script = default_winapp_service_script();
    let rt = TeshiEngine::new(
        RuntimeConfig {
            browser_service_script: script,
            winapp_service_script: winapp_script,
            embedded_no_preview_stream: false,
            requirements_root: None,
        },
        None,
    );
    rt.emit_initial_recent();

    open_project(
        Arc::clone(&rt),
        opts.project_root.to_string_lossy().into_owned(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("open project: {e}"))?;

    // The hosted UI is delivered by teshi.org. Keep a nonexistent fallback
    // only so the optional development static-file route remains type-stable;
    // production daemon startup never resolves or requires a bundled Web UI.
    let dist = opts.dist.unwrap_or_else(|| {
        opts.project_root
            .join(".teshi")
            .join("no-embedded-web-dist")
    });

    let addr: SocketAddr = format!("{}:{}", opts.host, opts.port)
        .parse()
        .context("invalid host or port in daemon options")?;
    // Bind before publishing the manifest. With the default port `0`, this is
    // where the OS chooses the actual port and removes the old probe/bind
    // race. A failed bind therefore never leaves a usable-looking manifest.
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("bind daemon listener")?;
    let bound_addr = listener.local_addr().context("read bound daemon address")?;
    let manifest = DaemonManifest {
        pid: std::process::id(),
        port: bound_addr.port(),
        started: chrono::Utc::now(),
    };
    manifest.save_manifest(&opts.project_root)?;
    info!("teshi daemon listening on {bound_addr}");

    // Graceful shutdown: cleanup manifest on exit
    let project_root = opts.project_root.clone();
    let shutdown = async move {
        // Wait for Ctrl+C
        tokio::signal::ctrl_c().await.ok();
        info!("daemon shutting down");
        remove_daemon_manifest(&project_root);
    };
    tokio::spawn(shutdown);

    run_server_with_listener(listener, rt, dist, Some(opts.project_root.clone()))
        .await
        .context("daemon server")?;

    remove_daemon_manifest(&opts.project_root);
    Ok(())
}

// ---- Auto-spawn client logic ----

/// Finds or starts the daemon for the given project root.
/// Returns the daemon's port.
/// If `requested_port` is `None`, the child daemon binds port `0` and writes
/// the OS-selected port to its manifest after binding.
pub async fn ensure_daemon(
    project_root: &std::path::Path,
    dist: Option<PathBuf>,
    requested_port: Option<u16>,
    host: &str,
) -> Result<u16> {
    // 1. Check if daemon is already running
    if let Some(port) = existing_daemon_port(project_root) {
        return Ok(port);
    }

    // 2. Let the child ask the OS for a port when no explicit diagnostic
    // override is given. The child publishes the actual bound port only after
    // its listener is live, so there is no probe/bind race here.
    let port = requested_daemon_port(requested_port);

    // 3. Spawn detached background daemon process
    spawn_daemon_background(project_root, port, host, dist.as_deref())?;

    // 4. Wait for daemon to write daemon.json (max 15 seconds)
    let manifest_path = project_root.join(".teshi").join("daemon.json");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("daemon failed to start within 15s");
        }
        if let Ok(data) = tokio::fs::read_to_string(&manifest_path).await {
            if let Ok(m) = serde_json::from_str::<DaemonManifest>(&data) {
                if m.is_daemon_alive() {
                    return Ok(m.port);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        existing_daemon_port, hosted_launch_url, requested_daemon_port, resolve_project_root,
        should_open_hosted_ui,
    };
    use chrono::Utc;
    use std::net::TcpListener;
    use teshi_engine::{DaemonManifest, DaemonManifestExt};

    #[test]
    fn hosted_launch_url_is_exact_and_fragment_scoped() {
        let token = "tk_test_1234567890";
        let url = hosted_launch_url(43123, token);
        assert_eq!(
            url,
            "https://teshi.org/app/#port=43123&token=tk_test_1234567890"
        );
        assert!(url.starts_with("https://teshi.org/app/#"));
        assert!(!url[..url.find('#').unwrap()].contains(token));
    }

    #[test]
    fn launcher_uses_os_port_by_default_and_preserves_explicit_diagnostic_port() {
        assert_eq!(requested_daemon_port(None), 0);
        assert_eq!(requested_daemon_port(Some(43123)), 43123);
    }

    #[test]
    fn no_open_is_a_side_effect_free_launch_mode() {
        assert!(!should_open_hosted_ui(true));
        assert!(should_open_hosted_ui(false));
    }

    #[test]
    fn explicit_project_selection_prepares_its_manifest_directory() {
        let root = std::env::temp_dir().join(format!("teshi-launcher-{}", uuid::Uuid::new_v4()));
        assert_eq!(resolve_project_root(Some(&root)), root);
        assert!(root.join(".teshi").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_manifest_is_removed_and_live_daemon_manifest_is_reused() {
        let root = std::env::temp_dir().join(format!("teshi-manifest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".teshi")).unwrap();
        let stale = DaemonManifest {
            pid: std::process::id(),
            port: 0,
            started: Utc::now(),
        };
        stale.save_manifest(&root).unwrap();
        assert_eq!(existing_daemon_port(&root), None);
        assert!(!DaemonManifest::manifest_path(&root).exists());

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = DaemonManifest {
            pid: std::process::id(),
            port: listener.local_addr().unwrap().port(),
            started: Utc::now(),
        };
        live.save_manifest(&root).unwrap();
        assert_eq!(existing_daemon_port(&root), Some(live.port));
        drop(listener);
        std::fs::remove_dir_all(root).unwrap();
    }
}
