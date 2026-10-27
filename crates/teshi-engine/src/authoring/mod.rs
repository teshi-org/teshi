//! Requirement and test-point authoring persistence.

mod create;
mod import;
pub use create::{
    create_requirement_document, requirement_timestamp_filename, save_requirement_draft,
};
mod projects;
pub use projects::{
    ensure_requirement_project, read_requirement_project, requirement_project_filter,
    snapshot_requirement_project, update_requirement_projects, RequirementProject,
    RequirementProjectSnapshot,
};
mod service;
mod store;

pub use import::{
    import_project_requirements, ImportMapping, ImportProjectOptions, ImportProjectPlan,
};
pub use service::{
    delete_requirement_document, list_requirement_documents, read_requirement_document,
    resolve_requirement_ref, resolve_requirement_ref_in_store, set_requirement_documents_iteration,
    update_requirement_document_body, update_requirement_document_priority,
    RequirementDocumentList, RequirementRefMatch, RequirementStoreError,
};
pub use store::{
    compute_document_revision, initialize_requirement_store, is_requirement_document_id_reserved,
    load_authoring_artifacts, save_requirement_document_index, save_requirement_markdown,
    save_test_points, set_requirement_document_iteration, AuthoringLoadResult,
    DEFAULT_REQUIREMENTS_DIR, DEFAULT_TESTPOINTS_DIR, REQUIREMENTS_INDEX_FILE,
    REQUIREMENTS_TRASH_FILE,
};
