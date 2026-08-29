use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use teshi_engine::send_sidecar_command_with_timeout;

/// One JPEG captured after a replayed Gherkin step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayScreenshotEntry {
    pub step_line: usize,
    pub step_keyword: String,
    pub step_text: String,
    pub screenshot_file: String,
    pub captured_at: String,
}

/// Index of screenshots for one feature replay run.
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
///
/// This path is for embedded Playwright and WinApp sidecars that answer
/// `cmd: screenshot` with a base64 JPEG. Chrome-bridge replay must use
/// [`save_screenshot_from_artifact`] after `capture_browser_screenshot`.
pub fn capture_and_save_screenshot(
    ws_url: &str,
    _project_root: &Path,
    feature_sanitized: &str,
    step_line: usize,
    step_keyword: &str,
    step_text: &str,
    screenshot_dir: &Path,
) -> Result<ReplayScreenshotEntry> {
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

    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("decode base64 screenshot")?;

    save_jpeg_bytes(
        &decoded,
        feature_sanitized,
        step_line,
        step_keyword,
        step_text,
        screenshot_dir,
    )
}

/// Copies a managed browser screenshot artifact into the replay screenshot directory.
///
/// # Errors
///
/// Returns an error when the artifact cannot be read or the replay file cannot be written.
pub fn save_screenshot_from_artifact(
    artifact_path: &Path,
    feature_sanitized: &str,
    step_line: usize,
    step_keyword: &str,
    step_text: &str,
    screenshot_dir: &Path,
) -> Result<ReplayScreenshotEntry> {
    let bytes = fs::read(artifact_path)
        .with_context(|| format!("read screenshot artifact {}", artifact_path.display()))?;
    save_jpeg_bytes(
        &bytes,
        feature_sanitized,
        step_line,
        step_keyword,
        step_text,
        screenshot_dir,
    )
}

/// Reads the managed artifact path from a `capture_browser_screenshot` payload.
///
/// # Errors
///
/// Returns an error when `artifact.path` is missing or empty.
pub fn artifact_path_from_screenshot_payload(payload: &serde_json::Value) -> Result<PathBuf> {
    let path = payload
        .get("artifact")
        .and_then(|artifact| artifact.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("screenshot response missing artifact.path"))?;
    Ok(PathBuf::from(path))
}

/// Writes JPEG bytes into the replay screenshot directory and returns an index entry.
pub fn save_jpeg_bytes(
    bytes: &[u8],
    feature_sanitized: &str,
    step_line: usize,
    step_keyword: &str,
    step_text: &str,
    screenshot_dir: &Path,
) -> Result<ReplayScreenshotEntry> {
    if bytes.is_empty() {
        return Err(anyhow!("screenshot bytes are empty"));
    }
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let filename = format!("{feature_sanitized}_L{step_line}_{unix_ms}.jpg");
    let filepath = screenshot_dir.join(&filename);

    fs::create_dir_all(screenshot_dir)
        .with_context(|| format!("create screenshot dir {}", screenshot_dir.display()))?;
    fs::write(&filepath, bytes)
        .with_context(|| format!("write screenshot {}", filepath.display()))?;

    Ok(ReplayScreenshotEntry {
        step_line,
        step_keyword: step_keyword.to_string(),
        step_text: step_text.to_string(),
        screenshot_file: filename,
        captured_at: iso_now(),
    })
}

/// Reads the existing index.json or creates a fresh one.
pub fn load_or_create_index(screenshot_dir: &Path, feature: &str) -> ReplayScreenshotsIndex {
    let index_path = screenshot_dir.join("index.json");
    if let Ok(text) = fs::read_to_string(&index_path)
        && let Ok(idx) = serde_json::from_str::<ReplayScreenshotsIndex>(&text)
    {
        return idx;
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

/// Format a Unix-epoch timestamp as a coarse ISO-like string.
pub fn iso_now() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    format!("{}", secs as f64 + millis as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_path_from_screenshot_payload_reads_managed_path() {
        let payload = json!({
            "ok": true,
            "operation": "capture_browser_screenshot",
            "artifact": {
                "path": "D:\\\\tmp\\\\shot.jpg",
                "format": "jpeg"
            }
        });
        let path = artifact_path_from_screenshot_payload(&payload).unwrap();
        assert!(path.ends_with("shot.jpg"));
    }

    #[test]
    fn artifact_path_from_screenshot_payload_rejects_missing_path() {
        let err = artifact_path_from_screenshot_payload(&json!({"ok": true})).unwrap_err();
        assert!(err.to_string().contains("artifact.path"));
    }

    #[test]
    fn save_jpeg_bytes_writes_named_file_and_index_entry() {
        let dir = tempfile::tempdir().unwrap();
        let entry = save_jpeg_bytes(
            b"fake-jpeg",
            "features_run_inspect",
            12,
            "Then",
            "the Run inspect surface is shown",
            dir.path(),
        )
        .unwrap();
        assert_eq!(entry.step_line, 12);
        assert!(
            entry
                .screenshot_file
                .starts_with("features_run_inspect_L12_")
        );
        assert!(entry.screenshot_file.ends_with(".jpg"));
        let written = fs::read(dir.path().join(&entry.screenshot_file)).unwrap();
        assert_eq!(written, b"fake-jpeg");
    }

    #[test]
    fn save_screenshot_from_artifact_copies_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("managed.jpeg");
        fs::write(&artifact, b"jpeg-bytes").unwrap();
        let dest = dir.path().join("replay");
        let entry = save_screenshot_from_artifact(
            &artifact,
            "feat",
            3,
            "When",
            "the user opens the Run surface",
            &dest,
        )
        .unwrap();
        let copied = fs::read(dest.join(&entry.screenshot_file)).unwrap();
        assert_eq!(copied, b"jpeg-bytes");
    }
}
