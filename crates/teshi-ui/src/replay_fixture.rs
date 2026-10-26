//! Deterministic external-service fixtures for the local GPUI screenshot dock.
//!
//! These fixtures exercise the real Run view without controlling a user's
//! applications. Durable storage and capture transports have separate engine tests.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};

use crate::backend::{ApiRunBackend, ApiRunEventDto, ApiScenarioSnapshot, BackendFuture};

/// Capture/execution condition selected by a screenshot acceptance scenario.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum ReplayFixtureMode {
    /// A successful step with an available image.
    #[default]
    Success,
    /// An earlier screenshot is available while a later step is still running.
    Running,
    /// A second step fails after an earlier successful capture.
    Failed,
    /// Executing the step succeeds but capturing it fails.
    CaptureFailed,
    /// The external backend has no screenshot capability.
    Unsupported,
    /// The recorded image is no longer readable.
    Missing,
}

#[derive(Default)]
struct FixtureState {
    mode: ReplayFixtureMode,
    project: usize,
    closed: bool,
    sequence: usize,
    runs: Vec<(usize, Value)>,
}

/// In-memory service boundary used only by the explicit local E2E harness.
#[derive(Clone, Default)]
pub struct ReplayFixtureBackend(Rc<RefCell<FixtureState>>);

impl ReplayFixtureBackend {
    /// Establish an independent scenario with no inherited runs or selections.
    pub fn reset(&self, mode: ReplayFixtureMode) {
        *self.0.borrow_mut() = FixtureState {
            mode,
            ..FixtureState::default()
        };
    }

    /// Seed an already-running or historical replay for a Given step.
    pub fn seed_run(&self) -> String {
        let mut state = self.0.borrow_mut();
        state.sequence += 1;
        let id = format!("fixture-project{}-run{}", state.project, state.sequence);
        let capture_status = match state.mode {
            ReplayFixtureMode::CaptureFailed => "failed",
            ReplayFixtureMode::Unsupported => "unsupported",
            _ => "available",
        };
        let capture = |number: usize, outcome: &str| {
            let mut event = json!({"type":"step_screenshot", "case_id":"fixture-case",
                "step_id":format!("fixture-step{number}"), "feature":"features/replay-fixture.feature",
                "step_line":number + 3, "step_keyword":"Then", "step_text":format!("marker {number} is visible"),
                "status":outcome, "capture_status":capture_status,
                "captured_at":"2026-10-22T00:00:00Z", "message":"Fixture capture unavailable"});
            if capture_status == "available" {
                event["screenshot_file"] = json!(format!("{id}-step{number}.png"));
            }
            event
        };
        let mut events = vec![
            json!({"type":"start_case", "case_id":"fixture-case", "name":"Review a replay image"}),
            capture(1, "passed"),
        ];
        let status = match state.mode {
            ReplayFixtureMode::Failed => {
                events.push(capture(2, "failed"));
                "failed"
            }
            ReplayFixtureMode::Missing => {
                events.push(capture(2, "passed"));
                "passed"
            }
            ReplayFixtureMode::Running => {
                events.push(json!({"type":"start_step", "step_id":"fixture-step2", "text":"later step is running"}));
                "running"
            }
            _ => "passed",
        };
        let run = json!({"id":id,"started_at":format!("2026-10-22T00:00:{:02}Z", state.sequence),"status":status,"events":events,"fixture_mode":format!("{:?}",state.mode)});
        let project = state.project;
        state.runs.push((project, run));
        id
    }

    /// Close/reopen the fixture project while retaining its service-side history.
    pub fn set_closed(&self, closed: bool) {
        self.0.borrow_mut().closed = closed;
    }

    /// Select the other isolated fixture project.
    pub fn switch_project(&self) {
        let mut state = self.0.borrow_mut();
        state.project = 1 - state.project;
    }

    /// Latest run in the selected fixture project, for setup of a live inspect view.
    pub fn latest_run_id(&self) -> Option<String> {
        let state = self.0.borrow();
        state
            .runs
            .iter()
            .rev()
            .find(|(project, _)| *project == state.project)
            .and_then(|(_, run)| run["id"].as_str().map(str::to_owned))
    }

    fn snapshot(&self, id: &str) -> Result<Value, String> {
        let state = self.0.borrow();
        if state.closed {
            return Err("No fixture project is open".into());
        }
        state
            .runs
            .iter()
            .find(|(project, run)| *project == state.project && run["id"] == id)
            .map(|(_, run)| run.clone())
            .ok_or_else(|| "Replay is outside the selected fixture project".into())
    }
}

impl ApiRunBackend for ReplayFixtureBackend {
    fn supports_replay_history(&self) -> bool {
        true
    }

    fn list_scenarios(&self) -> BackendFuture<Vec<ApiScenarioSnapshot>> {
        let closed = self.0.borrow().closed;
        Box::pin(async move {
            Ok(if closed {
                Vec::new()
            } else {
                vec![ApiScenarioSnapshot {
                    id: "fixture-scenario".into(),
                    feature_path: "features/replay-fixture.feature".into(),
                    name: "Review a replay image".into(),
                    tags: vec![],
                    engine_mode: "ui".into(),
                }]
            })
        })
    }

    fn start_run(&self, _ids: &[String]) -> BackendFuture<Vec<ApiRunEventDto>> {
        Box::pin(async { Err("The screenshot fixture uses pollable replay runs".into()) })
    }

    fn begin_replay(&self, _ids: &[String]) -> BackendFuture<String> {
        let backend = self.clone();
        Box::pin(async move {
            if backend.0.borrow().closed {
                return Err("No fixture project is open".into());
            }
            Ok(backend.seed_run())
        })
    }

    fn replay_history(&self) -> BackendFuture<Vec<Value>> {
        let state = self.0.borrow();
        let history = if state.closed {
            Vec::new()
        } else {
            state
                .runs
                .iter()
                .rev()
                .filter(|(project, _)| *project == state.project)
                .map(|(_, run)| {
                    let mut summary = run.clone();
                    summary["events"] = json!([]);
                    summary
                })
                .collect()
        };
        Box::pin(async move { Ok(history) })
    }

    fn replay_snapshot(&self, id: &str) -> BackendFuture<Value> {
        let result = self.snapshot(id);
        Box::pin(async move { result })
    }

    fn replay_image(&self, id: &str, filename: &str) -> BackendFuture<Vec<u8>> {
        let result = self.snapshot(id).and_then(|run| {
            if run["fixture_mode"] == "Missing" && filename.ends_with("step1.png") {
                return Err("Saved fixture image is missing".into());
            }
            if !run["events"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|event| event["screenshot_file"] == filename)
            {
                return Err("Image is not associated with this fixture run".into());
            }
            Ok(if filename.starts_with("fixture-project0-") {
                include_bytes!("../tests/fixtures/replay-first.png").to_vec()
            } else {
                include_bytes!("../tests/fixtures/replay-second.png").to_vec()
            })
        });
        Box::pin(async move { result })
    }

    fn get_exchange(&self, _id: &str, _redact: bool) -> BackendFuture<Value> {
        Box::pin(async { Err("No HTTP exchange in the screenshot fixture".into()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_survives_reopening_and_remains_project_scoped() {
        let backend = ReplayFixtureBackend::default();
        let first = backend.seed_run();
        let second = backend.seed_run();
        assert_ne!(first, second);
        let original = backend.snapshot(&first).unwrap();
        backend.set_closed(true);
        assert!(backend.snapshot(&first).is_err());
        backend.set_closed(false);
        assert_eq!(backend.snapshot(&first).unwrap(), original);
        backend.switch_project();
        let other = backend.seed_run();
        assert!(backend.snapshot(&first).is_err());
        assert!(backend.snapshot(&other).is_ok());
    }

    #[test]
    fn fixture_conditions_keep_capture_and_execution_outcomes_separate() {
        let backend = ReplayFixtureBackend::default();
        for (mode, status, capture) in [
            (ReplayFixtureMode::CaptureFailed, "passed", "failed"),
            (ReplayFixtureMode::Unsupported, "passed", "unsupported"),
            (ReplayFixtureMode::Running, "running", "available"),
            (ReplayFixtureMode::Failed, "failed", "available"),
        ] {
            backend.reset(mode);
            let id = backend.seed_run();
            let run = backend.snapshot(&id).unwrap();
            assert_eq!(run["status"], status);
            assert_eq!(run["events"][1]["status"], "passed");
            assert_eq!(run["events"][1]["capture_status"], capture);
            if mode == ReplayFixtureMode::Failed {
                assert_eq!(run["events"][2]["status"], "failed");
            }
        }
    }
}
