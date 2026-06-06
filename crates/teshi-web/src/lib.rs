//! Loopback HTTP server and WebSocket event stream for the teshi web UI.

mod server;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use teshi_runtime::{
    default_browser_service_script, default_winapp_service_script, open_project,
    start_browser_sidecar, BrowserMode, RuntimeConfig, TeshiRuntime,
};
use tracing::info;

pub use server::run_server;

/// CLI options for `teshi web`.
#[derive(Debug, Parser)]
pub struct WebOptions {
    /// Project directory to open on startup.
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// TCP port for the local server (default 1421).
    #[arg(long, default_value_t = 1421)]
    pub port: u16,
    /// Do not open the system browser automatically.
    #[arg(long)]
    pub no_open: bool,
    /// Directory of built frontend static files (`desktop/dist`).
    #[arg(long)]
    pub dist: Option<PathBuf>,
    /// Auto-start embedded browser after server starts.
    #[arg(long)]
    pub start_embedded: bool,
}

/// Runs the web host until the process is interrupted.
pub async fn run(opts: WebOptions) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

    let script = default_browser_service_script();
    let winapp_script = default_winapp_service_script();
    let rt = TeshiRuntime::new(
        RuntimeConfig {
            browser_service_script: script,
            winapp_service_script: winapp_script,
            embedded_no_preview_stream: false,
        },
        None,
    );
    rt.emit_initial_recent();

    if let Some(project) = opts.project {
        open_project(Arc::clone(&rt), project.to_string_lossy().into_owned())
            .await
            .map_err(|e| anyhow::anyhow!("open project: {e}"))?;
    }

    let dist = opts
        .dist
        .or_else(resolve_web_dist)
        .context(
            "frontend dist not found; install the full Windows MSI, run `npm run build` in desktop/, or pass --dist",
        )?;

    let addr = SocketAddr::from(([127, 0, 0, 1], opts.port));
    let url = format!("http://{addr}");

    let server = tokio::spawn(run_server(addr, Arc::clone(&rt), dist));

    if !opts.no_open {
        webbrowser::open(&url).context("open browser")?;
    }

    if opts.start_embedded {
        let trt = Arc::clone(&rt);
        tokio::spawn(async move {
            // Brief delay so the HTTP server finishes binding first
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Err(e) = start_browser_sidecar(trt, BrowserMode::Embedded).await {
                tracing::error!("auto-start embedded browser via web: {e:#?}");
            }
        });
    }

    info!("teshi web listening on {url}");
    server.await??;
    Ok(())
}

/// Resolves bundled or development frontend assets for `teshi web`.
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
        PathBuf::from("desktop/dist"),
        PathBuf::from("../desktop/dist"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_dir())
}
