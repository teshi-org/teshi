//! File change watcher for the active `.feature` file.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::events::RuntimeEvents;
use crate::gherkin::emit_feature_refresh;

/// Watches a single feature file for external edits.
pub struct FileWatcherState {
    watcher: Mutex<Option<RecommendedWatcher>>,
}

impl Default for FileWatcherState {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWatcherState {
    /// Creates an empty file watcher holder.
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
        }
    }

    /// Watches `path` and emits `feature-refreshed` on change.
    pub fn watch(&self, path: &Path, project_root: &Path, events: RuntimeEvents) -> Result<()> {
        self.clear()?;
        let path_clone = path.to_path_buf();
        let root = project_root.to_path_buf();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if res.is_ok() {
                    emit_feature_refresh(&events, &path_clone, &root);
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(300)),
        )?;
        watcher.watch(path, RecursiveMode::NonRecursive)?;
        *self.watcher.lock().unwrap() = Some(watcher);
        Ok(())
    }

    /// Stops watching the current feature file.
    pub fn clear(&self) -> Result<()> {
        *self.watcher.lock().unwrap() = None;
        Ok(())
    }
}
