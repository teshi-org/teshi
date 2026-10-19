//! Load and save authoring artifacts from the user-level requirement store
//! and project-local test points.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use teshi_core::authoring::{
    legacy_project_requirements_dir, mark_document_links_missing, re_resolve_document_links,
    requirement_title, resolve_requirement_store_path, testpoints_file, validate_loaded_artifacts,
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

/// Recoverable tombstone file inside the requirement store root.
pub const REQUIREMENTS_TRASH_FILE: &str = "_teshi-trash.json";

const REQUIREMENT_TRASH_VERSION: u32 = 1;

/// Result of loading authoring artifacts from disk.
#[derive(Debug, Clone)]
pub struct AuthoringLoadResult {
    /// Loaded artifacts when any authoring directory or file exists.
    pub artifacts: Option<AuthoringArtifacts>,
    /// Whether any authoring paths were present on disk.
    pub discovered: bool,
}

/// Saved content kept after a document is removed from the active index.
///
/// This is intentionally store-local and private to the engine boundary. The
/// document id and body remain available for recovery, while active readers
/// only see documents still present in `_teshi.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RequirementTombstone {
    pub meta: teshi_core::authoring::RequirementDocumentMeta,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RequirementTrash {
    #[serde(default = "default_requirement_trash_version")]
    pub version: u32,
    #[serde(default)]
    pub documents: Vec<RequirementTombstone>,
}

impl Default for RequirementTrash {
    fn default() -> Self {
        Self {
            version: REQUIREMENT_TRASH_VERSION,
            documents: Vec::new(),
        }
    }
}

fn default_requirement_trash_version() -> u32 {
    REQUIREMENT_TRASH_VERSION
}

fn index_path(requirements_root: &Path) -> PathBuf {
    requirements_root.join(REQUIREMENTS_INDEX_FILE)
}

pub(crate) fn trash_path(requirements_root: &Path) -> PathBuf {
    requirements_root.join(REQUIREMENTS_TRASH_FILE)
}

pub(crate) fn read_requirement_trash_unlocked(
    requirements_root: &Path,
) -> Result<RequirementTrash> {
    let path = trash_path(requirements_root);
    if !path.is_file() {
        return Ok(RequirementTrash::default());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let trash: RequirementTrash =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    for tombstone in &trash.documents {
        validate_requirement_path(&tombstone.meta.path).with_context(|| {
            format!(
                "requirement tombstone '{}' has an unsafe path '{}'",
                tombstone.meta.id, tombstone.meta.path
            )
        })?;
    }
    Ok(trash)
}

pub(crate) fn write_requirement_trash_unlocked(
    requirements_root: &Path,
    trash: &RequirementTrash,
) -> Result<()> {
    for tombstone in &trash.documents {
        validate_requirement_path(&tombstone.meta.path).with_context(|| {
            format!(
                "requirement tombstone '{}' has an unsafe path '{}'",
                tombstone.meta.id, tombstone.meta.path
            )
        })?;
    }
    let path = trash_path(requirements_root);
    if trash.documents.is_empty() {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove {}", path.display())),
        }
        return Ok(());
    }
    write_json_unlocked(&path, trash).with_context(|| format!("write {}", path.display()))
}

/// Repairs an interrupted delete while the store lock is held.
///
/// A tombstone with an active index entry means the index replacement did not
/// commit. Restore the body and discard that tombstone. A tombstone without an
/// active entry is a committed deletion; remove any identical leftover body
/// but retain the tombstone so its id stays reserved.
pub(crate) fn recover_requirement_store_unlocked(requirements_root: &Path) -> Result<()> {
    let index = read_requirement_index_unlocked(requirements_root)?;
    let mut trash = read_requirement_trash_unlocked(requirements_root)?;
    if trash.documents.is_empty() {
        return Ok(());
    }
    let Some(index) = index else {
        return Ok(());
    };

    let mut retained = Vec::with_capacity(trash.documents.len());
    let mut changed = false;
    for tombstone in trash.documents.drain(..) {
        let active = index
            .documents
            .iter()
            .find(|doc| doc.id == tombstone.meta.id);
        let markdown_path =
            resolve_requirement_store_path(requirements_root, &tombstone.meta.path)?;
        if active.is_some() {
            if !markdown_path.is_file() {
                replace_markdown(&markdown_path, tombstone.body.as_bytes())?;
            }
            changed = true;
        } else {
            if markdown_path.is_file()
                && fs::read_to_string(&markdown_path)
                    .with_context(|| format!("read {}", markdown_path.display()))?
                    == tombstone.body
            {
                fs::remove_file(&markdown_path)
                    .with_context(|| format!("remove {}", markdown_path.display()))?;
            }
            retained.push(tombstone);
        }
    }
    trash.documents = retained;
    if changed {
        write_requirement_trash_unlocked(requirements_root, &trash)?;
    }
    Ok(())
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

pub(crate) fn read_requirement_index_unlocked(
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
    if index_file.is_file() && trash_path(requirements_root).is_file() {
        with_requirement_store_lock(requirements_root, || {
            recover_requirement_store_unlocked(requirements_root)
        })?;
    }
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

/// Returns whether a document id belongs to a committed deletion tombstone.
///
/// Active documents always win over a stale tombstone, which lets recovery
/// clear an interrupted operation before an allocator makes a decision.
pub fn is_requirement_document_id_reserved(
    requirements_root: &Path,
    document_id: &str,
) -> Result<bool> {
    if !requirements_root.exists() {
        return Ok(false);
    }
    with_requirement_store_lock(requirements_root, || {
        recover_requirement_store_unlocked(requirements_root)?;
        let active = read_requirement_index_unlocked(requirements_root)?
            .is_some_and(|index| index.documents.iter().any(|doc| doc.id == document_id));
        if active {
            return Ok(false);
        }
        let trash = read_requirement_trash_unlocked(requirements_root)?;
        Ok(trash
            .documents
            .iter()
            .any(|tombstone| tombstone.meta.id == document_id))
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

/// Writes requirement Markdown and synchronizes its first-line H1 title and revision.
///
/// Creates parent directories as needed. The caller must ensure `relative_path` is
/// indexed in `index` before calling. The on-disk index is reloaded under the
/// store lock; only this document (plus caller-only unsaved entries) is merged
/// so concurrent writers cannot drop each other's documents.
///
/// # Errors
///
/// Returns an error when the title or path is invalid, the document is not
/// indexed, or persistence fails. An index write failure restores the old body.
pub fn save_requirement_markdown(
    requirements_root: &Path,
    index: &mut RequirementDocumentIndex,
    relative_path: &str,
    body: &str,
) -> Result<()> {
    resolve_requirement_store_path(requirements_root, relative_path)
        .with_context(|| format!("unsafe requirement path '{relative_path}'"))?;
    let title = requirement_title(body)?;

    with_requirement_store_lock(requirements_root, || {
        // Work on a candidate so a failed save leaves the caller's metadata intact.
        let mut candidate = index.clone();
        let meta = candidate
            .documents
            .iter_mut()
            .find(|d| d.path == relative_path)
            .context("requirement document is not indexed")?;
        meta.title = title.to_string();
        meta.revision = compute_document_revision(body);
        let disk =
            read_requirement_index_unlocked(requirements_root)?.unwrap_or_else(|| index.clone());
        let merged = upsert_saved_document(disk, &candidate, relative_path)?;
        write_requirement_markdown_unlocked(requirements_root, &merged, relative_path, body)?;
        *index = merged;
        Ok(())
    })
}

/// Commits the body and index under the caller's store lock, restoring the body
/// if the index cannot be replaced. Each individual replacement is atomic.
///
/// # Errors
///
/// Returns an error for unsafe paths, failed writes, or failed body restoration.
pub(crate) fn write_requirement_markdown_unlocked(
    requirements_root: &Path,
    index: &RequirementDocumentIndex,
    relative_path: &str,
    body: &str,
) -> Result<()> {
    validate_index_document_paths(index)?;
    let markdown_path = resolve_requirement_store_path(requirements_root, relative_path)?;
    let previous = match fs::read(&markdown_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("read {}", markdown_path.display()));
        }
    };
    replace_markdown(&markdown_path, body.as_bytes())?;
    if let Err(error) = write_requirement_index_unlocked(requirements_root, index) {
        let rollback = match previous {
            Some(bytes) => replace_markdown(&markdown_path, &bytes),
            None => fs::remove_file(&markdown_path).context("remove unsaved requirement document"),
        };
        rollback
            .with_context(|| format!("{error:#}; could not restore previous requirement body"))?;
        return Err(error);
    }
    Ok(())
}

pub(crate) fn replace_markdown(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("requirement document has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let temp = parent.join(format!(".teshi-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        fs::write(&temp, bytes).with_context(|| format!("write {}", temp.display()))?;
        fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
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
    super::service::set_requirement_documents_iteration(
        requirements_root,
        &[document_id],
        iteration,
    )
    .map_err(anyhow::Error::from)
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
    let known_ids: HashSet<&str> = artifacts
        .index
        .documents
        .iter()
        .map(|doc| doc.id.as_str())
        .collect();
    let missing_ids: HashSet<String> = artifacts
        .test_points
        .test_points
        .iter()
        .flat_map(|tp| tp.requirement_links.iter())
        .filter(|link| !known_ids.contains(link.document_id.as_str()))
        .map(|link| link.document_id.clone())
        .collect();
    for document_id in missing_ids {
        mark_document_links_missing(&document_id, &mut artifacts.test_points.test_points);
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
    fn reload_keeps_deleted_links_missing_and_reopens_approved_points() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());

        let path = testpoints_file(project.path());
        let mut test_points: TestPointsFile =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        test_points.test_points[0].review_state = ReviewState::Approved;
        save_test_points(project.path(), &test_points).unwrap();

        let index = read_requirement_index_unlocked(store.path())
            .unwrap()
            .expect("initialized index");
        crate::authoring::delete_requirement_document(
            store.path(),
            "doc-1",
            Some(index.documents[0].revision.as_str()),
        )
        .unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.documents.is_empty());
        assert_eq!(artifacts.test_points.test_points.len(), 1);
        let point = &artifacts.test_points.test_points[0];
        assert_eq!(point.review_state, ReviewState::NeedsReview);
        assert_eq!(
            point.requirement_links[0].resolution,
            ResolutionState::Stale
        );
    }

    #[test]
    fn reload_marks_deleted_links_missing_in_each_project() {
        let store = tempfile::tempdir().unwrap();
        let project_a = tempfile::tempdir().unwrap();
        let project_b = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project_a.path());

        let points_path = testpoints_file(project_a.path());
        let points: TestPointsFile =
            serde_json::from_str(&fs::read_to_string(points_path).unwrap()).unwrap();
        save_test_points(project_b.path(), &points).unwrap();

        let index = read_requirement_index_unlocked(store.path())
            .unwrap()
            .expect("initialized index");
        crate::authoring::delete_requirement_document(
            store.path(),
            "doc-1",
            Some(index.documents[0].revision.as_str()),
        )
        .unwrap();

        for project in [project_a, project_b] {
            let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
            let artifacts = loaded.artifacts.expect("artifacts");
            let point = &artifacts.test_points.test_points[0];
            assert_eq!(point.requirement_links[0].document_id, "doc-1");
            assert_eq!(
                point.requirement_links[0].resolution,
                ResolutionState::Stale
            );
        }
    }

    #[test]
    fn reload_after_committed_delete_keeps_tombstone_and_reserves_id() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_sample_store(store.path(), project.path());
        let index = read_requirement_index_unlocked(store.path())
            .unwrap()
            .expect("initialized index");

        crate::authoring::delete_requirement_document(
            store.path(),
            "doc-1",
            Some(index.documents[0].revision.as_str()),
        )
        .unwrap();
        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        let artifacts = loaded.artifacts.expect("artifacts");
        assert!(artifacts.index.documents.is_empty());
        assert!(artifacts.documents.is_empty());
        assert!(is_requirement_document_id_reserved(store.path(), "doc-1").unwrap());
        assert!(trash_path(store.path()).is_file());
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
        save_requirement_markdown(store.path(), &mut caller, "a.md", "# A-updated").unwrap();

        let reloaded: RequirementDocumentIndex = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(reloaded.documents.len(), 2);
        assert!(reloaded.documents.iter().any(|d| d.id == "doc-b"));
        assert_eq!(
            fs::read_to_string(store.path().join("a.md")).unwrap(),
            "# A-updated"
        );
    }

    #[test]
    fn markdown_save_rolls_back_body_and_metadata_when_index_write_fails() {
        let store = tempfile::tempdir().unwrap();
        let mut index = initialize_requirement_store(store.path()).unwrap();
        index.documents.push(RequirementDocumentMeta::new(
            "doc-1",
            "doc-1.md",
            "Original",
            compute_document_revision("# Original\n"),
        ));
        save_requirement_markdown(store.path(), &mut index, "doc-1.md", "# Original\n").unwrap();
        let before = index.clone();
        let index_bytes = fs::read(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap();
        fs::create_dir(store.path().join("_teshi.tmp")).unwrap();

        save_requirement_markdown(store.path(), &mut index, "doc-1.md", "# Renamed\n")
            .expect_err("index replacement must fail");
        assert_eq!(index, before);
        assert_eq!(
            fs::read(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
            index_bytes
        );
        assert_eq!(
            fs::read_to_string(store.path().join("doc-1.md")).unwrap(),
            "# Original\n"
        );

        index.documents.push(RequirementDocumentMeta::new(
            "doc-2",
            "doc-2.md",
            "New",
            compute_document_revision("# New\n"),
        ));
        let before = index.clone();
        save_requirement_markdown(store.path(), &mut index, "doc-2.md", "# New\n")
            .expect_err("new document must not survive a failed index write");
        assert_eq!(index, before);
        assert!(!store.path().join("doc-2.md").exists());
        assert_eq!(
            fs::read(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
            index_bytes
        );
    }

    #[test]
    fn markdown_save_rejects_invalid_titles_before_mutating_body_or_index() {
        let store = tempfile::tempdir().unwrap();
        let mut index = initialize_requirement_store(store.path()).unwrap();
        index.documents.push(RequirementDocumentMeta::new(
            "doc-1",
            "doc-1.md",
            "Original",
            compute_document_revision("# Original\n"),
        ));
        save_requirement_markdown(store.path(), &mut index, "doc-1.md", "# Original\n").unwrap();
        let before = index.clone();
        let index_bytes = fs::read(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap();
        for body in ["#", "#   \nBody", "## Wrong level", "Body\n# Later"] {
            save_requirement_markdown(store.path(), &mut index, "doc-1.md", body).unwrap_err();
            assert_eq!(index, before);
            assert_eq!(
                fs::read_to_string(store.path().join("doc-1.md")).unwrap(),
                "# Original\n"
            );
            assert_eq!(
                fs::read(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
                index_bytes
            );
        }
    }
}
