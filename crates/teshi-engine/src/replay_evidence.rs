//! Shared replay evidence storage and backend viewport capture adapter.

pub use teshi_replay_evidence::*;

use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::{fs, path::Path};

/// Capture an embedded browser or native application viewport.
///
/// # Errors
/// Returns an error for unsupported brokers, unavailable targets, or invalid captures.
pub fn capture_replay_viewport(project: &Path) -> Result<Vec<u8>> {
    let endpoint: Value =
        serde_json::from_slice(&fs::read(project.join(".teshi/cdp-endpoint.json"))?)?;
    // Chrome requires an explicit target and lease. Never bypass that boundary
    // with the legacy sidecar command or select an arbitrary connected tab.
    anyhow::ensure!(
        endpoint["mode"] != "chrome",
        "this backend requires an explicit browser target and screenshot capability"
    );
    let ws = endpoint["ws_url"]
        .as_str()
        .ok_or_else(|| anyhow!("missing capture endpoint"))?;
    let response = crate::send_sidecar_command_with_timeout(
        ws,
        json!({"cmd":"screenshot", "request_id":uuid::Uuid::new_v4().to_string()}),
        std::time::Duration::from_secs(15),
    )
    .map_err(|error| anyhow!(error))?;
    anyhow::ensure!(response["ok"] != false, "capture rejected: {}", response);
    let encoded = response["screenshot"]
        .as_str()
        .ok_or_else(|| anyhow!("capture returned no image"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    teshi_replay_evidence::validate_replay_image(&bytes)?;
    Ok(bytes)
}
