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
    /// Expanded case, execution step and scenario identities.
    pub identity: Option<(&'a str, &'a str, &'a str)>,
    /// Source step line, keyword, and text.
    pub source: (usize, &'a str, &'a str),
    /// Whether executing the step succeeded.
    pub passed: bool,
    /// Original backend execution response, kept independently of capture errors.
    pub response: Option<&'a serde_json::Value>,
    /// Whether the selected backend supports screenshots.
    pub capture_supported: bool,
    /// Original capture error, including backend codes.
    pub capture_error: Option<&'a str>,
    /// Whether the image was captured before a teardown action.
    pub before_close: bool,
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
        identity,
        source: (line, keyword, text),
        passed,
        response,
        capture_supported: supported,
        capture_error,
        before_close,
    } = step;
    let fallback = format!("replay:L{line}");
    let (case_id, step_id, scenario) = identity.unwrap_or(("replay", &fallback, ""));
    let mut event = json!({"type":"step_screenshot", "case_id":case_id, "step_id":step_id,
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
    if !scenario.is_empty() {
        event["scenario"] = json!(scenario);
    }
    if let Some(entry) = entry {
        event["screenshot_file"] = json!(entry.screenshot_file);
        event["captured_at"] = json!(entry.captured_at);
    }
    event["message"] = json!(capture_error.or(if !supported {
        Some("broker_capability_unavailable")
    } else {
        None
    }));
    event["execution_response"] = json!(response);
    event["source"] = json!("web_page");
    event["capture_timing"] = json!(if before_close {
        "before_close"
    } else {
        "after_step"
    });
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

    anyhow::ensure!(response["ok"] != false, "capture rejected: {}", response);
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

/// Resolves a screenshot's broker-generated filename inside this project's
/// managed browser artifact directory, independently of the process directory.
///
/// # Errors
///
/// Returns an error for missing/unmanaged artifacts, unsafe filenames, unreadable
/// files, or paths that resolve outside the project's managed directory.
pub fn artifact_path_from_screenshot_payload(
    project_root: &Path,
    payload: &serde_json::Value,
) -> Result<PathBuf> {
    let filename = payload
        .get("artifact")
        .and_then(|artifact| artifact.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("screenshot response missing artifact.path"))?;
    anyhow::ensure!(
        payload["artifact"]["managed"] == true,
        "screenshot artifact is not managed"
    );
    anyhow::ensure!(
        !matches!(filename, "." | "..")
            && filename
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "screenshot artifact.path must be a managed filename"
    );
    let project = project_root
        .canonicalize()
        .context("resolve screenshot project")?;
    let managed = project
        .join(".teshi/artifacts/browser")
        .canonicalize()
        .context("resolve managed browser artifact directory")?;
    anyhow::ensure!(
        managed.starts_with(&project),
        "browser artifact directory escaped project"
    );
    let path = managed
        .join(filename)
        .canonicalize()
        .context("resolve screenshot artifact")?;
    anyhow::ensure!(
        path.parent() == Some(managed.as_path()) && path.is_file(),
        "screenshot artifact escaped its managed directory"
    );
    Ok(path)
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
    fn artifact_path_from_screenshot_payload_resolves_project_managed_filename() {
        let project = tempfile::tempdir().unwrap();
        let managed = project.path().join(".teshi/artifacts/browser");
        fs::create_dir_all(&managed).unwrap();
        fs::write(managed.join("shot.jpeg"), b"captured-image").unwrap();
        let payload = json!({
            "ok": true,
            "operation": "capture_browser_screenshot",
            "artifact": {
                "path": "shot.jpeg",
                "format": "jpeg",
                "managed": true
            }
        });
        let path = artifact_path_from_screenshot_payload(project.path(), &payload).unwrap();
        assert_eq!(path, managed.join("shot.jpeg").canonicalize().unwrap());
        let replay = project.path().join("replay");
        let entry =
            save_screenshot_from_artifact(&path, "review", 3, "Then", "ready", &replay).unwrap();
        assert_eq!(
            fs::read(replay.join(entry.screenshot_file)).unwrap(),
            b"captured-image"
        );
    }

    #[test]
    fn artifact_path_from_screenshot_payload_rejects_missing_path() {
        let err = artifact_path_from_screenshot_payload(Path::new("."), &json!({"ok": true}))
            .unwrap_err();
        assert!(err.to_string().contains("artifact.path"));
    }

    #[test]
    fn screenshot_artifact_rejects_unmanaged_and_non_filename_paths() {
        for filename in [
            ".",
            "..",
            "../shot.jpeg",
            "folder/shot.jpeg",
            "folder\\shot.jpeg",
            "/shot.jpeg",
            "C:\\shot.jpeg",
            "shot.jpeg:stream",
        ] {
            let payload = json!({"artifact":{"path":filename,"managed":true}});
            let error =
                artifact_path_from_screenshot_payload(Path::new("."), &payload).unwrap_err();
            assert!(
                error.to_string().contains("managed filename"),
                "{filename}: {error}"
            );
        }
        let payload = json!({"artifact":{"path":"shot.jpeg","managed":false}});
        let error = artifact_path_from_screenshot_payload(Path::new("."), &payload).unwrap_err();
        assert!(error.to_string().contains("not managed"));
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
        let entry = {
            let bytes: &[u8] = &[
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 96,
                96, 96, 0, 0, 0, 4, 0, 1, 246, 23, 56, 85, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
                130,
            ];
            save_jpeg_bytes(bytes, "review", 3, "Given", "ready", run.directory()).unwrap()
        };
        record_cli_step(
            &mut run,
            project.path(),
            CliReplayStep {
                feature: "review.feature",
                identity: None,
                source: (3, "Given", "ready"),
                passed: true,
                response: None,
                capture_supported: true,
                capture_error: None,
                before_close: false,
            },
            Some(&entry),
        )
        .unwrap();
        record_cli_step(
            &mut run,
            project.path(),
            CliReplayStep {
                feature: "review.feature",
                identity: None,
                source: (4, "Then", "missing"),
                passed: false,
                response: None,
                capture_supported: true,
                capture_error: None,
                before_close: false,
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
