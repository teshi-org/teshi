//! Lazy reader for one scenario's newest replay runs.
//!
//! Explore shows what a run recorded, not what the UI happened to keep in
//! memory. Manifests are read directly so opening a historical step never
//! decodes capture images; only the selected run is parsed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::image_view::{self, ScreenshotRef};

/// Upper bound for one case's log rows; the pane is a viewer, not a log sink.
const MAX_LOGS: usize = 2000;

/// One case reconstructed from a run manifest.
#[derive(Debug, Clone)]
pub(crate) struct HistoryCase {
    pub started_at: String,
    pub run_status: String,
    pub case_id: String,
    pub feature: String,
    pub scenario: String,
    pub status: Option<String>,
    pub duration_ms: Option<u64>,
    pub message: Option<String>,
    pub stack: Option<String>,
    pub steps: Vec<HistoryStep>,
    pub logs: Vec<String>,
    pub exchanges: Vec<teshi_core::HttpExchange>,
    pub attachments: Vec<HistoryAttachment>,
    pub screenshots: Vec<ScreenshotRef>,
}

/// One step recorded by `start_step` / `end_step` or synthesized from a capture.
#[derive(Debug, Clone)]
pub(crate) struct HistoryStep {
    pub step_id: String,
    pub text: String,
    pub status: Option<String>,
    pub message: Option<String>,
}

/// One non-image artifact attached to the case.
#[derive(Debug, Clone)]
pub(crate) struct HistoryAttachment {
    pub kind: String,
    pub path: PathBuf,
    pub step_id: Option<String>,
}

/// Runs containing one scenario, discovered newest first.
///
/// The directory list is cheap; manifests are parsed only while the caller
/// asks for another match. Nothing is retained before Explore needs it.
pub(crate) struct ScenarioHistory {
    project: PathBuf,
    feature: String,
    scenario: String,
    lines: Option<(u64, u64)>,
    candidates: Vec<String>,
    cursor: usize,
    matches: Vec<HistoryCase>,
}

impl ScenarioHistory {
    /// Enumerates run directories without reading any manifest.
    pub(crate) fn open(
        project: &Path,
        feature: &str,
        scenario: &str,
        lines: Option<(u64, u64)>,
    ) -> Self {
        let root = project.join(".teshi/logs/replay-screenshots");
        let mut candidates: Vec<(String, SystemTime)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "legacy" || !entry.path().join("run.json").is_file() {
                    continue;
                }
                let modified = entry
                    .path()
                    .join("run.json")
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                candidates.push((name, modified));
            }
        }
        // Newest activity first; the id keeps ties deterministic.
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        Self {
            project: project.to_path_buf(),
            feature: feature.to_string(),
            scenario: scenario.to_string(),
            lines,
            candidates: candidates.into_iter().map(|(id, _)| id).collect(),
            cursor: 0,
            matches: Vec::new(),
        }
    }

    /// Discovers every match so the pane can show `position / total`.
    pub(crate) fn scan_all(&mut self) {
        while self.cursor < self.candidates.len() {
            self.scan_next();
        }
    }

    fn scan_next(&mut self) {
        let run_id = self.candidates[self.cursor].clone();
        self.cursor += 1;
        if let Some(case) = load_case(
            &self.project,
            &run_id,
            &self.feature,
            &self.scenario,
            self.lines,
        ) {
            self.matches.push(case);
        }
    }

    /// Matches discovered so far; `scan_all` makes this the full count.
    pub(crate) fn total(&self) -> usize {
        self.matches.len()
    }

    /// Case at `index`, newest first; `None` when history has no more runs.
    pub(crate) fn case_at(&self, index: usize) -> Option<&HistoryCase> {
        self.matches.get(index)
    }
}

/// Loads the case `run_id` recorded for one scenario.
fn load_case(
    project: &Path,
    run_id: &str,
    feature: &str,
    scenario: &str,
    lines: Option<(u64, u64)>,
) -> Option<HistoryCase> {
    let directory = project.join(".teshi/logs/replay-screenshots").join(run_id);
    let bytes = std::fs::read(directory.join("run.json")).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    // Conforming runs carry the current schema. Anything older has no step
    // plan and is deliberately ignored instead of being guessed at.
    if value["schema_version"].as_u64().unwrap_or(0) < 2 {
        return None;
    }
    let events = value["events"].as_array()?;
    let target_ids = target_case_ids(project, feature, scenario, lines, events)?;
    let window = target_window(events, &target_ids);
    let mut case = HistoryCase {
        started_at: value["started_at"].as_str().unwrap_or_default().to_string(),
        run_status: value["status"].as_str().unwrap_or_default().to_string(),
        case_id: target_ids.iter().next().cloned().unwrap_or_default(),
        feature: feature.to_string(),
        scenario: scenario.to_string(),
        status: None,
        duration_ms: None,
        message: None,
        stack: None,
        steps: Vec::new(),
        logs: Vec::new(),
        exchanges: Vec::new(),
        attachments: Vec::new(),
        screenshots: Vec::new(),
    };
    let mut seen_screenshots = HashSet::new();
    let mut seen_attachments = HashSet::new();
    let has_explicit_steps = events.iter().any(|event| {
        event["case_id"]
            .as_str()
            .is_some_and(|id| target_ids.contains(id))
            && matches!(event["type"].as_str(), Some("start_step" | "end_step"))
    });

    for (index, event) in events.iter().enumerate() {
        let kind = event["type"].as_str().unwrap_or_default();
        let event_case = event["case_id"].as_str();
        let targeted = event_case.is_some_and(|id| target_ids.contains(id));
        match kind {
            "start_case" if targeted => {
                if let Some(id) = event_case {
                    case.case_id = id.to_string();
                }
                let recorded =
                    normalize_feature(project, event["feature"].as_str().unwrap_or_default());
                if !recorded.is_empty() {
                    case.feature = recorded;
                }
                if let Some(name) = event["name"].as_str().filter(|name| !name.is_empty()) {
                    case.scenario = name.to_string();
                }
            }
            "start_step" if targeted => {
                let Some(step_id) = event["step_id"].as_str() else {
                    continue;
                };
                if let Some(step) = case.steps.iter_mut().find(|step| step.step_id == step_id) {
                    if let Some(text) = event["text"].as_str() {
                        step.text = text.to_string();
                    }
                } else {
                    case.steps.push(HistoryStep {
                        step_id: step_id.to_string(),
                        text: event["text"].as_str().unwrap_or_default().to_string(),
                        status: None,
                        message: None,
                    });
                }
            }
            "end_step" if targeted => {
                let Some(step_id) = event["step_id"].as_str() else {
                    continue;
                };
                let status = event["status"].as_str().map(str::to_string);
                let message = event["message"].as_str().map(str::to_string);
                if let Some(step) = case.steps.iter_mut().find(|step| step.step_id == step_id) {
                    step.status.clone_from(&status);
                    step.message.clone_from(&message);
                } else {
                    case.steps.push(HistoryStep {
                        step_id: step_id.to_string(),
                        text: String::new(),
                        status,
                        message,
                    });
                }
            }
            "case_passed" | "case_failed" | "case_skipped" | "case_blocked" if targeted => {
                case.status = Some(kind.trim_start_matches("case_").to_string());
                case.duration_ms = event["duration_ms"].as_u64();
                case.message = event["error"]["message"]
                    .as_str()
                    .or_else(|| event["reason"].as_str())
                    .map(str::to_string);
                case.stack = event["error"]["stack"].as_str().map(str::to_string);
                collect_attachments(
                    &directory,
                    project,
                    event,
                    &mut case.attachments,
                    &mut seen_attachments,
                );
            }
            "end_example" if targeted => {
                collect_attachments(
                    &directory,
                    project,
                    event,
                    &mut case.attachments,
                    &mut seen_attachments,
                );
            }
            "step_screenshot" if targeted => {
                let status = event["status"].as_str().unwrap_or("unknown");
                if event["capture_status"] == "available"
                    && let Some(file) = event["screenshot_file"]
                        .as_str()
                        .filter(|file| !file.is_empty())
                {
                    let capture = ScreenshotRef {
                        run_id: run_id.to_string(),
                        file: file.to_string(),
                        scenario: event["scenario"].as_str().unwrap_or_default().to_string(),
                        step_text: event["step_text"].as_str().unwrap_or_default().to_string(),
                        step_line: event["step_line"].as_u64(),
                        captured_at: event["captured_at"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        status: status.to_string(),
                        image_path: None,
                    };
                    if seen_screenshots.insert(file.to_string()) {
                        case.screenshots.push(capture);
                    }
                }
                // Runners without start_step/end_step still describe their
                // steps through captures.
                if !has_explicit_steps {
                    let step_id = event["step_id"].as_str().map(str::to_string).or_else(|| {
                        event["step_line"]
                            .as_u64()
                            .map(|line| format!("capture:L{line}"))
                    });
                    if let Some(step_id) = step_id {
                        let text = event["step_text"].as_str().unwrap_or_default().to_string();
                        let message = event["message"].as_str().map(str::to_string);
                        if let Some(existing) =
                            case.steps.iter_mut().find(|step| step.step_id == step_id)
                        {
                            if existing.text.is_empty() && !text.is_empty() {
                                existing.text = text;
                            }
                            existing.status = Some(status.to_string());
                            if message.is_some() {
                                existing.message = message;
                            }
                        } else {
                            case.steps.push(HistoryStep {
                                step_id,
                                text,
                                status: Some(status.to_string()),
                                message,
                            });
                        }
                    }
                }
            }
            "http_exchange" if targeted => {
                if let Ok(exchange) = teshi_core::HttpExchange::from_value(event) {
                    case.exchanges.push(exchange);
                }
            }
            "log" if targeted => {
                push_log(
                    &mut case.logs,
                    event["message"].as_str().unwrap_or_default(),
                );
            }
            "stderr" | "stdout"
                if window.is_some_and(|(first, last)| index >= first && index <= last) =>
            {
                push_log(
                    &mut case.logs,
                    event["message"].as_str().unwrap_or_default(),
                );
            }
            _ => {}
        }
    }

    if case.status.is_none() {
        let failed = case
            .steps
            .iter()
            .find(|step| step.status.as_deref() == Some("failed"));
        if failed.is_some() {
            case.status = Some("failed".into());
            case.message = failed.and_then(|step| step.message.clone());
        } else if !case.steps.is_empty() && value["test_status"].as_str() == Some("passed") {
            case.status = Some("passed".into());
        } else if !case.run_status.is_empty() && case.run_status != "running" {
            case.status = Some(case.run_status.clone());
        }
    }
    Some(case)
}

/// Case ids that belong to the selected scenario inside one run.
fn target_case_ids(
    project: &Path,
    feature: &str,
    scenario: &str,
    lines: Option<(u64, u64)>,
    events: &[Value],
) -> Option<HashSet<String>> {
    let start_cases = events
        .iter()
        .filter(|event| event["type"] == "start_case")
        .count();
    let mut ids = HashSet::new();
    let mut evidenced = HashSet::new();
    // Step events carry the Feature path even when `start_case` omits it.
    for event in events {
        let kind = event["type"].as_str().unwrap_or_default();
        if !matches!(kind, "step_screenshot" | "planned_step")
            || !feature_matches(project, feature, event)
        {
            continue;
        }
        let name = event["scenario"].as_str().unwrap_or_default();
        let recorded_line = event["step_line"].as_u64();
        let matches = if name.is_empty() {
            lines.is_some_and(|(first, last)| {
                recorded_line.is_some_and(|line| line >= first && line <= last)
            })
        } else {
            scenario.is_empty() || name == scenario
        };
        if matches && let Some(id) = event["case_id"].as_str() {
            ids.insert(id.to_string());
            evidenced.insert(id.to_string());
        }
    }
    for event in events {
        if event["type"] != "start_case" {
            continue;
        }
        let has_feature = !event["feature"].as_str().unwrap_or("").is_empty();
        if has_feature && !feature_matches(project, feature, event) {
            continue;
        }
        let name = event["name"].as_str().unwrap_or_default();
        let matches =
            name.is_empty() && start_cases == 1 || (!scenario.is_empty() && name == scenario);
        if !matches {
            continue;
        }
        let Some(id) = event["case_id"].as_str() else {
            continue;
        };
        // A featureless start_case is accepted only when no sibling case proved
        // a different Feature under the same scenario name.
        if has_feature || evidenced.is_empty() || evidenced.contains(id) {
            ids.insert(id.to_string());
        }
    }
    if ids.is_empty() {
        return None;
    }
    Some(ids)
}

fn feature_matches(project: &Path, feature: &str, event: &Value) -> bool {
    let recorded = normalize_feature(project, event["feature"].as_str().unwrap_or(""));
    recorded.is_empty() || feature.is_empty() || recorded == feature || recorded.ends_with(feature)
}

/// First and last event owned by the target case; case-less logs between them
/// belong to that case in a sequential run.
fn target_window(events: &[Value], target_ids: &HashSet<String>) -> Option<(usize, usize)> {
    let mut first = None;
    let mut last = None;
    for (index, event) in events.iter().enumerate() {
        if event["case_id"]
            .as_str()
            .is_some_and(|id| target_ids.contains(id))
        {
            first.get_or_insert(index);
            last = Some(index);
        }
    }
    Some((first?, last?))
}

fn collect_attachments(
    directory: &Path,
    project: &Path,
    event: &Value,
    attachments: &mut Vec<HistoryAttachment>,
    seen: &mut HashSet<String>,
) {
    let Some(items) = event["error"]["attachments"]
        .as_array()
        .or_else(|| event["attachments"].as_array())
    else {
        return;
    };
    for item in items {
        let Some(reported) = item["path"].as_str().filter(|path| !path.trim().is_empty()) else {
            continue;
        };
        if !seen.insert(reported.to_string()) {
            continue;
        }
        let kind = item["type"]
            .as_str()
            .or_else(|| item["kind"].as_str())
            .unwrap_or("artifact");
        attachments.push(HistoryAttachment {
            kind: kind.to_string(),
            path: resolve_artifact(directory, project, reported),
            step_id: item["step_id"].as_str().map(str::to_string),
        });
    }
}

fn resolve_artifact(directory: &Path, project: &Path, reported: &str) -> PathBuf {
    let path = Path::new(reported);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let in_run = directory.join(path);
    if in_run.exists() {
        return in_run;
    }
    project.join(path)
}

fn push_log(logs: &mut Vec<String>, message: &str) {
    if logs.len() < MAX_LOGS {
        logs.push(message.to_string());
    }
}

fn normalize_feature(root: &Path, recorded: &str) -> String {
    image_view::normalize_feature(root, recorded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(project: &Path, started_at: &str, events: Value) -> String {
        // The reader orders runs by manifest mtime; distinct ids keep runs apart.
        let id = format!(
            "run-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = project.join(".teshi/logs/replay-screenshots").join(&id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("run.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 2,
                "id": id,
                "started_at": started_at,
                "status": "passed",
                "test_status": "passed",
                "acceptance_status": "passed",
                "events": events,
            }))
            .unwrap(),
        )
        .unwrap();
        id
    }

    #[test]
    fn newest_run_wins_and_keeps_logs_steps_and_captures() {
        let project = tempfile::tempdir().unwrap();
        let events = serde_json::json!([
            {"type": "start_case", "case_id": "f0:s0", "feature": "demo.feature", "name": "First"},
            {"type": "stderr", "message": "starting"},
            {"type": "start_step", "case_id": "f0:s0", "step_id": "f0:s0:L3", "text": "Given a"},
            {"type": "end_step", "case_id": "f0:s0", "step_id": "f0:s0:L3", "status": "passed"},
            {"type": "step_screenshot", "case_id": "f0:s0", "step_id": "f0:s0:L3",
             "feature": "demo.feature", "scenario": "First", "step_line": 3,
             "step_text": "Given a", "status": "passed", "capture_status": "available",
             "screenshot_file": "one.png", "captured_at": "2026-11-21T10:00:00Z"},
            {"type": "case_passed", "case_id": "f0:s0", "duration_ms": 12}
        ]);
        let _older = run(project.path(), "2026-11-20T10:00:00Z", events.clone());
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _newer = run(project.path(), "2026-11-21T10:00:00Z", events);

        let mut history =
            ScenarioHistory::open(project.path(), "demo.feature", "First", Some((3, 3)));
        history.scan_all();
        let case = history.case_at(0).expect("newest case");
        assert_eq!(case.started_at, "2026-11-21T10:00:00Z");
        assert_eq!(case.status.as_deref(), Some("passed"));
        assert_eq!(case.steps.len(), 1);
        assert_eq!(case.logs, vec!["starting"]);
        assert_eq!(case.screenshots.len(), 1);
        history.scan_all();
        assert_eq!(
            history.case_at(1).map(|case| case.started_at.as_str()),
            Some("2026-11-20T10:00:00Z")
        );
    }

    #[test]
    fn featureless_start_case_is_resolved_by_its_step_events() {
        let project = tempfile::tempdir().unwrap();
        run(
            project.path(),
            "2026-11-21T10:00:00Z",
            serde_json::json!([
                {"type": "start_case", "case_id": "f0:s0", "name": "Detail"},
                {"type": "planned_step", "case_id": "f0:s0", "feature": "demo.feature", "step_line": 3},
                {"type": "step_screenshot", "case_id": "f0:s0", "feature": "demo.feature",
                 "step_line": 3, "step_text": "Given a", "status": "passed",
                 "capture_status": "available", "screenshot_file": "a.png"}
            ]),
        );
        let mut history =
            ScenarioHistory::open(project.path(), "demo.feature", "Detail", Some((3, 3)));
        history.scan_all();
        let case = history.case_at(0).expect("case by feature evidence");
        assert_eq!(case.screenshots.len(), 1);
    }

    #[test]
    fn same_scenario_name_in_another_feature_does_not_leak_in() {
        let project = tempfile::tempdir().unwrap();
        run(
            project.path(),
            "2026-11-21T10:00:00Z",
            serde_json::json!([
                {"type": "start_case", "case_id": "a", "name": "Detail"},
                {"type": "start_case", "case_id": "b", "name": "Detail"},
                {"type": "planned_step", "case_id": "a", "feature": "a.feature", "step_line": 3},
                {"type": "planned_step", "case_id": "b", "feature": "b.feature", "step_line": 30},
                {"type": "step_screenshot", "case_id": "a", "feature": "a.feature",
                 "step_line": 3, "status": "passed", "capture_status": "available",
                 "screenshot_file": "a.png"},
                {"type": "step_screenshot", "case_id": "b", "feature": "b.feature",
                 "step_line": 30, "status": "passed", "capture_status": "available",
                 "screenshot_file": "b.png"}
            ]),
        );
        let mut history =
            ScenarioHistory::open(project.path(), "a.feature", "Detail", Some((3, 3)));
        history.scan_all();
        let case = history.case_at(0).expect("a case");
        assert_eq!(case.screenshots.len(), 1);
        assert_eq!(case.screenshots[0].file, "a.png");
    }

    #[test]
    fn legacy_and_unrelated_runs_are_ignored() {
        let project = tempfile::tempdir().unwrap();
        let directory = project.path().join(".teshi/logs/replay-screenshots/legacy");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("index.json"), b"{}").unwrap();
        run(
            project.path(),
            "2026-11-20T10:00:00Z",
            serde_json::json!([
                {"type": "start_case", "case_id": "c", "feature": "other.feature", "name": "Other"}
            ]),
        );

        let mut history = ScenarioHistory::open(project.path(), "demo.feature", "First", None);
        history.scan_all();
        assert!(history.case_at(0).is_none());
    }

    #[test]
    fn scenario_without_a_name_matches_by_step_line() {
        let project = tempfile::tempdir().unwrap();
        let _id = run(
            project.path(),
            "2026-11-21T10:00:00Z",
            serde_json::json!([
                {"type": "step_screenshot", "case_id": "c", "feature": "demo.feature",
                 "step_line": 7, "step_text": "Then b", "status": "failed",
                 "capture_status": "available", "screenshot_file": "b.png"}
            ]),
        );
        let mut history =
            ScenarioHistory::open(project.path(), "demo.feature", "Second", Some((6, 8)));
        history.scan_all();
        let case = history.case_at(0).expect("case by line");
        assert_eq!(case.scenario, "Second");
        assert_eq!(case.status.as_deref(), Some("failed"));
    }

    #[test]
    fn multiple_screenshot_steps_without_step_events_are_all_loaded() {
        let project = tempfile::tempdir().unwrap();
        let _id = run(
            project.path(),
            "2026-11-21T10:00:00Z",
            serde_json::json!([
                {"type": "step_screenshot", "case_id": "c", "feature": "demo.feature",
                 "step_id": "c:L3", "step_line": 3, "step_text": "Given a", "status": "passed",
                 "capture_status": "available", "screenshot_file": "a.png"},
                {"type": "step_screenshot", "case_id": "c", "feature": "demo.feature",
                 "step_id": "c:L4", "step_line": 4, "step_text": "Then b", "status": "failed",
                 "message": "button missing", "capture_status": "available", "screenshot_file": "b.png"}
            ]),
        );
        let mut history =
            ScenarioHistory::open(project.path(), "demo.feature", "First", Some((2, 5)));
        history.scan_all();
        let case = history.case_at(0).expect("case with two capture steps");
        assert_eq!(case.steps.len(), 2);
        assert_eq!(case.steps[0].step_id, "c:L3");
        assert_eq!(case.steps[0].status.as_deref(), Some("passed"));
        assert_eq!(case.steps[1].step_id, "c:L4");
        assert_eq!(case.steps[1].status.as_deref(), Some("failed"));
        assert_eq!(case.status.as_deref(), Some("failed"));
        assert_eq!(case.message.as_deref(), Some("button missing"));
    }
}
