//! Document-id oriented requirement-store facade shared by CLI and other hosts.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use teshi_core::authoring::{
    InvalidIterationName, InvalidRequirementTitle, RequirementDocumentContent,
    RequirementDocumentIndex, RequirementDocumentMeta, RequirementIterationFilter,
    RequirementStoreId, normalize_iteration_name, requirement_title,
    resolve_requirement_store_path,
};

use super::store::{
    RequirementTombstone, compute_document_revision as hash_document_revision,
    read_requirement_index_unlocked, read_requirement_trash_unlocked,
    recover_requirement_store_unlocked, replace_markdown, with_requirement_store_lock,
    write_requirement_index_unlocked, write_requirement_markdown_unlocked,
    write_requirement_trash_unlocked,
};

/// One candidate returned when a document reference is ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementRefMatch {
    /// Stable document identifier.
    pub id: String,
    /// Store-relative Markdown path.
    pub path: String,
    /// Display title.
    pub title: String,
}

impl From<&RequirementDocumentMeta> for RequirementRefMatch {
    fn from(meta: &RequirementDocumentMeta) -> Self {
        Self {
            id: meta.id.clone(),
            path: meta.path.clone(),
            title: meta.title.clone(),
        }
    }
}

/// Listed documents from the current requirement store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementDocumentList {
    /// Stable identity of the opened store.
    pub store_id: RequirementStoreId,
    /// Absolute path of the store root.
    pub store_path: PathBuf,
    /// Documents matching the requested filter, in index order.
    pub documents: Vec<RequirementDocumentMeta>,
}

/// Failures from the document-id requirement-store facade.
#[derive(Debug)]
pub enum RequirementStoreError {
    /// `_teshi.json` is missing.
    Uninitialized {
        /// Store root that was opened.
        path: PathBuf,
    },
    /// Index exists but has no valid `store_id`.
    MissingStoreId {
        /// Store root that was opened.
        path: PathBuf,
    },
    /// No document matched the caller-supplied reference.
    NotFound {
        /// Original query or document id.
        query: String,
    },
    /// More than one document matched a non-id reference.
    Ambiguous {
        /// Original query.
        query: String,
        /// Matching documents.
        matches: Vec<RequirementRefMatch>,
    },
    /// Disk revision no longer matches the expected token.
    RevisionConflict {
        /// Document that could not be updated.
        document_id: String,
        /// Revision supplied by the caller.
        expected: String,
        /// Revision currently stored in the index.
        actual: String,
    },
    /// Iteration name failed validation.
    InvalidIterationName(InvalidIterationName),
    /// Markdown lacks a non-empty first-line H1 title.
    InvalidTitle(InvalidRequirementTitle),
    /// Batch update was called with no document ids.
    EmptyDocumentIdList,
    /// I/O or path-safety failure while reading or writing the store.
    Io(anyhow::Error),
}

impl RequirementStoreError {
    /// Stable machine-readable failure code for CLI/JSON adapters.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Uninitialized { .. } | Self::MissingStoreId { .. } => {
                "requirement_store_uninitialized"
            }
            Self::NotFound { .. } => "requirement_not_found",
            Self::Ambiguous { .. } => "ambiguous_requirement_ref",
            Self::RevisionConflict { .. } => "revision_conflict",
            Self::InvalidIterationName(_) => "invalid_iteration_name",
            Self::InvalidTitle(_) => "invalid_requirement_title",
            Self::EmptyDocumentIdList => "empty_document_id_list",
            Self::Io(_) => "requirement_store_io",
        }
    }

    /// Exit status for CLI adapters: `2` for ambiguous refs, `1` otherwise.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Ambiguous { .. } => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for RequirementStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uninitialized { path } => write!(
                f,
                "requirement store is not initialized at {}",
                path.display()
            ),
            Self::MissingStoreId { path } => write!(
                f,
                "requirement store is missing a valid store_id at {}",
                path.display()
            ),
            Self::NotFound { query } => {
                write!(f, "requirement document '{query}' not found")
            }
            Self::Ambiguous { query, matches } => {
                writeln!(f, "Multiple requirements matched \"{query}\":")?;
                writeln!(f)?;
                for item in matches {
                    writeln!(f, "  {}  {}", item.id, item.path)?;
                }
                write!(f, "\nUse the document ID.")
            }
            Self::RevisionConflict {
                document_id,
                expected,
                actual,
            } => write!(
                f,
                "requirement document '{document_id}' changed on disk (expected revision {expected}, found {actual})"
            ),
            Self::InvalidIterationName(err) => write!(f, "{err}"),
            Self::InvalidTitle(err) => write!(f, "{err}"),
            Self::EmptyDocumentIdList => {
                f.write_str("at least one requirement document id is required")
            }
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for RequirementStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidIterationName(err) => Some(err),
            Self::InvalidTitle(err) => Some(err),
            Self::Io(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for RequirementStoreError {
    fn from(err: anyhow::Error) -> Self {
        Self::Io(err)
    }
}

fn io_err(context: impl Into<String>, err: impl Into<anyhow::Error>) -> RequirementStoreError {
    RequirementStoreError::Io(err.into().context(context.into()))
}

fn with_store_lock<T>(
    requirements_root: &Path,
    f: impl FnOnce() -> Result<T, RequirementStoreError>,
) -> Result<T, RequirementStoreError> {
    match with_requirement_store_lock(requirements_root, || f().map_err(anyhow::Error::from)) {
        Ok(value) => Ok(value),
        Err(err) => match err.downcast::<RequirementStoreError>() {
            Ok(typed) => Err(typed),
            Err(err) => Err(RequirementStoreError::Io(err)),
        },
    }
}

fn require_initialized_index(
    requirements_root: &Path,
) -> Result<RequirementDocumentIndex, RequirementStoreError> {
    match read_requirement_index_unlocked(requirements_root).map_err(RequirementStoreError::from)? {
        Some(index) if index.store_id.is_some() => Ok(index),
        Some(_) => Err(RequirementStoreError::MissingStoreId {
            path: requirements_root.to_path_buf(),
        }),
        None => Err(RequirementStoreError::Uninitialized {
            path: requirements_root.to_path_buf(),
        }),
    }
}

/// Resolves a caller-supplied reference to a unique `document_id`.
///
/// Matching order is exact `document_id`, then path with `\\` normalized to `/`,
/// then a unique exact title. The first step that yields a unique hit wins.
///
/// # Errors
///
/// Returns [`RequirementStoreError::NotFound`] when nothing matches and
/// [`RequirementStoreError::Ambiguous`] when a path or title step matches more
/// than one document.
pub fn resolve_requirement_ref(
    index: &RequirementDocumentIndex,
    query: &str,
) -> Result<String, RequirementStoreError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(RequirementStoreError::NotFound {
            query: query.to_string(),
        });
    }

    let id_matches: Vec<&RequirementDocumentMeta> = index
        .documents
        .iter()
        .filter(|doc| doc.id == query)
        .collect();
    if id_matches.len() == 1 {
        return Ok(id_matches[0].id.clone());
    }
    if id_matches.len() > 1 {
        return Err(ambiguous(query, &id_matches));
    }

    let normalized_path = query.replace('\\', "/");
    let path_matches: Vec<&RequirementDocumentMeta> = index
        .documents
        .iter()
        .filter(|doc| doc.path == normalized_path)
        .collect();
    if path_matches.len() == 1 {
        return Ok(path_matches[0].id.clone());
    }
    if path_matches.len() > 1 {
        return Err(ambiguous(query, &path_matches));
    }

    let title_matches: Vec<&RequirementDocumentMeta> = index
        .documents
        .iter()
        .filter(|doc| doc.title == query)
        .collect();
    if title_matches.len() == 1 {
        return Ok(title_matches[0].id.clone());
    }
    if title_matches.len() > 1 {
        return Err(ambiguous(query, &title_matches));
    }

    Err(RequirementStoreError::NotFound {
        query: query.to_string(),
    })
}

fn ambiguous(query: &str, matches: &[&RequirementDocumentMeta]) -> RequirementStoreError {
    RequirementStoreError::Ambiguous {
        query: query.to_string(),
        matches: matches
            .iter()
            .copied()
            .map(RequirementRefMatch::from)
            .collect(),
    }
}

/// Lists documents in the current store, optionally filtered by iteration.
///
/// # Errors
///
/// Returns [`RequirementStoreError::Uninitialized`] or
/// [`RequirementStoreError::MissingStoreId`] when the store cannot be opened.
pub fn list_requirement_documents(
    requirements_root: &Path,
    filter: &RequirementIterationFilter,
) -> Result<RequirementDocumentList, RequirementStoreError> {
    let index = require_initialized_index(requirements_root)?;
    let store_id =
        index
            .store_id()
            .cloned()
            .ok_or_else(|| RequirementStoreError::MissingStoreId {
                path: requirements_root.to_path_buf(),
            })?;
    let documents = index
        .documents
        .into_iter()
        .filter(|doc| doc.matches_iteration_filter(filter))
        .collect();
    Ok(RequirementDocumentList {
        store_id,
        store_path: requirements_root.to_path_buf(),
        documents,
    })
}

/// Reads one document by stable id from the current store.
///
/// # Errors
///
/// Returns a store or not-found error, or [`RequirementStoreError::Io`] when the
/// Markdown file cannot be read.
pub fn read_requirement_document(
    requirements_root: &Path,
    document_id: &str,
) -> Result<RequirementDocumentContent, RequirementStoreError> {
    let index = require_initialized_index(requirements_root)?;
    let meta = index
        .documents
        .iter()
        .find(|doc| doc.id == document_id)
        .cloned()
        .ok_or_else(|| RequirementStoreError::NotFound {
            query: document_id.to_string(),
        })?;
    let markdown_path = resolve_requirement_store_path(requirements_root, &meta.path)
        .map_err(|err| io_err(format!("unsafe requirement path '{}'", meta.path), err))?;
    let body = fs::read_to_string(&markdown_path)
        .with_context(|| format!("read {}", markdown_path.display()))
        .map_err(RequirementStoreError::from)?;
    Ok(RequirementDocumentContent { meta, body })
}

/// Removes one saved requirement from the active library and records a
/// recoverable store-local tombstone.
///
/// The body and metadata are captured before the source file is removed. The
/// index replacement is the commit point: if it fails, the body and tombstone
/// are restored so callers never observe a half-deleted active document.
///
/// When `expected_revision` is supplied, the current index must still contain
/// that body revision. Deleted ids remain reserved by the tombstone and are
/// never reused by the default document allocator.
pub fn delete_requirement_document(
    requirements_root: &Path,
    document_id: &str,
    expected_revision: Option<&str>,
) -> Result<RequirementDocumentMeta, RequirementStoreError> {
    with_store_lock(requirements_root, || {
        recover_requirement_store_unlocked(requirements_root)
            .map_err(RequirementStoreError::from)?;
        let index = require_initialized_index(requirements_root)?;
        let document_index = index
            .documents
            .iter()
            .position(|doc| doc.id == document_id)
            .ok_or_else(|| RequirementStoreError::NotFound {
                query: document_id.to_string(),
            })?;
        let meta = index.documents[document_index].clone();
        let actual_revision = meta.revision.as_str().to_string();
        if let Some(expected) = expected_revision {
            if expected != actual_revision {
                return Err(RequirementStoreError::RevisionConflict {
                    document_id: document_id.to_string(),
                    expected: expected.to_string(),
                    actual: actual_revision,
                });
            }
        }

        let markdown_path = resolve_requirement_store_path(requirements_root, &meta.path)
            .map_err(|err| io_err(format!("unsafe requirement path '{}'", meta.path), err))?;
        let body = fs::read_to_string(&markdown_path)
            .with_context(|| format!("read {}", markdown_path.display()))
            .map_err(RequirementStoreError::from)?;

        let mut trash = read_requirement_trash_unlocked(requirements_root)
            .map_err(RequirementStoreError::from)?;
        if trash
            .documents
            .iter()
            .any(|tombstone| tombstone.meta.id == meta.id)
        {
            return Err(RequirementStoreError::Io(anyhow::anyhow!(
                "requirement document '{}' already has a deletion tombstone",
                meta.id
            )));
        }
        trash.documents.push(RequirementTombstone {
            meta: meta.clone(),
            body: body.clone(),
        });
        write_requirement_trash_unlocked(requirements_root, &trash)
            .map_err(RequirementStoreError::from)?;

        if let Err(error) = fs::remove_file(&markdown_path) {
            return Err(io_err(format!("remove {}", markdown_path.display()), error));
        }

        let mut replacement = index.clone();
        replacement.documents.remove(document_index);
        if let Err(error) = write_requirement_index_unlocked(requirements_root, &replacement) {
            let restore = replace_markdown(&markdown_path, body.as_bytes());
            if let Err(restore_error) = restore {
                return Err(io_err(
                    format!(
                        "{error:#}; could not restore deleted requirement body: {restore_error:#}"
                    ),
                    error,
                ));
            }
            trash
                .documents
                .retain(|tombstone| tombstone.meta.id != meta.id);
            if let Err(cleanup_error) = write_requirement_trash_unlocked(requirements_root, &trash)
            {
                return Err(io_err(
                    format!(
                        "{error:#}; restored body but could not remove deletion tombstone: {cleanup_error:#}"
                    ),
                    error,
                ));
            }
            return Err(RequirementStoreError::from(error));
        }

        Ok(meta)
    })
}

/// Writes a document body by id and synchronizes its first-line H1 title and revision.
///
/// When `expected_revision` is set and `force` is false, the on-disk revision
/// must still match. Identical content with an already synchronized title is a
/// successful no-op.
///
/// # Errors
///
/// Returns a store, not-found, revision-conflict, invalid-title, or I/O error.
pub fn update_requirement_document_body(
    requirements_root: &Path,
    document_id: &str,
    body: &str,
    expected_revision: Option<&str>,
    force: bool,
) -> Result<RequirementDocumentMeta, RequirementStoreError> {
    let title = requirement_title(body).map_err(RequirementStoreError::InvalidTitle)?;
    with_store_lock(requirements_root, || {
        let mut index = require_initialized_index(requirements_root)?;
        let doc_index = index
            .documents
            .iter()
            .position(|doc| doc.id == document_id)
            .ok_or_else(|| RequirementStoreError::NotFound {
                query: document_id.to_string(),
            })?;
        let current_revision = index.documents[doc_index].revision.as_str().to_string();
        if let Some(expected) = expected_revision {
            if !force && expected != current_revision {
                return Err(RequirementStoreError::RevisionConflict {
                    document_id: document_id.to_string(),
                    expected: expected.to_string(),
                    actual: current_revision,
                });
            }
        }

        let relative_path = index.documents[doc_index].path.clone();
        let markdown_path = resolve_requirement_store_path(requirements_root, &relative_path)
            .map_err(|err| io_err(format!("unsafe requirement path '{relative_path}'"), err))?;
        let existing = if markdown_path.is_file() {
            fs::read_to_string(&markdown_path)
                .with_context(|| format!("read {}", markdown_path.display()))
                .map_err(RequirementStoreError::from)?
        } else {
            String::new()
        };
        if existing == body && index.documents[doc_index].title == title {
            return Ok(index.documents[doc_index].clone());
        }

        index.documents[doc_index].title = title.to_string();
        index.documents[doc_index].revision = hash_document_revision(body);
        write_requirement_markdown_unlocked(requirements_root, &index, &relative_path, body)
            .map_err(RequirementStoreError::from)?;
        Ok(index.documents[doc_index].clone())
    })
}

/// Atomically sets or clears iteration on every listed document.
///
/// All ids are validated before any mutation. Path, body, and revision are
/// left unchanged.
///
/// # Errors
///
/// Returns a store, not-found, empty-list, or invalid-iteration error. A missing
/// id fails the whole batch.
pub fn set_requirement_documents_iteration(
    requirements_root: &Path,
    document_ids: &[&str],
    iteration: Option<&str>,
) -> Result<RequirementDocumentIndex, RequirementStoreError> {
    if document_ids.is_empty() {
        return Err(RequirementStoreError::EmptyDocumentIdList);
    }
    let normalized = match iteration {
        None => None,
        Some(raw) => Some(
            normalize_iteration_name(raw).map_err(RequirementStoreError::InvalidIterationName)?,
        ),
    };

    with_store_lock(requirements_root, || {
        let mut index = require_initialized_index(requirements_root)?;
        let mut positions = Vec::with_capacity(document_ids.len());
        for document_id in document_ids {
            let position = index
                .documents
                .iter()
                .position(|doc| doc.id == *document_id)
                .ok_or_else(|| RequirementStoreError::NotFound {
                    query: (*document_id).to_string(),
                })?;
            positions.push(position);
        }
        for position in positions {
            index.documents[position].iteration = normalized.clone();
        }
        write_requirement_index_unlocked(requirements_root, &index)
            .map_err(RequirementStoreError::from)?;
        Ok(index)
    })
}

/// Resolves `query` against the on-disk index of `requirements_root`.
///
/// # Errors
///
/// Returns store or reference-resolution errors.
pub fn resolve_requirement_ref_in_store(
    requirements_root: &Path,
    query: &str,
) -> Result<String, RequirementStoreError> {
    let index = require_initialized_index(requirements_root)?;
    resolve_requirement_ref(&index, query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::{
        REQUIREMENTS_INDEX_FILE, REQUIREMENTS_TRASH_FILE, initialize_requirement_store,
        is_requirement_document_id_reserved, load_authoring_artifacts,
        save_requirement_document_index, save_requirement_markdown,
    };
    use teshi_core::authoring::RequirementDocumentMeta;

    fn seed_store(root: &Path) -> RequirementDocumentIndex {
        let mut index = initialize_requirement_store(root).unwrap();
        let login = "# 登录需求\n";
        let mobile = "# 登录需求\n\nmobile";
        let checkout = "# Checkout\n";
        index.documents = vec![
            RequirementDocumentMeta::new(
                "doc-12",
                "auth/login.md",
                "登录需求",
                hash_document_revision(login),
            ),
            RequirementDocumentMeta::new(
                "doc-37",
                "mobile/login.md",
                "登录需求",
                hash_document_revision(mobile),
            ),
            RequirementDocumentMeta::new(
                "doc-9",
                "shop/checkout.md",
                "Checkout",
                hash_document_revision(checkout),
            ),
        ];
        fs::create_dir_all(root.join("auth")).unwrap();
        fs::create_dir_all(root.join("mobile")).unwrap();
        fs::create_dir_all(root.join("shop")).unwrap();
        save_requirement_markdown(root, &mut index, "auth/login.md", login).unwrap();
        save_requirement_markdown(root, &mut index, "mobile/login.md", mobile).unwrap();
        save_requirement_markdown(root, &mut index, "shop/checkout.md", checkout).unwrap();
        index.documents[0].iteration = Some("Sprint 12".into());
        save_requirement_document_index(root, &index).unwrap();
        index
    }

    #[test]
    fn resolve_prefers_exact_document_id() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        assert_eq!(resolve_requirement_ref(&index, "doc-12").unwrap(), "doc-12");
    }

    #[test]
    fn resolve_normalizes_path_separators() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        assert_eq!(
            resolve_requirement_ref(&index, r"auth\login.md").unwrap(),
            "doc-12"
        );
        assert_eq!(
            resolve_requirement_ref(&index, "auth/login.md").unwrap(),
            "doc-12"
        );
    }

    #[test]
    fn resolve_unique_title() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        assert_eq!(
            resolve_requirement_ref(&index, "Checkout").unwrap(),
            "doc-9"
        );
    }

    #[test]
    fn resolve_ambiguous_title_does_not_guess() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let err = resolve_requirement_ref(&index, "登录需求").unwrap_err();
        match err {
            RequirementStoreError::Ambiguous { ref matches, .. } => {
                let ids: Vec<_> = matches.iter().map(|item| item.id.as_str()).collect();
                assert!(ids.contains(&"doc-12"));
                assert!(ids.contains(&"doc-37"));
            }
            other => panic!("expected ambiguous, got {other}"),
        }
        assert_eq!(err.exit_code(), 2);
        assert_eq!(err.code(), "ambiguous_requirement_ref");
    }

    #[test]
    fn resolve_unknown_ref_is_not_found() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let err = resolve_requirement_ref(&index, "missing").unwrap_err();
        assert!(matches!(err, RequirementStoreError::NotFound { .. }));
        assert_eq!(err.code(), "requirement_not_found");
    }

    #[test]
    fn list_filters_by_iteration_and_unassigned() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let named = list_requirement_documents(
            store.path(),
            &RequirementIterationFilter::Named("Sprint 12".into()),
        )
        .unwrap();
        assert_eq!(named.documents.len(), 1);
        assert_eq!(named.documents[0].id, "doc-12");

        let unassigned =
            list_requirement_documents(store.path(), &RequirementIterationFilter::Unassigned)
                .unwrap();
        let ids: Vec<_> = unassigned
            .documents
            .iter()
            .map(|doc| doc.id.as_str())
            .collect();
        assert_eq!(ids, ["doc-37", "doc-9"]);
    }

    #[test]
    fn list_uninitialized_store_fails_closed() {
        let store = tempfile::tempdir().unwrap();
        let err =
            list_requirement_documents(store.path(), &RequirementIterationFilter::All).unwrap_err();
        assert_eq!(err.code(), "requirement_store_uninitialized");
    }

    #[test]
    fn list_missing_store_id_fails_closed() {
        let store = tempfile::tempdir().unwrap();
        fs::write(
            store.path().join(REQUIREMENTS_INDEX_FILE),
            r#"{"version":2,"documents":[]}"#,
        )
        .unwrap();
        let err =
            list_requirement_documents(store.path(), &RequirementIterationFilter::All).unwrap_err();
        assert_eq!(err.code(), "requirement_store_uninitialized");
    }

    #[test]
    fn update_rejects_stale_revision_unless_forced() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let expected = index.documents[0].revision.as_str().to_string();
        update_requirement_document_body(
            store.path(),
            "doc-12",
            "# Login\n\nchanged\n",
            None,
            true,
        )
        .unwrap();
        let err = update_requirement_document_body(
            store.path(),
            "doc-12",
            "# Login\n\nstale\n",
            Some(&expected),
            false,
        )
        .unwrap_err();
        assert_eq!(err.code(), "revision_conflict");
        let body = read_requirement_document(store.path(), "doc-12").unwrap();
        assert_eq!(body.body, "# Login\n\nchanged\n");

        let forced = update_requirement_document_body(
            store.path(),
            "doc-12",
            "# Login\n\nforced\n",
            Some(&expected),
            true,
        )
        .unwrap();
        assert_eq!(
            forced.revision,
            hash_document_revision("# Login\n\nforced\n")
        );
        assert_eq!(
            read_requirement_document(store.path(), "doc-12")
                .unwrap()
                .body,
            "# Login\n\nforced\n"
        );
    }

    #[test]
    fn identical_body_does_not_rewrite_revision() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let before = index.documents[2].revision.clone();
        let updated = update_requirement_document_body(
            store.path(),
            "doc-9",
            "# Checkout\n",
            Some(before.as_str()),
            false,
        )
        .unwrap();
        assert_eq!(updated.revision, before);
    }

    #[test]
    fn body_update_syncs_title_and_preserves_identity_path_and_iteration() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let original = &index.documents[0];
        let body = "#    用户认证需求    \r\n\r\nBody\r\n# Later heading\r\n";
        let updated = update_requirement_document_body(
            store.path(),
            &original.id,
            body,
            Some(original.revision.as_str()),
            false,
        )
        .unwrap();
        assert_eq!(updated.title, "用户认证需求");
        assert_eq!(updated.id, original.id);
        assert_eq!(updated.path, original.path);
        assert_eq!(updated.iteration, original.iteration);
        assert_eq!(
            read_requirement_document(store.path(), &original.id)
                .unwrap()
                .body,
            body
        );
        assert_eq!(
            resolve_requirement_ref_in_store(store.path(), "用户认证需求").unwrap(),
            original.id
        );
    }

    #[test]
    fn body_update_rejects_invalid_title_and_preserves_saved_document() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let original = read_requirement_document(store.path(), "doc-12").unwrap();
        for body in ["#", "#    \n", "No H1\n# Later"] {
            let error = update_requirement_document_body(store.path(), "doc-12", body, None, true)
                .unwrap_err();
            assert_eq!(error.code(), "invalid_requirement_title");
            assert_eq!(
                read_requirement_document(store.path(), "doc-12").unwrap(),
                original
            );
        }
    }

    #[test]
    fn delete_removes_active_document_keeps_body_and_reserves_id() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let deleted = delete_requirement_document(
            store.path(),
            "doc-12",
            Some(index.documents[0].revision.as_str()),
        )
        .unwrap();

        assert_eq!(deleted.id, "doc-12");
        assert!(!store.path().join("auth/login.md").exists());
        assert!(matches!(
            read_requirement_document(store.path(), "doc-12"),
            Err(RequirementStoreError::NotFound { .. })
        ));
        assert!(is_requirement_document_id_reserved(store.path(), "doc-12").unwrap());
        let trash: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_TRASH_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(trash["documents"][0]["meta"]["id"], "doc-12");
        assert_eq!(trash["documents"][0]["body"], "# 登录需求\n");
    }

    #[test]
    fn delete_rejects_missing_and_stale_documents() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let stale = delete_requirement_document(store.path(), "doc-12", Some("stale")).unwrap_err();
        assert!(matches!(
            stale,
            RequirementStoreError::RevisionConflict { .. }
        ));
        assert!(store.path().join("auth/login.md").exists());

        let missing = delete_requirement_document(store.path(), "missing", None).unwrap_err();
        assert!(matches!(missing, RequirementStoreError::NotFound { .. }));
        assert_eq!(
            require_initialized_index(store.path())
                .unwrap()
                .documents
                .len(),
            index.documents.len()
        );
    }

    #[test]
    fn delete_rejects_revision_from_a_stale_store_instance_after_body_edit() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let stale_revision = index.documents[0].revision.clone();
        let updated = update_requirement_document_body(
            store.path(),
            "doc-12",
            "# 登录需求\n\nupdated\n",
            Some(stale_revision.as_str()),
            false,
        )
        .unwrap();

        let error =
            delete_requirement_document(store.path(), "doc-12", Some(stale_revision.as_str()))
                .unwrap_err();
        assert!(matches!(
            error,
            RequirementStoreError::RevisionConflict { .. }
        ));
        assert_eq!(
            read_requirement_document(store.path(), "doc-12")
                .unwrap()
                .meta
                .revision,
            updated.revision
        );
        assert!(store.path().join("auth/login.md").is_file());
    }

    #[test]
    fn delete_rolls_back_body_and_tombstone_when_index_replace_fails() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        fs::create_dir(store.path().join("_teshi.tmp")).unwrap();

        let error = delete_requirement_document(
            store.path(),
            "doc-12",
            Some(index.documents[0].revision.as_str()),
        )
        .unwrap_err();
        assert_eq!(error.code(), "requirement_store_io");
        assert_eq!(
            fs::read_to_string(store.path().join("auth/login.md")).unwrap(),
            "# 登录需求\n"
        );
        assert!(!store.path().join(REQUIREMENTS_TRASH_FILE).exists());
        assert!(!is_requirement_document_id_reserved(store.path(), "doc-12").unwrap());
    }

    #[test]
    fn delete_rolls_back_before_body_mutation_when_tombstone_write_fails() {
        let store = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        fs::create_dir(store.path().join("_teshi-trash.tmp")).unwrap();

        delete_requirement_document(
            store.path(),
            "doc-12",
            Some(index.documents[0].revision.as_str()),
        )
        .unwrap_err();
        assert!(store.path().join("auth/login.md").exists());
        assert!(
            require_initialized_index(store.path())
                .unwrap()
                .documents
                .iter()
                .any(|doc| doc.id == "doc-12")
        );
    }

    #[test]
    fn load_recovers_an_interrupted_delete_with_active_index() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let index = seed_store(store.path());
        let body = fs::read_to_string(store.path().join("auth/login.md")).unwrap();
        fs::remove_file(store.path().join("auth/login.md")).unwrap();
        let trash = serde_json::json!({
            "version": 1,
            "documents": [{"meta": index.documents[0], "body": body}]
        });
        fs::write(
            store.path().join(REQUIREMENTS_TRASH_FILE),
            serde_json::to_string_pretty(&trash).unwrap(),
        )
        .unwrap();

        let loaded = load_authoring_artifacts(project.path(), store.path()).unwrap();
        assert!(
            loaded
                .artifacts
                .unwrap()
                .documents
                .iter()
                .any(|doc| doc.meta.id == "doc-12")
        );
        assert_eq!(
            fs::read_to_string(store.path().join("auth/login.md")).unwrap(),
            "# 登录需求\n"
        );
        assert!(!store.path().join(REQUIREMENTS_TRASH_FILE).exists());
    }

    #[test]
    fn batch_iteration_updates_all_ids_under_one_write() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let updated = set_requirement_documents_iteration(
            store.path(),
            &["doc-12", "doc-9"],
            Some(" Sprint 13 "),
        )
        .unwrap();
        assert_eq!(
            updated
                .documents
                .iter()
                .find(|doc| doc.id == "doc-12")
                .unwrap()
                .iteration
                .as_deref(),
            Some("Sprint 13")
        );
        assert_eq!(
            updated
                .documents
                .iter()
                .find(|doc| doc.id == "doc-9")
                .unwrap()
                .iteration
                .as_deref(),
            Some("Sprint 13")
        );
        let checkout_rev = updated
            .documents
            .iter()
            .find(|doc| doc.id == "doc-9")
            .unwrap()
            .revision
            .clone();
        assert_eq!(checkout_rev, hash_document_revision("# Checkout\n"));
    }

    #[test]
    fn batch_iteration_rolls_back_when_any_id_is_missing() {
        let store = tempfile::tempdir().unwrap();
        let before = seed_store(store.path());
        let err = set_requirement_documents_iteration(
            store.path(),
            &["doc-12", "doc-missing"],
            Some("Sprint 13"),
        )
        .unwrap_err();
        assert_eq!(err.code(), "requirement_not_found");
        let after = require_initialized_index(store.path()).unwrap();
        assert_eq!(
            after
                .documents
                .iter()
                .find(|doc| doc.id == "doc-12")
                .unwrap()
                .iteration,
            before
                .documents
                .iter()
                .find(|doc| doc.id == "doc-12")
                .unwrap()
                .iteration
        );
    }
}
