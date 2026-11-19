//! Project-scoped AI composer prompt history.
//!
//! Entries are appended when a prompt is sent, stored as JSONL under the app
//! data directory, and used for Up/Down recall and Ctrl+R search. Consecutive
//! duplicates are skipped. Newest entries are last in memory and first in
//! search results.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teshi_engine::{ProjectIdentity, app_data_dir};

/// Soft cap on retained prompts per project.
const MAX_ENTRIES: usize = 500;

/// One sent composer prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptEntry {
    /// Exact draft that was sent (placeholders already expanded).
    pub text: String,
}

/// In-memory MRU list backed by a per-project JSONL file.
#[derive(Debug, Clone, Default)]
pub struct PromptHistory {
    /// Oldest → newest.
    entries: Vec<PromptEntry>,
    path: Option<PathBuf>,
}

impl PromptHistory {
    /// Loads history for `project_root`, creating the store directory as needed.
    ///
    /// # Errors
    /// Returns I/O errors when the app data directory or history file cannot be
    /// created or read.
    pub fn load_for_project(project_root: &Path) -> std::io::Result<Self> {
        let path = history_path(project_root)?;
        let mut history = Self {
            entries: Vec::new(),
            path: Some(path.clone()),
        };
        if path.exists() {
            let file = File::open(&path)?;
            for line in BufReader::new(file).lines() {
                let line = line?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str::<PromptEntry>(trimmed)
                    && !entry.text.is_empty()
                {
                    history.entries.push(entry);
                }
            }
            if history.entries.len() > MAX_ENTRIES {
                let skip = history.entries.len() - MAX_ENTRIES;
                history.entries.drain(..skip);
            }
        }
        Ok(history)
    }

    /// Empty history with no backing file (tests / missing app data).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Newest-last slice used by Up/Down recall.
    pub fn prompts(&self) -> &[PromptEntry] {
        &self.entries
    }

    /// Newest-first texts matching `query` (case-insensitive substring).
    ///
    /// An empty query returns every entry.
    pub fn search(&self, query: &str) -> Vec<String> {
        let needle = query.trim().to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|entry| needle.is_empty() || entry.text.to_lowercase().contains(&needle))
            .map(|entry| entry.text.clone())
            .collect()
    }

    /// Appends a sent prompt when it is non-empty and not a consecutive duplicate.
    ///
    /// # Errors
    /// Returns I/O errors when the history file cannot be written.
    pub fn record(&mut self, text: &str) -> std::io::Result<()> {
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return Ok(());
        }
        if self
            .entries
            .last()
            .is_some_and(|entry| entry.text == trimmed)
        {
            return Ok(());
        }
        let entry = PromptEntry {
            text: trimmed.to_string(),
        };
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            serde_json::to_writer(&mut file, &entry).map_err(json_io_error)?;
            file.write_all(b"\n")?;
        }
        self.entries.push(entry);
        if self.entries.len() > MAX_ENTRIES {
            let skip = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..skip);
            self.rewrite()?;
        }
        Ok(())
    }

    fn rewrite(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(path)?;
        for entry in &self.entries {
            serde_json::to_writer(&mut file, entry).map_err(json_io_error)?;
            file.write_all(b"\n")?;
        }
        Ok(())
    }
}

fn history_path(project_root: &Path) -> std::io::Result<PathBuf> {
    let dir = app_data_dir()
        .map_err(|err| std::io::Error::other(err.to_string()))?
        .join("prompt_history");
    Ok(dir.join(format!("{}.jsonl", project_key(project_root))))
}

fn project_key(project_root: &Path) -> String {
    let identity = ProjectIdentity::from_root(project_root);
    let mut hasher = DefaultHasher::new();
    identity.root().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn json_io_error(err: serde_json::Error) -> std::io::Error {
    std::io::Error::other(err)
}

/// Live Ctrl+R search overlay state.
#[derive(Debug, Clone)]
pub struct HistorySearchState {
    /// Filter typed into the search field.
    pub query: String,
    /// Index into the filtered list (newest-first).
    pub selection: usize,
    /// Composer draft restored when search is cancelled.
    pub saved_draft: String,
}

impl HistorySearchState {
    /// Opens search while remembering the draft that Up/Down or Esc should restore.
    pub fn open(saved_draft: String) -> Self {
        Self {
            query: String::new(),
            selection: 0,
            saved_draft,
        }
    }
}

/// Fullscreen draft editor opened with Ctrl+O.
#[derive(Debug, Clone)]
pub struct ComposerEditorState {
    /// Editable copy of the composer draft.
    pub draft: crate::composer::Composer,
}

impl ComposerEditorState {
    /// Snapshots the live composer into a fullscreen editor buffer.
    pub fn open(source: &crate::composer::Composer) -> Self {
        let mut draft = crate::composer::Composer::default();
        draft.set_text(source.text().to_string());
        draft.set_cursor(source.cursor());
        Self { draft }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_skips_empty_and_consecutive_duplicates() {
        let mut history = PromptHistory::empty();
        history.record("").unwrap();
        history.record("one").unwrap();
        history.record("one").unwrap();
        history.record("two").unwrap();
        assert_eq!(
            history
                .prompts()
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
    }

    #[test]
    fn search_is_newest_first_and_case_insensitive() {
        let mut history = PromptHistory::empty();
        history.record("Alpha").unwrap();
        history.record("beta").unwrap();
        history.record("Alphabet").unwrap();
        assert_eq!(history.search("alp"), ["Alphabet", "Alpha"]);
        assert_eq!(history.search(""), ["Alphabet", "beta", "Alpha"]);
    }

    #[test]
    fn load_and_record_round_trip_on_disk() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => unsafe { std::env::set_var("TESHI_APP_DATA_DIR", value) },
                    None => unsafe { std::env::remove_var("TESHI_APP_DATA_DIR") },
                }
            }
        }
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let _restore = Restore(std::env::var_os("TESHI_APP_DATA_DIR"));
        unsafe {
            std::env::set_var("TESHI_APP_DATA_DIR", dir.path());
        }
        let project = dir.path().join("proj");
        fs::create_dir_all(&project).unwrap();

        let mut history = PromptHistory::load_for_project(&project).unwrap();
        history.record("hello\nworld").unwrap();
        history.record("next").unwrap();

        let reloaded = PromptHistory::load_for_project(&project).unwrap();
        assert_eq!(
            reloaded
                .prompts()
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            ["hello\nworld", "next"]
        );
    }
}
