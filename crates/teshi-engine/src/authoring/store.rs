//! Load and save authoring artifacts from the user-level requirement store
//! and project-local test points.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use teshi_core::authoring::{
    legacy_project_requirements_dir, normalize_iteration_name, re_resolve_document_links,
    resolve_requirement_store_path, testpoints_file, validate_loaded_artifacts,
    validate_requirement_path, AuthoringArtifacts, AuthoringDiagnostic, AuthoringSeverity,
    DocumentRevision, RequirementDocumentContent, RequirementDocumentIndex, TestPointsFile,
};

use crate::fs_util::{with_exclusive_lock, write_atomic, write_json_unlocked};

/// Default requirement directory name (also used for legacy project folders).
pub const DEFAULT_REQUIREMENTS_DIR: &str = "requirements";

/// Default test points directory name under the project root.
pub const DEFAULT_TESTPOINTS_DIR: &str = "testpoints";

/// Requirement index filename inside the requirement store root.
pub const REQUIREMENTS_INDEX_FILE: &str = "_teshi.json";

/// Result of loading authoring artifacts from disk.
#[derive(Debug, Clone)]
pub struct AuthoringLoadResult {
    /// Loaded artifacts when any authoring directory or file exists.
    pub artifacts: Option<AuthoringArtifacts>,
    /// Whether any authoring paths were present on disk.
    pub discovered: bool,
}

fn index_path(requirements_root: &Path) -> PathBuf {
    requirements_root.join(REQUIREMENTS_INDEX_FILE)
}

/// Runs `f` while holding the exclusive lock for `_teshi.json`.
///
/// Nested store mutations that also take this lock deadlock; use unlocked
/// index helpers from inside `f`.
pub(crate) fn with_requirement_store_lock<T>(
    requirements_root: &Path,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_exclusive_lock(&index_path(requirements_root), f)
}

fn validate_index_document_paths(index: &RequirementDocumentIndex) -> Result<()> {
    for doc in &index.documents {
        validate_requirement_path(&doc.path).with_context(|| {
            format!(
                "requirement document '{}' has an unsafe path '{}'",
                doc.id, doc.path
            )
        })?;
    }
    Ok(())
}

fn read_requirement_index_unlocked(
    requirements_root: &Path,
) -> Result<Option<RequirementDocumentIndex>> {
    let path = index_path(requirements_root);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let index = serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(index))
}

/// Writes `_teshi.json` without taking the store lock.
///
/// # Errors
///
/// Returns an error when any indexed path is unsafe or the write fails.
pub(crate) fn write_requirement_index_unlocked(
    requirements_root: &Path,
    index: &RequirementDocumentIndex,
) -> Result<()> {
    validate_index_document_paths(index)?;
    let path = index_path(requirements_root);
    write_json_unlocked(&path, index).with_context(|| format!("write {}", path.display()))
}

fn merge_requirement_indexes(
    disk: Option<RequirementDocumentIndex>,
    incoming: &RequirementDocumentIndex,
) -> Result<RequirementDocumentIndex> {
    let Some(mut merged) = disk else {
        validate_index_document_paths(incoming)?;
        return Ok(incoming.clone());
    };
    match (&merged.store_id, &incoming.store_id) {
        (Some(disk_id), Some(incoming_id)) if disk_id != incoming_id => {
            bail!(
                "requirement store identity mismatch: on-disk {disk_id} vs incoming {incoming_id}"
            );
        }
        (None, Some(incoming_id)) => merged.store_id = Some(incoming_id.clone()),
        _ => {}
    }
    if incoming.version > merged.version {
        merged.version = incoming.version;
    }
    for doc in &incoming.documents {
        validate_requirement_path(&doc.path).with_context(|| {
            format!(
                "requirement document '{}' has an unsafe path '{}'",
                doc.id, doc.path
            )
        })?;
        if let Some(slot) = merged
            .documents
            .iter_mut()
            .find(|existing| existing.id == doc.id)
        {
            *slot = doc.clone();
        } else {
            merged.documents.push(doc.clone());
        }
    }
    validate_index_document_paths(&merged)?;
    Ok(merged)
}

fn upsert_saved_document(
    disk: RequirementDocumentIndex,
    caller: &RequirementDocumentIndex,
    saved_path: &str,
) -> Result<RequirementDocumentIndex> {
    let mut merged = disk;
    if merged.store_id.is_none() {
        merged.store_id = caller.store_id.clone();
    }
    if caller.version > merged.version {
        merged.version = caller.version;
    }
    let disk_ids: HashSet<String> = merged.documents.iter().map(|d| d.id.clone()).collect();
    if let Some(meta) = caller.documents.iter().find(|d| d.path == saved_path) {
        validate_requirement_path(&meta.path).with_context(|| {
            format!(
                "requirement document '{}' has an unsafe path '{}'",
                meta.id, meta.path
            )
        })?;
        if let Some(slot) = merged
            .documents
            .iter_mut()
            .find(|existing| existing.id == meta.id)
        {
            *slot = meta.clone();
        } else {
            merged.documents.push(meta.clone());
        }
    }
    for doc in &caller.documents {
        if doc.path == saved_path || disk_ids.contains(&doc.id) {
            continue;
        }
        if merged
            .documents
            .iter()
            .any(|existing| existing.id == doc.id)
        {
            continue;
        }
        validate_requirement_path(&doc.path).with_context(|| {
            format!(
                "requirement document '{}' has an unsafe path '{}'",
                doc.id, doc.path
            )
        })?;
        merged.documents.push(doc.clone());
    }
    validate_index_document_paths(&merged)?;
    Ok(merged)
}

/// Computes a stable revision token from Markdown body content.
pub fn compute_document_revision(body: &str) -> DocumentRevision {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    DocumentRevision::new(format!("{:016x}", hasher.finish()))
}

fn directory_is_effectively_empty(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if !path.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let entry = entry.with_context(|| format!("read {}", path.display()))?;
        if is_requirement_store_lock_artifact(&entry.file_name().to_string_lossy()) {
            continue;
        }
        return Ok(false);
    }
    Ok(true)
}

/// Lock/temp siblings of `_teshi.json` created while holding the store lock.
pub(crate) fn is_requirement_store_lock_artifact(name: &str) -> bool {
    name.eq_ignore_ascii_case("_teshi.lock") || name.eq_ignore_ascii_case("_teshi.tmp")
}

fn migration_diagnostic(project_root: &Path) -> Option<AuthoringDiagnostic> {
    let legacy = legacy_project_requirements_dir(project_root);
    if !legacy.exists() {
        return None;
    }
    Some(AuthoringDiagnostic {
        severity: AuthoringSeverity::Warning,
        location: "requirements/".into(),
        record_id: None,
        message: "This project still has a local requirements/ directory. It is not loaded at runtime. Run `teshi requirements import-project` to copy documents into the user-level requirement library.".into(),
    })
}

fn global_store_identity_error(location: &str, message: impl Into<String>) -> AuthoringDiagnostic {
    AuthoringDiagnostic {
        severity: AuthoringSeverity::Error,
        location: location.into(),
        record_id: None,
        message: message.into(),
    }
}

/// Initializes an empty requirement store with a unique `store_id`.
///
/// Creates `requirements_root` and atomically writes a v2 `_teshi.json`.
/// Non-empty directories without a valid store identity are rejected.
///
/// # Errors
///
/// Returns an error when the directory cannot be created, is not empty, already
/// has an index, or the atomic write fails.
pub fn initialize_requirement_store(requirements_root: &Path) -> Result<RequirementDocumentIndex> {
    if requirements_root.exists() && !directory_is_effectively_empty(requirements_root)? {
        bail!(
            "cannot initialize requirement store at {}: directory is not empty; initialize or import explicitly",
            requirements_root.display()
        );
    }
    with_requirement_store_lock(requirements_root, || {
        initialize_requirement_store_unlocked(requirements_root)
    })
}

/// Initializes a store without taking the index lock.
///
/// Callers that already hold [`with_requirement_store_lock`] must use this
/// instead of [`initialize_requirement_store`].
pub(crate) fn initialize_requirement_store_unlocked(
    requirements_root: &Path,
) -> Result<RequirementDocumentIndex> {
    if requirements_root.exists() && !directory_is_effectively_empty(requirements_root)? {
        bail!(
            "cannot initialize requirement store at {}: directory is not empty; initialize or import explicitly",
            requirements_root.display()
        );
    }
    fs::create_dir_all(requirements_root)
        .with_context(|| format!("create {}", requirements_root.display()))?;
    let index_file = index_path(requirements_root);
    if index_file.is_file() {
        bail!(
            "requirement store already exists at {}",
            index_file.display()
        );
    }
    let index = RequirementDocumentIndex::initialize_empty_store();
    write_requirement_index_unlocked(requirements_root, &index)?;
    Ok(index)
}

/// Loads authoring artifacts from the global requirement store and project test points.
///
/// Requirement Markdown is read only from `requirements_root`. Project-local
/// `requirements/` is never loaded; when present it produces a migration diagnostic.
///
/// # Errors
///
/// Returns an error when an authoring file exists but cannot be read or parsed.
pub fn load_authoring_artifacts(
    project_root: &Path,
    requirements_root: &Path,
) -> Result<AuthoringLoadResult> {
    let index_file = index_path(requirements_root);
    let testpoints_path = testpoints_file(project_root);
    let has_index = index_file.is_file();
    let has_testpoints = testpoints_path.is_file();
    let store_dir_exists = requirements_root.is_dir();
    let legacy_dir_exists = legacy_project_requirements_dir(project_root).exists();

    let mut diagnostics = Vec::new();
    if let Some(diag) = migration_diagnostic(project_root) {
        diagnostics.push(diag);
    }

    if !has_index && !has_testpoints && !legacy_dir_exists {
        let empty_or_missing_store =
            !store_dir_exists || directory_is_effectively_empty(requirements_root)?;
        if empty_or_missing_store {
            return Ok(AuthoringLoadResult {
                artifacts: None,
                discovered: false,
            });
        }
    }

    let index = if has_index {
        let text = fs::read_to_string(&index_file)
            .with_context(|| format!("read {}", index_file.display()))?;
        let parsed: RequirementDocumentIndex = serde_json::from_str(&text)
            .with_context(|| format!("parse {}", index_file.display()))?;
        match parsed.store_id() {
            Some(_) => parsed,
            None => {
                diagnostics.push(global_store_identity_error(
                    REQUIREMENTS_INDEX_FILE,
                    "requirement store index is missing a valid store_id; initialize or import instead of generating an identity from the path",
                ));
                parsed
            }
        }
    } else if store_dir_exists && !directory_is_effectively_empty(requirements_root)? {
        diagnostics.push(global_store_identity_error(
            REQUIREMENTS_INDEX_FILE,
            "requirement store directory is not empty and has no _teshi.json; initialize or import instead of generating an identity from the path",
        ));
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
    if index.store_id.is_some() {
        for meta in &index.documents {
            let markdown_path = match resolve_requirement_store_path(requirements_root, &meta.path)
            {
                Ok(path) => path,
                Err(err) => {
                    diagnostics.push(AuthoringDiagnostic {
                        severity: AuthoringSeverity::Error,
                        location: REQUIREMENTS_INDEX_FILE.into(),
                        record_id: Some(meta.id.clone()),
                        message: err.to_string(),
                    });
                    continue;
                }
            };
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
    }

    let artifacts = AuthoringArtifacts {
        index,
        documents,
        test_points,
        diagnostics,
    };

    let validation_issues = validate_loaded_artifacts(&artifacts, project_root, requirements_root);
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
/// Reloads the on-disk index under the store lock and merges incoming documents
/// by id so a stale in-memory snapshot cannot drop documents written by another
/// process. Indexed paths are validated before any write.
///
/// # Errors
///
/// Returns an error when a path is unsafe, store identities disagree, or the
/// atomic write fails.
pub fn save_requirement_document_index(
    requirements_root: &Path,
    index: &RequirementDocumentIndex,
) -> Result<()> {
    with_requirement_store_lock(requirements_root, || {
        let disk = read_requirement_index_unlocked(requirements_root)?;
        let merged = merge_requirement_indexes(disk, index)?;
        write_requirement_index_unlocked(requirements_root, &merged)
    })
}

/// Writes requirement Markdown and updates the matching index entry revision.
///
/// Creates parent directories as needed. The caller must ensure `relative_path` is
/// indexed in `index` before calling. The on-disk index is reloaded under the
/// store lock; only this document (plus caller-only unsaved entries) is merged
/// so concurrent writers cannot drop each other's documents.
///
/// # Errors
///
/// Returns an error when the path is unsafe, the file cannot be written, or the
/// index save fails.
pub fn save_requirement_markdown(
    requirements_root: &Path,
    index: &mut RequirementDocumentIndex,
    relative_path: &str,
    body: &str,
) -> Result<()> {
    let markdown_path = resolve_requirement_store_path(requirements_root, relative_path)
        .with_context(|| format!("unsafe requirement path '{relative_path}'"))?;

    with_requirement_store_lock(requirements_root, || {
        fs::create_dir_all(requirements_root).context("create requirements directory")?;
        if let Some(parent) = markdown_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&markdown_path, body)
            .with_context(|| format!("write {}", markdown_path.display()))?;

        let revision = compute_document_revision(body);
        if let Some(meta) = index.documents.iter_mut().find(|d| d.path == relative_path) {
            meta.revision = revision;
        }

        let disk =
            read_requirement_index_unlocked(requirements_root)?.unwrap_or_else(|| index.clone());
        let merged = upsert_saved_document(disk, index, relative_path)?;
        write_requirement_index_unlocked(requirements_root, &merged)?;
        *index = merged;
        Ok(())
    })
}

/// Atomically updates a document's iteration without changing IDs, path, body, or revision.
///
/// Reloads the on-disk index under the store lock before writing so concurrent
/// mutations from other processes are preserved.
///
/// # Errors
///
/// Returns an error when the store is not initialized, the document is missing,
/// the iteration name is invalid, or the index cannot be written.
pub fn set_requirement_document_iteration(
    requirements_root: &Path,
    document_id: &str,
    iteration: Option<&str>,
) -> Result<RequirementDocumentIndex> {
    with_requirement_store_lock(requirements_root, || {
        let mut index = read_requirement_index_unlocked(requirements_root)?.with_context(|| {
            format!(
                "requirement store is not initialized at {}",
                requirements_root.display()
            )
        })?;
        if index.store_id.is_none() {
            bail!("requirement store is missing a valid store_id");
        }
        let normalized = match iteration {
            None => None,
            Some(raw) => {
                Some(normalize_iteration_name(raw).map_err(|err| anyhow::anyhow!("{err}"))?)
            }
        };
        let meta = index
            .documents
            .iter_mut()
            .find(|d| d.id == document_id)
            .with_context(|| format!("requirement document '{document_id}' not found"))?;
        meta.iteration = normalized;
        write_requirement_index_unlocked(requirements_root, &index)?;
        Ok(index)
    })
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
    let store_id = artifacts.index.store_id.clone();
    for doc in &artifacts.documents {
        re_resolve_document_links(
            &doc.body,
            store_id.as_ref(),
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

    fn write_sample_store(store_root: &Path, project_root: &Path) {
        fs::create_dir_all(store_root).unwrap();
        fs::write(store_root.join("auth.md"), "User can log in").unwrap();

        let mut index = RequirementDocumentIndex::initialize_empty_store();
        index.documents = vec![RequirementDocumentMeta::new(
            "doc-1",
            "auth.md",
            "Auth",
            compute_document_revision("User can log in"),
        )];
        save_requirement_document_index(store_root, &index).unwrap();

        let store_id = index.store_id.clone();
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
                    store_id,
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
        save_test_points(project_root, &test_points).unwrap();
    }

    #[test]
    fn load_roundtrip_valid_project() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        assert!(loaded.discovered);
        let artifacts = loaded.artifacts.expect("artifacts");
        assert_eq!(artifacts.index.documents.len(), 1);
        assert_eq!(artifacts.documents.len(), 1);
        assert_eq!(artifacts.test_points.test_points.len(), 1);
        assert!(artifacts.index.store_id.is_some());
        assert!(!artifacts.has_errors());
    }

    #[test]
    fn load_reports_duplicate_test_point_ids() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());

        let dup_path = testpoints_file(project.path());
        let mut file: TestPointsFile =
            serde_json::from_str(&fs::read_to_string(&dup_path).unwrap()).unwrap();
        file.test_points.push(file.test_points[0].clone());
        fs::write(&dup_path, serde_json::to_string_pretty(&file).unwrap()).unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("duplicate test point")));
    }

    #[test]
    fn load_reports_missing_indexed_markdown() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());
        fs::remove_file(store.path().join("auth.md")).unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("missing")));
    }

    #[test]
    fn atomic_write_preserves_previous_file_on_failure() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());

        let path = testpoints_file(project.path());
        let original = fs::read_to_string(&path).unwrap();

        let mut file: TestPointsFile = serde_json::from_str(&original).unwrap();
        file.test_points[0].title = "Updated".into();
        save_test_points(project.path(), &file).unwrap();

        let reloaded = fs::read_to_string(&path).unwrap();
        let parsed: TestPointsFile = serde_json::from_str(&reloaded).unwrap();
        assert_eq!(parsed.test_points[0].title, "Updated");
    }

    #[test]
    fn reload_marks_stale_links_after_external_edit() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());

        fs::write(store.path().join("auth.md"), "changed body").unwrap();
        let mut index: RequirementDocumentIndex = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        index.documents[0].revision = compute_document_revision("changed body");
        save_requirement_document_index(store.path(), &index).unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
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
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        assert!(!loaded.discovered);
        assert!(loaded.artifacts.is_none());
    }

    #[test]
    fn moved_store_keeps_the_same_store_id() {
        let original = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(original.path(), project.path());
        let first = load_authoring_artifacts(project.path(), original.path())
            .unwrap()
            .artifacts
            .unwrap();
        let store_id = first.index.store_id.clone().expect("store id");

        let moved = tempfile::tempdir().unwrap();
        for entry in fs::read_dir(original.path()).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), moved.path().join(entry.file_name())).unwrap();
        }

        let second = load_authoring_artifacts(project.path(), moved.path())
            .unwrap()
            .artifacts
            .unwrap();
        assert_eq!(second.index.store_id.as_ref(), Some(&store_id));
        assert_eq!(
            second.test_points.test_points[0].requirement_links[0]
                .store_id
                .as_ref(),
            Some(&store_id)
        );
        assert!(!second.has_errors());
    }

    #[test]
    fn nonempty_uninitialized_store_fails_closed() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fs::write(store.path().join("notes.md"), "# leftover").unwrap();
        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not empty")));
        assert!(artifacts.index.store_id.is_none());
        assert!(artifacts.documents.is_empty());
    }

    #[test]
    fn index_missing_store_id_fails_closed() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fs::write(
            store.path().join(REQUIREMENTS_INDEX_FILE),
            r#"{"version":2,"documents":[]}"#,
        )
        .unwrap();
        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("store_id")));
    }

    #[test]
    fn wrong_store_id_on_links_does_not_match_current_documents() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());
        let other = teshi_core::authoring::RequirementStoreId::parse("reqstore-other").unwrap();
        let path = testpoints_file(project.path());
        let mut file: TestPointsFile =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        file.test_points[0].requirement_links[0].store_id = Some(other);
        save_test_points(project.path(), &file).unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("does not match")));
    }

    #[test]
    fn legacy_project_requirements_are_not_loaded() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let legacy = project.path().join("requirements");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("old.md"), "legacy only").unwrap();
        fs::write(
            legacy.join(REQUIREMENTS_INDEX_FILE),
            r#"{"version":1,"documents":[{"id":"doc-old","path":"old.md","title":"Old","revision":"r"}]}"#,
        )
        .unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.documents.is_empty());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| d.message.contains("import-project")));
    }

    #[test]
    fn initialize_empty_store_writes_stable_identity() {
        let store = tempfile::tempdir().unwrap();
        let index = initialize_requirement_store(store.path()).unwrap();
        let store_id = index.store_id.clone().expect("store id");
        let reloaded = load_authoring_artifacts(store.path(), store.path())
            .unwrap()
            .artifacts
            .unwrap();
        assert_eq!(reloaded.index.store_id.as_ref(), Some(&store_id));
    }

    #[test]
    fn set_iteration_does_not_change_revision() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());
        let before = load_authoring_artifacts(project.path(), store.path())
            .unwrap()
            .artifacts
            .unwrap();
        let revision = before.index.documents[0].revision.clone();
        let updated =
            set_requirement_document_iteration(store.path(), "doc-1", Some(" Sprint 1 ")).unwrap();
        assert_eq!(updated.documents[0].iteration.as_deref(), Some("Sprint 1"));
        assert_eq!(updated.documents[0].revision, revision);
        assert_eq!(updated.store_id, before.index.store_id);
    }

    #[test]
    fn load_does_not_read_paths_that_escape_the_store() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let secret = store
            .path()
            .parent()
            .expect("tempdir parent")
            .join("secret.txt");
        fs::write(&secret, "TOP SECRET").unwrap();
        let mut index = RequirementDocumentIndex::initialize_empty_store();
        index.documents = vec![RequirementDocumentMeta::new(
            "doc-1",
            "../secret.txt",
            "Secret",
            compute_document_revision("TOP SECRET"),
        )];
        fs::write(
            store.path().join(REQUIREMENTS_INDEX_FILE),
            serde_json::to_string_pretty(&index).unwrap(),
        )
        .unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts
            .documents
            .iter()
            .all(|d| !d.body.contains("TOP SECRET")));
        assert!(artifacts.has_errors());
        assert!(artifacts
            .diagnostics
            .iter()
            .any(|d| { d.message.contains("..") || d.message.contains("must not contain") }));
        let _ = fs::remove_file(&secret);
    }

    #[test]
    fn save_markdown_rejects_paths_that_escape_the_store() {
        let store = tempfile::tempdir().unwrap();
        let mut index = initialize_requirement_store(store.path()).unwrap();
        index.documents.push(RequirementDocumentMeta::new(
            "doc-1",
            "../escape.md",
            "Escape",
            compute_document_revision("x"),
        ));
        let err = save_requirement_markdown(store.path(), &mut index, "../escape.md", "leaked")
            .unwrap_err();
        assert!(err.to_string().contains("unsafe") || err.to_string().contains(".."));
        assert!(!store.path().parent().unwrap().join("escape.md").is_file());
    }

    #[test]
    fn stale_index_snapshot_does_not_drop_disk_documents() {
        let store = tempfile::tempdir().unwrap();
        let mut index = initialize_requirement_store(store.path()).unwrap();
        index.documents.push(RequirementDocumentMeta::new(
            "doc-a",
            "a.md",
            "A",
            compute_document_revision("A"),
        ));
        fs::write(store.path().join("a.md"), "A").unwrap();
        save_requirement_document_index(store.path(), &index).unwrap();

        let mut stale = index.clone();
        stale.documents.clear();
        stale.documents.push(RequirementDocumentMeta::new(
            "doc-b",
            "b.md",
            "B",
            compute_document_revision("B"),
        ));
        save_requirement_document_index(store.path(), &stale).unwrap();

        let reloaded: RequirementDocumentIndex = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        let ids: Vec<_> = reloaded.documents.iter().map(|d| d.id.as_str()).collect();
        assert!(
            ids.contains(&"doc-a"),
            "concurrent document was dropped: {ids:?}"
        );
        assert!(ids.contains(&"doc-b"));
    }

    #[test]
    fn markdown_save_preserves_documents_missing_from_caller_snapshot() {
        let store = tempfile::tempdir().unwrap();
        let mut index = initialize_requirement_store(store.path()).unwrap();
        index.documents = vec![
            RequirementDocumentMeta::new("doc-a", "a.md", "A", compute_document_revision("A")),
            RequirementDocumentMeta::new("doc-b", "b.md", "B", compute_document_revision("B")),
        ];
        fs::write(store.path().join("a.md"), "A").unwrap();
        fs::write(store.path().join("b.md"), "B").unwrap();
        save_requirement_document_index(store.path(), &index).unwrap();

        let mut caller = index.clone();
        caller.documents.retain(|d| d.id == "doc-a");
        save_requirement_markdown(store.path(), &mut caller, "a.md", "A-updated").unwrap();

        let reloaded: RequirementDocumentIndex = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(reloaded.documents.len(), 2);
        assert!(reloaded.documents.iter().any(|d| d.id == "doc-b"));
        assert_eq!(
            fs::read_to_string(store.path().join("a.md")).unwrap(),
            "A-updated"
        );
    }
}
