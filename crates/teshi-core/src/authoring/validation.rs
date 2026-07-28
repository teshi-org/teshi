//! Authoring artifact validation (pure logic, no I/O).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::authoring::{
    HierarchyPath, RequirementDocumentIndex, RequirementDocumentMeta, ReviewState, TestPointsFile,
    TextRange,
};

/// Severity of an authoring validation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringSeverity {
    /// Blocks generation and must be fixed before use.
    Error,
    /// Visible issue that does not always block unrelated operations.
    Warning,
}

/// A single validation issue found while loading authoring artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringDiagnostic {
    pub severity: AuthoringSeverity,
    /// Project-relative path or logical artifact name (e.g. `requirements/_teshi.json`).
    pub location: String,
    /// Optional test-point or document identifier implicated in the issue.
    pub record_id: Option<String>,
    pub message: String,
}

/// Markdown body loaded for one requirement document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementDocumentContent {
    pub meta: RequirementDocumentMeta,
    pub body: String,
}

/// Loaded authoring artifacts plus validation diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringArtifacts {
    pub index: RequirementDocumentIndex,
    pub documents: Vec<RequirementDocumentContent>,
    pub test_points: TestPointsFile,
    pub diagnostics: Vec<AuthoringDiagnostic>,
}

impl AuthoringArtifacts {
    /// Returns `true` when any error-level diagnostic is present.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == AuthoringSeverity::Error)
    }
}

/// Validates a requirement index for duplicate IDs and basic field constraints.
pub fn validate_requirement_index(index: &RequirementDocumentIndex) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();

    for doc in &index.documents {
        if doc.id.trim().is_empty() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "requirements/_teshi.json".into(),
                record_id: None,
                message: "requirement document id must not be empty".into(),
            });
        } else if !seen_ids.insert(doc.id.clone()) {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "requirements/_teshi.json".into(),
                record_id: Some(doc.id.clone()),
                message: format!("duplicate requirement document id '{}'", doc.id),
            });
        }

        if doc.path.trim().is_empty() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "requirements/_teshi.json".into(),
                record_id: Some(doc.id.clone()),
                message: "requirement document path must not be empty".into(),
            });
        } else if !seen_paths.insert(doc.path.clone()) {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "requirements/_teshi.json".into(),
                record_id: Some(doc.id.clone()),
                message: format!("duplicate requirement document path '{}'", doc.path),
            });
        }

        if doc.title.trim().is_empty() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Warning,
                location: "requirements/_teshi.json".into(),
                record_id: Some(doc.id.clone()),
                message: "requirement document title is empty".into(),
            });
        }

        if doc.revision.as_str().trim().is_empty() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "requirements/_teshi.json".into(),
                record_id: Some(doc.id.clone()),
                message: "requirement document revision must not be empty".into(),
            });
        }
    }

    diagnostics
}

/// Validates hierarchy paths for empty segments.
pub fn validate_hierarchy_path(path: &HierarchyPath) -> Option<AuthoringDiagnostic> {
    if path.segments().is_empty() {
        return Some(AuthoringDiagnostic {
            severity: AuthoringSeverity::Error,
            location: "testpoints/testpoints.json".into(),
            record_id: None,
            message: "hierarchy path must contain at least one segment".into(),
        });
    }
    for (idx, segment) in path.segments().iter().enumerate() {
        if segment.trim().is_empty() {
            return Some(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "testpoints/testpoints.json".into(),
                record_id: None,
                message: format!("hierarchy path segment {idx} must not be empty"),
            });
        }
    }
    None
}

/// Validates a text range against document character length.
pub fn validate_text_range(
    range: TextRange,
    char_len: u32,
    location: &str,
    record_id: Option<&str>,
) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();
    if !range.is_non_empty() {
        diagnostics.push(AuthoringDiagnostic {
            severity: AuthoringSeverity::Error,
            location: location.into(),
            record_id: record_id.map(str::to_string),
            message: "requirement link range must be non-empty".into(),
        });
    }
    if range.end.offset() > char_len {
        diagnostics.push(AuthoringDiagnostic {
            severity: AuthoringSeverity::Error,
            location: location.into(),
            record_id: record_id.map(str::to_string),
            message: format!(
                "requirement link end offset {} exceeds document length {char_len}",
                range.end.offset()
            ),
        });
    }
    if range.start.offset() > range.end.offset() {
        diagnostics.push(AuthoringDiagnostic {
            severity: AuthoringSeverity::Error,
            location: location.into(),
            record_id: record_id.map(str::to_string),
            message: "requirement link start offset must not exceed end offset".into(),
        });
    }
    diagnostics
}

/// Validates quote selectors for well-formed anchors.
pub fn validate_quote_selector(
    quote: &str,
    location: &str,
    record_id: Option<&str>,
) -> Vec<AuthoringDiagnostic> {
    if quote.trim().is_empty() {
        vec![AuthoringDiagnostic {
            severity: AuthoringSeverity::Error,
            location: location.into(),
            record_id: record_id.map(str::to_string),
            message: "requirement link quote must not be empty".into(),
        }]
    } else {
        Vec::new()
    }
}

/// Validates test points against the index and on-disk feature references.
pub fn validate_test_points(
    file: &TestPointsFile,
    index: &RequirementDocumentIndex,
    project_root: &Path,
) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen_ids = HashSet::new();
    let doc_ids: HashMap<&str, &RequirementDocumentMeta> =
        index.documents.iter().map(|d| (d.id.as_str(), d)).collect();

    for tp in &file.test_points {
        if tp.id.trim().is_empty() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "testpoints/testpoints.json".into(),
                record_id: None,
                message: "test point id must not be empty".into(),
            });
            continue;
        }

        if !seen_ids.insert(tp.id.clone()) {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "testpoints/testpoints.json".into(),
                record_id: Some(tp.id.clone()),
                message: format!("duplicate test point id '{}'", tp.id),
            });
        }

        if tp.title.trim().is_empty() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "testpoints/testpoints.json".into(),
                record_id: Some(tp.id.clone()),
                message: "test point title must not be empty".into(),
            });
        }

        if tp.objective.trim().is_empty() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "testpoints/testpoints.json".into(),
                record_id: Some(tp.id.clone()),
                message: "test point objective must not be empty".into(),
            });
        }

        if let Some(diag) = validate_hierarchy_path(&tp.hierarchy_path) {
            diagnostics.push(AuthoringDiagnostic {
                record_id: Some(tp.id.clone()),
                ..diag
            });
        }

        if !is_valid_review_state(tp.review_state) {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: "testpoints/testpoints.json".into(),
                record_id: Some(tp.id.clone()),
                message: format!("invalid review state {:?}", tp.review_state),
            });
        }

        for link in &tp.requirement_links {
            if link.document_id.trim().is_empty() {
                diagnostics.push(AuthoringDiagnostic {
                    severity: AuthoringSeverity::Error,
                    location: "testpoints/testpoints.json".into(),
                    record_id: Some(tp.id.clone()),
                    message: "requirement link document_id must not be empty".into(),
                });
                continue;
            }

            if !doc_ids.contains_key(link.document_id.as_str()) {
                diagnostics.push(AuthoringDiagnostic {
                    severity: AuthoringSeverity::Error,
                    location: "testpoints/testpoints.json".into(),
                    record_id: Some(tp.id.clone()),
                    message: format!(
                        "requirement link references unknown document '{}'",
                        link.document_id
                    ),
                });
            }

            diagnostics.extend(validate_quote_selector(
                &link.quote.quote,
                "testpoints/testpoints.json",
                Some(&tp.id),
            ));
        }

        for scenario_ref in &tp.scenario_refs {
            if scenario_ref.feature_path.trim().is_empty() {
                diagnostics.push(AuthoringDiagnostic {
                    severity: AuthoringSeverity::Error,
                    location: "testpoints/testpoints.json".into(),
                    record_id: Some(tp.id.clone()),
                    message: "scenario reference feature_path must not be empty".into(),
                });
                continue;
            }

            if scenario_ref.feature_path.contains('\\') {
                diagnostics.push(AuthoringDiagnostic {
                    severity: AuthoringSeverity::Error,
                    location: "testpoints/testpoints.json".into(),
                    record_id: Some(tp.id.clone()),
                    message: format!(
                        "scenario reference feature_path must use forward slashes: '{}'",
                        scenario_ref.feature_path
                    ),
                });
            }

            let feature_path = project_root.join(&scenario_ref.feature_path);
            if !feature_path.is_file() {
                diagnostics.push(AuthoringDiagnostic {
                    severity: AuthoringSeverity::Error,
                    location: "testpoints/testpoints.json".into(),
                    record_id: Some(tp.id.clone()),
                    message: format!(
                        "scenario reference feature file not found: '{}'",
                        scenario_ref.feature_path
                    ),
                });
            }
        }
    }

    diagnostics
}

/// Cross-validates loaded documents, ranges, and missing files.
pub fn validate_loaded_artifacts(
    artifacts: &AuthoringArtifacts,
    project_root: &Path,
) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = validate_requirement_index(&artifacts.index);
    diagnostics.extend(validate_test_points(
        &artifacts.test_points,
        &artifacts.index,
        project_root,
    ));

    let docs_by_id: HashMap<&str, &RequirementDocumentContent> = artifacts
        .documents
        .iter()
        .map(|d| (d.meta.id.as_str(), d))
        .collect();

    for doc in &artifacts.index.documents {
        let req_path = requirements_root(project_root).join(&doc.path);
        if !req_path.is_file() {
            diagnostics.push(AuthoringDiagnostic {
                severity: AuthoringSeverity::Error,
                location: doc.path.clone(),
                record_id: Some(doc.id.clone()),
                message: "indexed requirement markdown file is missing".into(),
            });
        }
    }

    for tp in &artifacts.test_points.test_points {
        for link in &tp.requirement_links {
            let Some(content) = docs_by_id.get(link.document_id.as_str()) else {
                continue;
            };
            let char_len = content.body.chars().count() as u32;
            diagnostics.extend(validate_text_range(
                link.position,
                char_len,
                &content.meta.path,
                Some(&tp.id),
            ));
            diagnostics.extend(validate_quote_selector(
                &link.quote.quote,
                &content.meta.path,
                Some(&tp.id),
            ));
        }
    }

    diagnostics
}

/// Default requirement root relative to the project directory.
pub fn requirements_root(project_root: &Path) -> PathBuf {
    project_root.join("requirements")
}

/// Default test points file path relative to the project directory.
pub fn testpoints_file(project_root: &Path) -> PathBuf {
    project_root.join("testpoints").join("testpoints.json")
}

fn is_valid_review_state(state: ReviewState) -> bool {
    matches!(
        state,
        ReviewState::Proposed
            | ReviewState::Approved
            | ReviewState::Rejected
            | ReviewState::NeedsReview
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::{
        DocumentRevision, QuoteSelector, RequirementDocumentMeta, RequirementLink, ResolutionState,
        ScenarioRef, TestPoint,
    };

    fn sample_index() -> RequirementDocumentIndex {
        RequirementDocumentIndex {
            version: 1,
            documents: vec![RequirementDocumentMeta {
                id: "doc-1".into(),
                path: "auth.md".into(),
                title: "Auth".into(),
                revision: DocumentRevision::new("rev-1"),
            }],
        }
    }

    #[test]
    fn duplicate_document_ids_are_reported() {
        let index = RequirementDocumentIndex {
            documents: vec![
                RequirementDocumentMeta {
                    id: "dup".into(),
                    path: "a.md".into(),
                    title: "A".into(),
                    revision: DocumentRevision::new("r1"),
                },
                RequirementDocumentMeta {
                    id: "dup".into(),
                    path: "b.md".into(),
                    title: "B".into(),
                    revision: DocumentRevision::new("r2"),
                },
            ],
            ..Default::default()
        };
        let issues = validate_requirement_index(&index);
        assert!(issues.iter().any(|i| i.message.contains("duplicate")));
    }

    #[test]
    fn empty_hierarchy_segment_is_reported() {
        let path = HierarchyPath::new(vec!["Auth".into(), "".into()]);
        let issue = validate_hierarchy_path(&path).expect("issue");
        assert!(issue.message.contains("segment 1"));
    }

    #[test]
    fn unknown_document_reference_is_reported() {
        let file = TestPointsFile {
            test_points: vec![TestPoint {
                id: "tp-1".into(),
                title: "T".into(),
                objective: "O".into(),
                preconditions: None,
                expected_outcomes: None,
                hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
                review_state: ReviewState::Proposed,
                requirement_links: vec![RequirementLink {
                    document_id: "missing".into(),
                    document_revision: "rev".into(),
                    position: TextRange::new(0, 1),
                    quote: QuoteSelector {
                        quote: "x".into(),
                        prefix: String::new(),
                        suffix: String::new(),
                    },
                    resolution: ResolutionState::Resolved,
                }],
                scenario_refs: Vec::new(),
            }],
            ..Default::default()
        };
        let issues = validate_test_points(&file, &sample_index(), Path::new("."));
        assert!(
            issues
                .iter()
                .any(|i| i.message.contains("unknown document"))
        );
    }

    #[test]
    fn invalid_range_is_reported() {
        let issues = validate_text_range(TextRange::new(5, 3), 10, "auth.md", Some("tp-1"));
        assert!(issues.iter().any(|i| i.message.contains("non-empty")));
    }

    #[test]
    fn missing_feature_reference_is_reported() {
        let file = TestPointsFile {
            test_points: vec![TestPoint {
                id: "tp-1".into(),
                title: "T".into(),
                objective: "O".into(),
                preconditions: None,
                expected_outcomes: None,
                hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
                review_state: ReviewState::Proposed,
                requirement_links: Vec::new(),
                scenario_refs: vec![ScenarioRef {
                    feature_path: "features/missing.feature".into(),
                    scenario_name: None,
                    scenario_line: None,
                }],
            }],
            ..Default::default()
        };
        let issues = validate_test_points(&file, &sample_index(), Path::new("."));
        assert!(issues.iter().any(|i| i.message.contains("not found")));
    }

    #[test]
    fn table_driven_validation_failures() {
        let cases: Vec<(&str, RequirementDocumentIndex, TestPointsFile, bool)> = vec![
            (
                "duplicate document id",
                RequirementDocumentIndex {
                    documents: vec![
                        RequirementDocumentMeta {
                            id: "dup".into(),
                            path: "a.md".into(),
                            title: "A".into(),
                            revision: DocumentRevision::new("r1"),
                        },
                        RequirementDocumentMeta {
                            id: "dup".into(),
                            path: "b.md".into(),
                            title: "B".into(),
                            revision: DocumentRevision::new("r2"),
                        },
                    ],
                    ..Default::default()
                },
                TestPointsFile::default(),
                true,
            ),
            (
                "empty hierarchy path",
                sample_index(),
                TestPointsFile {
                    test_points: vec![TestPoint {
                        id: "tp-empty-hierarchy".into(),
                        title: "T".into(),
                        objective: "O".into(),
                        preconditions: None,
                        expected_outcomes: None,
                        hierarchy_path: HierarchyPath::new(vec![]),
                        review_state: ReviewState::Proposed,
                        requirement_links: Vec::new(),
                        scenario_refs: Vec::new(),
                    }],
                    ..Default::default()
                },
                true,
            ),
            (
                "empty quote selector",
                sample_index(),
                TestPointsFile {
                    test_points: vec![TestPoint {
                        id: "tp-empty-quote".into(),
                        title: "T".into(),
                        objective: "O".into(),
                        preconditions: None,
                        expected_outcomes: None,
                        hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
                        review_state: ReviewState::Proposed,
                        requirement_links: vec![RequirementLink {
                            document_id: "doc-1".into(),
                            document_revision: "rev".into(),
                            position: TextRange::new(0, 1),
                            quote: QuoteSelector {
                                quote: "   ".into(),
                                prefix: String::new(),
                                suffix: String::new(),
                            },
                            resolution: ResolutionState::Resolved,
                        }],
                        scenario_refs: Vec::new(),
                    }],
                    ..Default::default()
                },
                true,
            ),
            (
                "valid minimal project",
                sample_index(),
                TestPointsFile {
                    test_points: vec![TestPoint {
                        id: "tp-valid".into(),
                        title: "T".into(),
                        objective: "O".into(),
                        preconditions: None,
                        expected_outcomes: None,
                        hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
                        review_state: ReviewState::Proposed,
                        requirement_links: vec![RequirementLink {
                            document_id: "doc-1".into(),
                            document_revision: "rev".into(),
                            position: TextRange::new(0, 1),
                            quote: QuoteSelector {
                                quote: "x".into(),
                                prefix: String::new(),
                                suffix: String::new(),
                            },
                            resolution: ResolutionState::Resolved,
                        }],
                        scenario_refs: Vec::new(),
                    }],
                    ..Default::default()
                },
                false,
            ),
        ];

        for (name, index, file, expect_error) in cases {
            let index_issues = validate_requirement_index(&index);
            let tp_issues = validate_test_points(&file, &index, Path::new("."));
            let has_error = index_issues
                .iter()
                .chain(&tp_issues)
                .any(|i| i.severity == AuthoringSeverity::Error);
            assert_eq!(
                has_error, expect_error,
                "case '{name}' expected error={expect_error}"
            );
        }
    }
}
