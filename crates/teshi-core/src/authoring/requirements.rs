//! Requirement document index DTOs for a user-level requirement store.
//!
//! Global stores persist `_teshi.json` beside Markdown files. Document identity
//! is the pair `(store_id, document_id)` so moving the store directory does not
//! break project test-point links.

use std::collections::BTreeSet;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Schema version written for newly initialized global requirement stores.
pub const REQUIREMENT_INDEX_VERSION: u32 = 2;

/// Filename of the requirement store index written at the store root.
pub const REQUIREMENT_INDEX_FILE: &str = "_teshi.json";

/// Stable identity for a user-level requirement store.
///
/// Generated once when the store is first initialized and never changed when
/// the directory is moved or opened through a different local path. Combined
/// with a document ID, this is the durable identity of a requirement document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequirementStoreId(String);

impl RequirementStoreId {
    /// Prefix used when generating new store identities.
    pub const PREFIX: &'static str = "reqstore-";

    /// Generates a new unique store identity.
    pub fn generate() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pid = std::process::id();
        Self(format!("{}{nanos:016x}{pid:08x}", Self::PREFIX))
    }

    /// Parses a persisted store identity.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRequirementStoreId`] when the value is empty after trim
    /// or contains ASCII control characters.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, InvalidRequirementStoreId> {
        let trimmed = value.as_ref().trim();
        if trimmed.is_empty() {
            return Err(InvalidRequirementStoreId::Empty);
        }
        if trimmed.chars().any(|ch| ch.is_control()) {
            return Err(InvalidRequirementStoreId::ControlCharacters);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Returns the identity as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequirementStoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a persisted store identity could not be accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidRequirementStoreId {
    /// Empty or whitespace-only value.
    Empty,
    /// Value contained ASCII control characters.
    ControlCharacters,
}

impl fmt::Display for InvalidRequirementStoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("requirement store id must not be empty"),
            Self::ControlCharacters => {
                f.write_str("requirement store id must not contain control characters")
            }
        }
    }
}

impl std::error::Error for InvalidRequirementStoreId {}

/// Filter applied to requirement documents by iteration metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RequirementIterationFilter {
    /// Include every document in the current store.
    #[default]
    All,
    /// Include only documents whose iteration equals this name.
    Named(String),
    /// Include only documents with no iteration assignment.
    Unassigned,
}

/// How the Requirements tree groups documents after filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RequirementGroupMode {
    /// Preserve the Markdown relative-path hierarchy.
    #[default]
    Path,
    /// Group by iteration, then preserve path hierarchy inside each iteration.
    Iteration,
}

/// Opaque content revision for a requirement Markdown document.
///
/// Typically a content hash or monotonic revision token updated whenever the
/// Markdown body changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DocumentRevision(pub String);

impl DocumentRevision {
    /// Creates a revision token from persisted storage.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the revision string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why an iteration name was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidIterationName {
    /// Empty or whitespace-only after trim.
    Empty,
    /// Contained ASCII control characters.
    ControlCharacters,
}

impl fmt::Display for InvalidIterationName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("iteration name must not be empty"),
            Self::ControlCharacters => {
                f.write_str("iteration name must not contain control characters")
            }
        }
    }
}

impl std::error::Error for InvalidIterationName {}

/// Trims and validates a user-supplied iteration name.
///
/// # Errors
///
/// Returns [`InvalidIterationName`] when the trimmed value is empty or contains
/// ASCII control characters. Other Unicode text is preserved as-is and is
/// case-sensitive.
pub fn normalize_iteration_name(raw: &str) -> Result<String, InvalidIterationName> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(InvalidIterationName::Empty);
    }
    if trimmed.chars().any(|ch| ch.is_control()) {
        return Err(InvalidIterationName::ControlCharacters);
    }
    Ok(trimmed.to_string())
}

/// Error raised when a requirement Markdown path is not a safe store-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidRequirementPath {
    /// The path is empty or whitespace-only.
    Empty,
    /// The path contains ASCII or Unicode control characters.
    ControlCharacters,
    /// The path is absolute, rooted, or includes a Windows drive prefix.
    NotRelative,
    /// The path contains `.` or `..` components that can escape the store.
    ParentOrCurrentDir,
    /// The path targets the reserved store index file.
    ReservedIndexName,
}

impl fmt::Display for InvalidRequirementPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "requirement path must not be empty"),
            Self::ControlCharacters => {
                write!(f, "requirement path must not contain control characters")
            }
            Self::NotRelative => {
                write!(
                    f,
                    "requirement path must be relative to the requirement store"
                )
            }
            Self::ParentOrCurrentDir => {
                write!(
                    f,
                    "requirement path must not contain '.' or '..' components"
                )
            }
            Self::ReservedIndexName => {
                write!(f, "requirement path must not be '{REQUIREMENT_INDEX_FILE}'")
            }
        }
    }
}

impl std::error::Error for InvalidRequirementPath {}

/// Returns `true` when `path` is a store-relative Markdown path that cannot escape the store root.
pub fn is_safe_requirement_path(path: &str) -> bool {
    validate_requirement_path(path).is_ok()
}

/// Validates that `path` is a normalized relative path without root, prefix, or parent components.
///
/// Indexed paths must stay inside the requirement store: no absolute paths, no drive prefixes,
/// no `.` or `..` components, no empty or control-character values, and not the reserved
/// `_teshi.json` index file name.
pub fn validate_requirement_path(path: &str) -> Result<(), InvalidRequirementPath> {
    if path.is_empty() || path.trim().is_empty() {
        return Err(InvalidRequirementPath::Empty);
    }
    if path.chars().any(|ch| ch.is_control()) {
        return Err(InvalidRequirementPath::ControlCharacters);
    }
    let parsed = Path::new(path);
    if parsed.is_absolute() || parsed.has_root() {
        return Err(InvalidRequirementPath::NotRelative);
    }

    let mut saw_normal = false;
    for component in parsed.components() {
        match component {
            Component::Normal(_) => saw_normal = true,
            Component::CurDir | Component::ParentDir => {
                return Err(InvalidRequirementPath::ParentOrCurrentDir);
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(InvalidRequirementPath::NotRelative);
            }
        }
    }
    if !saw_normal {
        return Err(InvalidRequirementPath::Empty);
    }
    if parsed
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(REQUIREMENT_INDEX_FILE))
    {
        return Err(InvalidRequirementPath::ReservedIndexName);
    }
    Ok(())
}

/// Joins a validated relative path onto `root` and rejects joins that leave the store.
///
/// # Errors
///
/// Returns [`InvalidRequirementPath`] when `relative` is unsafe or the joined path would
/// escape `root`.
pub fn resolve_requirement_store_path(
    root: &Path,
    relative: &str,
) -> Result<PathBuf, InvalidRequirementPath> {
    validate_requirement_path(relative)?;
    let joined = root.join(relative);
    let normalized_root = normalize_path_lexically(root);
    let normalized_joined = normalize_path_lexically(&joined);
    if !path_is_inside(&normalized_joined, &normalized_root) {
        return Err(InvalidRequirementPath::NotRelative);
    }
    Ok(joined)
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn path_is_inside(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

/// English label used in the UI for documents with no iteration assignment.
pub const UNASSIGNED_ITERATION_LABEL: &str = "Unassigned";

/// Metadata for one indexed requirement Markdown document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementDocumentMeta {
    /// Stable document identifier used by anchors and generation sources.
    pub id: String,
    /// Path relative to the requirement store root (e.g. `auth/login.md`).
    pub path: String,
    /// Display title shown in the Requirements tree.
    pub title: String,
    /// Last observed content revision for anchor invalidation.
    pub revision: DocumentRevision,
    /// Optional user-defined iteration name. `None` means Unassigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<String>,
}

impl RequirementDocumentMeta {
    /// Creates metadata with no iteration assignment.
    pub fn new(
        id: impl Into<String>,
        path: impl Into<String>,
        title: impl Into<String>,
        revision: DocumentRevision,
    ) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            title: title.into(),
            revision,
            iteration: None,
        }
    }

    /// Returns `true` when this document has no iteration assignment.
    pub fn is_unassigned(&self) -> bool {
        self.iteration.is_none()
    }

    /// Returns `true` when this document belongs to `filter`.
    pub fn matches_iteration_filter(&self, filter: &RequirementIterationFilter) -> bool {
        match filter {
            RequirementIterationFilter::All => true,
            RequirementIterationFilter::Named(name) => self.iteration.as_deref() == Some(name),
            RequirementIterationFilter::Unassigned => self.iteration.is_none(),
        }
    }
}

/// Top-level index persisted at `<requirements_root>/_teshi.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementDocumentIndex {
    /// Schema version for forward-compatible migrations.
    #[serde(default = "default_index_version")]
    pub version: u32,
    /// Stable store identity required for initialized global stores.
    ///
    /// Absent on legacy v1 project indexes until they are imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_id: Option<RequirementStoreId>,
    /// Indexed requirement documents keyed by stable identity.
    #[serde(default)]
    pub documents: Vec<RequirementDocumentMeta>,
}

fn default_index_version() -> u32 {
    REQUIREMENT_INDEX_VERSION
}

impl Default for RequirementDocumentIndex {
    fn default() -> Self {
        Self {
            version: REQUIREMENT_INDEX_VERSION,
            store_id: None,
            documents: Vec::new(),
        }
    }
}

impl RequirementDocumentIndex {
    /// Creates an empty v2 index with a newly generated store identity.
    pub fn initialize_empty_store() -> Self {
        Self {
            version: REQUIREMENT_INDEX_VERSION,
            store_id: Some(RequirementStoreId::generate()),
            documents: Vec::new(),
        }
    }

    /// Returns the store identity when present.
    pub fn store_id(&self) -> Option<&RequirementStoreId> {
        self.store_id.as_ref()
    }

    /// Sorted unique iteration names discovered from document metadata.
    ///
    /// Names are returned as stored (case-sensitive). Unassigned documents do
    /// not contribute a name.
    pub fn discovered_iteration_names(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for doc in &self.documents {
            if let Some(name) = doc.iteration.as_ref() {
                names.insert(name.clone());
            }
        }
        names.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_serde_roundtrip() {
        let index = RequirementDocumentIndex {
            version: 2,
            store_id: Some(RequirementStoreId::parse("reqstore-abc").unwrap()),
            documents: vec![RequirementDocumentMeta::new(
                "doc-1",
                "auth/login.md",
                "Login",
                DocumentRevision::new("abc123"),
            )],
        };
        let json = serde_json::to_string_pretty(&index).expect("serialize");
        let back: RequirementDocumentIndex = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, index);
    }

    #[test]
    fn v1_index_deserializes_without_store_id() {
        let json = r#"{
            "version": 1,
            "documents": [{
                "id": "doc-1",
                "path": "auth.md",
                "title": "Auth",
                "revision": "rev"
            }]
        }"#;
        let index: RequirementDocumentIndex = serde_json::from_str(json).expect("deserialize");
        assert_eq!(index.version, 1);
        assert!(index.store_id.is_none());
        assert_eq!(index.documents[0].id, "doc-1");
        assert!(index.documents[0].iteration.is_none());
    }

    #[test]
    fn missing_iteration_field_is_unassigned() {
        let json = r#"{
            "id": "doc-1",
            "path": "a.md",
            "title": "A",
            "revision": "r"
        }"#;
        let meta: RequirementDocumentMeta = serde_json::from_str(json).expect("deserialize");
        assert!(meta.is_unassigned());
    }

    #[test]
    fn iteration_name_rejects_blank_and_control_chars() {
        assert_eq!(
            normalize_iteration_name("  ").unwrap_err(),
            InvalidIterationName::Empty
        );
        assert_eq!(
            normalize_iteration_name("sprint\u{0007}1").unwrap_err(),
            InvalidIterationName::ControlCharacters
        );
        assert_eq!(normalize_iteration_name(" Sprint 1 ").unwrap(), "Sprint 1");
        assert_ne!(
            normalize_iteration_name("Sprint 1").unwrap(),
            normalize_iteration_name("sprint 1").unwrap()
        );
    }

    #[test]
    fn store_id_parse_rejects_empty_and_controls() {
        assert!(RequirementStoreId::parse("").is_err());
        assert!(RequirementStoreId::parse("  ").is_err());
        assert!(RequirementStoreId::parse("id\nvalue").is_err());
        assert_eq!(
            RequirementStoreId::parse(" reqstore-ok ").unwrap().as_str(),
            "reqstore-ok"
        );
    }

    #[test]
    fn requirement_path_rejects_escape_and_reserved_names() {
        assert_eq!(
            validate_requirement_path("").unwrap_err(),
            InvalidRequirementPath::Empty
        );
        assert_eq!(
            validate_requirement_path("../secret.txt").unwrap_err(),
            InvalidRequirementPath::ParentOrCurrentDir
        );
        assert_eq!(
            validate_requirement_path("./doc.md").unwrap_err(),
            InvalidRequirementPath::ParentOrCurrentDir
        );
        assert_eq!(
            validate_requirement_path("/tmp/doc.md").unwrap_err(),
            InvalidRequirementPath::NotRelative
        );
        assert_eq!(
            validate_requirement_path(REQUIREMENT_INDEX_FILE).unwrap_err(),
            InvalidRequirementPath::ReservedIndexName
        );
        assert!(validate_requirement_path("doc.md").is_ok());
        assert!(validate_requirement_path("nested/doc.md").is_ok());
        let root = PathBuf::from("/store");
        assert!(resolve_requirement_store_path(&root, "doc.md").is_ok());
        assert!(resolve_requirement_store_path(&root, "../secret.txt").is_err());
    }

    #[test]
    fn discovered_iteration_names_are_sorted_and_unique() {
        let index = RequirementDocumentIndex {
            store_id: Some(RequirementStoreId::generate()),
            documents: vec![
                RequirementDocumentMeta {
                    iteration: Some("Beta".into()),
                    ..RequirementDocumentMeta::new("a", "a.md", "A", DocumentRevision::new("1"))
                },
                RequirementDocumentMeta {
                    iteration: None,
                    ..RequirementDocumentMeta::new("b", "b.md", "B", DocumentRevision::new("1"))
                },
                RequirementDocumentMeta {
                    iteration: Some("Alpha".into()),
                    ..RequirementDocumentMeta::new("c", "c.md", "C", DocumentRevision::new("1"))
                },
                RequirementDocumentMeta {
                    iteration: Some("Alpha".into()),
                    ..RequirementDocumentMeta::new("d", "d.md", "D", DocumentRevision::new("1"))
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            index.discovered_iteration_names(),
            vec!["Alpha".to_string(), "Beta".to_string()]
        );
    }

    #[test]
    fn omitted_iteration_is_skipped_on_serialize() {
        let meta = RequirementDocumentMeta::new("d", "d.md", "D", DocumentRevision::new("r"));
        let json = serde_json::to_string(&meta).expect("serialize");
        assert!(!json.contains("iteration"));
    }
}
