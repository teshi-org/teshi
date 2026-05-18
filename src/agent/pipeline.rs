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
                 - Ask questions to understand what the user needs\n\
                 - Ask about: feature name, user story, scenarios, edge cases, error conditions\n\
                 - Do NOT create any files yet\n\
                 - When you have enough information, call submit_requirements"
            }
            Self::Planning => {
                "\n\n## Current Phase: Scenario Planning\n\
                 Based on the gathered requirements, design the scenario structure.\n\
                 - Plan which scenarios to include (happy path, error cases, edge cases)\n\
                 - Use Scenario Outline + Examples for data-driven variations\n\
                 - When the plan is ready, call generate_plan to record it"
            }
            Self::Writing => {
                "\n\n## Current Phase: Feature Writing\n\
                 Execute the approved plan by creating the feature files.\n\
                 - Use create_feature_file and insert_scenario tools\n\
                 - Reuse existing step patterns from Project Context\n\
                 - After writing, call validate_feature"
            }
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
