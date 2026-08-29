//! Engine tags (`@api` / `@ui`), `[API]` step markers, and mismatch checks.
//!
//! Scenario engine tags override Feature tags (no union). Mixed mode is both
//! `@api` and `@ui` on the Scenario itself. Untagged scenarios stay UI-only so
//! existing projects keep their current runner path.

use crate::gherkin::{BddFeature, BddScenario, BddStep};

/// How Teshi should execute a scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineMode {
    /// Browser / WinApp bindings only (default when no engine tags are present).
    Ui,
    /// HTTP API sidecar / behave helper only.
    Api,
    /// Walk steps: `[API]` → API sidecar, otherwise UI bindings.
    Mixed,
}

/// Result of stripping a leading `[API]` token from step body text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiMarker {
    /// Whether the token was present.
    pub is_api: bool,
}

/// Returns true when `tag` is an engine selector (`@api` or `@ui`).
#[must_use]
pub fn is_engine_tag(tag: &str) -> bool {
    matches!(normalize_tag(tag).as_str(), "api" | "ui")
}

/// Strip a leading `@` and lowercase for comparison.
#[must_use]
pub fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('@').to_ascii_lowercase()
}

/// Strip a leading `[API]` token from Gherkin step body text (after the keyword).
///
/// The original string is unchanged when the marker is absent. Leading whitespace
/// after the marker is dropped so behave matching sees the remaining phrase.
#[must_use]
pub fn strip_api_marker(text: &str) -> (bool, String) {
    let trimmed = text.trim_start();
    let rest = trimmed
        .strip_prefix("[API]")
        .or_else(|| trimmed.strip_prefix("[api]"));
    match rest {
        Some(after) => (true, after.trim_start().to_string()),
        None => (false, text.to_string()),
    }
}

/// True when this step is marked `[API]` immediately after the keyword.
#[must_use]
pub fn step_is_api(step: &BddStep) -> bool {
    strip_api_marker(&step.text).0
}

fn tag_set_has(tags: &[String], name: &str) -> bool {
    tags.iter().any(|tag| normalize_tag(tag) == name)
}

fn engine_tags_present(tags: &[String]) -> bool {
    tags.iter().any(|tag| is_engine_tag(tag))
}

/// Resolve engine mode: Scenario engine tags win if any are present; otherwise Feature tags.
#[must_use]
pub fn resolve_engine_mode(feature_tags: &[String], scenario_tags: &[String]) -> EngineMode {
    let tags = if engine_tags_present(scenario_tags) {
        scenario_tags
    } else {
        feature_tags
    };
    let api = tag_set_has(tags, "api");
    let ui = tag_set_has(tags, "ui");
    match (api, ui) {
        (true, true) => EngineMode::Mixed,
        (true, false) => EngineMode::Api,
        (false, true) => EngineMode::Ui,
        (false, false) => EngineMode::Ui,
    }
}

/// Resolve mode for a parsed feature/scenario pair.
#[must_use]
pub fn scenario_engine_mode(feature: &BddFeature, scenario: &BddScenario) -> EngineMode {
    resolve_engine_mode(&feature.tags, &scenario.tags)
}

/// Background plus scenario steps in execution order.
#[must_use]
pub fn scenario_steps<'a>(feature: &'a BddFeature, scenario: &'a BddScenario) -> Vec<&'a BddStep> {
    let mut steps = Vec::new();
    if let Some(background) = &feature.background {
        steps.extend(background.steps.iter());
    }
    steps.extend(scenario.steps.iter());
    steps
}

/// Why a scenario cannot run under its resolved engine tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineMismatch {
    /// Human-readable reason (English, user-facing).
    pub message: String,
}

/// Fail when `@api`-only contains a UI step, `@ui`-only contains `[API]`, or
/// untagged UI mode still contains `[API]` steps.
///
/// # Errors
///
/// Returns [`EngineMismatch`] when tags and step markers disagree.
pub fn validate_scenario_steps(mode: EngineMode, steps: &[&BddStep]) -> Result<(), EngineMismatch> {
    let has_api = steps.iter().any(|step| step_is_api(step));
    let has_ui = steps.iter().any(|step| !step_is_api(step));
    match mode {
        EngineMode::Api if has_ui => Err(EngineMismatch {
            message: "scenario is @api-only but contains a step without [API]; \
                      tag the scenario @api @ui for mixed runs or mark HTTP steps with [API]"
                .into(),
        }),
        EngineMode::Ui if has_api => Err(EngineMismatch {
            message: "scenario is UI-only but contains an [API] step; \
                      tag the scenario @api or @api @ui"
                .into(),
        }),
        EngineMode::Mixed | EngineMode::Api | EngineMode::Ui => Ok(()),
    }
}

/// Validate a feature/scenario pair using resolved tags and all executable steps.
///
/// # Errors
///
/// Returns [`EngineMismatch`] when tags and step markers disagree.
pub fn validate_feature_scenario(
    feature: &BddFeature,
    scenario: &BddScenario,
) -> Result<EngineMode, EngineMismatch> {
    let mode = scenario_engine_mode(feature, scenario);
    validate_scenario_steps(mode, &scenario_steps(feature, scenario))?;
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gherkin_lang::StepKeywordType;
    use crate::parse_feature;
    use std::path::PathBuf;

    fn step(text: &str) -> BddStep {
        BddStep {
            keyword: "When".into(),
            keyword_type: StepKeywordType::When,
            text: text.into(),
            line_number: 1,
            doc_string: None,
            data_table: None,
        }
    }

    #[test]
    fn strip_api_marker_removes_token_and_space() {
        let (is_api, rest) = strip_api_marker("[API] I create a user named \"Ada\"");
        assert!(is_api);
        assert_eq!(rest, "I create a user named \"Ada\"");
        let (is_api, rest) = strip_api_marker("I click save");
        assert!(!is_api);
        assert_eq!(rest, "I click save");
    }

    #[test]
    fn scenario_tag_overrides_feature_without_union() {
        let mode = resolve_engine_mode(&["@api".into()], &["@ui".into()]);
        assert_eq!(mode, EngineMode::Ui);
        let mixed = resolve_engine_mode(&["@api".into()], &["@api".into(), "@ui".into()]);
        assert_eq!(mixed, EngineMode::Mixed);
        let inherit = resolve_engine_mode(&["@api".into()], &["@smoke".into()]);
        assert_eq!(inherit, EngineMode::Api);
    }

    #[test]
    fn api_only_with_ui_step_is_mismatch() {
        let steps = [step("I click save")];
        let err = validate_scenario_steps(EngineMode::Api, &steps.iter().collect::<Vec<_>>())
            .expect_err("mismatch");
        assert!(err.message.contains("@api-only"));
    }

    #[test]
    fn ui_only_with_api_step_is_mismatch() {
        let steps = [step("[API] I create a user")];
        let err = validate_scenario_steps(EngineMode::Ui, &steps.iter().collect::<Vec<_>>())
            .expect_err("mismatch");
        assert!(err.message.contains("UI-only"));
    }

    #[test]
    fn mixed_allows_both_step_kinds() {
        let steps = [step("I click save"), step("[API] I create a user")];
        let refs: Vec<&BddStep> = steps.iter().collect();
        assert!(validate_scenario_steps(EngineMode::Mixed, &refs).is_ok());
    }

    #[test]
    fn parsed_feature_override_and_mismatch() {
        let source = "\
@api
Feature: Billing
  Scenario: UI path
    When I click save
  @ui
  Scenario: Forced UI
    When I click save
  @api @ui
  Scenario: Mixed
    When I click save
    And [API] I create a user named \"Ada\"
";
        let feature = parse_feature(source, PathBuf::from("billing.feature"));
        let ui_path = &feature.scenarios[0];
        let err = validate_feature_scenario(&feature, ui_path).expect_err("api feature + ui step");
        assert!(err.message.contains("@api-only"));

        let forced_ui = &feature.scenarios[1];
        assert_eq!(
            validate_feature_scenario(&feature, forced_ui).unwrap(),
            EngineMode::Ui
        );

        let mixed = &feature.scenarios[2];
        assert_eq!(
            validate_feature_scenario(&feature, mixed).unwrap(),
            EngineMode::Mixed
        );
    }
}
