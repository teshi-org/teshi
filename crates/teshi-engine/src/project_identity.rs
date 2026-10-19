//! Stable project identities and generations used by project-scoped state.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A normalized project root. The path is canonicalized when it exists and
/// otherwise made absolute without requiring the target to exist.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectIdentity {
    root: PathBuf,
}

impl ProjectIdentity {
    /// Resolve a project root while preserving platform path semantics.
    ///
    /// # Errors
    /// Returns filesystem canonicalization or current-directory resolution errors.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let resolved = if path.exists() {
            std::fs::canonicalize(path)
                .with_context(|| format!("canonicalize project root {}", path.display()))?
        } else if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .context("resolve current directory")?
                .join(path)
        };
        Ok(Self {
            root: normalize_path(&resolved),
        })
    }

    /// Build an identity from a path already resolved by a caller.
    pub fn from_root(path: impl AsRef<Path>) -> Self {
        Self {
            root: normalize_path(path.as_ref()),
        }
    }

    /// Returns the normalized root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A short label suitable for a project picker.
    pub fn display_name(&self) -> String {
        self.root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.root.display().to_string())
    }
}

/// Monotonic generation for asynchronous project-scoped work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProjectGeneration(u64);

impl ProjectGeneration {
    /// Generation used before the first successful project switch.
    pub const INITIAL: Self = Self(0);

    /// Construct a generation from a recorded event sequence.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the numeric generation for persisted or diagnostic data.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Advance the generation, saturating only at the representable limit.
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Identity plus generation attached to a project-scoped event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStamp {
    pub identity: ProjectIdentity,
    pub generation: ProjectGeneration,
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            // Windows accepts the extended prefix for the same filesystem
            // path. Strip it so aliases compare equal without lowercasing
            // case-sensitive paths on other platforms.
            Component::Prefix(prefix) if cfg!(windows) => {
                let raw = prefix.as_os_str().to_string_lossy();
                if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
                    normalized.push(format!(r"\\{unc}"));
                } else if let Some(path) = raw.strip_prefix(r"\\?\") {
                    normalized.push(path);
                } else {
                    normalized.push(prefix.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn existing_relative_and_absolute_paths_share_identity() {
        let root = tempdir().unwrap();
        let relative = root.path().join(".").join("nested");
        std::fs::create_dir_all(&relative).unwrap();
        let absolute = std::fs::canonicalize(root.path().join("nested")).unwrap();
        assert_eq!(
            ProjectIdentity::new(&relative).unwrap(),
            ProjectIdentity::new(&absolute).unwrap()
        );
    }

    #[test]
    fn missing_relative_path_is_absolute_and_stable() {
        let root = tempdir().unwrap();
        let path = root.path().join("missing");
        let identity = ProjectIdentity::from_root(&path);
        assert!(identity.root().is_absolute());
        assert_eq!(identity, ProjectIdentity::from_root(identity.root()));
    }

    #[test]
    fn generations_are_monotonic() {
        let initial = ProjectGeneration::INITIAL;
        assert_eq!(initial.next().value(), 1);
        assert_eq!(initial.next().next().value(), 2);
    }
}
