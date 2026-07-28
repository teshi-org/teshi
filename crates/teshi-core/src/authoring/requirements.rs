//! Requirement document index DTOs for `requirements/_teshi.json`.

use serde::{Deserialize, Serialize};

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

/// Metadata for one indexed requirement Markdown document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementDocumentMeta {
    /// Stable document identifier used by anchors and generation sources.
    pub id: String,
    /// Path relative to the project requirement root (e.g. `auth/login.md`).
    pub path: String,
    /// Display title shown in the Requirements tree.
    pub title: String,
    /// Last observed content revision for anchor invalidation.
    pub revision: DocumentRevision,
}

/// Top-level index persisted at `requirements/_teshi.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementDocumentIndex {
    /// Schema version for forward-compatible migrations.
    #[serde(default = "default_index_version")]
    pub version: u32,
    /// Indexed requirement documents keyed by stable identity.
    #[serde(default)]
    pub documents: Vec<RequirementDocumentMeta>,
}

fn default_index_version() -> u32 {
    1
}

impl Default for RequirementDocumentIndex {
    fn default() -> Self {
        Self {
            version: default_index_version(),
            documents: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_serde_roundtrip() {
        let index = RequirementDocumentIndex {
            version: 1,
            documents: vec![RequirementDocumentMeta {
                id: "doc-1".into(),
                path: "auth/login.md".into(),
                title: "Login".into(),
                revision: DocumentRevision::new("abc123"),
            }],
        };
        let json = serde_json::to_string_pretty(&index).expect("serialize");
        let back: RequirementDocumentIndex = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, index);
    }
}
