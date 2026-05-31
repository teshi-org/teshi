//! File change watcher for the active `.feature` file.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Manager};

use crate::gherkin_cmd::emit_feature_refresh;
use crate::project::ProjectState;

pub struct FileWatcherState {
    watcher: Mutex<Option<RecommendedWatcher>>,
}

impl FileWatcherState {
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
        }
    }

    pub fn watch(&self, path: &PathBuf, app: AppHandle) -> Result<()> {
        self.clear()?;
        let path_clone = path.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if res.is_ok() {
                    if let Some(root_state) = app.try_state::<ProjectState>() {
                        if let Some(project_root) = root_state.root.lock().unwrap().clone() {
                            emit_feature_refresh(&app, &path_clone, &project_root);
                        }
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(300)),
        )?;
        watcher.watch(path, RecursiveMode::NonRecursive)?;
        *self.watcher.lock().unwrap() = Some(watcher);
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        *self.watcher.lock().unwrap() = None;
        Ok(())
    }
}
