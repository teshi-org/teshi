//! Requirement-document and test-point authoring DTOs (pure data).
//!
//! Persistence, discovery, and project integration live in `teshi-engine`.
//! Anchor resolution and validation helpers live in sibling modules here.

pub mod anchors;
pub mod requirements;
pub mod testpoints;
pub mod validation;

pub use anchors::{CharPosition, QuoteSelector, RequirementLink, ResolutionState, TextRange};
pub use requirements::{DocumentRevision, RequirementDocumentIndex, RequirementDocumentMeta};
pub use testpoints::{HierarchyPath, ReviewState, ScenarioRef, TestPoint, TestPointsFile};
pub use validation::{
    AuthoringArtifacts, AuthoringDiagnostic, AuthoringSeverity, RequirementDocumentContent,
    requirements_root, testpoints_file, validate_hierarchy_path, validate_loaded_artifacts,
    validate_quote_selector, validate_requirement_index, validate_test_points, validate_text_range,
};
