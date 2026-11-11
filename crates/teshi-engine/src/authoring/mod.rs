//! Requirement and test-point authoring persistence.

mod create;
mod import;
pub use create::{
    create_requirement_document, requirement_timestamp_filename, save_requirement_draft,
};
pub mod organization;
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
    add_requirement_documents_tags, clear_requirement_tag_style, delete_requirement_document,
    list_requirement_documents, read_requirement_document, read_requirement_tag_styles,
    remove_requirement_documents_tags, resolve_requirement_ref, resolve_requirement_ref_in_store,
    set_requirement_documents_iteration, set_requirement_documents_tags,
    set_requirement_tag_styles, update_requirement_document_body,
    update_requirement_document_priority, update_requirement_tag_style, RequirementDocumentList,
    RequirementRefMatch, RequirementStoreError, RequirementTagStylePatch,
};
pub use store::{
    compute_document_revision, initialize_requirement_store, is_requirement_document_id_reserved,
    load_authoring_artifacts, save_requirement_document_index, save_requirement_markdown,
    save_test_points, set_requirement_document_iteration, AuthoringLoadResult,
    DEFAULT_REQUIREMENTS_DIR, DEFAULT_TESTPOINTS_DIR, REQUIREMENTS_INDEX_FILE,
    REQUIREMENTS_TRASH_FILE,
};
