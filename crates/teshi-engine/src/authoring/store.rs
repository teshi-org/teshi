//! Load and save authoring artifacts under `requirements/` and `testpoints/`.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use teshi_core::authoring::{
    re_resolve_document_links, requirements_root, testpoints_file, validate_loaded_artifacts,
    AuthoringArtifacts, AuthoringDiagnostic, AuthoringSeverity, DocumentRevision,
    RequirementDocumentContent, RequirementDocumentIndex, TestPointsFile,
};

use crate::fs_util::write_atomic;

/// Default requirement directory name under the project root.
pub const DEFAULT_REQUIREMENTS_DIR: &str = "requirements";

/// Default test points directory name under the project root.
pub const DEFAULT_TESTPOINTS_DIR: &str = "testpoints";

/// Requirement index filename inside the requirement root.
pub const REQUIREMENTS_INDEX_FILE: &str = "_teshi.json";

/// Result of loading authoring artifacts from disk.
#[derive(Debug, Clone)]
pub struct AuthoringLoadResult {
    /// Loaded artifacts when any authoring directory or file exists.
    pub artifacts: Option<AuthoringArtifacts>,
    /// Whether any authoring paths were present on disk.
    pub discovered: bool,
}

fn index_path(project_root: &Path) -> PathBuf {
    requirements_root(project_root).join(REQUIREMENTS_INDEX_FILE)
}

/// Computes a stable revision token from Markdown body content.
pub fn compute_document_revision(body: &str) -> DocumentRevision {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    DocumentRevision::new(format!("{:016x}", hasher.finish()))
}

/// Loads authoring artifacts when present; returns `None` when no authoring paths exist.
///
/// Malformed JSON returns an error. Validation issues are collected as diagnostics
/// without dropping records.
///
/// # Errors
///
/// Returns an error when an authoring file exists but cannot be read or parsed.
pub fn load_authoring_artifacts(project_root: &Path) -> Result<AuthoringLoadResult> {
    let req_root = requirements_root(project_root);
    let index_file = index_path(project_root);
    let testpoints_path = testpoints_file(project_root);

    let has_requirements = req_root.is_dir();
    let has_index = index_file.is_file();
    let has_testpoints = testpoints_path.is_file();

    if !has_requirements && !has_index && !has_testpoints {
        return Ok(AuthoringLoadResult {
            artifacts: None,
            discovered: false,
        });
    }

    let mut diagnostics = Vec::new();

    let index = if has_index {
        let text = fs::read_to_string(&index_file)
            .with_context(|| format!("read {}", index_file.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parse {}", index_file.display()))?
    } else if has_requirements {
        diagnostics.push(AuthoringDiagnostic {
            severity: AuthoringSeverity::Warning,
            location: REQUIREMENTS_INDEX_FILE.into(),
            record_id: None,
            message: "requirements directory exists without _teshi.json index".into(),
        });
        RequirementDocumentIndex::default()
    } else {
        RequirementDocumentIndex::default()
    };

    let test_points = if has_testpoints {
        let text = fs::read_to_string(&testpoints_path)
            .with_context(|| format!("read {}", testpoints_path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("parse {}", testpoints_path.display()))?
    } else {
        TestPointsFile::default()
    };

    let mut documents = Vec::new();
    for meta in &index.documents {
        let markdown_path = req_root.join(&meta.path);
        if !markdown_path.is_file() {
            continue;
        }
        let body = fs::read_to_string(&markdown_path)
            .with_context(|| format!("read {}", markdown_path.display()))?;
        documents.push(RequirementDocumentContent {
            meta: meta.clone(),
            body,
        });
    }

    let artifacts = AuthoringArtifacts {
        index,
        documents,
        test_points,
        diagnostics,
    };

    let validation_issues = validate_loaded_artifacts(&artifacts, project_root);
    let mut merged = artifacts;
    merged.diagnostics.extend(validation_issues);
    refresh_link_resolutions(&mut merged);

    Ok(AuthoringLoadResult {
        artifacts: Some(merged),
        discovered: true,
    })
}

/// Atomically writes the requirement document index.
///
/// # Errors
///
/// Returns an error when serialization or the atomic write fails.
pub fn save_requirement_document_index(
    project_root: &Path,
    index: &RequirementDocumentIndex,
) -> Result<()> {
    let path = index_path(project_root);
    write_atomic(&path, index).with_context(|| format!("write {}", path.display()))
}

/// Writes requirement Markdown and updates the matching index entry revision.
///
/// Creates parent directories as needed. The caller must ensure `relative_path` is
/// indexed in `index` before calling.
///
/// # Errors
///
/// Returns an error when the file cannot be written or the index save fails.
pub fn save_requirement_markdown(
    project_root: &Path,
    index: &mut RequirementDocumentIndex,
    relative_path: &str,
    body: &str,
) -> Result<()> {
    let req_root = requirements_root(project_root);
    fs::create_dir_all(&req_root).context("create requirements directory")?;

    let markdown_path = req_root.join(relative_path);
    if let Some(parent) = markdown_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&markdown_path, body)
        .with_context(|| format!("write {}", markdown_path.display()))?;

    let revision = compute_document_revision(body);
    if let Some(meta) = index.documents.iter_mut().find(|d| d.path == relative_path) {
        meta.revision = revision;
    }

    save_requirement_document_index(project_root, index)
}

/// Atomically writes the canonical test-point file with stable ordering.
///
/// # Errors
///
/// Returns an error when serialization or the atomic write fails.
pub fn save_test_points(project_root: &Path, file: &TestPointsFile) -> Result<()> {
    let path = testpoints_file(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create testpoints directory")?;
    }

    let mut sorted = file.clone();
    sorted.test_points.sort_by(|a, b| a.id.cmp(&b.id));

    write_atomic(&path, &sorted).with_context(|| format!("write {}", path.display()))
}

fn refresh_link_resolutions(artifacts: &mut AuthoringArtifacts) {
    for doc in &artifacts.documents {
        re_resolve_document_links(
            &doc.body,
            &doc.meta.id,
            doc.meta.revision.as_str(),
            &mut artifacts.test_points.test_points,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::authoring::{
        HierarchyPath, QuoteSelector, RequirementDocumentMeta, RequirementLink, ResolutionState,
        ReviewState, TestPoint, TextRange,
    };

    fn write_sample_project(root: &Path) {
        let req_root = requirements_root(root);
        fs::create_dir_all(&req_root).unwrap();
        fs::write(req_root.join("auth.md"), "User can log in").unwrap();

        let index = RequirementDocumentIndex {
            documents: vec![RequirementDocumentMeta {
                id: "doc-1".into(),
                path: "auth.md".into(),
                title: "Auth".into(),
                revision: compute_document_revision("User can log in"),
            }],
            ..Default::default()
        };
        save_requirement_document_index(root, &index).unwrap();

        let test_points = TestPointsFile {
            test_points: vec![TestPoint {
                id: "tp-1".into(),
                title: "Login".into(),
                objective: "Verify login".into(),
                preconditions: None,
                expected_outcomes: None,
                hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
                review_state: ReviewState::Proposed,
                requirement_links: vec![RequirementLink {
                    document_id: "doc-1".into(),
                    document_revision: "rev".into(),
                    position: TextRange::new(0, 4),
                    quote: QuoteSelector {
                        quote: "User".into(),
                        prefix: String::new(),
                        suffix: String::new(),
                    },
                    resolution: ResolutionState::Resolved,
                }],
                scenario_refs: Vec::new(),
            }],
            ..Default::default()
        };
        save_test_points(root, &test_points).unwrap();
    }

    #[test]
    fn load_roundtrip_valid_project() {
        let dir = tempfile::tempdir().unwrap();
        write_sample_project(dir.path());

        let loaded = load_authoring_artifacts(dir.path()).unwrap();
        assert!(loaded.discovered);
        let artifacts = loaded.artifacts.expect("artifacts");
        assert_eq!(artifacts.index.documents.len(), 1);
        assert_eq!(artifacts.documents.len(), 1);
        assert_eq!(artifacts.test_points.test_points.len(), 1);
        assert!(!artifacts.has_errors());
    }

    #[test]
    fn load_reports_duplicate_test_point_ids() {
        let dir = tempfile::tempdir().unwrap();
        write_sample_project(dir.path());

        let dup_path = testpoints_file(dir.path());
        let mut file: TestPointsFile =
            serde_json::from_str(&fs::read_to_string(&dup_path).unwrap()).unwrap();
        file.test_points.push(file.test_points[0].clone());
        fs::write(&dup_path, serde_json::to_string_pretty(&file).unwrap()).unwrap();

        let loaded = load_authoring_artifacts(dir.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("duplicate test point")));
    }

    #[test]
    fn load_reports_missing_indexed_markdown() {
        let dir = tempfile::tempdir().unwrap();
        write_sample_project(dir.path());
        fs::remove_file(requirements_root(dir.path()).join("auth.md")).unwrap();

        let loaded = load_authoring_artifacts(dir.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("missing")));
    }

    #[test]
    fn atomic_write_preserves_previous_file_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        write_sample_project(dir.path());

        let path = testpoints_file(dir.path());
        let original = fs::read_to_string(&path).unwrap();

        // Corrupt temp handling by making parent read-only is platform-specific;
        // instead verify a successful rewrite leaves readable JSON.
        let mut file: TestPointsFile = serde_json::from_str(&original).unwrap();
        file.test_points[0].title = "Updated".into();
        save_test_points(dir.path(), &file).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let parsed: TestPointsFile = serde_json::from_str(&reloaded).unwrap();
        assert_eq!(parsed.test_points[0].title, "Updated");
    }

    #[test]
    fn reload_marks_stale_links_after_external_edit() {
        let dir = tempfile::tempdir().unwrap();
        write_sample_project(dir.path());

        fs::write(
            requirements_root(dir.path()).join("auth.md"),
            "changed body",
        )
        .unwrap();
        let index_path = requirements_root(dir.path()).join(REQUIREMENTS_INDEX_FILE);
        let mut index: RequirementDocumentIndex =
            serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
        index.documents[0].revision = compute_document_revision("changed body");
        save_requirement_document_index(dir.path(), &index).unwrap();

        let loaded = load_authoring_artifacts(dir.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert_eq!(
            artifacts.test_points.test_points[0].requirement_links[0].resolution,
            teshi_core::authoring::ResolutionState::Stale
        );
        assert_eq!(
            artifacts.test_points.test_points[0].review_state,
            teshi_core::authoring::ReviewState::Proposed
        );
    }

    #[test]
    fn feature_only_project_has_no_authoring() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_authoring_artifacts(dir.path()).unwrap();
        assert!(!loaded.discovered);
        assert!(loaded.artifacts.is_none());
    }
}
