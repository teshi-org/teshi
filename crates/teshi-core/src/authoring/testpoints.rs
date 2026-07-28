//! Test-point DTOs for `testpoints/testpoints.json`.

use serde::{Deserialize, Serialize};

use super::anchors::RequirementLink;

/// Human review state for a test point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Awaiting human review (new, edited, or reset after meaningful change).
    Proposed,
    /// Explicitly approved for scenario planning.
    Approved,
    /// Explicitly rejected by a human reviewer.
    Rejected,
    /// Previously approved but invalidated by stale anchors or required re-review.
    NeedsReview,
}

/// Business hierarchy path used by the Test Points tree.
///
/// Each segment must be non-empty; validation is performed by `teshi-engine`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HierarchyPath(pub Vec<String>);

impl HierarchyPath {
    /// Creates a hierarchy path from persisted segments.
    pub fn new(segments: Vec<String>) -> Self {
        Self(segments)
    }

    /// Returns the path segments.
    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

/// Reference from a test point to a realized Gherkin scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioRef {
    /// Project-relative path to the `.feature` file.
    pub feature_path: String,
    /// Scenario name for navigation when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_name: Option<String>,
    /// 1-based scenario line for precise navigation when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_line: Option<usize>,
}

/// A durable, non-Gherkin verification intent linked to requirement text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPoint {
    /// Stable identifier referenced by generation and scenario metadata.
    pub id: String,
    /// Short label shown in trees and review panes.
    pub title: String,
    /// What behavior must be verified.
    pub objective: String,
    /// Optional natural-language preconditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preconditions: Option<String>,
    /// Optional natural-language expected outcomes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_outcomes: Option<String>,
    /// Business hierarchy path for the Test Points tree.
    pub hierarchy_path: HierarchyPath,
    /// Current human review state.
    pub review_state: ReviewState,
    /// Trace links to requirement source ranges.
    #[serde(default)]
    pub requirement_links: Vec<RequirementLink>,
    /// Downstream Gherkin scenarios that realize this intent.
    #[serde(default)]
    pub scenario_refs: Vec<ScenarioRef>,
}

/// Canonical test-point store persisted at `testpoints/testpoints.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPointsFile {
    /// Schema version for forward-compatible migrations.
    #[serde(default = "default_testpoints_version")]
    pub version: u32,
    /// All test points in the project.
    #[serde(default)]
    pub test_points: Vec<TestPoint>,
}

fn default_testpoints_version() -> u32 {
    1
}

impl Default for TestPointsFile {
    fn default() -> Self {
        Self {
            version: default_testpoints_version(),
            test_points: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::{QuoteSelector, RequirementLink, ResolutionState, TextRange};

    #[test]
    fn review_state_serde_roundtrip() {
        for state in [
            ReviewState::Proposed,
            ReviewState::Approved,
            ReviewState::Rejected,
            ReviewState::NeedsReview,
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let back: ReviewState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, state);
        }
    }

    #[test]
    fn test_points_file_serde_roundtrip() {
        let file = TestPointsFile {
            version: 1,
            test_points: vec![TestPoint {
                id: "tp-1".into(),
                title: "Valid login".into(),
                objective: "Verify successful authentication".into(),
                preconditions: Some("User account exists".into()),
                expected_outcomes: Some("Dashboard is shown".into()),
                hierarchy_path: HierarchyPath::new(vec!["Authentication".into(), "Login".into()]),
                review_state: ReviewState::Proposed,
                requirement_links: vec![RequirementLink {
                    document_id: "doc-1".into(),
                    document_revision: "rev-1".into(),
                    position: TextRange::new(0, 5),
                    quote: QuoteSelector {
                        quote: "login".into(),
                        prefix: String::new(),
                        suffix: String::new(),
                    },
                    resolution: ResolutionState::Resolved,
                }],
                scenario_refs: vec![ScenarioRef {
                    feature_path: "features/auth.feature".into(),
                    scenario_name: Some("Successful login".into()),
                    scenario_line: Some(12),
                }],
            }],
        };
        let json = serde_json::to_string_pretty(&file).expect("serialize");
        let back: TestPointsFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, file);
    }
}
