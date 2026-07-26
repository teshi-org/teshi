//! Generation pipeline module — requirement gathering, planning, and writing stages.
//!
//! This module defines the stage state machine, data structures, and prompt extensions
//! used by the AI agent to manage the multi-stage feature generation pipeline.

use serde::{Deserialize, Serialize};

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
    /// LLM is designing the scenario structure.
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
            Self::Planning => "Scenario Planning",
            Self::Writing => "Feature Writing",
            Self::Confirming => "Awaiting Confirmation",
            Self::Validating => "Validation",
            Self::Complete => "Complete",
        }
    }

    /// Guidance text to inject into the system prompt for this stage.
    pub fn prompt_guidance(&self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Gathering => {
                "\n\n## Current Phase: Requirements Gathering\n\
                 You are gathering requirements to create a new feature.\n\
                 - Ask questions to understand what the user needs, or accept pasted requirement text\n\
                 - Ask about: feature name, user story, scenarios (test points), edge cases, error conditions\n\
                 - Do NOT create any files yet\n\
                 - Do NOT produce FreeMind `.mm` or mock HTML — test points become Gherkin scenarios later\n\
                 - When you have enough information, call submit_requirements"
            }
            Self::Planning => {
                "\n\n## Current Phase: Scenario Planning\n\
                 Based on the gathered requirements, design the scenario structure.\n\
                 - Plan which scenarios to include (happy path, error cases, edge cases)\n\
                 - Plan scenarios so each one is self-contained — treat every scenario\n\
                   as independently runnable, not as steps in a sequence\n\
                 - Use Scenario Outline + Examples for data-driven variations\n\
                 - When the plan is ready, call generate_plan to record it"
            }
            Self::Writing => {
                "\n\n## Current Phase: Feature Writing\n\
                 Execute the approved plan by creating the feature files.\n\
                 - Use create_feature_file and insert_scenario tools\n\
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

/// Stores the requirements gathered from the user during the gathering phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub feature_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub scenario_descriptions: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_default_is_idle() {
        let stage = GenerationStage::default();
        assert_eq!(stage, GenerationStage::Idle);
    }

    #[test]
    fn stage_labels_are_not_empty_for_active_stages() {
        assert_eq!(GenerationStage::Gathering.label(), "Requirements Gathering");
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
    fn planning_stage_has_guidance() {
        let guidance = GenerationStage::Planning.prompt_guidance();
        assert!(!guidance.is_empty());
        assert!(guidance.contains("generate_plan"));
        assert!(guidance.contains("Scenario Outline"));
    }

    #[test]
    fn writing_stage_instructions() {
        let guidance = GenerationStage::Writing.prompt_guidance();
        assert!(guidance.contains("create_feature_file"));
        assert!(guidance.contains("validate_feature"));
    }

    #[test]
    fn stage_serde_roundtrip() {
        let stages = vec![
            GenerationStage::Idle,
            GenerationStage::Gathering,
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
}
