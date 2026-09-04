//! Explicit import of legacy project `requirements/` into the global store.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use teshi_core::authoring::{
    legacy_project_requirements_dir, resolve_requirement_store_path, testpoints_file,
    validate_requirement_path, DocumentRevision, RequirementDocumentIndex, RequirementDocumentMeta,
    RequirementStoreId, TestPointsFile,
};

use super::store::{
    compute_document_revision, initialize_requirement_store_unlocked,
    is_requirement_store_lock_artifact, save_test_points, with_requirement_store_lock,
    write_requirement_index_unlocked, REQUIREMENTS_INDEX_FILE,
};

/// Options for [`import_project_requirements`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportProjectOptions {
    /// Compute and return the plan without writing.
    pub dry_run: bool,
    /// Write the planned copies, index, and rewritten test points.
    pub apply: bool,
}

/// One document mapping from a legacy project store into the global store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportMapping {
    /// Document ID in the source project index.
    pub source_id: String,
    /// Document ID that will be used in the target store.
    pub target_id: String,
    /// Source path relative to the project `requirements/` directory.
    pub source_path: String,
    /// Target path relative to the global store root.
    pub target_path: String,
    /// Human-readable action (`copy`, `reuse`, `remap-id`, `remap-path`).
    pub action: String,
}

/// Planned or applied import of a project's legacy requirements.
#[derive(Debug, Clone)]
pub struct ImportProjectPlan {
    /// Absolute target store directory.
    pub target_store_path: PathBuf,
    /// Target store identity, or a placeholder when initialization is deferred.
    pub target_store_id: String,
    /// Absolute source project directory.
    pub source_project: PathBuf,
    /// Per-document mapping.
    pub mappings: Vec<ImportMapping>,
    /// Whether any ID or path remapping is required.
    pub has_conflicts: bool,
    /// Number of Markdown files copied on apply (0 for dry-run).
    pub copied_documents: usize,
}

/// Imports `<project>/requirements/` into the current global requirement store.
///
/// Source files are never deleted. Dry-run does not create a store or rewrite
/// test points. Apply uses a copy journal so a failed write can delete only
/// newly created files that the previous target index did not reference.
///
/// # Errors
///
/// Returns an error when the source cannot be read, the target cannot be
/// initialized, confirmation is required but `apply` is false with conflicts,
/// or a write fails after cleanup.
pub fn import_project_requirements(
    project_root: &Path,
    requirements_root: &Path,
    options: ImportProjectOptions,
) -> Result<ImportProjectPlan> {
    let source_root = legacy_project_requirements_dir(project_root);
    if !source_root.exists() {
        bail!(
            "no legacy requirements directory at {}",
            source_root.display()
        );
    }

    let source_index = load_legacy_index(&source_root)?;
    let source_bodies = load_source_bodies(&source_root, &source_index)?;
    let mut target_index = load_or_empty_target(requirements_root)?;
    let dest_revisions = dest_file_revisions(requirements_root)?;

    let mappings = plan_mappings(
        &source_index,
        &source_bodies,
        &target_index,
        &dest_revisions,
    );
    let has_conflicts = mappings
        .iter()
        .any(|m| m.action == "remap-id" || m.action == "remap-path");

    let target_store_id = target_index
        .store_id
        .as_ref()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "(will initialize)".into());

    let mut plan = ImportProjectPlan {
        target_store_path: requirements_root.to_path_buf(),
        target_store_id,
        source_project: project_root.to_path_buf(),
        mappings,
        has_conflicts,
        copied_documents: 0,
    };

    if options.dry_run || !options.apply {
        return Ok(plan);
    }

    with_requirement_store_lock(requirements_root, || {
        target_index = load_or_empty_target(requirements_root)?;
        if target_index.store_id.is_none() {
            target_index = initialize_requirement_store_unlocked(requirements_root)?;
        }
        let dest_revisions = dest_file_revisions(requirements_root)?;
        plan.mappings = plan_mappings(
            &source_index,
            &source_bodies,
            &target_index,
            &dest_revisions,
        );
        plan.has_conflicts = plan
            .mappings
            .iter()
            .any(|m| m.action == "remap-id" || m.action == "remap-path");
        let store_id = target_index
            .store_id
            .clone()
            .context("initialized store missing store_id")?;
        plan.target_store_id = store_id.to_string();

        let index_file = requirements_root.join(REQUIREMENTS_INDEX_FILE);
        let previous_index = fs::read(&index_file).ok();
        let previous_paths: HashSet<String> = target_index
            .documents
            .iter()
            .map(|d| d.path.clone())
            .collect();
        let mut copied = Vec::new();
        let result = apply_import(
            project_root,
            requirements_root,
            &source_root,
            &source_bodies,
            &plan.mappings,
            &mut target_index,
            &store_id,
            &mut copied,
        );
        if let Err(error) = result {
            restore_index_file(&index_file, previous_index.as_deref());
            cleanup_copied(requirements_root, &copied, &previous_paths);
            return Err(error);
        }
        plan.copied_documents = copied.len();
        Ok(())
    })?;
    Ok(plan)
}

fn load_legacy_index(source_root: &Path) -> Result<RequirementDocumentIndex> {
    let index_file = source_root.join(REQUIREMENTS_INDEX_FILE);
    if !index_file.is_file() {
        bail!(
            "legacy requirements directory has no _teshi.json at {}",
            index_file.display()
        );
    }
    let text = fs::read_to_string(&index_file)
        .with_context(|| format!("read {}", index_file.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", index_file.display()))
}

fn load_source_bodies(
    source_root: &Path,
    index: &RequirementDocumentIndex,
) -> Result<HashMap<String, String>> {
    let mut bodies = HashMap::new();
    for meta in &index.documents {
        let path = resolve_requirement_store_path(source_root, &meta.path).with_context(|| {
            format!(
                "legacy requirement document '{}' has an unsafe path '{}'",
                meta.id, meta.path
            )
        })?;
        let body = fs::read_to_string(&path)
            .with_context(|| format!("read source document {}", path.display()))?;
        bodies.insert(meta.id.clone(), body);
    }
    Ok(bodies)
}

fn load_or_empty_target(requirements_root: &Path) -> Result<RequirementDocumentIndex> {
    let index_file = requirements_root.join(REQUIREMENTS_INDEX_FILE);
    if index_file.is_file() {
        let text = fs::read_to_string(&index_file)
            .with_context(|| format!("read {}", index_file.display()))?;
        return serde_json::from_str(&text)
            .with_context(|| format!("parse {}", index_file.display()));
    }
    if requirements_root.exists() {
        for entry in fs::read_dir(requirements_root)
            .with_context(|| format!("read {}", requirements_root.display()))?
        {
            let entry = entry.with_context(|| format!("read {}", requirements_root.display()))?;
            let name = entry.file_name();
            if is_requirement_store_lock_artifact(&name.to_string_lossy()) {
                continue;
            }
            bail!(
                "target requirement store at {} is not empty and has no _teshi.json (found {})",
                requirements_root.display(),
                name.to_string_lossy()
            );
        }
    }
    Ok(RequirementDocumentIndex::default())
}

fn dest_file_revisions(requirements_root: &Path) -> Result<HashMap<String, DocumentRevision>> {
    let mut out = HashMap::new();
    if !requirements_root.is_dir() {
        return Ok(out);
    }
    collect_dest_file_revisions(requirements_root, requirements_root, &mut out)?;
    Ok(out)
}

fn collect_dest_file_revisions(
    dir: &Path,
    root: &Path,
    out: &mut HashMap<String, DocumentRevision>,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_dest_file_revisions(&path, root, out)?;
            continue;
        }
        let Some(relative) = path
            .strip_prefix(root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
        else {
            continue;
        };
        if relative == REQUIREMENTS_INDEX_FILE
            || relative.ends_with(".lock")
            || relative.ends_with(".tmp")
        {
            continue;
        }
        if validate_requirement_path(&relative).is_err() {
            continue;
        }
        let body = fs::read_to_string(&path).unwrap_or_default();
        out.insert(relative, compute_document_revision(&body));
    }
    Ok(())
}

fn restore_index_file(path: &Path, previous: Option<&[u8]>) {
    match previous {
        Some(bytes) => {
            let _ = fs::write(path, bytes);
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

fn plan_mappings(
    source: &RequirementDocumentIndex,
    source_bodies: &HashMap<String, String>,
    target: &RequirementDocumentIndex,
    dest_file_revisions: &HashMap<String, DocumentRevision>,
) -> Vec<ImportMapping> {
    let by_id: HashMap<&str, &RequirementDocumentMeta> = target
        .documents
        .iter()
        .map(|d| (d.id.as_str(), d))
        .collect();
    let by_path: HashMap<&str, &RequirementDocumentMeta> = target
        .documents
        .iter()
        .map(|d| (d.path.as_str(), d))
        .collect();
    let mut used_ids: HashSet<String> = target.documents.iter().map(|d| d.id.clone()).collect();
    let mut used_paths: HashSet<String> = target.documents.iter().map(|d| d.path.clone()).collect();
    used_paths.extend(dest_file_revisions.keys().cloned());
    let mut mappings = Vec::new();

    for meta in &source.documents {
        let body = source_bodies
            .get(&meta.id)
            .map(String::as_str)
            .unwrap_or("");
        let source_rev = compute_document_revision(body);

        if let Some(existing) = by_id.get(meta.id.as_str()) {
            let existing_same_content =
                existing.path == meta.path && existing.revision == source_rev;
            if existing_same_content {
                mappings.push(ImportMapping {
                    source_id: meta.id.clone(),
                    target_id: existing.id.clone(),
                    source_path: meta.path.clone(),
                    target_path: existing.path.clone(),
                    action: "reuse".into(),
                });
                continue;
            }
            let target_id = next_available_id(&meta.id, &mut used_ids);
            let target_path = if used_paths.contains(&meta.path) {
                next_available_path(&meta.path, &mut used_paths)
            } else {
                used_paths.insert(meta.path.clone());
                meta.path.clone()
            };
            mappings.push(ImportMapping {
                source_id: meta.id.clone(),
                target_id,
                source_path: meta.path.clone(),
                target_path,
                action: "remap-id".into(),
            });
            continue;
        }

        if let Some(existing) = by_path.get(meta.path.as_str()) {
            let existing_body_matches = existing.revision == source_rev;
            if existing_body_matches {
                used_ids.insert(existing.id.clone());
                mappings.push(ImportMapping {
                    source_id: meta.id.clone(),
                    target_id: existing.id.clone(),
                    source_path: meta.path.clone(),
                    target_path: existing.path.clone(),
                    action: "reuse".into(),
                });
                continue;
            }
            let target_path = next_available_path(&meta.path, &mut used_paths);
            used_ids.insert(meta.id.clone());
            mappings.push(ImportMapping {
                source_id: meta.id.clone(),
                target_id: meta.id.clone(),
                source_path: meta.path.clone(),
                target_path,
                action: "remap-path".into(),
            });
            continue;
        }

        if used_paths.contains(&meta.path) {
            let same_content = dest_file_revisions
                .get(&meta.path)
                .is_some_and(|rev| rev == &source_rev);
            if same_content {
                used_ids.insert(meta.id.clone());
                mappings.push(ImportMapping {
                    source_id: meta.id.clone(),
                    target_id: meta.id.clone(),
                    source_path: meta.path.clone(),
                    target_path: meta.path.clone(),
                    action: "reuse".into(),
                });
                continue;
            }
            let target_path = next_available_path(&meta.path, &mut used_paths);
            used_ids.insert(meta.id.clone());
            mappings.push(ImportMapping {
                source_id: meta.id.clone(),
                target_id: meta.id.clone(),
                source_path: meta.path.clone(),
                target_path,
                action: "remap-path".into(),
            });
            continue;
        }

        used_ids.insert(meta.id.clone());
        used_paths.insert(meta.path.clone());
        mappings.push(ImportMapping {
            source_id: meta.id.clone(),
            target_id: meta.id.clone(),
            source_path: meta.path.clone(),
            target_path: meta.path.clone(),
            action: "copy".into(),
        });
    }
    mappings
}

fn next_available_id(base: &str, used: &mut HashSet<String>) -> String {
    let candidate = format!("{base}-imported");
    if used.insert(candidate.clone()) {
        return candidate;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-imported-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

fn next_available_path(path: &str, used: &mut HashSet<String>) -> String {
    let (stem, ext) = split_md_path(path);
    let candidate = format!("{stem}-imported{ext}");
    if used.insert(candidate.clone()) {
        return candidate;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{stem}-imported-{n}{ext}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

fn split_md_path(path: &str) -> (&str, &str) {
    match path.rfind('.') {
        Some(idx) => (&path[..idx], &path[idx..]),
        None => (path, ""),
    }
}

/// Copies mapped documents into the target store and rewrites project links.
///
/// The extra parameters keep the import transaction's paths, mappings, and
/// cleanup journal together rather than threading a one-off context type.
#[allow(clippy::too_many_arguments)]
fn apply_import(
    project_root: &Path,
    requirements_root: &Path,
    source_root: &Path,
    source_bodies: &HashMap<String, String>,
    mappings: &[ImportMapping],
    target_index: &mut RequirementDocumentIndex,
    store_id: &RequirementStoreId,
    copied: &mut Vec<PathBuf>,
) -> Result<()> {
    let rewritten_test_points = prepare_rewritten_test_points(project_root, mappings, store_id)?;
    let source_by_id = load_legacy_index(source_root)?;
    let source_lookup: HashMap<&str, &RequirementDocumentMeta> = source_by_id
        .documents
        .iter()
        .map(|d| (d.id.as_str(), d))
        .collect();

    for mapping in mappings {
        let meta = source_lookup
            .get(mapping.source_id.as_str())
            .with_context(|| format!("missing source document {}", mapping.source_id))?;
        let body = source_bodies
            .get(&mapping.source_id)
            .context("missing source body")?;
        let dest = resolve_requirement_store_path(requirements_root, &mapping.target_path)
            .with_context(|| {
                format!(
                    "import target path '{}' is not inside the requirement store",
                    mapping.target_path
                )
            })?;

        if mapping.action == "reuse" {
            if !target_index
                .documents
                .iter()
                .any(|d| d.id == mapping.target_id)
            {
                target_index.documents.push(RequirementDocumentMeta {
                    id: mapping.target_id.clone(),
                    path: mapping.target_path.clone(),
                    title: meta.title.clone(),
                    revision: compute_document_revision(body),
                    iteration: meta.iteration.clone(),
                });
            }
            continue;
        }

        if dest.is_file() {
            let existing = fs::read_to_string(&dest)
                .with_context(|| format!("read existing {}", dest.display()))?;
            if existing != *body {
                bail!(
                    "refusing to overwrite existing file at {} with different content",
                    dest.display()
                );
            }
            target_index.documents.push(RequirementDocumentMeta {
                id: mapping.target_id.clone(),
                path: mapping.target_path.clone(),
                title: meta.title.clone(),
                revision: compute_document_revision(body),
                iteration: meta.iteration.clone(),
            });
            continue;
        }

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&dest, body).with_context(|| format!("copy {}", dest.display()))?;
        copied.push(dest);
        target_index.documents.push(RequirementDocumentMeta {
            id: mapping.target_id.clone(),
            path: mapping.target_path.clone(),
            title: meta.title.clone(),
            revision: compute_document_revision(body),
            iteration: meta.iteration.clone(),
        });
    }

    write_requirement_index_unlocked(requirements_root, target_index)?;
    if let Some(file) = rewritten_test_points {
        save_test_points(project_root, &file)?;
    }
    Ok(())
}

fn prepare_rewritten_test_points(
    project_root: &Path,
    mappings: &[ImportMapping],
    store_id: &RequirementStoreId,
) -> Result<Option<TestPointsFile>> {
    let path = testpoints_file(project_root);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut file: TestPointsFile =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let id_map: HashMap<&str, &str> = mappings
        .iter()
        .map(|m| (m.source_id.as_str(), m.target_id.as_str()))
        .collect();
    for tp in &mut file.test_points {
        for link in &mut tp.requirement_links {
            if let Some(target_id) = id_map.get(link.document_id.as_str()) {
                link.document_id = (*target_id).to_string();
            }
            link.store_id = Some(store_id.clone());
        }
    }
    Ok(Some(file))
}

fn cleanup_copied(requirements_root: &Path, copied: &[PathBuf], previous_paths: &HashSet<String>) {
    for path in copied {
        let relative = path
            .strip_prefix(requirements_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
            .unwrap_or_default();
        if previous_paths.contains(&relative) {
            continue;
        }
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::authoring::{
        HierarchyPath, QuoteSelector, RequirementLink, ResolutionState, ReviewState, TestPoint,
        TextRange,
    };

    fn write_legacy_project(project: &Path, docs: &[(&str, &str, &str)]) {
        let req = legacy_project_requirements_dir(project);
        fs::create_dir_all(&req).unwrap();
        let mut index = RequirementDocumentIndex {
            version: 1,
            store_id: None,
            documents: Vec::new(),
        };
        for (id, path, body) in docs {
            if let Some(parent) = req.join(path).parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(req.join(path), body).unwrap();
            index.documents.push(RequirementDocumentMeta::new(
                *id,
                *path,
                *id,
                compute_document_revision(body),
            ));
        }
        fs::write(
            req.join(REQUIREMENTS_INDEX_FILE),
            serde_json::to_string_pretty(&index).unwrap(),
        )
        .unwrap();
    }

    fn write_test_point(project: &Path, document_id: &str) {
        let file = TestPointsFile {
            test_points: vec![TestPoint {
                id: "tp-1".into(),
                title: "T".into(),
                objective: "O".into(),
                preconditions: None,
                expected_outcomes: None,
                hierarchy_path: HierarchyPath::new(vec!["A".into()]),
                review_state: ReviewState::Proposed,
                requirement_links: vec![RequirementLink {
                    store_id: None,
                    document_id: document_id.into(),
                    document_revision: "rev".into(),
                    position: TextRange::new(0, 4),
                    quote: QuoteSelector {
                        quote: "User".into(),
                        prefix: String::new(),
                        suffix: String::new(),
                    },
                    resolution: ResolutionState::Resolved,
                }],
                scenario_refs: Vec::new(),
            }],
            ..Default::default()
        };
        save_test_points(project, &file).unwrap();
    }

    #[test]
    fn dry_run_reports_copy_plan_without_writing() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "auth.md", "User can log in")]);
        let plan = import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: true,
                apply: false,
            },
        )
        .unwrap();
        assert_eq!(plan.mappings.len(), 1);
        assert_eq!(plan.mappings[0].action, "copy");
        assert!(!store.path().join(REQUIREMENTS_INDEX_FILE).exists());
        assert!(legacy_project_requirements_dir(project.path())
            .join("auth.md")
            .is_file());
    }

    #[test]
    fn conflict_free_import_keeps_ids_and_rewrites_links() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "auth.md", "User can log in")]);
        write_test_point(project.path(), "doc-1");
        let plan = import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        assert_eq!(plan.mappings[0].target_id, "doc-1");
        assert!(store.path().join("auth.md").is_file());
        assert!(legacy_project_requirements_dir(project.path())
            .join("auth.md")
            .is_file());
        let file: TestPointsFile =
            serde_json::from_str(&fs::read_to_string(testpoints_file(project.path())).unwrap())
                .unwrap();
        assert_eq!(
            file.test_points[0].requirement_links[0].document_id,
            "doc-1"
        );
        assert_eq!(
            file.test_points[0].requirement_links[0]
                .store_id
                .as_ref()
                .unwrap()
                .as_str(),
            plan.target_store_id
        );
    }

    #[test]
    fn same_content_reuses_existing_document() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "auth.md", "User can log in")]);
        import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        let project2 = tempfile::tempdir().unwrap();
        write_legacy_project(project2.path(), &[("doc-1", "auth.md", "User can log in")]);
        let plan = import_project_requirements(
            project2.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        assert_eq!(plan.mappings[0].action, "reuse");
        let index: RequirementDocumentIndex = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(index.documents.len(), 1);
    }

    #[test]
    fn different_content_same_id_remaps_deterministically() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "auth.md", "User can log in")]);
        import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        let project2 = tempfile::tempdir().unwrap();
        write_legacy_project(project2.path(), &[("doc-1", "auth.md", "Different body")]);
        write_test_point(project2.path(), "doc-1");
        let plan = import_project_requirements(
            project2.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: true,
                apply: false,
            },
        )
        .unwrap();
        assert!(plan.has_conflicts);
        assert_eq!(plan.mappings[0].target_id, "doc-1-imported");
        let before = fs::read_to_string(testpoints_file(project2.path())).unwrap();
        let applied = import_project_requirements(
            project2.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        assert_eq!(applied.mappings[0].target_id, "doc-1-imported");
        let file: TestPointsFile =
            serde_json::from_str(&fs::read_to_string(testpoints_file(project2.path())).unwrap())
                .unwrap();
        assert_eq!(
            file.test_points[0].requirement_links[0].document_id,
            "doc-1-imported"
        );
        assert_ne!(
            before,
            fs::read_to_string(testpoints_file(project2.path())).unwrap()
        );
    }

    #[test]
    fn repeat_import_is_idempotent_reuse() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "auth.md", "User can log in")]);
        import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        let again = import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        assert_eq!(again.mappings[0].action, "reuse");
        let index: RequirementDocumentIndex = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(index.documents.len(), 1);
    }

    #[test]
    fn invalid_test_points_do_not_commit_target_index() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "auth.md", "User can log in")]);
        crate::initialize_requirement_store(store.path()).unwrap();
        let original = fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap();
        fs::create_dir_all(project.path().join("testpoints")).unwrap();
        fs::write(
            project.path().join("testpoints/testpoints.json"),
            "{not-json",
        )
        .unwrap();

        let err = import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("parse") || err.to_string().contains("expected"));
        assert!(!store.path().join("auth.md").is_file());
        assert_eq!(
            fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
            original
        );
    }

    #[test]
    fn unindexed_destination_file_with_different_content_is_not_overwritten() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "notes.md", "incoming body")]);
        crate::initialize_requirement_store(store.path()).unwrap();
        fs::write(store.path().join("notes.md"), "keep me").unwrap();

        let plan = import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: true,
                apply: false,
            },
        )
        .unwrap();
        assert_eq!(plan.mappings[0].action, "remap-path");
        assert_ne!(plan.mappings[0].target_path, "notes.md");

        import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(store.path().join("notes.md")).unwrap(),
            "keep me"
        );
        assert!(store.path().join(&plan.mappings[0].target_path).is_file());
    }

    #[test]
    fn unindexed_identical_destination_file_is_adopted() {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write_legacy_project(project.path(), &[("doc-1", "notes.md", "same body")]);
        crate::initialize_requirement_store(store.path()).unwrap();
        fs::write(store.path().join("notes.md"), "same body").unwrap();

        let plan = import_project_requirements(
            project.path(),
            store.path(),
            ImportProjectOptions {
                dry_run: false,
                apply: true,
            },
        )
        .unwrap();
        assert_eq!(plan.mappings[0].action, "reuse");
        assert_eq!(
            fs::read_to_string(store.path().join("notes.md")).unwrap(),
            "same body"
        );
        let index: RequirementDocumentIndex = serde_json::from_str(
            &fs::read_to_string(store.path().join(REQUIREMENTS_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(index.documents.len(), 1);
        assert_eq!(index.documents[0].id, "doc-1");
    }
}
