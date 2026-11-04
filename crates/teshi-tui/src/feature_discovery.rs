use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Selects the default Feature directory while leaving project configuration at the root.
///
/// # Errors
/// Returns directory read errors or a missing system-language match.
pub(crate) fn default_feature_directory(root: &Path, locale: Option<&str>) -> Result<PathBuf> {
    let features = root.join("features");
    if !features.is_dir() {
        return Ok(root.to_path_buf());
    }
    let mut languages = Vec::new();
    for entry in std::fs::read_dir(&features).context("read features directory")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && is_language_tag(&name) {
            languages.push((name, entry.path()));
        }
    }
    if languages.is_empty() {
        return Ok(features);
    }
    let locale = locale.unwrap_or_default().replace('_', "-");
    if let Some((_, path)) = languages
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&locale))
    {
        return Ok(path.clone());
    }
    anyhow::bail!("No features language directory matches system language '{locale}'")
}

fn is_language_tag(name: &str) -> bool {
    let mut parts = name.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    let languages = teshi_core::gherkin_lang::GherkinLanguages::global();
    let known_language = languages.get(name).code.eq_ignore_ascii_case(name)
        || languages.get(language).code.eq_ignore_ascii_case(language);
    known_language
        && (2..=3).contains(&language.len())
        && language.bytes().all(|c| c.is_ascii_alphabetic())
        && parts.all(|part| {
            (2..=8).contains(&part.len()) && part.bytes().all(|c| c.is_ascii_alphanumeric())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlocalized_features_use_the_whole_features_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("features/api/nested")).unwrap();
        assert_eq!(
            default_feature_directory(root.path(), Some("zh-CN")).unwrap(),
            root.path().join("features")
        );
    }

    #[test]
    fn localized_features_select_only_the_system_language() {
        let root = tempfile::tempdir().unwrap();
        for language in ["zh-CN", "en-US"] {
            std::fs::create_dir_all(root.path().join("features").join(language)).unwrap();
        }
        assert_eq!(
            default_feature_directory(root.path(), Some("zh_CN")).unwrap(),
            root.path().join("features/zh-CN")
        );
        assert!(default_feature_directory(root.path(), Some("fr-FR")).is_err());
    }

    #[test]
    fn missing_features_falls_back_to_the_project_root() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            default_feature_directory(root.path(), Some("en-US")).unwrap(),
            root.path()
        );
    }
}

/// A single in-flight scan owned by one project context.
#[derive(Default)]
pub(crate) struct BackgroundDiscovery {
    receiver: Option<std::sync::mpsc::Receiver<Result<teshi_core::gherkin::BddProject, String>>>,
}

impl BackgroundDiscovery {
    /// Whether a directory scan is awaiting completion.
    pub(crate) fn is_loading(&self) -> bool {
        self.receiver.is_some()
    }

    /// Starts or polls a scan without walking the directory on the UI thread.
    #[cfg(test)]
    pub(crate) fn poll(
        &mut self,
        root: &Path,
        recursive: bool,
    ) -> Option<Result<teshi_core::gherkin::BddProject, String>> {
        self.poll_excluding(root, recursive, &[])
    }

    /// Discovers additions without reparsing files already owned by the editor.
    /// Existing-file edits remain handled by the disk-stamp conflict workflow.
    pub(crate) fn poll_excluding(
        &mut self,
        root: &Path,
        recursive: bool,
        known: &[teshi_core::gherkin::BddFeature],
    ) -> Option<Result<teshi_core::gherkin::BddProject, String>> {
        if self.receiver.is_none() {
            let known: std::collections::HashSet<_> =
                known.iter().map(|f| f.file_path.clone()).collect();
            let root = root.to_path_buf();
            let (sender, receiver) = std::sync::mpsc::channel();
            if let Err(error) = std::thread::Builder::new()
                .name("feature-discovery".into())
                .spawn(move || {
                    let snapshot = if !root.exists() {
                        Ok(teshi_core::gherkin::BddProject {
                            root_dir: root,
                            features: Vec::new(),
                        })
                    } else {
                        (|| {
                            let paths =
                                teshi_core::gherkin::discover_feature_paths(&root, recursive)?;
                            let features = paths
                                .into_iter()
                                .filter(|path| !known.contains(path))
                                .map(|path| {
                                    let source = std::fs::read_to_string(&path)?;
                                    Ok(teshi_core::gherkin::parse_feature(&source, path))
                                })
                                .collect::<std::io::Result<Vec<_>>>()?;
                            Ok(teshi_core::gherkin::BddProject {
                                root_dir: root,
                                features,
                            })
                        })()
                        .map_err(|e: std::io::Error| e.to_string())
                    };
                    let _ = sender.send(snapshot);
                })
            {
                return Some(Err(error.to_string()));
            }
            self.receiver = Some(receiver);
        }
        match self.receiver.as_ref()?.try_recv() {
            Ok(result) => {
                self.receiver = None;
                Some(result)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.receiver = None;
                Some(Err("Feature discovery worker disconnected".into()))
            }
        }
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;

    fn snapshot(
        worker: &mut BackgroundDiscovery,
        root: &Path,
    ) -> Result<teshi_core::gherkin::BddProject, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = worker.poll(root, true) {
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "scan worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn refresh_does_not_read_known_feature_contents() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing.feature");
        // Invalid UTF-8 would fail a full reparse. Known files belong to the
        // editor's conflict-aware reload path, not background discovery.
        std::fs::write(&existing, [0xff]).unwrap();
        std::fs::write(root.path().join("added.feature"), "Feature: Added").unwrap();
        let known = vec![teshi_core::gherkin::parse_feature(
            "Feature: Existing",
            existing,
        )];
        let mut worker = BackgroundDiscovery::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let result = loop {
            if let Some(result) = worker.poll_excluding(root.path(), true, &known) {
                break result.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(result.features.len(), 1);
        assert_eq!(result.features[0].name, "Added");
        assert!(!worker.is_loading());
    }

    #[test]
    fn worker_preserves_missing_scope_and_reports_read_errors() {
        let root = tempfile::tempdir().unwrap();
        let mut worker = BackgroundDiscovery::default();
        assert!(
            snapshot(&mut worker, &root.path().join("missing"))
                .unwrap()
                .features
                .is_empty()
        );
        let file = root.path().join("not-a-directory");
        std::fs::write(&file, "fixture").unwrap();
        assert!(snapshot(&mut worker, &file).is_err());
        assert!(
            snapshot(&mut worker, root.path())
                .unwrap()
                .features
                .is_empty()
        );
    }

    #[test]
    fn a_dropped_scan_cannot_feed_another_project() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("old.feature"), "Feature: Old\n").unwrap();
        let mut old_worker = BackgroundDiscovery::default();
        let _ = old_worker.poll(first.path(), true);
        drop(old_worker);
        let mut current = BackgroundDiscovery::default();
        let loaded = snapshot(&mut current, second.path()).unwrap();
        assert!(loaded.features.is_empty());
        assert_eq!(loaded.root_dir, second.path());
    }
}
