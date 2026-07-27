//! Generation pipeline module — requirement gathering, test-point proposal, and writing stages.
//!
//! This module defines the stage state machine, data structures, and prompt extensions
//! used by the AI agent to manage the multi-stage feature generation pipeline.

use serde::{Deserialize, Serialize};
use teshi_core::authoring::{ResolutionState, ReviewState, TestPoint, TextRange};

/// Tracks the current generation pipeline phase.
/// The LLM drives the process via tool calls; this just records state
/// so the system prompt can inject stage-appropriate instructions.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerationStage {
    /// Default — no generation in progress.
    #[default]
    Idle,
    /// LLM is asking questions to understand requirements.
    Gathering,
    /// LLM is proposing non-Gherkin test points from submitted requirements.
    GeneratingTestPoints,
    /// Human review of proposed test points; agent loop is paused.
    ReviewingTestPoints,
    /// LLM is designing the scenario structure from approved test points.
    Planning,
    /// LLM is writing .feature files.
    Writing,
    /// User confirmation of pending changes.
    Confirming,
    /// LLM is validating the generated output.
    Validating,
    /// Generation complete.
    Complete,
}

impl GenerationStage {
    /// Human-readable label for the stage.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Gathering => "Requirements Gathering",
            Self::GeneratingTestPoints => "Generating Test Points",
            Self::ReviewingTestPoints => "Reviewing Test Points",
            Self::Planning => "Scenario Planning",
            Self::Writing => "Feature Writing",
            Self::Confirming => "Awaiting Confirmation",
            Self::Validating => "Validation",
            Self::Complete => "Complete",
        }
    }

    /// Returns `true` when the agent loop must pause for human review.
    ///
    /// File-change `ApprovalMode::{Auto, Bypass}` must not resume this stage.
    pub fn is_human_review_gate(self) -> bool {
        matches!(self, Self::ReviewingTestPoints)
    }

    /// Guidance text to inject into the system prompt for this stage.
    pub fn prompt_guidance(&self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Gathering => {
                "\n\n## Current Phase: Requirements Gathering\n\
                 You are gathering requirements to create a new feature.\n\
                 - Ask questions to understand what the user needs, or accept pasted requirement text\n\
                 - Prefer referencing persisted requirement documents/ranges when available\n\
                 - Ask about: feature name, user story, verification intents, edge cases, error conditions\n\
                 - Do NOT create any files yet\n\
                 - Do NOT produce FreeMind `.mm` or mock HTML\n\
                 - Do NOT call generate_plan yet — test points must be proposed and approved first\n\
                 - When you have enough information, call submit_requirements"
            }
            Self::GeneratingTestPoints => {
                "\n\n## Current Phase: Generating Test Points\n\
                 Propose non-Gherkin verification intents (test points) from the submitted requirements.\n\
                 - Each test point needs: title, objective, hierarchy_path, and requirement links when sources exist\n\
                 - Do NOT write Given/When/Then steps inside test points\n\
                 - Do NOT call generate_plan yet\n\
                 - Call propose_test_points with the structured proposals"
            }
            Self::ReviewingTestPoints => {
                "\n\n## Current Phase: Reviewing Test Points\n\
                 Proposed test points are awaiting explicit human approval in the Test Points tab.\n\
                 - Do NOT call generate_plan\n\
                 - Do NOT approve, reject, or modify review state via tools\n\
                 - Wait for the user to approve eligible test points and continue generation"
            }
            Self::Planning => {
                "\n\n## Current Phase: Scenario Planning\n\
                 Design Gherkin scenarios that realize the approved test points.\n\
                 - Plan which scenarios to include (happy path, error cases, edge cases)\n\
                 - Every scenario MUST reference one or more approved test-point IDs via test_point_ids\n\
                 - Plan scenarios so each one is self-contained — treat every scenario\n\
                   as independently runnable, not as steps in a sequence\n\
                 - Use Scenario Outline + Examples for data-driven variations\n\
                 - When the plan is ready, call generate_plan to record it"
            }
            Self::Writing => {
                "\n\n## Current Phase: Feature Writing\n\
                 Execute the approved plan by creating the feature files.\n\
                 - Use create_feature_file and insert_scenario tools\n\
                 - Pass each scenario's test_point_ids to insert_scenario so Teshi embeds @teshi-tp:<id> tags\n\
                 - Reuse existing step patterns from Project Context\n\
                 - After writing, call validate_feature AND run_tests to verify\n\
                   that each new scenario is executable\n\
                 \n\
                 ## Scenario Independence Rules\n\
                 - Each scenario must have a complete Given/When/Then chain —\n\
                   no missing keywords. Missing When or Then is an ERROR.\n\
                 - A scenario's Given must independently establish ALL preconditions\n\
                   for that scenario. Do NOT write scenarios that assume another\n\
                   scenario has already been executed.\n\
                 - For example, if Scenario A logs in, Scenario B cannot say\n\
                   'Given I am still logged in' because runners execute scenarios\n\
                   independently and in arbitrary order.\n\
                 - If you need shared state, use Background (which runs before\n\
                   each scenario independently)."
            }
            Self::Confirming => "", // User-action phase, no LLM guidance needed
            Self::Validating => {
                "\n\n## Current Phase: Validation\n\
                 Review the generated feature for completeness.\n\
                 - Check that every scenario has proper Given/When/Then chain\n\
                 - Check for duplicate scenario names\n\
                 - Verify Examples tables match placeholders\n\
                 - Call validate_feature to run automated checks\n\
                 - If issues found, fix them and re-validate"
            }
            Self::Complete => "",
        }
    }
}

/// A generation source referencing a persisted requirement document and optional range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementSourceRef {
    /// Stable requirement document ID from `requirements/_teshi.json`.
    pub document_id: String,
    /// Optional Unicode character range within the document; omit for whole-document source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<TextRange>,
}

/// Stores the requirements gathered from the user during the gathering phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub feature_name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Free-text intent descriptions from conversational/pasted input (compatibility).
    #[serde(default)]
    pub scenario_descriptions: Vec<String>,
    /// Selected requirement document/range identities as generation sources.
    #[serde(default)]
    pub source_refs: Vec<RequirementSourceRef>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Requirement {
    /// Returns `true` when at least one usable source (document ref or pasted text) is present.
    pub fn has_usable_sources(&self) -> bool {
        !self.source_refs.is_empty() || !self.scenario_descriptions.is_empty()
    }
}

/// The full plan for generating feature files, submitted during the planning phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationPlan {
    pub features: Vec<FeaturePlan>,
}

/// A single feature file in the generation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeaturePlan {
    pub file_name: String,
    pub feature_name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub background_steps: Vec<String>,
    pub scenarios: Vec<ScenarioPlan>,
}

/// A single scenario within a feature plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioPlan {
    #[serde(default)]
    pub is_outline: bool,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub steps: Vec<String>,
    #[serde(default)]
    pub examples_headers: Vec<String>,
    #[serde(default)]
    pub examples_rows: Vec<Vec<String>>,
    /// Approved test-point IDs this scenario realizes.
    #[serde(default)]
    pub test_point_ids: Vec<String>,
}

/// Resumable generation session persisted under `.teshi/generation-state.json`.
///
/// Restoring this state never grants approval; review state comes only from
/// persisted test-point artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenerationSessionState {
    /// Active pipeline stage.
    pub stage: GenerationStage,
    /// Last submitted requirement sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<Requirement>,
    /// Latest accepted scenario plan, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<GenerationPlan>,
}

/// Validates that every referenced test-point ID is approved and fully resolved.
///
/// # Errors
///
/// Returns a list of actionable diagnostics when any ID is missing, unapproved,
/// rejected, proposed, needs review, stale, or when a scenario omits IDs.
pub fn validate_plan_test_point_ids(
    plan: &GenerationPlan,
    test_points: &[TestPoint],
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let mut referenced = 0usize;

    for feature in &plan.features {
        for scenario in &feature.scenarios {
            if scenario.test_point_ids.is_empty() {
                errors.push(format!(
                    "scenario '{}' in '{}' must reference at least one approved test_point_id",
                    scenario.name, feature.file_name
                ));
                continue;
            }
            for id in &scenario.test_point_ids {
                referenced += 1;
                match test_points.iter().find(|tp| &tp.id == id) {
                    None => errors.push(format!("unknown test point id '{id}'")),
                    Some(tp) => {
                        if tp.review_state != ReviewState::Approved {
                            errors.push(format!(
                                "test point '{id}' is {:?} (must be Approved)",
                                tp.review_state
                            ));
                        }
                        if tp
                            .requirement_links
                            .iter()
                            .any(|l| l.resolution == ResolutionState::Stale)
                        {
                            errors.push(format!(
                                "test point '{id}' has stale requirement links and cannot be planned"
                            ));
                        }
                    }
                }
            }
        }
    }

    if referenced == 0 && errors.is_empty() {
        errors.push("plan must reference at least one approved test-point id".into());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Collects IDs of test points eligible for scenario planning (Approved + resolved links).
pub fn approved_resolved_test_point_ids(test_points: &[TestPoint]) -> Vec<String> {
    test_points
        .iter()
        .filter(|tp| {
            tp.review_state == ReviewState::Approved
                && tp
                    .requirement_links
                    .iter()
                    .all(|l| l.resolution == ResolutionState::Resolved)
        })
        .map(|tp| tp.id.clone())
        .collect()
}

/// Attempts the human-only transition from Reviewing Test Points to Planning.
///
/// Returns the new stage on success. `ApprovalMode` is intentionally unused:
/// Auto/Bypass must never invoke this transition.
///
/// # Errors
///
/// Returns an error when the stage is wrong or no eligible approved test points exist.
pub fn continue_from_review(
    current: GenerationStage,
    test_points: &[TestPoint],
) -> Result<GenerationStage, String> {
    if current != GenerationStage::ReviewingTestPoints {
        return Err(format!(
            "continue generation requires Reviewing Test Points (current: {})",
            current.label()
        ));
    }
    let approved = approved_resolved_test_point_ids(test_points);
    if approved.is_empty() {
        return Err(
            "approve at least one test point with resolved requirement links before continuing"
                .into(),
        );
    }
    Ok(GenerationStage::Planning)
}

/// Infers a resumable stage from persisted session state and current test points.
///
/// Restart never auto-approves. If the session says Planning but no approved
/// resolved test points remain, the stage falls back to Reviewing Test Points.
pub fn restore_stage_after_reload(
    saved: &GenerationSessionState,
    test_points: &[TestPoint],
) -> GenerationStage {
    match saved.stage {
        GenerationStage::Idle | GenerationStage::Complete => saved.stage,
        GenerationStage::Confirming => {
            // File confirmation is ephemeral; resume writing if a plan exists.
            if saved.plan.is_some() {
                GenerationStage::Writing
            } else {
                GenerationStage::Idle
            }
        }
        GenerationStage::Planning | GenerationStage::Writing | GenerationStage::Validating => {
            if approved_resolved_test_point_ids(test_points).is_empty() {
                GenerationStage::ReviewingTestPoints
            } else {
                saved.stage
            }
        }
        GenerationStage::Gathering
        | GenerationStage::GeneratingTestPoints
        | GenerationStage::ReviewingTestPoints => saved.stage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::authoring::{
        HierarchyPath, QuoteSelector, RequirementLink, ResolutionState, ReviewState, TextRange,
    };

    fn sample_tp(id: &str, state: ReviewState, resolution: ResolutionState) -> TestPoint {
        TestPoint {
            id: id.into(),
            title: id.into(),
            objective: "obj".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
            review_state: state,
            requirement_links: vec![RequirementLink {
                document_id: "doc-1".into(),
                document_revision: "rev".into(),
                position: TextRange::new(0, 4),
                quote: QuoteSelector {
                    quote: "text".into(),
                    prefix: String::new(),
                    suffix: String::new(),
                },
                resolution,
            }],
            scenario_refs: Vec::new(),
        }
    }

    #[test]
    fn stage_default_is_idle() {
        let stage = GenerationStage::default();
        assert_eq!(stage, GenerationStage::Idle);
    }

    #[test]
    fn stage_labels_are_not_empty_for_active_stages() {
        assert_eq!(GenerationStage::Gathering.label(), "Requirements Gathering");
        assert_eq!(
            GenerationStage::GeneratingTestPoints.label(),
            "Generating Test Points"
        );
        assert_eq!(
            GenerationStage::ReviewingTestPoints.label(),
            "Reviewing Test Points"
        );
        assert_eq!(GenerationStage::Planning.label(), "Scenario Planning");
        assert_eq!(GenerationStage::Writing.label(), "Feature Writing");
        assert_eq!(GenerationStage::Confirming.label(), "Awaiting Confirmation");
        assert_eq!(GenerationStage::Validating.label(), "Validation");
        assert_eq!(GenerationStage::Complete.label(), "Complete");
    }

    #[test]
    fn idle_stage_has_empty_guidance() {
        assert!(GenerationStage::Idle.prompt_guidance().is_empty());
        assert!(GenerationStage::Complete.prompt_guidance().is_empty());
        assert!(GenerationStage::Confirming.prompt_guidance().is_empty());
    }

    #[test]
    fn gathering_stage_has_guidance() {
        let guidance = GenerationStage::Gathering.prompt_guidance();
        assert!(!guidance.is_empty());
        assert!(guidance.contains("submit_requirements"));
        assert!(guidance.contains("Requirements Gathering"));
    }

    #[test]
    fn generating_test_points_guides_propose_tool() {
        let guidance = GenerationStage::GeneratingTestPoints.prompt_guidance();
        assert!(guidance.contains("propose_test_points"));
        assert!(
            !guidance.contains("generate_plan yet")
                || guidance.contains("Do NOT call generate_plan")
        );
    }

    #[test]
    fn reviewing_stage_is_human_gate() {
        assert!(GenerationStage::ReviewingTestPoints.is_human_review_gate());
        assert!(!GenerationStage::Planning.is_human_review_gate());
        let guidance = GenerationStage::ReviewingTestPoints.prompt_guidance();
        assert!(guidance.contains("Do NOT call generate_plan"));
    }

    #[test]
    fn planning_stage_has_guidance() {
        let guidance = GenerationStage::Planning.prompt_guidance();
        assert!(!guidance.is_empty());
        assert!(guidance.contains("generate_plan"));
        assert!(guidance.contains("test_point_ids"));
        assert!(guidance.contains("Scenario Outline"));
    }

    #[test]
    fn writing_stage_instructions() {
        let guidance = GenerationStage::Writing.prompt_guidance();
        assert!(guidance.contains("create_feature_file"));
        assert!(guidance.contains("validate_feature"));
        assert!(guidance.contains("test_point_ids"));
        assert!(guidance.contains("@teshi-tp:"));
    }

    #[test]
    fn stage_serde_roundtrip() {
        let stages = vec![
            GenerationStage::Idle,
            GenerationStage::Gathering,
            GenerationStage::GeneratingTestPoints,
            GenerationStage::ReviewingTestPoints,
            GenerationStage::Planning,
            GenerationStage::Writing,
            GenerationStage::Confirming,
            GenerationStage::Validating,
            GenerationStage::Complete,
        ];
        for stage in &stages {
            let json = serde_json::to_string(stage).unwrap();
            let back: GenerationStage = serde_json::from_str(&json).unwrap();
            assert_eq!(*stage, back);
        }
    }

    #[test]
    fn requirement_accepts_source_refs_or_pasted_text() {
        let pasted = Requirement {
            feature_name: "Auth".into(),
            description: None,
            scenario_descriptions: vec!["login works".into()],
            source_refs: vec![],
            tags: vec![],
        };
        assert!(pasted.has_usable_sources());

        let docs = Requirement {
            feature_name: "Auth".into(),
            description: None,
            scenario_descriptions: vec![],
            source_refs: vec![RequirementSourceRef {
                document_id: "doc-1".into(),
                range: Some(TextRange::new(0, 10)),
            }],
            tags: vec![],
        };
        assert!(docs.has_usable_sources());

        let empty = Requirement {
            feature_name: "Auth".into(),
            description: None,
            scenario_descriptions: vec![],
            source_refs: vec![],
            tags: vec![],
        };
        assert!(!empty.has_usable_sources());
    }

    #[test]
    fn continue_from_review_requires_approved_resolved() {
        let proposed = vec![sample_tp(
            "tp-1",
            ReviewState::Proposed,
            ResolutionState::Resolved,
        )];
        let err =
            continue_from_review(GenerationStage::ReviewingTestPoints, &proposed).unwrap_err();
        assert!(err.contains("approve at least one"));

        let approved = vec![sample_tp(
            "tp-1",
            ReviewState::Approved,
            ResolutionState::Resolved,
        )];
        assert_eq!(
            continue_from_review(GenerationStage::ReviewingTestPoints, &approved).unwrap(),
            GenerationStage::Planning
        );

        let wrong_stage = continue_from_review(GenerationStage::Gathering, &approved).unwrap_err();
        assert!(wrong_stage.contains("Reviewing Test Points"));
    }

    #[test]
    fn validate_plan_rejects_unapproved_and_stale() {
        let plan = GenerationPlan {
            features: vec![FeaturePlan {
                file_name: "auth.feature".into(),
                feature_name: "Auth".into(),
                tags: vec![],
                background_steps: vec![],
                scenarios: vec![ScenarioPlan {
                    is_outline: false,
                    name: "Login".into(),
                    tags: vec![],
                    steps: vec!["Given x".into()],
                    examples_headers: vec![],
                    examples_rows: vec![],
                    test_point_ids: vec!["tp-1".into()],
                }],
            }],
        };

        let proposed = vec![sample_tp(
            "tp-1",
            ReviewState::Proposed,
            ResolutionState::Resolved,
        )];
        let err = validate_plan_test_point_ids(&plan, &proposed).unwrap_err();
        assert!(err.iter().any(|e| e.contains("Proposed")));

        let stale = vec![sample_tp(
            "tp-1",
            ReviewState::Approved,
            ResolutionState::Stale,
        )];
        let err = validate_plan_test_point_ids(&plan, &stale).unwrap_err();
        assert!(err.iter().any(|e| e.contains("stale")));

        let ok = vec![sample_tp(
            "tp-1",
            ReviewState::Approved,
            ResolutionState::Resolved,
        )];
        assert!(validate_plan_test_point_ids(&plan, &ok).is_ok());
    }

    #[test]
    fn restore_falls_back_to_review_without_approvals() {
        let saved = GenerationSessionState {
            stage: GenerationStage::Planning,
            requirement: None,
            plan: None,
        };
        let proposed = vec![sample_tp(
            "tp-1",
            ReviewState::Proposed,
            ResolutionState::Resolved,
        )];
        assert_eq!(
            restore_stage_after_reload(&saved, &proposed),
            GenerationStage::ReviewingTestPoints
        );
    }

    #[test]
    fn restore_keeps_reviewing_without_approving() {
        let saved = GenerationSessionState {
            stage: GenerationStage::ReviewingTestPoints,
            requirement: None,
            plan: None,
        };
        let proposed = vec![sample_tp(
            "tp-1",
            ReviewState::Proposed,
            ResolutionState::Resolved,
        )];
        assert_eq!(
            restore_stage_after_reload(&saved, &proposed),
            GenerationStage::ReviewingTestPoints
        );
        assert_eq!(proposed[0].review_state, ReviewState::Proposed);
    }
}
