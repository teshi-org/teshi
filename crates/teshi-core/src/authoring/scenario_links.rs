//! Teshi-owned Gherkin encoding that links scenarios to test-point IDs.
//!
//! Convention (selected for parser compatibility):
//! - One tag per ID: `@teshi-tp:<id>`
//! - Tags sit on the line immediately above `Scenario:` / `Scenario Outline:`
//! - Multiple IDs ⇒ multiple tags (avoids comma-in-ID ambiguity)
//! - Existing scenarios with no `@teshi-tp:*` tags are treated as unlinked

use crate::authoring::testpoints::{ScenarioRef, TestPoint};
use crate::gherkin::BddProject;

/// Prefix for Teshi test-point tags embedded in Gherkin.
pub const TESHI_TP_TAG_PREFIX: &str = "@teshi-tp:";

const ENCODED_ID_PREFIX: &str = "~v1~";

/// Formats a single test-point ID as a Gherkin tag.
///
/// IDs containing token-breaking or encoding characters are versioned and
/// percent-encoded so the tag remains one lossless token.
pub fn format_teshi_tp_tag(id: &str) -> String {
    let needs_encoding = id.starts_with(ENCODED_ID_PREFIX)
        || id.chars().any(|character| {
            character.is_whitespace() || character == '@' || character == '%' || character == '~'
        });
    if !needs_encoding {
        return format!("{TESHI_TP_TAG_PREFIX}{id}");
    }

    let mut encoded = String::new();
    for character in id.chars() {
        if character.is_whitespace() || character == '@' || character == '%' || character == '~' {
            let mut buffer = [0; 4];
            for byte in character.encode_utf8(&mut buffer).as_bytes() {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        } else {
            encoded.push(character);
        }
    }
    format!("{TESHI_TP_TAG_PREFIX}{ENCODED_ID_PREFIX}{encoded}")
}

/// Converts test-point IDs into `@teshi-tp:<id>` tags.
pub fn teshi_tp_tags(ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|id| !id.trim().is_empty())
        .map(|id| format_teshi_tp_tag(id))
        .collect()
}

/// Extracts test-point IDs from scenario tags, ignoring unrelated tags.
pub fn parse_teshi_tp_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .filter_map(|tag| {
            let trimmed = tag.trim();
            trimmed
                .strip_prefix(TESHI_TP_TAG_PREFIX)
                .map(decode_test_point_id)
                .filter(|id| !id.is_empty())
        })
        .collect()
}

/// Extracts IDs while preserving legacy marker-shaped IDs known to the project.
///
/// A tag is ignored when its raw and decoded forms identify two different test
/// points, because that legacy/new encoding collision is inherently ambiguous.
pub fn parse_teshi_tp_tags_for_test_points(
    tags: &[String],
    test_points: &[TestPoint],
) -> Vec<String> {
    tags.iter()
        .filter_map(|tag| {
            let raw = tag.trim().strip_prefix(TESHI_TP_TAG_PREFIX)?;
            if raw.is_empty() {
                return None;
            }
            let decoded = decode_test_point_id(raw);
            let raw_exists = test_points.iter().any(|test_point| test_point.id == raw);
            let decoded_exists = test_points
                .iter()
                .any(|test_point| test_point.id == decoded);
            match (raw_exists, decoded_exists, raw == decoded) {
                (true, true, false) => None,
                (true, _, _) => Some(raw.to_string()),
                (_, true, _) => Some(decoded),
                _ => Some(decoded),
            }
        })
        .collect()
}

fn decode_test_point_id(encoded: &str) -> String {
    let Some(encoded) = encoded.strip_prefix(ENCODED_ID_PREFIX) else {
        return encoded.to_string();
    };
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&encoded[index + 1..index + 3], 16)
        {
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| encoded.to_string())
}

/// Merges caller tags with test-point tags, preserving order and deduplicating.
pub fn merge_scenario_tags(existing_tags: &[String], test_point_ids: &[String]) -> Vec<String> {
    let mut merged = Vec::new();
    for tag in existing_tags {
        let t = tag.trim();
        if t.is_empty() {
            continue;
        }
        let normalized = if t.starts_with('@') {
            t.to_string()
        } else {
            format!("@{t}")
        };
        if !merged.iter().any(|m: &String| m == &normalized) {
            merged.push(normalized);
        }
    }
    for tp_tag in teshi_tp_tags(test_point_ids) {
        if !merged.iter().any(|m| m == &tp_tag) {
            merged.push(tp_tag);
        }
    }
    merged
}

/// Rebuilds `scenario_refs` on test points from `@teshi-tp:*` tags in the project.
///
/// Unknown tag IDs are ignored (tolerant of drift). Scenarios without Teshi tags
/// remain unlinked and do not clear other test points' refs except via the full rebuild.
pub fn sync_scenario_refs_from_project(project: &BddProject, test_points: &mut [TestPoint]) {
    for tp in test_points.iter_mut() {
        tp.scenario_refs.clear();
    }

    let root = &project.root_dir;
    for feature in &project.features {
        let feature_path = feature
            .file_path
            .strip_prefix(root)
            .unwrap_or(&feature.file_path)
            .to_string_lossy()
            .replace('\\', "/");

        let mut all_scenarios = Vec::new();
        for sc in &feature.scenarios {
            all_scenarios.push(sc);
        }
        for rule in &feature.rules {
            for sc in &rule.scenarios {
                all_scenarios.push(sc);
            }
        }

        for sc in all_scenarios {
            let ids = parse_teshi_tp_tags_for_test_points(&sc.tags, test_points);
            if ids.is_empty() {
                continue;
            }
            let scenario_ref = ScenarioRef {
                feature_path: feature_path.clone(),
                scenario_name: Some(sc.name.clone()),
                scenario_line: Some(sc.line_number),
            };
            for id in ids {
                if let Some(tp) = test_points.iter_mut().find(|tp| tp.id == id)
                    && !tp.scenario_refs.iter().any(|r| {
                        r.feature_path == scenario_ref.feature_path
                            && r.scenario_name == scenario_ref.scenario_name
                            && r.scenario_line == scenario_ref.scenario_line
                    })
                {
                    tp.scenario_refs.push(scenario_ref.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::{HierarchyPath, ReviewState};
    use crate::gherkin::parse_feature;
    use std::path::PathBuf;

    #[test]
    fn format_and_parse_roundtrip() {
        let tags = teshi_tp_tags(&["tp-1".into(), "tp-login".into()]);
        assert_eq!(tags, vec!["@teshi-tp:tp-1", "@teshi-tp:tp-login"]);
        assert_eq!(
            parse_teshi_tp_tags(&tags),
            vec!["tp-1".to_string(), "tp-login".to_string()]
        );
    }

    #[test]
    fn parse_ignores_unrelated_tags() {
        let tags = vec!["@smoke".into(), "@teshi-tp:tp-1".into(), "@security".into()];
        assert_eq!(parse_teshi_tp_tags(&tags), vec!["tp-1".to_string()]);
    }

    #[test]
    fn unlinked_scenarios_yield_empty() {
        assert!(parse_teshi_tp_tags(&["@smoke".into()]).is_empty());
        assert!(parse_teshi_tp_tags(&[]).is_empty());
    }

    #[test]
    fn merge_dedupes_and_normalizes() {
        let merged = merge_scenario_tags(
            &["smoke".into(), "@teshi-tp:tp-1".into()],
            &["tp-1".into(), "tp-2".into()],
        );
        assert_eq!(
            merged,
            vec![
                "@smoke".to_string(),
                "@teshi-tp:tp-1".to_string(),
                "@teshi-tp:tp-2".to_string()
            ]
        );
    }

    #[test]
    fn special_characters_are_encoded_without_trimming() {
        let ids = vec![" tp 1 ".into(), "owner@example".into(), "100%".into()];
        let tags = teshi_tp_tags(&ids);
        assert_eq!(
            tags,
            vec![
                "@teshi-tp:~v1~%20tp%201%20",
                "@teshi-tp:~v1~owner%40example",
                "@teshi-tp:~v1~100%25"
            ]
        );
        assert_eq!(parse_teshi_tp_tags(&tags), ids);
    }

    #[test]
    fn parser_preserves_teshi_tp_tags() {
        let content = "Feature: Auth\n\n  @smoke @teshi-tp:tp-1 @teshi-tp:tp-2\n  Scenario: Login\n    Given x\n";
        let feature = parse_feature(content, PathBuf::from("auth.feature"));
        assert_eq!(feature.scenarios.len(), 1);
        let ids = parse_teshi_tp_tags(&feature.scenarios[0].tags);
        assert_eq!(ids, vec!["tp-1".to_string(), "tp-2".to_string()]);
    }

    #[test]
    fn sync_scenario_refs_from_parsed_project() {
        let content = "Feature: Auth\n\n  @teshi-tp:tp-1\n  Scenario: Login\n    Given x\n\n  Scenario: Unlinked\n    Given y\n";
        let feature = parse_feature(content, PathBuf::from("/proj/auth.feature"));
        let project = BddProject {
            root_dir: PathBuf::from("/proj"),
            features: vec![feature],
        };
        let mut tps = vec![TestPoint {
            id: "tp-1".into(),
            title: "t".into(),
            objective: "o".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["A".into()]),
            review_state: ReviewState::Approved,
            requirement_links: vec![],
            scenario_refs: vec![],
        }];
        sync_scenario_refs_from_project(&project, &mut tps);
        assert_eq!(tps[0].scenario_refs.len(), 1);
        assert_eq!(tps[0].scenario_refs[0].feature_path, "auth.feature");
        assert_eq!(
            tps[0].scenario_refs[0].scenario_name.as_deref(),
            Some("Login")
        );
    }

    #[test]
    fn sync_scenario_refs_matches_encoded_test_point_id() {
        let content = "Feature: Auth\n\n  @teshi-tp:~v1~%20tp%201%40owner%20\n  Scenario: Login\n    Given x\n";
        let feature = parse_feature(content, PathBuf::from("/proj/auth.feature"));
        let project = BddProject {
            root_dir: PathBuf::from("/proj"),
            features: vec![feature],
        };
        let mut tps = vec![TestPoint {
            id: " tp 1@owner ".into(),
            title: "t".into(),
            objective: "o".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["A".into()]),
            review_state: ReviewState::Approved,
            requirement_links: vec![],
            scenario_refs: vec![],
        }];

        sync_scenario_refs_from_project(&project, &mut tps);

        assert_eq!(tps[0].scenario_refs.len(), 1);
    }

    #[test]
    fn sync_preserves_legacy_marker_shaped_ids() {
        let content =
            "Feature: Auth\n\n  @teshi-tp:~v1~discount%20code\n  Scenario: Apply\n    Given x\n";
        let feature = parse_feature(content, PathBuf::from("/proj/auth.feature"));
        let project = BddProject {
            root_dir: PathBuf::from("/proj"),
            features: vec![feature],
        };
        let mut tps = vec![TestPoint {
            id: "~v1~discount%20code".into(),
            title: "t".into(),
            objective: "o".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["A".into()]),
            review_state: ReviewState::Approved,
            requirement_links: vec![],
            scenario_refs: vec![],
        }];

        sync_scenario_refs_from_project(&project, &mut tps);

        assert_eq!(tps[0].scenario_refs.len(), 1);
    }

    #[test]
    fn known_id_parser_ignores_ambiguous_legacy_encoding_collision() {
        let tags = vec!["@teshi-tp:~v1~a%20b".into()];
        let make_test_point = |id: &str| TestPoint {
            id: id.into(),
            title: "t".into(),
            objective: "o".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["A".into()]),
            review_state: ReviewState::Approved,
            requirement_links: vec![],
            scenario_refs: vec![],
        };
        let test_points = vec![make_test_point("~v1~a%20b"), make_test_point("a b")];

        assert!(parse_teshi_tp_tags_for_test_points(&tags, &test_points).is_empty());
    }
}
