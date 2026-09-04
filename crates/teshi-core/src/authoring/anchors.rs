//! Requirement anchor selectors and link records.
//!
//! Anchors combine Unicode character positions with quote/context selectors so
//! links can be re-resolved after document edits.

use serde::{Deserialize, Serialize};

use super::requirements::RequirementStoreId;

/// A Unicode scalar offset into requirement Markdown content.
///
/// Offsets count [`char`] boundaries (not UTF-8 bytes) so persisted ranges
/// match user-visible selection in the TUI and external editors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CharPosition(pub u32);

impl CharPosition {
    /// Creates a position after validating the offset is representable.
    pub fn new(offset: u32) -> Self {
        Self(offset)
    }

    /// Returns the underlying scalar offset.
    pub fn offset(self) -> u32 {
        self.0
    }
}

/// A half-open `[start, end)` range of Unicode scalar offsets in a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    /// Inclusive start offset.
    pub start: CharPosition,
    /// Exclusive end offset; must be strictly greater than `start` for non-empty ranges.
    pub end: CharPosition,
}

impl TextRange {
    /// Creates a range from validated offsets.
    pub fn new(start: u32, end: u32) -> Self {
        Self {
            start: CharPosition(start),
            end: CharPosition(end),
        }
    }

    /// Returns `true` when the range contains at least one character.
    pub fn is_non_empty(self) -> bool {
        self.start.0 < self.end.0
    }
}

/// Exact quoted text plus bounded prefix/suffix context for disambiguation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteSelector {
    /// Exact text covered by the anchor.
    pub quote: String,
    /// Characters immediately before `quote` (bounded length at persistence time).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    /// Characters immediately after `quote` (bounded length at persistence time).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suffix: String,
}

/// Whether a requirement link currently resolves to a unique range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    /// The link resolves to exactly one range in the current document revision.
    #[default]
    Resolved,
    /// The link cannot be uniquely resolved; human review is required.
    Stale,
}

/// A trace link from a test point to a requirement text range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementLink {
    /// Store identity of the linked requirement document.
    ///
    /// Absent on unmigrated project-local links. Resolvers must not guess a
    /// matching document in the current store when this field is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_id: Option<RequirementStoreId>,
    /// Stable requirement document identifier from `_teshi.json`.
    pub document_id: String,
    /// Content revision observed when the anchor was created or last resolved.
    pub document_revision: String,
    /// Character range at the time of the last successful resolution.
    pub position: TextRange,
    /// Quote selector used to re-anchor after edits.
    pub quote: QuoteSelector,
    /// Current resolution outcome for this link.
    #[serde(default)]
    pub resolution: ResolutionState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_range_serde_roundtrip() {
        let range = TextRange::new(4, 12);
        let json = serde_json::to_string(&range).expect("serialize");
        let back: TextRange = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, range);
        assert!(back.is_non_empty());
    }

    #[test]
    fn requirement_link_serde_roundtrip() {
        let link = RequirementLink {
            store_id: Some(RequirementStoreId::parse("reqstore-1").unwrap()),
            document_id: "doc-auth".into(),
            document_revision: "rev-1".into(),
            position: TextRange::new(10, 25),
            quote: QuoteSelector {
                quote: "user logs in".into(),
                prefix: "When ".into(),
                suffix: " successfully".into(),
            },
            resolution: ResolutionState::Resolved,
        };
        let json = serde_json::to_string(&link).expect("serialize");
        let back: RequirementLink = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, link);
    }

    #[test]
    fn legacy_link_without_store_id_deserializes() {
        let json = r#"{
            "document_id": "doc-1",
            "document_revision": "rev",
            "position": {"start": 0, "end": 4},
            "quote": {"quote": "User"}
        }"#;
        let link: RequirementLink = serde_json::from_str(json).expect("deserialize");
        assert!(link.store_id.is_none());
        assert_eq!(link.document_id, "doc-1");
    }
}
