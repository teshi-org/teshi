//! Explicit project ownership for existing isolated terminal fixtures.

pub fn associate(
    project: &std::path::Path,
    index: &mut teshi_core::authoring::RequirementDocumentIndex,
) {
    let id = "1cc0f9ac-8b8a-4a64-b7a0-4033eb63c670";
    std::fs::create_dir_all(project.join(".teshi")).unwrap();
    std::fs::write(
        project.join(".teshi/project.json"),
        serde_json::json!({"id": id}).to_string(),
    )
    .unwrap();
    for doc in &mut index.documents {
        doc.projects.insert(id.to_string(), Default::default());
    }
}
