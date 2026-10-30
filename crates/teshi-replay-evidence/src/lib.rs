//! Project-scoped, durable replay events and screenshot evidence.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Persisted run identity and incremental events shared by CLI and GPUI hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRunSnapshot {
    /// Schema version; older manifests remain readable.
    #[serde(default)]
    pub schema_version: u32,
    /// Opaque run identifier; never a caller-supplied filesystem path.
    pub id: String,
    /// UTC creation time.
    pub started_at: String,
    /// Running or terminal outcome, including partial, blocked and interrupted runs.
    pub status: String,
    /// Original execution outcome, independent of evidence completeness.
    #[serde(default)]
    pub test_status: String,
    /// Combined execution and evidence acceptance.
    #[serde(default)]
    pub acceptance_status: String,
    /// Per-execution screenshot coverage.
    #[serde(default)]
    pub evidence: teshi_core::replay_evidence::EvidenceSummary,
    /// Complete, incomplete, or explicitly not applicable visual evidence.
    #[serde(default)]
    pub evidence_status: String,
    /// Build of the runner; target identity is recorded separately in events.
    #[serde(default)]
    pub runner_version: Value,
    /// Caller-supplied target version, or an explicit explanation when unknown.
    #[serde(default)]
    pub target_version: Value,
    /// Events retain step identity and capture outcomes, including failures.
    pub events: Vec<Value>,
}

/// Single writer for one immutable run directory.
pub struct ReplayEvidenceRun {
    directory: PathBuf,
    snapshot: ReplayRunSnapshot,
    _owner: fs::File,
}

fn root(project: &Path) -> PathBuf {
    project.join(".teshi/logs/replay-screenshots")
}

/// Whether an explicit teardown action requires its last frame before execution.
/// Clicks that happen to close a window must be identified by their binding.
pub fn capture_before_action(action: &str) -> bool {
    matches!(
        action,
        "close" | "close_tab" | "close_window" | "quit" | "exit" | "terminate" | "expect_exit"
    )
}

/// UTC wall-clock capture time shared by runner adapters.
pub fn capture_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn managed_root(project: &Path) -> Result<PathBuf> {
    let project = project.canonicalize()?;
    let managed = root(&project)
        .canonicalize()
        .context("replay history unavailable")?;
    anyhow::ensure!(
        managed.starts_with(&project),
        "replay history is outside its project"
    );
    Ok(managed)
}

fn run_directory(project: &Path, id: &str) -> Result<PathBuf> {
    if id == "legacy" {
        return managed_root(project);
    }
    let parsed = uuid::Uuid::parse_str(id).context("invalid replay run id")?;
    anyhow::ensure!(parsed.to_string() == id, "invalid replay run id");
    let managed = managed_root(project)?;
    let directory = managed
        .join(id)
        .canonicalize()
        .context("replay run unavailable")?;
    anyhow::ensure!(
        directory.parent() == Some(managed.as_path()),
        "replay run is outside its project"
    );
    Ok(directory)
}

impl ReplayEvidenceRun {
    /// Create an isolated run without replacing earlier evidence.
    ///
    /// # Errors
    /// Returns an error when the evidence directory or initial manifest cannot be written.
    pub fn create(project: &Path) -> Result<Self> {
        let snapshot = ReplayRunSnapshot {
            schema_version: 2,
            id: uuid::Uuid::new_v4().to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            status: "running".into(),
            test_status: "running".into(),
            acceptance_status: "running".into(),
            evidence: Default::default(),
            evidence_status: "complete".into(),
            runner_version: serde_json::to_value(teshi_core::version::build_identity())?,
            target_version: json!({"status":"unknown","reason":"target_version_not_reported"}),
            events: Vec::new(),
        };
        fs::create_dir_all(root(project))?;
        let directory = managed_root(project)?.join(&snapshot.id);
        fs::create_dir(&directory)?;
        let owner = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.join("owner.lock"))?;
        owner.lock()?;
        let run = Self {
            directory,
            snapshot,
            _owner: owner,
        };
        run.save()?;
        Ok(run)
    }

    /// Run identifier for polling and history selection.
    pub fn id(&self) -> &str {
        &self.snapshot.id
    }

    /// Directory in which this run stores its images.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    fn save(&self) -> Result<()> {
        write_atomic(&self.directory.join("run.json"), &self.snapshot)
    }

    /// Persist an event before exposing it to observers.
    ///
    /// # Errors
    /// Returns an error when the incremental manifest cannot be published.
    pub fn record(&mut self, mut event: Value) -> Result<()> {
        if event["type"] == "target_identity" {
            self.snapshot.target_version = event.clone();
        }
        if event["type"] == "step_screenshot"
            && event["capture_status"] == "not_applicable"
            && self.snapshot.events.iter().any(|planned| {
                planned["type"] == "planned_step"
                    && planned["screenshot_required"] == true
                    && planned["case_id"] == event["case_id"]
                    && planned["step_id"] == event["step_id"]
                    && planned["attempt"].as_u64().unwrap_or(1)
                        == event["attempt"].as_u64().unwrap_or(1)
            })
        {
            event["capture_status"] = json!("failed");
            event["message"] = json!("required_ui_capture_cannot_be_not_applicable");
        }
        if event["type"] == "step_screenshot" && event["capture_status"] == "available" {
            match validate_capture_files(&self.directory, &event) {
                Ok((width, height, format)) => {
                    event["width"] = json!(width);
                    event["height"] = json!(height);
                    event["format"] = json!(format);
                }
                Err(error) => {
                    event["capture_status"] = json!("failed");
                    event["message"] = json!(error.to_string());
                }
            }
        }
        if event["step_id"].is_string() {
            if event["attempt"].is_null() {
                event["attempt"] = json!(1);
            }
            event["recorded_at"] = json!(chrono::Utc::now().to_rfc3339());
            if event["type"] == "step_screenshot"
                && event["capture_status"] != "available"
                && event["message"].is_null()
            {
                event["message"] = json!(if event["capture_status"] == "not_applicable" {
                    "nonvisual_step"
                } else {
                    "capture_unavailable"
                });
            }
        }
        if matches!(
            event["type"].as_str(),
            Some("case_passed" | "case_failed" | "end_example")
        ) {
            let case = event["case_id"].as_str().unwrap_or_default();
            let case = if event["type"] == "end_example" {
                format!("{case}:example-{}", event["example_line"])
            } else {
                case.to_string()
            };
            let counts = teshi_core::replay_evidence::summarize_case(&self.snapshot.events, &case);
            let test = if event["type"] == "case_passed" || event["status"] == "passed" {
                "passed"
            } else {
                "failed"
            };
            event["test_status"] = json!(test);
            event["evidence_status"] = json!(counts.status());
            event["acceptance_status"] = json!(counts.acceptance(test));
            event["evidence"] = json!(counts);
        }
        if event["type"] == "end_run" {
            self.snapshot.status = if let Some(status) = event["status"].as_str().filter(|status| {
                matches!(
                    *status,
                    "passed"
                        | "failed"
                        | "partial"
                        | "blocked"
                        | "manual"
                        | "unbound"
                        | "interrupted"
                )
            }) {
                status
            } else if event["failed"].as_u64().unwrap_or(0) > 0 {
                "failed"
            } else {
                "passed"
            }
            .into();
            self.snapshot.test_status = self.snapshot.status.clone();
        }
        if event["type"] == "end_run" {
            self.finalize_missing();
        }
        self.snapshot.events.push(event);
        self.refresh();
        self.save()?;
        if self.snapshot.test_status != "running" {
            let index_path = self.directory.join("index.json");
            if index_path.is_file() {
                let mut index: Value = serde_json::from_slice(&fs::read(&index_path)?)?;
                index["test_status"] = json!(self.snapshot.test_status);
                index["status"] = json!(self.snapshot.acceptance_status);
                index["evidence"] = json!(self.snapshot.evidence);
                write_atomic(&index_path, &index)?;
            }
        }
        Ok(())
    }

    fn finalize_missing(&mut self) {
        let mut missing = teshi_core::replay_evidence::missing_capture_rows(&self.snapshot.events);
        for event in &mut missing {
            event["recorded_at"] = json!(capture_timestamp());
        }
        self.snapshot.events.extend(missing);
    }

    fn refresh(&mut self) {
        self.snapshot.evidence =
            teshi_core::replay_evidence::summarize(&self.snapshot.events, None);
        self.snapshot.acceptance_status = self
            .snapshot
            .evidence
            .acceptance(&self.snapshot.test_status)
            .into();
        self.snapshot.status = self.snapshot.acceptance_status.clone();
        self.snapshot.evidence_status = self.snapshot.evidence.status().into();
    }

    /// Current durable execution and evidence results.
    pub fn snapshot(&self) -> &ReplayRunSnapshot {
        &self.snapshot
    }

    /// Fail acceptance when assertions passed but required evidence is incomplete.
    ///
    /// # Errors
    /// Returns an error for missing, failed, or unexecuted required captures.
    pub fn ensure_accepted(&self) -> Result<()> {
        anyhow::ensure!(
            self.snapshot.acceptance_status == "passed",
            "replay acceptance {} (tests: {}, evidence: {:?})",
            self.snapshot.acceptance_status,
            self.snapshot.test_status,
            self.snapshot.evidence
        );
        Ok(())
    }

    /// Copy a captured image and publish its step association immediately.
    ///
    /// # Errors
    /// Returns an error for empty/oversized captures or failed persistence.
    pub fn screenshot(&mut self, bytes: &[u8], mut metadata: Value) -> Result<Value> {
        anyhow::ensure!(
            !bytes.is_empty() && bytes.len() <= 16 * 1024 * 1024,
            "invalid screenshot size"
        );
        validate_replay_image(bytes)?;
        let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "png"
        } else {
            "jpg"
        };
        let filename = format!("{}.{}", uuid::Uuid::new_v4(), extension);
        fs::write(self.directory.join(&filename), bytes)?;
        metadata["type"] = json!("step_screenshot");
        if metadata["capture_status"] != "failed" {
            metadata["capture_status"] = json!("available");
        }
        metadata["screenshot_file"] = json!(filename);
        metadata["captured_at"] = json!(chrono::Utc::now().to_rfc3339());
        self.record(metadata)?;
        self.snapshot
            .events
            .last()
            .cloned()
            .context("capture record missing")
    }
}

impl Drop for ReplayEvidenceRun {
    fn drop(&mut self) {
        // Early returns and unwinding must not leave failed runs looking active.
        if self.snapshot.status == "running" {
            self.snapshot.status = "interrupted".into();
            self.snapshot.test_status = "interrupted".into();
            self.finalize_missing();
            self.refresh();
            let _ = self.save();
        }
    }
}

/// Read a run from the selected project using its opaque identifier.
///
/// # Errors
/// Returns an error for invalid IDs, escaped paths, or unreadable manifests.
pub fn load_replay_run(project: &Path, id: &str) -> Result<ReplayRunSnapshot> {
    if id == "legacy" {
        let directory = managed_root(project)?;
        let index = directory.join("index.json").canonicalize()?;
        anyhow::ensure!(
            index.parent() == Some(directory.as_path()),
            "legacy index escaped its project"
        );
        let index: Value = serde_json::from_slice(&fs::read(index)?)?;
        let mut events = vec![
            json!({"type":"start_case", "case_id":"legacy", "feature":index["feature"], "name":index["feature"]}),
        ];
        for step in index["steps"].as_array().into_iter().flatten() {
            anyhow::ensure!(step.is_object(), "legacy screenshot entry is invalid");
            let mut event = step.clone();
            event["type"] = json!("step_screenshot");
            event["case_id"] = json!("legacy");
            event["step_id"] = json!(format!("legacy:L{}", step["step_line"]));
            event["feature"] = index["feature"].clone();
            event["capture_status"] = json!("available");
            event["status"] = json!("unknown");
            events.push(event);
        }
        return Ok(ReplayRunSnapshot {
            schema_version: 0,
            test_status: "unknown".into(),
            acceptance_status: "unknown".into(),
            evidence: teshi_core::replay_evidence::summarize(&events, None),
            evidence_status: "unknown".into(),
            runner_version: Value::Null,
            target_version: Value::Null,
            id: "legacy".into(),
            started_at: index["started_at"]
                .as_str()
                .unwrap_or("Legacy screenshots")
                .into(),
            status: if index["status"] == "completed" {
                "completed"
            } else {
                "unknown"
            }
            .into(),
            events,
        });
    }
    let path = run_directory(project, id)?.join("run.json");
    let canonical = path.canonicalize()?;
    anyhow::ensure!(
        canonical.parent() == path.parent(),
        "replay manifest escaped its run"
    );
    let mut snapshot: ReplayRunSnapshot = serde_json::from_slice(&fs::read(canonical)?)?;
    anyhow::ensure!(snapshot.id == id, "replay identity mismatch");
    // OS file locks disappear on process death. PID-only checks would confuse
    // reused PIDs with the writer and leave interrupted runs active forever.
    if snapshot.status == "running" {
        let lock_path = path.with_file_name("owner.lock");
        if lock_path.exists() {
            let owner = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock_path)?;
            if owner.try_lock().is_ok() {
                snapshot = serde_json::from_slice(&fs::read(&path)?)?;
                if snapshot.status != "running" {
                    return Ok(snapshot);
                }
                snapshot.status = "interrupted".into();
                snapshot.test_status = "interrupted".into();
                snapshot.acceptance_status = "interrupted".into();
                snapshot.events.push(
                    json!({"type":"end_run","status":"interrupted", "reason":"writer_exited"}),
                );
                write_atomic(&path, &snapshot)?;
                let index_path = path.with_file_name("index.json");
                if let Ok(bytes) = fs::read(&index_path) {
                    if let Ok(mut index) = serde_json::from_slice::<Value>(&bytes) {
                        index["status"] = json!("interrupted");
                        index["completed_at"] = json!(chrono::Utc::now().to_rfc3339());
                        write_atomic(&index_path, &index)?;
                    }
                }
            }
        }
    }
    if snapshot.schema_version >= 2 {
        for event in &mut snapshot.events {
            if event["type"] == "step_screenshot" && event["capture_status"] == "available" {
                let directory = path.parent().context("missing run directory")?;
                let valid = validate_capture_files(directory, event).is_ok();
                if !valid {
                    event["capture_status"] = json!("failed");
                    event["message"] = json!("saved_screenshot_missing_or_invalid");
                }
            }
        }
        snapshot.evidence = teshi_core::replay_evidence::summarize(&snapshot.events, None);
        snapshot.acceptance_status = snapshot.evidence.acceptance(&snapshot.test_status).into();
        snapshot.status = snapshot.acceptance_status.clone();
        snapshot.evidence_status = snapshot.evidence.status().into();
        let case_ids: std::collections::BTreeSet<_> = snapshot
            .events
            .iter()
            .filter(|event| matches!(event["type"].as_str(), Some("case_passed" | "case_failed")))
            .filter_map(|event| event["case_id"].as_str())
            .collect();
        let cases: std::collections::BTreeMap<String, _> = case_ids
            .into_iter()
            .map(|id| {
                (
                    id.to_string(),
                    teshi_core::replay_evidence::summarize_case(&snapshot.events, id),
                )
            })
            .collect();
        let examples: std::collections::BTreeMap<String, _> = snapshot
            .events
            .iter()
            .filter(|event| event["type"] == "end_example")
            .filter_map(|event| {
                event["case_id"]
                    .as_str()
                    .map(|id| format!("{id}:example-{}", event["example_line"]))
            })
            .map(|id| {
                let counts = teshi_core::replay_evidence::summarize_case(&snapshot.events, &id);
                (id, counts)
            })
            .collect();
        for event in &mut snapshot.events {
            if !matches!(
                event["type"].as_str(),
                Some("case_passed" | "case_failed" | "end_example")
            ) {
                continue;
            }
            let case = event["case_id"].as_str().unwrap_or_default();
            let counts = if event["type"] == "end_example" {
                examples.get(&format!("{case}:example-{}", event["example_line"]))
            } else {
                cases.get(case)
            };
            let Some(counts) = counts else {
                continue;
            };
            let test = if event["type"] == "case_passed" || event["status"] == "passed" {
                "passed"
            } else {
                "failed"
            };
            event["evidence"] = json!(counts);
            event["evidence_status"] = json!(counts.status());
            event["acceptance_status"] = json!(counts.acceptance(test));
        }
    }
    if snapshot.schema_version < 2 {
        // Historical captures do not define the expected step plan. They can be
        // inspected, but cannot prove completeness under the current policy.
        snapshot.test_status = snapshot.status.clone();
        snapshot.evidence = teshi_core::replay_evidence::summarize(&snapshot.events, None);
        snapshot.evidence_status = "unknown".into();
        snapshot.acceptance_status = if snapshot.test_status == "passed" {
            "unverified".into()
        } else {
            snapshot.test_status.clone()
        };
        snapshot.status = snapshot.acceptance_status.clone();
    }
    Ok(snapshot)
}

/// List run summaries, newest first, without loading image bytes into the UI.
///
/// # Errors
/// Returns an error when the project history directory cannot be enumerated.
pub fn list_replay_runs(project: &Path) -> Result<Vec<ReplayRunSnapshot>> {
    if !root(project).exists() {
        return Ok(Vec::new());
    }
    let mut runs = Vec::new();
    if let Ok(mut legacy) = load_replay_run(project, "legacy") {
        legacy.events.clear();
        runs.push(legacy);
    }
    for entry in fs::read_dir(root(project))? {
        let entry = entry?;
        if let Some(id) = entry.file_name().to_str() {
            if let Ok(mut run) = load_replay_run(project, id) {
                run.events.clear();
                runs.push(run);
            }
        }
    }
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(runs)
}

/// Load only an image explicitly associated with a step in this run.
///
/// # Errors
/// Returns an error for missing, oversized, unassociated, or escaped images.
pub fn load_replay_image(project: &Path, id: &str, filename: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(
        Path::new(filename)
            .file_name()
            .and_then(|name| name.to_str())
            == Some(filename),
        "invalid screenshot name"
    );
    let run = load_replay_run(project, id)?;
    anyhow::ensure!(run.events.iter().any(|event| event["type"] == "step_screenshot" && event["screenshot_file"] == filename), "screenshot does not belong to this run");
    let directory = run_directory(project, id)?;
    let path = directory
        .join(filename)
        .canonicalize()
        .context("saved screenshot is missing")?;
    anyhow::ensure!(
        path.parent() == Some(directory.as_path()),
        "screenshot escaped its run"
    );
    anyhow::ensure!(
        fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
        "screenshot is too large"
    );
    let bytes = fs::read(path)?;
    validate_replay_image(&bytes)?;
    Ok(bytes)
}

fn validate_capture_files(directory: &Path, event: &Value) -> Result<(u32, u32, &'static str)> {
    let read = |key: &str| -> Result<Vec<u8>> {
        let filename = event[key]
            .as_str()
            .context("capture artifact name missing")?;
        anyhow::ensure!(
            Path::new(filename).file_name().and_then(|v| v.to_str()) == Some(filename),
            "invalid capture artifact name"
        );
        let path = directory
            .join(filename)
            .canonicalize()
            .context("saved capture artifact missing")?;
        anyhow::ensure!(
            path.parent() == Some(directory),
            "capture artifact escaped its run"
        );
        anyhow::ensure!(
            fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
            "capture artifact too large"
        );
        Ok(fs::read(path)?)
    };
    let bytes = read("screenshot_file")?;
    validate_replay_image(&bytes)?;
    if event["source"] == "pty" {
        anyhow::ensure!(!read("svg_file")?.is_empty(), "terminal SVG missing");
        read("text_file")?;
    }
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else {
        "jpeg"
    };
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .into_dimensions()?;
    Ok((width, height, format))
}

/// Record target executable content identity without substituting runner identity.
/// An unresolved target retains an explicit reason instead of an invented version.
pub fn target_executable_identity(program: &Path) -> Value {
    use sha2::{Digest, Sha256};
    let identity = (|| -> std::io::Result<String> {
        use std::io::Read;
        let mut file = fs::File::open(program)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
        Ok(format!("{:x}", hash.finalize()))
    })();
    match identity {
        Ok(hash) => json!({"type":"target_identity","program":program,"sha256":hash}),
        Err(error) => {
            json!({"type":"target_identity","program":program,"status":"unknown","reason":error.to_string()})
        }
    }
}

/// Validate image format, dimensions and bounded decoding before publishing.
///
/// # Errors
/// Returns decoding, size or unsupported-format errors.
pub fn validate_replay_image(bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= 16 * 1024 * 1024,
        "invalid screenshot size"
    );
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    anyhow::ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Jpeg | image::ImageFormat::Png)
        ),
        "unsupported screenshot format"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().context("saved screenshot is unreadable")?;
    Ok(())
}

fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    use std::io::Write;
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("manifest has no parent")?)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.flush()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_failure_blocks_acceptance_without_changing_tests_or_losing_following_steps() {
        let project = tempfile::tempdir().unwrap();
        let mut run = ReplayEvidenceRun::create(project.path()).unwrap();
        for line in 1..=3 {
            run.record(json!({"type":"planned_step","case_id":"row1","step_id":format!("L{line}"),"feature":"a.feature"})).unwrap();
        }
        run.record(json!({"type":"step_screenshot","case_id":"row1","step_id":"L1","capture_status":"failed","message":"executor_busy"})).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let image = run
            .screenshot(
                &png.into_inner(),
                json!({"case_id":"row1","step_id":"L2","status":"passed"}),
            )
            .unwrap();
        run.record(json!({"type":"end_run","failed":0})).unwrap();
        assert_eq!(run.snapshot().test_status, "passed");
        assert_eq!(run.snapshot().acceptance_status, "incomplete");
        assert_eq!(
            (
                run.snapshot().evidence.required,
                run.snapshot().evidence.saved,
                run.snapshot().evidence.failed,
                run.snapshot().evidence.not_executed
            ),
            (3, 1, 1, 1)
        );
        assert!(run.ensure_accepted().is_err());
        let restored = load_replay_run(project.path(), run.id()).unwrap();
        assert_eq!(restored.events[3]["message"], "executor_busy");
        fs::remove_file(
            run.directory()
                .join(image["screenshot_file"].as_str().unwrap()),
        )
        .unwrap();
        let restored = load_replay_run(project.path(), run.id()).unwrap();
        assert_eq!(restored.evidence.saved, 0);
        assert_eq!(restored.evidence.failed, 2);
    }

    #[test]
    fn nonvisual_output_is_explicitly_exempt_and_unrecorded_capture_is_missing() {
        let project = tempfile::tempdir().unwrap();
        let mut run = ReplayEvidenceRun::create(project.path()).unwrap();
        run.record(json!({"type":"step_screenshot","step_id":"cli","capture_status":"not_applicable","source":"cli","message":"no viewport","stdout":"ok","stderr":"","exit_code":0})).unwrap();
        run.record(json!({"type":"end_run","failed":0})).unwrap();
        assert!(run.ensure_accepted().is_ok());
        assert_eq!(run.snapshot().evidence.not_applicable, 1);
        let mut other = ReplayEvidenceRun::create(project.path()).unwrap();
        other
            .record(json!({"type":"start_step","step_id":"ui"}))
            .unwrap();
        other.record(json!({"type":"end_run","failed":0})).unwrap();
        assert_eq!(other.snapshot().evidence.missing, 1);
        assert_eq!(other.snapshot().status, "incomplete");
    }

    #[test]
    fn old_run_without_an_expected_plan_is_unverified_and_is_not_rewritten() {
        let project = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let directory = root(project.path()).join(&id);
        fs::create_dir_all(&directory).unwrap();
        let original =
            json!({"id":id,"started_at":"2026-10-20","status":"passed","events":[]}).to_string();
        fs::write(directory.join("run.json"), &original).unwrap();
        let snapshot = load_replay_run(project.path(), &id).unwrap();
        assert_eq!(snapshot.test_status, "passed");
        assert_eq!(snapshot.acceptance_status, "unverified");
        assert_eq!(snapshot.evidence_status, "unknown");
        assert_eq!(
            fs::read_to_string(directory.join("run.json")).unwrap(),
            original
        );
    }

    #[test]
    fn an_unacceptable_frame_is_retained_without_becoming_accepted() {
        let project = tempfile::tempdir().unwrap();
        let mut run = ReplayEvidenceRun::create(project.path()).unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let image=run.screenshot(&bytes.into_inner(),json!({"step_id":"closing","capture_status":"failed","message":"engine_did_not_confirm_capture_before_close"})).unwrap();
        run.record(json!({"type":"end_run","failed":0})).unwrap();
        assert_eq!(run.snapshot().status, "incomplete");
        assert_eq!(run.snapshot().evidence.failed, 1);
        assert!(!load_replay_image(
            project.path(),
            run.id(),
            image["screenshot_file"].as_str().unwrap()
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn legacy_index_is_read_without_rewriting_or_inventing_step_results() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir_all(root(project.path())).unwrap();
        let index = json!({"feature":"old.feature", "status":"completed", "started_at":"123.5",
            "steps":[{"step_line":8,"step_keyword":"Then","step_text":"image exists","screenshot_file":"old.jpg","captured_at":"123.6"}]});
        let raw = index.to_string();
        fs::write(root(project.path()).join("index.json"), &raw).unwrap();
        let run = load_replay_run(project.path(), "legacy").unwrap();
        assert_eq!(run.events[1]["status"], "unknown");
        assert_eq!(run.events[1]["screenshot_file"], "old.jpg");
        assert_eq!(list_replay_runs(project.path()).unwrap()[0].id, "legacy");
        assert_eq!(
            fs::read_to_string(root(project.path()).join("index.json")).unwrap(),
            raw
        );
        assert!(!root(project.path()).join("run.json").exists());
    }

    #[test]
    fn captures_are_available_before_completion_and_survive_failure_and_reopen() {
        let project = tempfile::tempdir().unwrap();
        let mut run = ReplayEvidenceRun::create(project.path()).unwrap();
        let id = run.id().to_string();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let bytes = png.into_inner();
        let image = run
            .screenshot(&bytes, json!({"step_id":"one", "status":"failed"}))
            .unwrap();
        assert_eq!(
            load_replay_run(project.path(), &id).unwrap().status,
            "running"
        );
        assert_eq!(
            load_replay_image(
                project.path(),
                &id,
                image["screenshot_file"].as_str().unwrap()
            )
            .unwrap(),
            bytes
        );
        run.record(json!({"type":"end_run", "failed":1})).unwrap();
        drop(run);
        assert_eq!(
            load_replay_run(project.path(), &id).unwrap().status,
            "failed"
        );
        let second = ReplayEvidenceRun::create(project.path()).unwrap();
        assert_ne!(second.id(), id);
        assert_eq!(list_replay_runs(project.path()).unwrap().len(), 2);
        assert!(load_replay_image(
            project.path(),
            second.id(),
            image["screenshot_file"].as_str().unwrap()
        )
        .is_err());
        let other = tempfile::tempdir().unwrap();
        assert!(load_replay_run(other.path(), &id).is_err());
        assert!(load_replay_run(project.path(), "../run").is_err());
        assert!(load_replay_image(project.path(), &id, "../image.jpg").is_err());
        let image_path = run_directory(project.path(), &id)
            .unwrap()
            .join(image["screenshot_file"].as_str().unwrap());
        fs::write(&image_path, b"\xff\xd8corrupt").unwrap();
        assert!(load_replay_image(
            project.path(),
            &id,
            image["screenshot_file"].as_str().unwrap()
        )
        .is_err());
        fs::remove_file(
            run_directory(project.path(), &id)
                .unwrap()
                .join(image["screenshot_file"].as_str().unwrap()),
        )
        .unwrap();
        assert!(load_replay_image(
            project.path(),
            &id,
            image["screenshot_file"].as_str().unwrap()
        )
        .is_err());
    }

    #[test]
    fn abandoned_writer_is_interrupted_but_live_writer_is_not() {
        let project = tempfile::tempdir().unwrap();
        let mut run = ReplayEvidenceRun::create(project.path()).unwrap();
        run.record(json!({"type":"step_result", "status":"passed"}))
            .unwrap();
        let id = run.id().to_owned();
        let path = run.directory().join("run.json");
        let live = fs::read(&path).unwrap();
        assert_eq!(
            load_replay_run(project.path(), &id).unwrap().status,
            "running"
        );
        drop(run);
        // Reproduce the durable state left by abrupt process death: a running
        // manifest and an unlocked owner file, with its last result intact.
        fs::write(&path, live).unwrap();
        let recovered = load_replay_run(project.path(), &id).unwrap();
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(recovered.events[0]["status"], "passed");
        let persisted: ReplayRunSnapshot =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted.status, "interrupted");
    }
}
