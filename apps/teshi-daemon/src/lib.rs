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

pub use server::run_server;

// ---- User-facing CLI options ----

/// CLI options for `teshi web`.
#[derive(Debug, Parser)]
pub struct WebOptions {
    /// Project directory to open on startup.
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// TCP port for the local server (default: auto-pick).
    #[arg(long)]
    pub port: Option<u16>,
    /// Host address to bind the local server (default: 0.0.0.0).
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,
    /// Do not open the system browser automatically.
    #[arg(long)]
    pub no_open: bool,
    /// Directory of built GPUI WASM files (`apps/teshi-web/dist`).
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
    #[arg(long)]
    pub port: u16,
    #[arg(long, default_value = "0.0.0.0")]
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
    let project_root = if let Some(ref proj) = opts.project {
        let root = proj.clone();
        // Ensure .teshi/ directory exists
        std::fs::create_dir_all(root.join(".teshi")).ok();
        root
    } else if let Some(root) = find_project_root(None) {
        root
    } else {
        // No project — use a user-level daemon directory so the web UI can
        // start without a project (welcome screen → user picks a project later).
        let fallback =
            teshi_engine::app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("teshi"));
        let root = fallback.join("daemon");
        std::fs::create_dir_all(root.join(".teshi")).ok();
        root
    };

    let dist = opts
        .dist
        .or_else(resolve_web_dist)
        .context(
            "GPUI WASM dist not found; install the full Windows MSI, run `scripts/build-teshi-web.sh`, or pass --dist",
        )?;

    // Ensure daemon is running
    let port = ensure_daemon(&project_root, Some(dist.clone()), opts.port, &opts.host).await?;

    let url = format!("http://127.0.0.1:{port}");

    if !opts.no_open {
        webbrowser::open(&url).context("open browser")?;
    }

    if opts.start_embedded {
        // Use reqwest to trigger embedded browser start via daemon API
        let client = reqwest::Client::new();
        let api_url = format!("http://127.0.0.1:{port}/api/v1/browser/start");
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Err(e) = client
                .post(&api_url)
                .json(&serde_json::json!({"mode": "embedded"}))
                .send()
                .await
            {
                tracing::error!("auto-start embedded browser via daemon: {e:#?}");
            }
        });
    }

    info!("teshi web → {url}");
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

    // Write daemon manifest
    let manifest = DaemonManifest {
        pid: std::process::id(),
        port: opts.port,
        started: chrono::Utc::now(),
    };
    manifest.save_manifest(&opts.project_root)?;

    // Create TeshiEngine
    let script = default_browser_service_script();
    let winapp_script = default_winapp_service_script();
    let rt = TeshiEngine::new(
        RuntimeConfig {
            browser_service_script: script,
            winapp_service_script: winapp_script,
            embedded_no_preview_stream: false,
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

    let dist = opts.dist.or_else(resolve_web_dist).unwrap_or_else(|| {
        opts.project_root
            .join("apps")
            .join("teshi-web")
            .join("dist")
    });

    let addr: SocketAddr = format!("{}:{}", opts.host, opts.port)
        .parse()
        .context("invalid host or port in daemon options")?;
    info!("teshi daemon listening on {addr}");

    // Graceful shutdown: cleanup manifest on exit
    let project_root = opts.project_root.clone();
    let shutdown = async move {
        // Wait for Ctrl+C
        tokio::signal::ctrl_c().await.ok();
        info!("daemon shutting down");
        remove_daemon_manifest(&project_root);
    };
    tokio::spawn(shutdown);

    run_server(addr, rt, dist, Some(opts.project_root.clone()))
        .await
        .context("daemon server")?;

    remove_daemon_manifest(&opts.project_root);
    Ok(())
}

// ---- Auto-spawn client logic ----

/// Finds or starts the daemon for the given project root.
/// Returns the daemon's port.
/// If `requested_port` is `Some`, tries to use that port; falls back to a free port if it's unavailable.
pub async fn ensure_daemon(
    project_root: &std::path::Path,
    dist: Option<PathBuf>,
    requested_port: Option<u16>,
    host: &str,
) -> Result<u16> {
    // 1. Check if daemon is already running
    if let Some(manifest) = DaemonManifest::load_manifest(project_root) {
        if manifest.is_daemon_alive() {
            return Ok(manifest.port);
        }
        // Stale manifest — clean up
        remove_daemon_manifest(project_root);
    }

    // 2. Pick port: use requested_port if provided, otherwise default to 20253
    let port = requested_port.unwrap_or(20253);

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

// ---- Helpers ----

/// Resolves bundled or development frontend assets.
fn resolve_web_dist() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for candidate in [
                exe_dir.join("share").join("web"),
                exe_dir.join("../share/web"),
            ] {
                if candidate.is_dir() {
                    return Some(candidate);
                }
            }
        }
    }

    [
        PathBuf::from("apps/teshi-web/dist"),
        PathBuf::from("../apps/teshi-web/dist"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_dir())
}
