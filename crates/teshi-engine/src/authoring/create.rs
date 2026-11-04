//! Collision-safe creation shared by requirement hosts.

use super::store::*;
use anyhow::{bail, Context, Result};
use std::{fs, io::Write, path::Path};
use teshi_core::authoring::{
    normalize_iteration_name, requirement_title, RequirementDocumentMeta, RequirementPriority,
};

/// Persists a UI draft without overwriting an ID claimed since the draft opened.
///
/// # Errors
/// Returns a conflict for existing or tombstoned identities and paths, retaining
/// the caller's draft. I/O failures roll back the newly created body.
pub fn save_requirement_draft(
    root: &Path,
    incoming: &mut teshi_core::authoring::RequirementDocumentIndex,
    path: &str,
    body: &str,
) -> Result<()> {
    let title = requirement_title(body)?;
    let target = teshi_core::authoring::resolve_requirement_store_path(root, path)?;
    let mut meta = incoming
        .documents
        .iter()
        .find(|d| d.path == path)
        .context("draft is not indexed")?
        .clone();
    meta.title = title.into();
    meta.revision = compute_document_revision(body);
    with_requirement_store_lock(root, || {
        let mut disk =
            read_requirement_index_unlocked(root)?.context("store is not initialized")?;
        if disk.store_id != incoming.store_id {
            bail!("requirement store identity changed");
        }
        let trash = read_requirement_trash_unlocked(root)?;
        if disk
            .documents
            .iter()
            .any(|d| d.id == meta.id || d.path == meta.path)
            || trash
                .documents
                .iter()
                .any(|d| d.meta.id == meta.id || d.meta.path == meta.path)
        {
            bail!("requirement creation conflict: {}; draft retained", meta.id);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let result = file
            .write_all(body.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        disk.documents.push(meta);
        let result = result
            .map_err(anyhow::Error::from)
            .and_then(|()| write_requirement_index_unlocked(root, &disk));
        if let Err(error) = result {
            fs::remove_file(&target).context(format!("{error:#}; rollback failed"))?;
            return Err(error);
        }
        *incoming = disk;
        Ok(())
    })
}

/// Returns a UTC microsecond filename. Existing paths are never renamed.
pub fn requirement_timestamp_filename() -> String {
    format!("{}.md", chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ"))
}

/// Creates a document under the store lock, allocating identity independently of its path.
/// Explicit projects start as planned; omission never links the current directory.
///
/// # Errors
/// Rejects invalid titles, metadata, stores and failed writes. An index write failure
/// removes the newly created body; existing bodies and tombstones are never overwritten.
pub fn create_requirement_document(
    root: &Path,
    body: &str,
    priority: Option<RequirementPriority>,
    iteration: Option<&str>,
    project_id: Option<&str>,
) -> Result<RequirementDocumentMeta> {
    create_requirement_document_with_clock(
        root,
        body,
        priority,
        iteration,
        project_id,
        requirement_timestamp_filename,
    )
}

fn create_requirement_document_with_clock(
    root: &Path,
    body: &str,
    priority: Option<RequirementPriority>,
    iteration: Option<&str>,
    project_id: Option<&str>,
    clock: impl FnOnce() -> String,
) -> Result<RequirementDocumentMeta> {
    let title = requirement_title(body)?;
    let iteration = iteration.map(normalize_iteration_name).transpose()?;
    with_requirement_store_lock(root, || {
        let mut index = match read_requirement_index_unlocked(root)? {
            Some(index) if index.store_id.is_some() => index,
            Some(_) => bail!("requirement store has no identity"),
            None => initialize_requirement_store_unlocked(root)?,
        };
        let trash = read_requirement_trash_unlocked(root)?;
        let id = (1u64..)
            .map(|n| format!("doc-{n}"))
            .find(|id| {
                !index.documents.iter().any(|d| &d.id == id)
                    && !trash.documents.iter().any(|d| &d.meta.id == id)
                    && !root.join(format!("{id}.md")).exists()
            })
            .context("document ID space exhausted")?;
        let base = clock();
        // The timestamp describes wall time, not a monotonic clock. A deterministic
        // suffix handles equal timestamps and clock rollback without replacing files.
        let mut suffix = 0u64;
        let (path, mut file) = loop {
            let path = if suffix == 0 {
                base.clone()
            } else {
                format!("{}-{suffix:06}.md", base.trim_end_matches(".md"))
            };
            suffix += 1;
            if index.documents.iter().any(|d| d.path == path)
                || trash.documents.iter().any(|d| d.meta.path == path)
            {
                continue;
            }
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join(&path))
            {
                Ok(file) => break (path, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        };
        let mut meta =
            RequirementDocumentMeta::new(id, &path, title, compute_document_revision(body));
        meta.priority = priority;
        meta.iteration = iteration;
        if let Some(project_id) = project_id {
            meta.projects
                .insert(project_id.to_owned(), Default::default());
        }
        index.documents.push(meta.clone());
        let result = file
            .write_all(body.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        let result = result
            .map_err(anyhow::Error::from)
            .and_then(|()| write_requirement_index_unlocked(root, &index));
        if let Err(error) = result {
            fs::remove_file(root.join(&path)).context(format!("{error:#}; rollback failed"))?;
            return Err(error);
        }
        Ok(meta)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_filename_has_utc_microsecond_precision() {
        let name = requirement_timestamp_filename();
        assert_eq!(name.len(), 26);
        let stamp = name.strip_suffix(".md").unwrap();
        chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S%.6fZ").unwrap();
    }

    #[test]
    fn equal_clock_and_rollback_preserve_existing_documents() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let create_at = |timestamp: &str, body: &str| {
            create_requirement_document_with_clock(root, body, None, None, None, || {
                timestamp.to_owned()
            })
            .unwrap()
        };
        let first = create_at("20261027T120000.000000Z.md", "# First\n");
        let second = create_at("20261027T120000.000000Z.md", "# Second\n");
        let earlier = create_at("20261027T115959.000000Z.md", "# Earlier\n");
        assert_eq!(first.path, "20261027T120000.000000Z.md");
        assert_eq!(second.path, "20261027T120000.000000Z-000001.md");
        assert_eq!(earlier.path, "20261027T115959.000000Z.md");
        assert_ne!(first.id, second.id);
        assert_ne!(second.id, earlier.id);
        for (document, expected) in [
            (first, "# First\n"),
            (second, "# Second\n"),
            (earlier, "# Earlier\n"),
        ] {
            assert_eq!(
                fs::read_to_string(root.join(document.path)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn creation_never_reuses_a_tombstoned_identity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let first = create_requirement_document(root, "# Original\n", None, None, None).unwrap();
        crate::authoring::delete_requirement_document(root, &first.id, None).unwrap();
        let trash_before = fs::read(root.join(REQUIREMENTS_TRASH_FILE)).unwrap();
        let second =
            create_requirement_document(root, "# Replacement\n", None, None, None).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            fs::read(root.join(REQUIREMENTS_TRASH_FILE)).unwrap(),
            trash_before
        );
        assert_eq!(
            fs::read_to_string(root.join(second.path)).unwrap(),
            "# Replacement\n"
        );
    }

    #[test]
    fn invalid_body_does_not_initialize_or_modify_the_store() {
        let directory = tempfile::tempdir().unwrap();
        assert!(create_requirement_document(directory.path(), "No H1", None, None, None).is_err());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn malformed_index_rejects_draft_without_modifying_store_or_buffer() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut draft = initialize_requirement_store(root).unwrap();
        // A malformed persisted index is rejected before the draft can replace it.
        fs::write(root.join(REQUIREMENTS_INDEX_FILE), "invalid json").unwrap();
        fs::write(root.join("existing.md"), "# Existing\n").unwrap();
        draft.documents.push(RequirementDocumentMeta::new(
            "doc-1",
            "draft.md",
            "Draft",
            compute_document_revision("# Draft\n"),
        ));
        assert!(save_requirement_draft(root, &mut draft, "draft.md", "# Draft\n").is_err());
        assert!(!root.join("draft.md").exists());
        assert_eq!(
            fs::read_to_string(root.join("existing.md")).unwrap(),
            "# Existing\n"
        );
        assert_eq!(
            fs::read_to_string(root.join(REQUIREMENTS_INDEX_FILE)).unwrap(),
            "invalid json"
        );
        assert_eq!(draft.documents.len(), 1);
    }

    #[test]
    fn failed_index_write_removes_new_body_and_preserves_existing_document() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let existing = create_requirement_document(root, "# Existing\n", None, None, None).unwrap();
        let index_before = fs::read(root.join(REQUIREMENTS_INDEX_FILE)).unwrap();
        fs::create_dir(root.join("_teshi.tmp")).unwrap();
        assert!(create_requirement_document(root, "# Rejected\n", None, None, None).is_err());
        assert_eq!(
            fs::read(root.join(REQUIREMENTS_INDEX_FILE)).unwrap(),
            index_before
        );
        let bodies: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
            .collect();
        assert_eq!(bodies, vec![root.join(&existing.path)]);
        assert_eq!(fs::read_to_string(&bodies[0]).unwrap(), "# Existing\n");
    }

    #[test]
    fn concurrent_creators_keep_independent_bodies_and_ids() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let root = root.clone();
                std::thread::spawn(move || {
                    let body = format!("# Requirement {n}\n\nBody {n}\n");
                    let meta = create_requirement_document(&root, &body, None, None, None).unwrap();
                    assert_eq!(fs::read_to_string(root.join(&meta.path)).unwrap(), body);
                    meta
                })
            })
            .collect();
        let documents: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let ids: std::collections::HashSet<_> = documents.iter().map(|d| &d.id).collect();
        let paths: std::collections::HashSet<_> = documents.iter().map(|d| &d.path).collect();
        assert_eq!(ids.len(), 8);
        assert_eq!(paths.len(), 8);
        assert!(documents
            .iter()
            .all(|d| d.projects.is_empty() && d.path.contains('Z')));
    }

    #[test]
    fn stale_draft_cannot_replace_a_concurrent_creation() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut draft = initialize_requirement_store(root).unwrap();
        draft.documents.push(RequirementDocumentMeta::new(
            "doc-1",
            "draft.md",
            "Draft",
            compute_document_revision("# Draft\n"),
        ));
        let saved = create_requirement_document(root, "# Winner\n", None, None, None).unwrap();
        assert!(save_requirement_draft(root, &mut draft, "draft.md", "# Draft\n").is_err());
        assert!(!root.join("draft.md").exists());
        assert_eq!(
            fs::read_to_string(root.join(saved.path)).unwrap(),
            "# Winner\n"
        );
    }
}
