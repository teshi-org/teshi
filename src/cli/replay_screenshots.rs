use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use teshi_runtime::send_sidecar_command_with_timeout;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayScreenshotEntry {
    pub step_line: usize,
    pub step_keyword: String,
    pub step_text: String,
    pub screenshot_file: String,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayScreenshotsIndex {
    pub feature: String,
    pub started_at: String,
    pub steps: Vec<ReplayScreenshotEntry>,
    pub completed_at: Option<String>,
    pub status: String,
}

/// Captures a screenshot via the sidecar WebSocket, saves to disk as JPEG,
/// and returns a metadata entry suitable for the replay index.
pub fn capture_and_save_screenshot(
    ws_url: &str,
    _project_root: &Path,
    feature_sanitized: &str,
    step_line: usize,
    step_keyword: &str,
    step_text: &str,
    screenshot_dir: &Path,
) -> Result<ReplayScreenshotEntry> {
    // 1. Send screenshot command via WebSocket
    let response = send_sidecar_command_with_timeout(
        ws_url,
        json!({
            "cmd": "screenshot",
            "request_id": "replay-screenshot",
        }),
        std::time::Duration::from_secs(15),
    )
    .map_err(|e| anyhow!("screenshot WebSocket command failed: {e}"))?;

    let b64 = response
        .get("screenshot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("screenshot response missing 'screenshot' field"))?;

    // 2. Decode base64
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("decode base64 screenshot")?;

    // 3. Build filename and write
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let filename = format!("{}_L{}_{}.jpg", feature_sanitized, step_line, unix_ms);
    let filepath = screenshot_dir.join(&filename);

    fs::create_dir_all(screenshot_dir)
        .with_context(|| format!("create screenshot dir {}", screenshot_dir.display()))?;
    fs::write(&filepath, &decoded)
        .with_context(|| format!("write screenshot {}", filepath.display()))?;

    // 4. Build ISO 8601 timestamp
    let captured_at = iso_now();

    Ok(ReplayScreenshotEntry {
        step_line,
        step_keyword: step_keyword.to_string(),
        step_text: step_text.to_string(),
        screenshot_file: filename,
        captured_at,
    })
}

/// Reads the existing index.json or creates a fresh one.
pub fn load_or_create_index(screenshot_dir: &Path, feature: &str) -> ReplayScreenshotsIndex {
    let index_path = screenshot_dir.join("index.json");
    if let Ok(text) = fs::read_to_string(&index_path) {
        if let Ok(idx) = serde_json::from_str::<ReplayScreenshotsIndex>(&text) {
            return idx;
        }
    }
    ReplayScreenshotsIndex {
        feature: feature.to_string(),
        started_at: iso_now(),
        steps: Vec::new(),
        completed_at: None,
        status: "in_progress".to_string(),
    }
}

/// Writes index.json to the screenshot directory.
pub fn save_index(screenshot_dir: &Path, index: &ReplayScreenshotsIndex) -> Result<()> {
    let path = screenshot_dir.join("index.json");
    let text = serde_json::to_string_pretty(index).context("serialize screenshot index")?;
    fs::write(&path, &text).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn iso_now() -> String {
    // Simple ISO-like timestamp. We don't have chrono in tree deps as
    // a direct dep of the CLI crate, so use local time via std.
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // Format as ISO 8601 without external dep: YYYY-MM-DDTHH:MM:SS.fffZ
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    // Use a simple approach: unix timestamp string
    format!("{}", secs as f64 + millis as f64 / 1000.0)
}
