use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use teshi_engine::replay_evidence::ReplayEvidenceRun;
use teshi_engine::send_sidecar_command_with_timeout;

/// Create a CLI run visible in the GPUI evidence history.
///
/// # Errors
/// Returns an error when the run manifest cannot be created.
pub fn create_cli_replay(
    project: &Path,
    feature: &str,
    scenario: Option<&str>,
) -> Result<ReplayEvidenceRun> {
    let mut run = ReplayEvidenceRun::create(project)?;
    run.record(json!({"type":"start_case", "case_id":"replay", "feature":feature, "name":scenario.unwrap_or(feature)}))?;
    Ok(run)
}

/// Step identity and execution outcome recorded by a CLI replay.
pub struct CliReplayStep<'a> {
    /// Source Feature path relative to the project.
    pub feature: &'a str,
    /// Source step line, keyword, and text.
    pub source: (usize, &'a str, &'a str),
    /// Whether executing the step succeeded.
    pub passed: bool,
    /// Whether the selected backend supports screenshots.
    pub capture_supported: bool,
}

/// Persist the capture outcome independently from the step execution outcome.
///
/// # Errors
/// Returns an error when the capture association or run outcome cannot be saved.
pub fn record_cli_step(
    run: &mut ReplayEvidenceRun,
    project: &Path,
    step: CliReplayStep<'_>,
    entry: Option<&ReplayScreenshotEntry>,
) -> Result<()> {
    let CliReplayStep {
        feature,
        source: (line, keyword, text),
        passed,
        capture_supported: supported,
    } = step;
    let step_id = format!("replay:L{line}");
    let mut event = json!({"type":"step_screenshot", "case_id":"replay", "step_id":step_id,
        "feature":feature, "step_line":line, "step_keyword":keyword, "step_text":text,
        "status":if passed {"passed"} else {"failed"},
        "capture_status":if entry.is_some() {"available"} else if supported {"failed"} else {"unsupported"}});
    if let Ok(source) = fs::read_to_string(project.join(feature)) {
        let parsed = teshi_core::parse_feature(&source, project.join(feature));
        if let Some(scenario) = parsed
            .all_scenarios()
            .into_iter()
            .rev()
            .find(|scenario| scenario.line_number <= line)
        {
            event["scenario"] = json!(scenario.name);
        }
    }
    if let Some(entry) = entry {
        event["screenshot_file"] = json!(entry.screenshot_file);
        event["captured_at"] = json!(entry.captured_at);
    }
    run.record(event)?;
    let mut index = load_or_create_index(run.directory(), feature);
    index.feature = feature.to_string();
    if !passed {
        index.status = "failed".into();
        index.completed_at = Some(iso_now());
    }
    save_index(run.directory(), &index)?;
    if !passed {
        run.record(json!({"type":"end_run", "failed":1}))?;
    }
    Ok(())
}

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

    static CAPTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = CAPTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let filename = format!(
        "{feature_sanitized}_L{step_line}_{unix_ms}_{}_{sequence}.jpg",
        std::process::id()
    );
    let filepath = screenshot_dir.join(&filename);

    fs::create_dir_all(screenshot_dir)
        .with_context(|| format!("create screenshot dir {}", screenshot_dir.display()))?;
    fs::write(&filepath, bytes)
        .with_context(|| format!("write screenshot {}", filepath.display()))?;

    let entry = ReplayScreenshotEntry {
        step_line,
        step_keyword: step_keyword.to_string(),
        step_text: step_text.to_string(),
        screenshot_file: filename,
        captured_at: iso_now(),
    };
    let mut index = load_or_create_index(screenshot_dir, feature_sanitized);
    index.steps.push(entry.clone());
    save_index(screenshot_dir, &index)?;
    Ok(entry)
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
    let value: serde_json::Value = serde_json::from_str(&text)?;
    teshi_engine::write_atomic(&path, &value)?;
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
        // Sidecar payloads use Windows paths; PathBuf::ends_with is separator-sensitive.
        assert!(path.to_string_lossy().ends_with("shot.jpg"), "{path:?}");
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
    #[test]
    fn cli_failure_retains_prior_capture_and_publishes_failed_outcome() {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("review.feature"),
            "Feature: Review\n  Scenario: Capture\n    Given ready\n    Then missing\n",
        )
        .unwrap();
        let mut run = create_cli_replay(project.path(), "review.feature", Some("Capture")).unwrap();
        let entry =
            save_jpeg_bytes(b"fixture", "review", 3, "Given", "ready", run.directory()).unwrap();
        record_cli_step(
            &mut run,
            project.path(),
            CliReplayStep {
                feature: "review.feature",
                source: (3, "Given", "ready"),
                passed: true,
                capture_supported: true,
            },
            Some(&entry),
        )
        .unwrap();
        record_cli_step(
            &mut run,
            project.path(),
            CliReplayStep {
                feature: "review.feature",
                source: (4, "Then", "missing"),
                passed: false,
                capture_supported: true,
            },
            None,
        )
        .unwrap();
        let snapshot =
            teshi_engine::replay_evidence::load_replay_run(project.path(), run.id()).unwrap();
        assert_eq!(snapshot.status, "failed");
        assert_eq!(snapshot.events[1]["scenario"], "Capture");
        assert_eq!(snapshot.events[1]["capture_status"], "available");
        assert_eq!(snapshot.events[2]["capture_status"], "failed");
        assert!(run.directory().join(entry.screenshot_file).is_file());
        let index = load_or_create_index(run.directory(), "review.feature");
        assert_eq!(index.status, "failed");
        assert_eq!(index.steps.len(), 1);
        assert_eq!(index.feature, "review.feature");
    }
}
