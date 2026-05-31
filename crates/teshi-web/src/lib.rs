//! Loopback HTTP server and WebSocket event stream for the teshi web UI.

mod server;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use teshi_runtime::{default_browser_service_script, open_project, RuntimeConfig, TeshiRuntime};
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
}

/// Runs the web host until the process is interrupted.
pub async fn run(opts: WebOptions) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

    let script = default_browser_service_script();
    let rt = TeshiRuntime::new(
        RuntimeConfig {
            browser_service_script: script,
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
        .or_else(|| {
            [
                PathBuf::from("desktop/dist"),
                PathBuf::from("../desktop/dist"),
            ]
            .into_iter()
            .find(|candidate| candidate.is_dir())
        })
        .context("frontend dist not found; run `npm run build` in desktop/ or pass --dist")?;

    let addr = SocketAddr::from(([127, 0, 0, 1], opts.port));
    let url = format!("http://{addr}");

    let server = tokio::spawn(run_server(addr, Arc::clone(&rt), dist));

    if !opts.no_open {
        webbrowser::open(&url).context("open browser")?;
    }

    info!("teshi web listening on {url}");
    server.await??;
    Ok(())
}
