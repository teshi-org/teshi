//! Requirement-document and test-point authoring DTOs (pure data).
//!
//! Persistence, discovery, and project integration live in `teshi-engine`.
//! Anchor resolution and validation helpers live in sibling modules here.

pub mod anchors;
pub mod positions;
pub mod requirements;
pub mod resolve;
pub mod testpoints;
pub mod validation;

pub use anchors::{CharPosition, QuoteSelector, RequirementLink, ResolutionState, TextRange};
pub use positions::{
    byte_offset_to_char_position, char_position_to_byte_offset, char_position_to_line_col,
    document_char_len, line_col_range_to_char_range, line_col_to_char_position,
    slice_by_char_range,
};
pub use requirements::{DocumentRevision, RequirementDocumentIndex, RequirementDocumentMeta};
pub use resolve::{
    ANCHOR_CONTEXT_CHARS, create_requirement_link, re_resolve_document_links,
    resolve_requirement_link,
};
pub use testpoints::{HierarchyPath, ReviewState, ScenarioRef, TestPoint, TestPointsFile};
pub use validation::{
    AuthoringArtifacts, AuthoringDiagnostic, AuthoringSeverity, RequirementDocumentContent,
    requirements_root, testpoints_file, validate_hierarchy_path, validate_loaded_artifacts,
    validate_quote_selector, validate_requirement_index, validate_test_points, validate_text_range,
};
