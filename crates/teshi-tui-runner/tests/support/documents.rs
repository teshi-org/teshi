//! Resolve a requirement body path through the store index.
//!
//! Bodies carry timestamp names, so tests must not assume `doc-N.md`.

use std::path::{Path, PathBuf};

pub fn document_path(store: &Path, id: &str) -> Option<PathBuf> {
    let index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.join("_teshi.json")).ok()?).ok()?;
    index["documents"]
        .as_array()?
        .iter()
        .find(|document| document["id"] == id)
        .and_then(|document| document["path"].as_str())
        .map(|path| store.join(path))
}
