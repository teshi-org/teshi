//! Generation pipeline module — requirement gathering, test-point proposal, and writing stages.
//!
//! This module defines the stage state machine, data structures, and prompt extensions
//! used by the AI agent to manage the multi-stage feature generation pipeline.

use serde::{Deserialize, Serialize};
use teshi_core::authoring::{
    RequirementDocumentContent, RequirementDocumentIndex, RequirementDocumentMeta,
    RequirementIterationFilter, RequirementStoreId, ResolutionState, ReviewState, TestPoint,
    TextRange, document_char_len,
};

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
                 - Use `list_requirement_documents` then `read_requirement_document` to inspect the current store on demand\n\
                 - Do NOT assume pasted or conversational text is a persisted requirement-library source\n\
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

/// Fixed requirement-library window for one project generation session.
///
/// The scope belongs to the project session (`.teshi/generation-state.json`),
/// not to the Requirements tab view. Tools and stage transitions must re-check
/// store identity and iteration membership against the live global index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementSourceScope {
    /// Identity of the current user-level requirement store.
    pub store_id: RequirementStoreId,
    /// Iteration window: all documents, one named iteration, or unassigned.
    pub iteration: RequirementIterationFilter,
}

/// Why restored generation state cannot continue against the current store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeRestoreError {
    /// Saved session referenced requirement documents but has no store identity.
    MissingStoreIdentity,
    /// Saved `store_id` does not match the currently opened requirement store.
    StoreMismatch {
        /// Identity persisted with the session.
        saved: String,
        /// Identity of the store Teshi currently has open.
        current: String,
    },
    /// Named iteration no longer exists in the current index.
    IterationMissing {
        /// Iteration name that was saved with the session.
        name: String,
    },
    /// A source document left the iteration, disappeared, or changed revision.
    SourceDrift {
        /// Actionable diagnostic for the user.
        detail: String,
    },
}

impl std::fmt::Display for ScopeRestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingStoreIdentity => f.write_str(
                "generation session references requirement documents without a store identity; confirm the current requirement library or import the project",
            ),
            Self::StoreMismatch { saved, current } => write!(
                f,
                "requirement store mismatch: session is bound to '{saved}', current store is '{current}'"
            ),
            Self::IterationMissing { name } => write!(
                f,
                "saved iteration '{name}' is no longer present in the requirement index; reselect the generation source scope"
            ),
            Self::SourceDrift { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for ScopeRestoreError {}

/// A generation source referencing a persisted requirement document and optional range.
///
/// `store_id` and `document_revision` are filled by the system after validation;
/// agents supply `document_id` and an optional Unicode range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementSourceRef {
    /// Store that owned the document when the ref was accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_id: Option<RequirementStoreId>,
    /// Stable requirement document ID from the store index.
    pub document_id: String,
    /// Content revision captured when the ref was accepted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub document_revision: String,
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
    /// Confirmed requirement-library window for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_scope: Option<RequirementSourceScope>,
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

/// Returns index metadata that currently belongs to `scope`.
///
/// # Errors
///
/// Returns an error when the live index is missing or has a different `store_id`.
pub fn documents_in_scope<'a>(
    index: &'a RequirementDocumentIndex,
    scope: &RequirementSourceScope,
) -> Result<Vec<&'a RequirementDocumentMeta>, String> {
    let current = index.store_id.as_ref().ok_or_else(|| {
        "requirement store is uninitialized; initialize or import before using document sources"
            .to_string()
    })?;
    if current != &scope.store_id {
        return Err(format!(
            "requirement store mismatch: active scope is '{}', current store is '{}'",
            scope.store_id, current
        ));
    }
    Ok(index
        .documents
        .iter()
        .filter(|doc| doc.matches_iteration_filter(&scope.iteration))
        .collect())
}

/// Returns the live document identified by `(store_id, document_id)` when it is in `scope`.
///
/// # Errors
///
/// Returns an error for store mismatch, missing documents, or iteration membership drift.
pub fn read_document_in_scope<'a>(
    documents: &'a [RequirementDocumentContent],
    index: &RequirementDocumentIndex,
    scope: &RequirementSourceScope,
    store_id: &RequirementStoreId,
    document_id: &str,
) -> Result<&'a RequirementDocumentContent, String> {
    if store_id != &scope.store_id {
        return Err(format!(
            "requirement store '{store_id}' is outside the active scope '{}'",
            scope.store_id
        ));
    }
    let in_scope = documents_in_scope(index, scope)?;
    if !in_scope.iter().any(|meta| meta.id == document_id) {
        return Err(format!(
            "requirement document '{document_id}' is outside the active generation source scope"
        ));
    }
    documents
        .iter()
        .find(|doc| doc.meta.id == document_id)
        .ok_or_else(|| {
            format!("requirement document '{document_id}' is missing from the current store")
        })
}

/// Agent-supplied source identity before the system fills store and revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedSourceRef {
    /// Optional store identity supplied by the model; must match the active scope when present.
    pub store_id: Option<RequirementStoreId>,
    /// Stable document ID in the current store.
    pub document_id: String,
    /// Optional Unicode range; omit for the whole document.
    pub range: Option<TextRange>,
}

/// Validates submitted source refs against the live store and fills store/revision.
///
/// # Errors
///
/// Returns an error when any ref is outside the scope, the range is invalid, or
/// the document/revision cannot be resolved. No partial list is returned.
pub fn resolve_submitted_source_refs(
    submitted: &[SubmittedSourceRef],
    scope: &RequirementSourceScope,
    index: &RequirementDocumentIndex,
    documents: &[RequirementDocumentContent],
) -> Result<Vec<RequirementSourceRef>, String> {
    let mut resolved = Vec::with_capacity(submitted.len());
    for raw in submitted {
        if let Some(store_id) = &raw.store_id
            && store_id != &scope.store_id
        {
            return Err(format!(
                "requirement store '{store_id}' does not match the active scope '{}'",
                scope.store_id
            ));
        }
        let document =
            read_document_in_scope(documents, index, scope, &scope.store_id, &raw.document_id)?;
        if let Some(range) = &raw.range {
            let len = document_char_len(&document.body);
            if !range.is_non_empty() || range.end.offset() > len {
                return Err(format!(
                    "source range for document '{}' is invalid (start={}, end={}, document_chars={len})",
                    raw.document_id,
                    range.start.offset(),
                    range.end.offset()
                ));
            }
        }
        resolved.push(RequirementSourceRef {
            store_id: Some(scope.store_id.clone()),
            document_id: raw.document_id.clone(),
            document_revision: document.meta.revision.as_str().to_string(),
            range: raw.range,
        });
    }
    Ok(resolved)
}

/// Re-checks previously accepted source refs still match the live store and scope.
///
/// # Errors
///
/// Returns an error when a ref is missing `store_id`, left the iteration, or
/// its document revision changed.
pub fn revalidate_source_refs(
    refs: &[RequirementSourceRef],
    scope: &RequirementSourceScope,
    index: &RequirementDocumentIndex,
    documents: &[RequirementDocumentContent],
) -> Result<(), String> {
    for source in refs {
        let Some(store_id) = source.store_id.as_ref() else {
            return Err(format!(
                "requirement source '{}' is missing store identity; reselect sources after import",
                source.document_id
            ));
        };
        let document =
            read_document_in_scope(documents, index, scope, store_id, &source.document_id)?;
        if source.document_revision != document.meta.revision.as_str() {
            return Err(format!(
                "requirement document '{}' revision changed from '{}' to '{}'; reconfirm generation sources",
                source.document_id,
                source.document_revision,
                document.meta.revision.as_str()
            ));
        }
    }
    Ok(())
}

/// Re-checks proposed requirement links against the active generation scope.
///
/// # Errors
///
/// Returns an error when a link points at another store or a document outside
/// the confirmed iteration window.
pub fn validate_test_point_links_in_scope(
    test_points: &[TestPoint],
    scope: &RequirementSourceScope,
    index: &RequirementDocumentIndex,
) -> Result<(), String> {
    let allowed = documents_in_scope(index, scope)?;
    for tp in test_points {
        for link in &tp.requirement_links {
            match &link.store_id {
                Some(store_id) if store_id != &scope.store_id => {
                    return Err(format!(
                        "test point '{}' links to store '{store_id}' which is outside the active scope '{}'",
                        tp.id, scope.store_id
                    ));
                }
                None => {
                    return Err(format!(
                        "test point '{}' has a requirement link without store identity; import the project first",
                        tp.id
                    ));
                }
                Some(_) => {}
            }
            if !allowed.iter().any(|meta| meta.id == link.document_id) {
                return Err(format!(
                    "test point '{}' links to document '{}' outside the active generation source scope",
                    tp.id, link.document_id
                ));
            }
        }
    }
    Ok(())
}

/// Evaluates whether a restored session can continue against the current store.
///
/// Free-text-only sessions without document refs may continue without a stored
/// scope. Document-backed sessions without store identity are paused.
///
/// # Errors
///
/// Returns [`ScopeRestoreError`] when the session must pause for user confirmation.
pub fn evaluate_restored_scope(
    saved: &GenerationSessionState,
    current_store_id: Option<&RequirementStoreId>,
    index: &RequirementDocumentIndex,
    documents: &[RequirementDocumentContent],
) -> Result<(), ScopeRestoreError> {
    let has_document_refs = saved
        .requirement
        .as_ref()
        .is_some_and(|req| !req.source_refs.is_empty());
    let Some(scope) = saved.source_scope.as_ref() else {
        if has_document_refs {
            return Err(ScopeRestoreError::MissingStoreIdentity);
        }
        return Ok(());
    };
    let current = match current_store_id {
        Some(id) => id,
        None => {
            return Err(ScopeRestoreError::StoreMismatch {
                saved: scope.store_id.to_string(),
                current: "(uninitialized)".into(),
            });
        }
    };
    if current != &scope.store_id {
        return Err(ScopeRestoreError::StoreMismatch {
            saved: scope.store_id.to_string(),
            current: current.to_string(),
        });
    }
    if let RequirementIterationFilter::Named(name) = &scope.iteration
        && !index.discovered_iteration_names().iter().any(|n| n == name)
    {
        return Err(ScopeRestoreError::IterationMissing { name: name.clone() });
    }
    if let Some(requirement) = saved.requirement.as_ref() {
        revalidate_source_refs(&requirement.source_refs, scope, index, documents)
            .map_err(|detail| ScopeRestoreError::SourceDrift { detail })?;
    }
    Ok(())
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
        HierarchyPath, QuoteSelector, RequirementDocumentContent, RequirementDocumentIndex,
        RequirementIterationFilter, RequirementLink, RequirementStoreId, ResolutionState,
        ReviewState, TextRange,
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
                store_id: None,
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
                store_id: None,
                document_id: "doc-1".into(),
                document_revision: String::new(),
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
            source_scope: None,
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
            source_scope: None,
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

    fn sample_store() -> RequirementStoreId {
        RequirementStoreId::parse("reqstore-test").unwrap()
    }

    fn sample_index(iteration: Option<&str>) -> RequirementDocumentIndex {
        let mut meta = teshi_core::authoring::RequirementDocumentMeta::new(
            "doc-1",
            "login.md",
            "Login",
            teshi_core::authoring::DocumentRevision::new("rev-1"),
        );
        meta.iteration = iteration.map(str::to_string);
        RequirementDocumentIndex {
            version: 2,
            store_id: Some(sample_store()),
            documents: vec![meta],
        }
    }

    fn sample_documents(body: &str, iteration: Option<&str>) -> Vec<RequirementDocumentContent> {
        let mut meta = teshi_core::authoring::RequirementDocumentMeta::new(
            "doc-1",
            "login.md",
            "Login",
            teshi_core::authoring::DocumentRevision::new("rev-1"),
        );
        meta.iteration = iteration.map(str::to_string);
        vec![RequirementDocumentContent {
            meta,
            body: body.to_string(),
        }]
    }

    fn sample_scope(iteration: RequirementIterationFilter) -> RequirementSourceScope {
        RequirementSourceScope {
            store_id: sample_store(),
            iteration,
        }
    }

    #[test]
    fn gathering_guides_on_demand_store_reads() {
        let guidance = GenerationStage::Gathering.prompt_guidance();
        assert!(guidance.contains("list_requirement_documents"));
        assert!(guidance.contains("read_requirement_document"));
        assert!(guidance.contains("persisted requirement-library source"));
    }

    #[test]
    fn resolve_source_refs_fills_store_and_revision() {
        let scope = sample_scope(RequirementIterationFilter::All);
        let index = sample_index(Some("Sprint 1"));
        let documents = sample_documents("hello world", Some("Sprint 1"));
        let resolved = resolve_submitted_source_refs(
            &[SubmittedSourceRef {
                store_id: None,
                document_id: "doc-1".into(),
                range: Some(TextRange::new(0, 5)),
            }],
            &scope,
            &index,
            &documents,
        )
        .unwrap();
        assert_eq!(resolved[0].store_id.as_ref(), Some(&sample_store()));
        assert_eq!(resolved[0].document_revision, "rev-1");
    }

    #[test]
    fn resolve_source_refs_rejects_wrong_store_and_out_of_scope() {
        let scope = sample_scope(RequirementIterationFilter::Named("Sprint 1".into()));
        let index = sample_index(Some("Sprint 1"));
        let documents = sample_documents("hello", Some("Sprint 1"));
        let other = RequirementStoreId::parse("reqstore-other").unwrap();
        let err = resolve_submitted_source_refs(
            &[SubmittedSourceRef {
                store_id: Some(other),
                document_id: "doc-1".into(),
                range: None,
            }],
            &scope,
            &index,
            &documents,
        )
        .unwrap_err();
        assert!(err.contains("does not match the active scope"));

        let unassigned_index = sample_index(None);
        let unassigned_docs = sample_documents("hello", None);
        let err = resolve_submitted_source_refs(
            &[SubmittedSourceRef {
                store_id: None,
                document_id: "doc-1".into(),
                range: None,
            }],
            &scope,
            &unassigned_index,
            &unassigned_docs,
        )
        .unwrap_err();
        assert!(err.contains("outside the active generation source scope"));
    }

    #[test]
    fn revalidate_detects_revision_drift_and_reclassified_iteration() {
        let scope = sample_scope(RequirementIterationFilter::Named("Sprint 1".into()));
        let index = sample_index(Some("Sprint 1"));
        let documents = sample_documents("hello", Some("Sprint 1"));
        let refs = vec![RequirementSourceRef {
            store_id: Some(sample_store()),
            document_id: "doc-1".into(),
            document_revision: "rev-1".into(),
            range: None,
        }];
        assert!(revalidate_source_refs(&refs, &scope, &index, &documents).is_ok());

        let mut drifted = documents.clone();
        drifted[0].meta.revision = teshi_core::authoring::DocumentRevision::new("rev-2");
        let err = revalidate_source_refs(&refs, &scope, &index, &drifted).unwrap_err();
        assert!(err.contains("revision changed"));

        let moved_index = sample_index(Some("Sprint 2"));
        let moved_docs = sample_documents("hello", Some("Sprint 2"));
        let err = revalidate_source_refs(&refs, &scope, &moved_index, &moved_docs).unwrap_err();
        assert!(err.contains("outside the active generation source scope"));
    }

    #[test]
    fn restore_pauses_on_store_mismatch_missing_iteration_and_legacy_docs() {
        let scope = sample_scope(RequirementIterationFilter::Named("Sprint 1".into()));
        let index = sample_index(Some("Sprint 1"));
        let documents = sample_documents("hello", Some("Sprint 1"));
        let saved = GenerationSessionState {
            stage: GenerationStage::Gathering,
            requirement: Some(Requirement {
                feature_name: "Auth".into(),
                description: None,
                scenario_descriptions: vec![],
                source_refs: vec![RequirementSourceRef {
                    store_id: Some(sample_store()),
                    document_id: "doc-1".into(),
                    document_revision: "rev-1".into(),
                    range: None,
                }],
                tags: vec![],
            }),
            plan: None,
            source_scope: Some(scope.clone()),
        };
        assert!(evaluate_restored_scope(&saved, Some(&sample_store()), &index, &documents).is_ok());

        let other = RequirementStoreId::parse("reqstore-other").unwrap();
        let err = evaluate_restored_scope(&saved, Some(&other), &index, &documents).unwrap_err();
        assert!(matches!(err, ScopeRestoreError::StoreMismatch { .. }));

        let missing_iter = sample_index(Some("Sprint 9"));
        let err = evaluate_restored_scope(&saved, Some(&sample_store()), &missing_iter, &documents)
            .unwrap_err();
        assert!(matches!(err, ScopeRestoreError::IterationMissing { .. }));

        let legacy = GenerationSessionState {
            stage: GenerationStage::Gathering,
            requirement: saved.requirement.clone(),
            plan: None,
            source_scope: None,
        };
        let err = evaluate_restored_scope(&legacy, Some(&sample_store()), &index, &documents)
            .unwrap_err();
        assert_eq!(err, ScopeRestoreError::MissingStoreIdentity);

        let free_text = GenerationSessionState {
            stage: GenerationStage::Gathering,
            requirement: Some(Requirement {
                feature_name: "Auth".into(),
                description: None,
                scenario_descriptions: vec!["login".into()],
                source_refs: vec![],
                tags: vec![],
            }),
            plan: None,
            source_scope: None,
        };
        assert!(
            evaluate_restored_scope(&free_text, Some(&sample_store()), &index, &documents).is_ok()
        );
    }

    #[test]
    fn validate_test_point_links_rejects_out_of_scope() {
        let scope = sample_scope(RequirementIterationFilter::Named("Sprint 1".into()));
        let index = sample_index(Some("Sprint 1"));
        let mut tp = sample_tp("tp-1", ReviewState::Proposed, ResolutionState::Resolved);
        tp.requirement_links[0].store_id = Some(sample_store());
        assert!(validate_test_point_links_in_scope(&[tp.clone()], &scope, &index).is_ok());

        let other = RequirementStoreId::parse("reqstore-other").unwrap();
        tp.requirement_links[0].store_id = Some(other);
        let err = validate_test_point_links_in_scope(&[tp], &scope, &index).unwrap_err();
        assert!(err.contains("outside the active scope"));
    }
}
