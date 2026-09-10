//! Filesystem adapters and preflight helpers for the pure core validator.
//!
//! `teshi-core` deliberately does not read files.  Commands and runtime
//! integrations use this module to resolve a selected Feature scope, load its
//! source, and hand the resulting borrowed sources to the canonical validator.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use teshi_core::{validate_feature_sources, FeatureSource, ValidationReport};

/// Recursively discover `.feature` files below a project root in stable order.
pub fn discover_feature_files(project_root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_feature_files(project_root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

/// Validate one explicit Feature, or every Feature below the project root.
pub fn validate_feature_scope(
    project_root: &Path,
    feature_path: Option<&Path>,
) -> Result<ValidationReport> {
    let paths = match feature_path {
        Some(path) => {
            let resolved = resolve_feature_path(project_root, path);
            if resolved.is_dir() {
                discover_feature_files(&resolved)?
            } else {
                vec![resolved]
            }
        }
        None => discover_feature_files(project_root)?,
    };

    let mut loaded = Vec::with_capacity(paths.len());
    for path in paths {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("read Feature source {}", path.display()))?;
        let display_path = display_feature_path(project_root, &path);
        loaded.push((display_path, content));
    }

    let sources = loaded.iter().map(|(path, content)| FeatureSource {
        path: Path::new(path),
        content,
    });
    Ok(validate_feature_sources(sources))
}

/// Resolve a user-provided Feature path relative to the project root.
pub fn resolve_feature_path(project_root: &Path, feature_path: &Path) -> PathBuf {
    if feature_path.is_absolute() {
        feature_path.to_path_buf()
    } else {
        project_root.join(feature_path)
    }
}

/// Render a Feature path relative to the project root when possible.
pub fn display_feature_path(project_root: &Path, feature_path: &Path) -> String {
    feature_path
        .strip_prefix(project_root)
        .unwrap_or(feature_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_feature_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("read project directory {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("enumerate project directory {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let file_type = entry
            .file_type()
            .with_context(|| format!("inspect {}", entry.path().display()))?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_feature_files(&path, out)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "feature")
        {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discovers_and_validates_nested_sources_in_stable_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        let nested = temp.path().join("features").join("zh-CN");
        fs::create_dir_all(&nested).expect("create feature directory");
        fs::write(
            temp.path().join("features").join("a.feature"),
            "Feature: A\n  Scenario: S\n    Given ready\n",
        )
        .expect("write first feature");
        fs::write(
            nested.join("b.feature"),
            "# language: zh-CN\n功能: B\n  场景: S\n    当错误\n",
        )
        .expect("write second feature");

        let report = validate_feature_scope(temp.path(), None).expect("validate scope");
        assert_eq!(
            report.scope,
            vec!["features/a.feature", "features/zh-CN/b.feature"]
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_step_separator"));
    }

    #[test]
    fn explicit_missing_feature_is_an_io_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let error = validate_feature_scope(temp.path(), Some(Path::new("missing.feature")))
            .expect_err("missing Feature should fail discovery");
        assert!(error.to_string().contains("missing.feature"));
    }

    #[test]
    fn explicit_directory_scope_excludes_sibling_features() {
        let temp = tempfile::tempdir().expect("tempdir");
        let selected = temp.path().join("selected");
        let sibling = temp.path().join("sibling");
        fs::create_dir_all(&selected).expect("create selected directory");
        fs::create_dir_all(&sibling).expect("create sibling directory");
        fs::write(
            selected.join("ok.feature"),
            "Feature: Selected\n  Scenario: Works\n    Given the page is open\n    When I act\n    Then it succeeds\n",
        )
        .expect("write selected Feature");
        fs::write(
            sibling.join("broken.feature"),
            "Feature: Sibling\n  Scenario: Broken\n    Givenready\n",
        )
        .expect("write sibling Feature");

        let report = validate_feature_scope(temp.path(), Some(Path::new("selected")))
            .expect("validate selected directory");
        assert_eq!(report.scope, vec!["selected/ok.feature"]);
        assert!(
            !report.has_errors(),
            "sibling leaked into scope: {report:?}"
        );
    }
}
