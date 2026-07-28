//! Requirement and test-point authoring persistence.

mod store;

pub use store::{
    compute_document_revision, load_authoring_artifacts, save_requirement_document_index,
    save_requirement_markdown, save_test_points, AuthoringLoadResult, DEFAULT_REQUIREMENTS_DIR,
    DEFAULT_TESTPOINTS_DIR, REQUIREMENTS_INDEX_FILE,
};
