//! Project-scoped, durable replay events and screenshot evidence.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Persisted run identity and incremental events shared by CLI and GPUI hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRunSnapshot {
    /// Opaque run identifier; never a caller-supplied filesystem path.
    pub id: String,
    /// UTC creation time.
    pub started_at: String,
    /// Running or terminal outcome, including partial, blocked and interrupted runs.
    pub status: String,
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
            id: uuid::Uuid::new_v4().to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            status: "running".into(),
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
        crate::write_atomic(&self.directory.join("run.json"), &self.snapshot)
    }

    /// Persist an event before exposing it to observers.
    ///
    /// # Errors
    /// Returns an error when the incremental manifest cannot be published.
    pub fn record(&mut self, event: Value) -> Result<()> {
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
        }
        self.snapshot.events.push(event);
        self.save()
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
        metadata["capture_status"] = json!("available");
        metadata["screenshot_file"] = json!(filename);
        metadata["captured_at"] = json!(chrono::Utc::now().to_rfc3339());
        self.record(metadata.clone())?;
        Ok(metadata)
    }
}

impl Drop for ReplayEvidenceRun {
    fn drop(&mut self) {
        // Early returns and unwinding must not leave failed runs looking active.
        if self.snapshot.status == "running" {
            self.snapshot.status = "interrupted".into();
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
                snapshot.events.push(
                    json!({"type":"end_run","status":"interrupted", "reason":"writer_exited"}),
                );
                crate::write_atomic(&path, &snapshot)?;
                let index_path = path.with_file_name("index.json");
                if let Ok(bytes) = fs::read(&index_path) {
                    if let Ok(mut index) = serde_json::from_slice::<Value>(&bytes) {
                        index["status"] = json!("interrupted");
                        index["completed_at"] = json!(chrono::Utc::now().to_rfc3339());
                        crate::write_atomic(&index_path, &index)?;
                    }
                }
            }
        }
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

fn validate_replay_image(bytes: &[u8]) -> Result<()> {
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
    let encoded = response["screenshot"]
        .as_str()
        .ok_or_else(|| anyhow!("capture returned no image"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    validate_replay_image(&bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

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
