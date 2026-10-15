//! Last explicit backend choice, separate from conversation snapshots.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub(crate) enum Selection {
    Native,
    Acp { agent_id: String },
}

pub(crate) fn load(root: &Path) -> Result<Option<Selection>> {
    match std::fs::read(root.join("last-selection.json")) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn save(root: &Path, selection: &Selection) -> Result<()> {
    teshi_engine::write_atomic(&root.join("last-selection.json"), selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_native_replaces_acp_and_missing_is_distinct() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(load(root.path()).unwrap(), None);
        let acp = Selection::Acp {
            agent_id: "alpha".into(),
        };
        save(root.path(), &acp).unwrap();
        assert_eq!(load(root.path()).unwrap(), Some(acp));
        save(root.path(), &Selection::Native).unwrap();
        assert_eq!(load(root.path()).unwrap(), Some(Selection::Native));
        std::fs::write(root.path().join("last-selection.json"), "invalid").unwrap();
        assert!(load(root.path()).is_err());
    }
}
