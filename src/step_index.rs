//! Step reuse index: normalizes step body text (keyword-agnostic, trimmed) and tracks every
//! location where each unique step appears across the entire project.

use std::collections::HashMap;

use crate::gherkin::BddProject;

/// Where a step occurs within the project tree.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StepLocation {
    pub feature_idx: usize,
    pub scenario_idx: usize,
    pub step_idx: usize,
}

/// Maps normalized step body text → all locations that share it.
#[derive(Debug, Clone)]
pub struct StepIndex {
    /// All usages keyed by normalized step body.
    ///
    /// Populated by [`StepIndex::build`]. Exposed for tooling; the binary currently uses only [`StepIndex::reuse_count`].
    #[allow(dead_code)]
    pub usages: HashMap<String, Vec<StepLocation>>,
}

impl StepIndex {
    /// Builds the index by scanning every step in every scenario (including backgrounds).
    pub fn build(project: &BddProject) -> Self {
        let mut usages: HashMap<String, Vec<StepLocation>> = HashMap::new();
        for (fi, feature) in project.features.iter().enumerate() {
            // Background steps count towards reuse
            if let Some(bg) = &feature.background {
                for (si, step) in bg.steps.iter().enumerate() {
                    let key = normalize(&step.text);
                    usages.entry(key).or_default().push(StepLocation {
                        feature_idx: fi,
                        scenario_idx: usize::MAX, // sentinel for background
                        step_idx: si,
                    });
                }
            }
            for (sci, scenario) in feature.scenarios.iter().enumerate() {
                for (sti, step) in scenario.steps.iter().enumerate() {
                    let key = normalize(&step.text);
                    usages.entry(key).or_default().push(StepLocation {
                        feature_idx: fi,
                        scenario_idx: sci,
                        step_idx: sti,
                    });
                }
            }
        }
        Self { usages }
    }

    /// Returns the total usage count for a step body (>= 2 means reused).
    ///
    /// Exercised by unit tests; kept for future UI that surfaces reuse stats.
    #[allow(dead_code)]
    pub fn reuse_count(&self, step_text: &str) -> usize {
        let key = normalize(step_text);
        self.usages.get(&key).map_or(0, |v| v.len())
    }
}

/// Keyword-agnostic normalization: lowercase, collapse whitespace, trim.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gherkin::{BddProject, parse_feature};
    use std::path::PathBuf;

    fn sample_project() -> BddProject {
        let a = parse_feature(
            "\
Feature: A
  Scenario: S1
    Given I am on the login page
    When I click submit

  Scenario: S2
    Given I am on the login page
    Then I see the dashboard
",
            PathBuf::from("a.feature"),
        );
        let b = parse_feature(
            "\
Feature: B
  Scenario: S3
    When I am on the login page
    And I click submit
",
            PathBuf::from("b.feature"),
        );
        BddProject {
            root_dir: PathBuf::from("."),
            features: vec![a, b],
        }
    }

    #[test]
    fn counts_cross_feature_reuse() {
        let project = sample_project();
        let index = StepIndex::build(&project);
        // "I am on the login page" appears in S1, S2, S3 → 3 uses
        assert_eq!(index.reuse_count("I am on the login page"), 3);
        // "I click submit" appears in S1 and S3 → 2 uses
        assert_eq!(index.reuse_count("I click submit"), 2);
        // "I see the dashboard" appears only once
        assert_eq!(index.reuse_count("I see the dashboard"), 1);
    }

    #[test]
    fn normalization_is_case_and_whitespace_insensitive() {
        let project = sample_project();
        let index = StepIndex::build(&project);
        assert_eq!(index.reuse_count("i am on the login page"), 3);
        assert_eq!(index.reuse_count("I  am  on  the  login  page"), 3);
    }
}
