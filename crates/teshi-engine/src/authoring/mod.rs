//! Requirement and test-point authoring persistence.

mod import;
mod service;
mod store;

pub use import::{
    import_project_requirements, ImportMapping, ImportProjectOptions, ImportProjectPlan,
};
pub use service::{
    list_requirement_documents, read_requirement_document, resolve_requirement_ref,
    resolve_requirement_ref_in_store, set_requirement_documents_iteration,
    update_requirement_document_body, RequirementDocumentList, RequirementRefMatch,
    RequirementStoreError,
};
pub use store::{
    compute_document_revision, initialize_requirement_store, load_authoring_artifacts,
    save_requirement_document_index, save_requirement_markdown, save_test_points,
    set_requirement_document_iteration, AuthoringLoadResult, DEFAULT_REQUIREMENTS_DIR,
    DEFAULT_TESTPOINTS_DIR, REQUIREMENTS_INDEX_FILE,
};
