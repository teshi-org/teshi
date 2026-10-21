//! Durable project membership for the user-level requirement library.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use teshi_core::authoring::{
    RequirementDocumentContent, RequirementDocumentIndex, RequirementProjectFilter,
    RequirementProjectStatus, RequirementStoreId,
};

use super::store::{
    read_requirement_index_unlocked, recover_requirement_store_unlocked, replace_markdown,
    with_requirement_store_lock, write_requirement_index_unlocked,
};

/// Portable project identity persisted in `.teshi/project.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementProject {
    /// Random identity, independent of checkout location and display name.
    pub id: String,
}

/// Reads a project's identity without creating files or assigning old requirements.
///
/// # Errors
/// Returns malformed identity or filesystem errors; missing files return `None`.
pub fn read_requirement_project(project_root: &Path) -> Result<Option<RequirementProject>> {
    let path = project_root.join(".teshi/project.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let project: RequirementProject =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    uuid::Uuid::parse_str(&project.id).context("invalid requirement project ID")?;
    Ok(Some(project))
}

/// Creates a durable identity on the first project membership mutation.
///
/// Commit `.teshi/project.json` to share identity across clones. Independent forks
/// should create their own identity before associating requirements.
///
/// # Errors
/// Returns an error for an invalid project directory, malformed identity, or failed write.
pub fn ensure_requirement_project(project_root: &Path) -> Result<RequirementProject> {
    if !project_root.is_dir() {
        bail!(
            "project directory does not exist: {}",
            project_root.display()
        );
    }
    if let Some(project) = read_requirement_project(project_root)? {
        return Ok(project);
    }
    let dir = project_root.join(".teshi");
    fs::create_dir_all(&dir).context("create project metadata directory")?;
    // Keep the lock inode: removing it lets concurrent Unix callers lock different files.
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("project.lock"))
        .context("open project identity lock")?;
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.write().context("lock project identity")?;
    if let Some(project) = read_requirement_project(project_root)? {
        return Ok(project);
    }
    let project = RequirementProject {
        id: uuid::Uuid::new_v4().to_string(),
    };
    replace_markdown(
        &dir.join("project.json"),
        &serde_json::to_vec_pretty(&project)?,
    )?;
    Ok(project)
}

/// Returns the current-project filter, matching nothing until the project has an identity.
///
/// # Errors
/// Returns malformed identity or filesystem errors.
pub fn requirement_project_filter(project_root: &Path) -> Result<RequirementProjectFilter> {
    Ok(RequirementProjectFilter::Project(
        read_requirement_project(project_root)?
            .map(|p| p.id)
            .unwrap_or_default(),
    ))
}

/// Associates or removes documents atomically, preserving other projects and metadata.
///
/// `linked = true` is idempotent and never resets existing delivery state.
/// A status update requires an existing association. All IDs are validated before writing.
///
/// # Errors
/// Returns an error for missing documents, missing membership, or storage failures.
pub fn update_requirement_projects(
    requirements_root: &Path,
    project_root: &Path,
    document_ids: &[&str],
    linked: bool,
    status: Option<RequirementProjectStatus>,
) -> Result<RequirementDocumentIndex> {
    if document_ids.is_empty() {
        bail!("at least one requirement document is required");
    }
    with_requirement_store_lock(requirements_root, || {
        recover_requirement_store_unlocked(requirements_root)?;
        let mut index = read_requirement_index_unlocked(requirements_root)?
            .filter(|index| index.store_id.is_some())
            .context("requirement store is not initialized")?;
        for id in document_ids {
            if !index.documents.iter().any(|doc| doc.id == *id) {
                bail!("requirement document '{id}' not found");
            }
        }
        let project = if linked && status.is_none() {
            ensure_requirement_project(project_root)?
        } else {
            read_requirement_project(project_root)?
                .context("project has no requirement identity")?
        };
        for id in document_ids {
            let doc = index
                .documents
                .iter_mut()
                .find(|doc| doc.id == *id)
                .context("document disappeared")?;
            if let Some(status) = status {
                let current = doc
                    .projects
                    .get_mut(&project.id)
                    .context("requirement is not linked to this project")?;
                *current = status;
            } else if linked {
                doc.projects.entry(project.id.clone()).or_default();
            } else {
                doc.projects.remove(&project.id);
            }
        }
        write_requirement_index_unlocked(requirements_root, &index)?;
        Ok(index)
    })
}

/// Portable, read-only copy of a project's requirement sources and recorded states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementProjectSnapshot {
    /// Snapshot format version.
    pub version: u32,
    /// Project identity shared by the exported associations.
    pub project: RequirementProject,
    /// Original library identity, preserving test-point source references.
    pub store_id: RequirementStoreId,
    /// Original document identities, revisions, metadata, and full Markdown bodies.
    pub documents: Vec<RequirementDocumentContent>,
}

/// Captures a project's linked documents consistently under the store lock.
///
/// # Errors
/// Returns an error for missing identity, invalid store, or unreadable source documents.
pub fn snapshot_requirement_project(
    requirements_root: &Path,
    project_root: &Path,
) -> Result<RequirementProjectSnapshot> {
    let project =
        read_requirement_project(project_root)?.context("project has no requirement identity")?;
    with_requirement_store_lock(requirements_root, || {
        recover_requirement_store_unlocked(requirements_root)?;
        let index = read_requirement_index_unlocked(requirements_root)?
            .context("requirement store is not initialized")?;
        let store_id = index
            .store_id
            .context("requirement store has no identity")?;
        let mut documents = Vec::new();
        for meta in index
            .documents
            .into_iter()
            .filter(|doc| doc.projects.contains_key(&project.id))
        {
            let path = teshi_core::authoring::resolve_requirement_store_path(
                requirements_root,
                &meta.path,
            )?;
            let body =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            let mut meta = meta;
            meta.revision = super::store::compute_document_revision(&body);
            meta.projects.retain(|id, _| id == &project.id);
            documents.push(RequirementDocumentContent { meta, body });
        }
        Ok(RequirementProjectSnapshot {
            version: 1,
            project,
            store_id,
            documents,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compute_document_revision, initialize_requirement_store, load_authoring_artifacts,
        save_requirement_document_index, save_requirement_markdown,
    };
    use teshi_core::authoring::RequirementDocumentMeta;

    fn fixture(store: &Path) -> RequirementDocumentIndex {
        let mut index = initialize_requirement_store(store).unwrap();
        index.documents.push(RequirementDocumentMeta::new(
            "doc-1",
            "one.md",
            "One",
            compute_document_revision("# One\n"),
        ));
        save_requirement_markdown(store, &mut index, "one.md", "# One\n").unwrap();
        index.documents.push(RequirementDocumentMeta::new(
            "doc-2",
            "two.md",
            "Two",
            compute_document_revision("# Two\n"),
        ));
        save_requirement_markdown(store, &mut index, "two.md", "# Two\n").unwrap();
        index
    }

    #[test]
    fn project_identity_is_lazy_and_survives_a_directory_move() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        fs::create_dir(&a).unwrap();
        assert_eq!(read_requirement_project(&a).unwrap(), None);
        assert!(!a.join(".teshi").exists());
        let id = ensure_requirement_project(&a).unwrap();
        assert_eq!(id, ensure_requirement_project(&a.join(".")).unwrap());
        fs::rename(&a, &b).unwrap();
        assert_eq!(id, read_requirement_project(&b).unwrap().unwrap());
        fs::create_dir(&a).unwrap();
        assert_ne!(id, ensure_requirement_project(&a).unwrap());
    }

    #[test]
    fn project_states_and_unlink_are_independent_and_stale_saves_preserve_them() {
        let root = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut stale = fixture(root.path());
        let original_store = stale.store_id.clone();
        assert!(stale
            .documents
            .iter()
            .all(|doc| doc.matches_project_filter(&RequirementProjectFilter::Unassigned)));
        update_requirement_projects(root.path(), a.path(), &["doc-1"], true, None).unwrap();
        update_requirement_projects(root.path(), b.path(), &["doc-1"], true, None).unwrap();
        update_requirement_projects(
            root.path(),
            a.path(),
            &["doc-1"],
            true,
            Some(RequirementProjectStatus::Verified),
        )
        .unwrap();
        update_requirement_projects(root.path(), a.path(), &["doc-1"], true, None).unwrap();
        save_requirement_document_index(root.path(), &stale).unwrap();
        save_requirement_markdown(root.path(), &mut stale, "one.md", "# One\n").unwrap();
        let a_id = read_requirement_project(a.path()).unwrap().unwrap().id;
        let b_id = read_requirement_project(b.path()).unwrap().unwrap().id;
        assert_eq!(stale.store_id, original_store);
        assert_eq!(
            stale.documents[0].projects[&a_id],
            RequirementProjectStatus::Verified
        );
        assert_eq!(
            stale.documents[0].projects[&b_id],
            RequirementProjectStatus::Planned
        );
        let index =
            update_requirement_projects(root.path(), a.path(), &["doc-1"], false, None).unwrap();
        assert!(!index.documents[0].projects.contains_key(&a_id));
        assert!(index.documents[0].projects.contains_key(&b_id));
        assert_eq!(
            fs::read_to_string(root.path().join("one.md")).unwrap(),
            "# One\n"
        );
        assert!(!a.path().join("testpoints").exists());
    }

    #[test]
    fn invalid_batch_or_missing_membership_never_partially_updates_the_store() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fixture(store.path());
        let before = fs::read(store.path().join("_teshi.json")).unwrap();
        assert!(update_requirement_projects(
            store.path(),
            project.path(),
            &["doc-1", "missing"],
            true,
            None
        )
        .is_err());
        assert!(!project.path().join(".teshi").exists());
        assert_eq!(before, fs::read(store.path().join("_teshi.json")).unwrap());
        update_requirement_projects(store.path(), project.path(), &["doc-1"], true, None).unwrap();
        let before = fs::read(store.path().join("_teshi.json")).unwrap();
        assert!(update_requirement_projects(
            store.path(),
            project.path(),
            &["doc-1", "doc-2"],
            true,
            Some(RequirementProjectStatus::Verified)
        )
        .is_err());
        assert_eq!(before, fs::read(store.path().join("_teshi.json")).unwrap());
    }

    #[test]
    fn snapshot_contains_portable_sources_only_for_the_requested_project() {
        let store = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let index = fixture(store.path());
        update_requirement_projects(store.path(), a.path(), &["doc-1"], true, None).unwrap();
        update_requirement_projects(store.path(), b.path(), &["doc-1", "doc-2"], true, None)
            .unwrap();
        let snapshot = snapshot_requirement_project(store.path(), a.path()).unwrap();
        assert_eq!(Some(snapshot.store_id.clone()), index.store_id);
        assert_eq!(snapshot.documents.len(), 1);
        assert_eq!(snapshot.documents[0].body, "# One\n");
        assert_eq!(snapshot.documents[0].meta.id, "doc-1");
        assert_eq!(snapshot.documents[0].meta.projects.len(), 1);
        let json = serde_json::to_string(&snapshot).unwrap();
        drop(store);
        let restored: RequirementProjectSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.documents[0].body, "# One\n");
    }

    #[test]
    fn old_metadata_stays_unassigned_and_loading_does_not_create_project_identity() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fixture(store.path());
        let before = fs::read(store.path().join("_teshi.json")).unwrap();
        assert!(!String::from_utf8_lossy(&before).contains("projects"));
        let loaded = load_authoring_artifacts(project.path(), store.path())
            .unwrap()
            .artifacts
            .unwrap();
        assert!(loaded
            .index
            .documents
            .iter()
            .all(|doc| doc.projects.is_empty()));
        assert!(!project.path().join(".teshi").exists());
        assert_eq!(before, fs::read(store.path().join("_teshi.json")).unwrap());
    }

    #[test]
    fn malformed_identity_is_rejected_without_replacement() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join(".teshi")).unwrap();
        let path = project.path().join(".teshi/project.json");
        fs::write(&path, "{\"id\":\"invalid\"}").unwrap();
        assert!(ensure_requirement_project(project.path()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "{\"id\":\"invalid\"}");
    }

    #[test]
    fn concurrent_first_links_share_one_project_identity() {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        fixture(store.path());
        std::thread::scope(|scope| {
            let mut threads = Vec::new();
            for id in ["doc-1", "doc-2", "doc-1", "doc-2"] {
                let store = store.path();
                let project = project.path();
                threads.push(scope.spawn(move || {
                    update_requirement_projects(store, project, &[id], true, None).unwrap()
                }));
            }
            for thread in threads {
                thread.join().unwrap();
            }
        });
        let project_id = read_requirement_project(project.path())
            .unwrap()
            .unwrap()
            .id;
        let index = read_requirement_index_unlocked(store.path())
            .unwrap()
            .unwrap();
        assert!(index
            .documents
            .iter()
            .all(|doc| doc.projects.len() == 1 && doc.projects.contains_key(&project_id)));
    }
}
