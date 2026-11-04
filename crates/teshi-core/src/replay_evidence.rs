//! Backend-independent screenshot completeness and acceptance rules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Counts refer to step executions, never to image files or source lines alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceSummary {
    /// Executions requiring visual evidence, including unexecuted planned steps.
    pub required: usize,
    /// Executions with their required visual artifacts saved.
    pub saved: usize,
    /// Executions whose capture explicitly failed or is unsupported.
    pub failed: usize,
    /// Planned executions which never started.
    pub not_executed: usize,
    /// Started executions with no capture outcome.
    pub missing: usize,
    /// Executions explicitly classified as nonvisual.
    pub not_applicable: usize,
}

impl EvidenceSummary {
    /// Whether all required executions have visual evidence.
    pub fn complete(&self) -> bool {
        self.required == self.saved
    }

    /// Explicit visual-evidence outcome independent of test execution.
    pub fn status(&self) -> &'static str {
        if self.required == 0 && self.not_applicable > 0 {
            "not_applicable"
        } else if self.complete() {
            "complete"
        } else {
            "incomplete"
        }
    }

    /// Combine execution and evidence without rewriting the assertion result.
    pub fn acceptance<'a>(&self, test_status: &'a str) -> &'a str {
        if test_status == "passed" && !self.complete() {
            "incomplete"
        } else {
            test_status
        }
    }
}

/// Reduce the common event stream, scoped optionally to one Feature.
///
/// Case, expanded example, step, and attempt identities must be stable within a
/// run. A later attempt cannot fill an earlier attempt's missing capture.
pub fn summarize(events: &[Value], feature: Option<&str>) -> EvidenceSummary {
    #[derive(Default)]
    struct Step {
        feature: Option<String>,
        started: bool,
        applicable: bool,
        planned: bool,
        capture: Option<String>,
    }
    let mut steps: BTreeMap<(String, String, u64), Step> = BTreeMap::new();
    for event in events {
        let kind = event["type"].as_str().unwrap_or_default();
        if !matches!(
            kind,
            "planned_step" | "start_step" | "step_result" | "end_step" | "step_screenshot"
        ) {
            continue;
        }
        let Some(id) = event["step_id"].as_str() else {
            continue;
        };
        let key = (
            event["case_id"].as_str().unwrap_or_default().to_string(),
            id.to_string(),
            event["attempt"].as_u64().unwrap_or(1),
        );
        let step = steps.entry(key).or_insert_with(|| Step {
            applicable: true,
            ..Step::default()
        });
        if let Some(path) = event["feature"].as_str() {
            step.feature = Some(path.into());
        }
        step.started |= kind != "planned_step"
            && !(kind == "step_screenshot" && event["capture_status"] == "not_executed");
        if kind == "planned_step" {
            step.planned = true;
            step.applicable = event["screenshot_required"] != false;
        } else if !step.planned && event["screenshot_required"] == false {
            step.applicable = false;
        }
        if kind == "step_screenshot" {
            step.capture = event["capture_status"].as_str().map(str::to_owned);
            if !step.planned && step.capture.as_deref() == Some("not_applicable") {
                step.applicable = false;
            }
        }
    }
    let mut summary = EvidenceSummary::default();
    for step in steps
        .values()
        .filter(|step| feature.is_none_or(|path| step.feature.as_deref() == Some(path)))
    {
        if !step.applicable {
            summary.not_applicable += 1;
            continue;
        }
        summary.required += 1;
        if !step.started {
            summary.not_executed += 1;
        } else {
            match step.capture.as_deref() {
                Some("available") => summary.saved += 1,
                Some("failed" | "unsupported" | "not_applicable") => summary.failed += 1,
                _ => summary.missing += 1,
            }
        }
    }
    summary
}

/// Compute completeness for exactly one expanded scenario execution.
pub fn summarize_case(events: &[Value], case_id: &str) -> EvidenceSummary {
    let examples = format!("{case_id}:example-");
    let events: Vec<_> = events
        .iter()
        .filter(|event| {
            event["case_id"]
                .as_str()
                .is_some_and(|id| id == case_id || id.starts_with(&examples))
        })
        .cloned()
        .collect();
    summarize(&events, None)
}

/// Project missing capture rows for inspection without inventing execution events.
/// Hosts may persist these outcomes on completion; they never imply execution.
pub fn missing_capture_rows(events: &[Value]) -> Vec<Value> {
    let key = |event: &Value| {
        (
            event["case_id"].clone(),
            event["step_id"].clone(),
            event["attempt"].as_u64().unwrap_or(1),
        )
    };
    let mut seen = std::collections::HashSet::new();
    events
        .iter()
        .filter(|event| {
            event["type"] == "planned_step"
                && seen.insert(serde_json::to_string(&key(event)).unwrap_or_default())
        })
        .filter(|planned| {
            !events
                .iter()
                .any(|event| event["type"] == "step_screenshot" && key(event) == key(planned))
        })
        .map(|planned| {
            let started = events.iter().any(|event| {
                matches!(
                    event["type"].as_str(),
                    Some("start_step" | "step_result" | "end_step")
                ) && key(event) == key(planned)
            });
            let mut row = planned.clone();
            row["type"] = serde_json::json!("step_screenshot");
            row["capture_status"] =
                serde_json::json!(if started { "missing" } else { "not_executed" });
            row["message"] = serde_json::json!(if started {
                "capture_outcome_not_recorded"
            } else {
                "step_not_executed"
            });
            row
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unavailable_ui_backend_cannot_exempt_a_required_step() {
        let events = vec![
            json!({"type":"planned_step","case_id":"ui","step_id":"one","screenshot_required":true}),
            json!({"type":"step_screenshot","case_id":"ui","step_id":"one","capture_status":"not_applicable","screenshot_required":false}),
        ];
        let summary = summarize(&events, None);
        assert_eq!(
            (summary.required, summary.failed, summary.not_applicable),
            (1, 1, 0)
        );
        assert_eq!(summary.acceptance("passed"), "incomplete");
    }

    #[test]
    fn attempts_and_examples_cannot_fill_each_others_gaps() {
        let events = vec![
            json!({"type":"planned_step","case_id":"row1","step_id":"L4","feature":"a"}),
            json!({"type":"step_screenshot","case_id":"row1","step_id":"L4","feature":"a","capture_status":"failed","message":"executor_busy"}),
            json!({"type":"step_screenshot","case_id":"row1","step_id":"L4","attempt":2,"feature":"a","capture_status":"available"}),
            json!({"type":"planned_step","case_id":"row2","step_id":"L4","feature":"a"}),
            json!({"type":"step_screenshot","case_id":"api","step_id":"L8","feature":"b","capture_status":"not_applicable"}),
        ];
        let summary = summarize(&events, Some("a"));
        assert_eq!(
            (
                summary.required,
                summary.saved,
                summary.failed,
                summary.not_executed
            ),
            (3, 1, 1, 1)
        );
        assert_eq!(summary.acceptance("passed"), "incomplete");
        assert_eq!(summary.acceptance("failed"), "failed");
        assert_eq!(summarize(&events, None).not_applicable, 1);
    }
}
