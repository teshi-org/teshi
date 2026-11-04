use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, ColumnFocus, MainTab};

#[derive(Default)]
pub(crate) struct FeatureBrowser {
    pub(crate) query: String,
    pub(crate) searching: bool,
    applied_query: String,
    pending_search: Option<Instant>,
    pub(crate) directory: Option<String>,
    expanded: BTreeSet<String>,
    rows: RefCell<Option<(u64, Rc<Vec<Row>>)>>,
    width: std::cell::Cell<usize>,
    files: RefCell<Option<(u64, Vec<SearchFile>)>>,
    directory_rows: RefCell<HashMap<String, usize>>,
    file_rows: RefCell<HashMap<(usize, bool), usize>>,
    path_rows: RefCell<HashMap<std::path::PathBuf, usize>>,
}

struct SearchFile {
    path: String,
    searchable: String,
    index: usize,
    deleted: bool,
}

#[derive(Clone)]
pub(crate) enum Row {
    Directory(String, usize),
    File(usize, String, bool),
}

impl App {
    fn feature_browser_filter_query(&self) -> &str {
        if self.feature_browser.pending_search.is_some() {
            &self.feature_browser.applied_query
        } else if self.feature_browser.query.trim().is_empty() {
            ""
        } else {
            &self.feature_browser.query
        }
    }

    fn commit_feature_browser_search(&mut self) {
        self.feature_browser.pending_search = None;
        self.feature_browser.applied_query = self.feature_browser.query.clone();
        if self.explore_selected_feature_row().is_none() {
            self.explore_set_feature_row(0);
        }
    }

    fn schedule_feature_browser_search(&mut self) {
        if self.feature_browser.query.trim().is_empty() {
            self.commit_feature_browser_search();
        } else {
            self.feature_browser.pending_search = Some(Instant::now());
        }
    }

    pub(crate) fn poll_feature_browser_search(&mut self) {
        if self
            .feature_browser
            .pending_search
            .is_some_and(|at| at.elapsed() >= SEARCH_DEBOUNCE)
        {
            self.commit_feature_browser_search();
        }
    }

    pub(crate) fn paste_feature_browser_search(&mut self, text: &str) {
        for ch in text.chars() {
            if !self
                .handle_feature_browser_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            {
                break;
            }
        }
    }

    pub(crate) fn feature_browser_rows(&self) -> Rc<Vec<Row>> {
        // Hash only tree inputs: scenario edits and selection changes do not
        // require sorting and allocating the entire directory tree again.
        let mut key = std::collections::hash_map::DefaultHasher::new();
        self.project.root_dir.hash(&mut key);
        self.feature_discovery_root.hash(&mut key);
        if let Some(catalog) = &self.lazy_features {
            catalog.revision.hash(&mut key);
        } else {
            self.project.features.len().hash(&mut key);
            for feature in &self.project.features {
                feature.file_path.hash(&mut key);
                feature.name.hash(&mut key);
            }
        }
        self.git_status.deleted.len().hash(&mut key);
        for feature in &self.git_status.deleted {
            feature.path.hash(&mut key);
        }
        let files_key = key.finish();
        self.feature_browser_filter_query().hash(&mut key);
        self.feature_browser.expanded.hash(&mut key);
        let key = key.finish();
        if let Some((cached_key, rows)) = self.feature_browser.rows.borrow().as_ref()
            && *cached_key == key
        {
            return Rc::clone(rows);
        }
        if self
            .feature_browser
            .files
            .borrow()
            .as_ref()
            .map(|(key, _)| *key)
            != Some(files_key)
        {
            *self.feature_browser.files.borrow_mut() =
                Some((files_key, self.build_feature_browser_files()));
        }
        let rows = Rc::new(self.build_feature_browser_rows());
        let mut directories = self.feature_browser.directory_rows.borrow_mut();
        let mut files = self.feature_browser.file_rows.borrow_mut();
        let mut paths = self.feature_browser.path_rows.borrow_mut();
        directories.clear();
        files.clear();
        paths.clear();
        // Index visible positions once; navigation near the bottom must not
        // compare every preceding filesystem path on each key or frame.
        for (position, row) in rows.iter().enumerate() {
            match row {
                Row::Directory(path, _) => {
                    directories.insert(path.clone(), position);
                }
                Row::File(index, _, deleted) => {
                    files.insert((*index, *deleted), position);
                    if !deleted
                        && let Some(catalog) = &self.lazy_features
                        && let Some(path) = catalog.paths.get(*index)
                    {
                        paths.insert(path.clone(), position);
                    }
                }
            }
        }
        // Width depends only on tree labels, not selection or Git markers.
        use unicode_width::UnicodeWidthStr;
        self.feature_browser.width.set(
            rows.iter()
                .map(|row| match row {
                    Row::Directory(path, depth) => {
                        4 + depth * 2 + path.rsplit('/').next().unwrap_or(path).width()
                    }
                    Row::File(_, label, _) => 4 + label.width(),
                })
                .max()
                .unwrap_or(0),
        );
        *self.feature_browser.rows.borrow_mut() = Some((key, Rc::clone(&rows)));
        rows
    }

    /// Returns the maximum display width of the most recently resolved tree.
    pub(crate) fn feature_browser_width(&self) -> usize {
        self.feature_browser.width.get()
    }

    fn build_feature_browser_files(&self) -> Vec<SearchFile> {
        let mut files = BTreeMap::new();
        let display_root = self
            .feature_discovery_root
            .as_deref()
            .unwrap_or(&self.project.root_dir);
        if let Some(catalog) = &self.lazy_features {
            for (index, file_path) in catalog.paths.iter().enumerate() {
                let path = file_path
                    .strip_prefix(display_root)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let searchable = path.to_lowercase();
                files.insert((path, false), (index, searchable));
            }
        } else {
            for (index, feature) in self.project.features.iter().enumerate() {
                let path = feature
                    .file_path
                    .strip_prefix(display_root)
                    .unwrap_or(&feature.file_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let searchable =
                    format!("{}\n{}", path.to_lowercase(), feature.name.to_lowercase());
                files.insert((path, false), (index, searchable));
            }
        }
        for (index, feature) in self.git_status.deleted.iter().enumerate() {
            let absolute = if feature.path.is_absolute() {
                feature.path.clone()
            } else {
                self.project.root_dir.join(&feature.path)
            };
            let path = absolute
                .strip_prefix(display_root)
                .unwrap_or(&absolute)
                .to_string_lossy()
                .replace('\\', "/");
            let searchable = path.to_lowercase();
            files.insert((path, true), (index, searchable));
        }
        files
            .into_iter()
            .map(|((path, deleted), (index, searchable))| SearchFile {
                path,
                searchable,
                index,
                deleted,
            })
            .collect()
    }

    fn build_feature_browser_rows(&self) -> Vec<Row> {
        let query = self.feature_browser_filter_query().to_lowercase();
        let files = self.feature_browser.files.borrow();
        let Some((_, files)) = files.as_ref() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let mut emitted = BTreeSet::new();
        for SearchFile {
            path,
            searchable,
            index,
            deleted,
        } in files
        {
            if !query.is_empty() && !searchable.contains(&query) {
                continue;
            }
            let parts: Vec<_> = path
                .split('/')
                .filter(|part| !part.is_empty() && *part != ".")
                .collect();
            let mut hidden = false;
            for depth in 0..parts.len().saturating_sub(1) {
                let directory = parts[..=depth].join("/");
                if emitted.insert(directory.clone()) {
                    rows.push(Row::Directory(directory.clone(), depth));
                }
                if query.is_empty() && !self.feature_browser.expanded.contains(&directory) {
                    hidden = true;
                    break;
                }
            }
            if !hidden {
                rows.push(Row::File(
                    *index,
                    format!(
                        "{}{}",
                        "  ".repeat(parts.len().saturating_sub(1)),
                        parts
                            .last()
                            .unwrap_or(&"")
                            .strip_suffix(".feature")
                            .unwrap_or(parts.last().unwrap_or(&""))
                    ),
                    *deleted,
                ));
            }
        }
        rows
    }

    /// Resolves the selected visible row using the current tree's indexes.
    pub(crate) fn feature_browser_selected_row(&self) -> Option<usize> {
        self.feature_browser_rows();
        if let Some(path) = &self.feature_browser.directory {
            return self
                .feature_browser
                .directory_rows
                .borrow()
                .get(path)
                .copied();
        }
        if let Some(index) = self.explore_selected_deleted_feature {
            return self
                .feature_browser
                .file_rows
                .borrow()
                .get(&(index, true))
                .copied();
        }
        if let Some(catalog) = &self.lazy_features {
            return catalog
                .selected
                .as_ref()
                .and_then(|path| self.feature_browser.path_rows.borrow().get(path).copied());
        }
        self.feature_browser
            .file_rows
            .borrow()
            .get(&(self.explore_selected_feature, false))
            .copied()
    }

    pub(crate) fn feature_directory_collapsed(&self, path: &str) -> bool {
        self.feature_browser_filter_query().is_empty()
            && !self.feature_browser.expanded.contains(path)
    }

    pub(crate) fn handle_feature_browser_key(&mut self, event: KeyEvent) -> bool {
        if self.active_tab != MainTab::Explore
            || self.explore_edit_mode
            || self.explore_focus != ColumnFocus::Feature
            || self.pending_delete.is_some()
            || self.quit_pending_confirm
            || self.auth_panel_active
            || self.model_panel_active
            || self.session_panel_active
            || self.approval_panel_active
            || self.agent_profile_panel_active
            || self.change_summary_visible
            || self.scenario_dropdown_open
            || self.step_keyword_picker.is_some()
            || self.step_input_active
            || self.generation_scope_prompt.is_some()
            || self.has_external_change_prompt()
            || self.has_agent_change_prompt()
            || self.has_acp_permission_prompt()
            || !matches!(
                self.project_switch.overlay,
                crate::project_context::SwitchOverlay::None
            )
        {
            return false;
        }
        if self.feature_browser.searching {
            match event.code {
                KeyCode::Esc => {
                    self.feature_browser.query.clear();
                    self.feature_browser.searching = false;
                }
                KeyCode::Enter => self.feature_browser.searching = false,
                KeyCode::Backspace => {
                    self.feature_browser.query.pop();
                }
                KeyCode::Char(ch)
                    if !event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.feature_browser.query.push(ch)
                }
                KeyCode::Up | KeyCode::Down => return false,
                _ => return true,
            }
            if matches!(event.code, KeyCode::Esc | KeyCode::Enter) {
                self.commit_feature_browser_search();
            } else {
                self.schedule_feature_browser_search();
            }
            return true;
        }
        match event.code {
            KeyCode::Char('/') if event.modifiers.is_empty() => {
                self.feature_browser.searching = true;
                true
            }
            KeyCode::Esc if !self.feature_browser.query.is_empty() => {
                self.feature_browser.query.clear();
                self.commit_feature_browser_search();
                true
            }
            KeyCode::Enter | KeyCode::Left | KeyCode::Right => {
                let Some(path) = self.feature_browser.directory.clone() else {
                    return false;
                };
                match event.code {
                    KeyCode::Left => {
                        self.feature_browser.expanded.remove(&path);
                    }
                    KeyCode::Right => {
                        self.feature_browser.expanded.insert(path);
                    }
                    _ => {
                        if !self.feature_browser.expanded.remove(&path) {
                            self.feature_browser.expanded.insert(path);
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_selection_tracks_filter_expansion_and_catalog_changes() {
        let mut app = App::from_args().unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut catalog = crate::lazy_features::LazyFeatures::new(root.path().into(), false);
        catalog.paths = vec![
            root.path().join("a/one.feature"),
            root.path().join("z.feature"),
        ];
        catalog.selected = Some(catalog.paths[1].clone());
        app.feature_discovery_root = Some(root.path().into());
        app.lazy_features = Some(catalog);
        app.git_status = Default::default();
        assert_eq!(app.feature_browser_selected_row(), Some(1));
        app.feature_browser.expanded.insert("a".into());
        assert_eq!(app.feature_browser_selected_row(), Some(2));
        app.feature_browser.query = "ONE".into();
        assert_eq!(app.feature_browser_selected_row(), None);
        app.feature_browser.directory = Some("a".into());
        assert_eq!(app.feature_browser_selected_row(), Some(0));
        app.feature_browser.directory = None;
        app.feature_browser.query.clear();
        let catalog = app.lazy_features.as_mut().unwrap();
        catalog.paths.insert(0, root.path().join("a/added.feature"));
        catalog.revision += 1;
        assert_eq!(app.feature_browser_selected_row(), Some(3));
        app.feature_browser.query = "added".into();
        let rows = app.feature_browser_rows();
        assert!(matches!(rows.last(), Some(Row::File(0, _, false))));
    }

    #[test]
    fn lazy_tree_survives_selected_content_changes() {
        let mut app = App::from_args().unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut catalog = crate::lazy_features::LazyFeatures::new(root.path().into(), false);
        catalog.paths = vec![
            root.path().join("one.feature"),
            root.path().join("two.feature"),
        ];
        app.feature_discovery_root = Some(root.path().into());
        app.lazy_features = Some(catalog);
        app.git_status = Default::default();
        let initial = app.feature_browser_rows();
        app.project.features = vec![teshi_core::gherkin::parse_feature(
            "Feature: Two",
            root.path().join("two.feature"),
        )];
        assert!(Rc::ptr_eq(&initial, &app.feature_browser_rows()));
        let catalog = app.lazy_features.as_mut().unwrap();
        catalog.paths.push(root.path().join("three.feature"));
        catalog.revision += 1;
        assert_eq!(app.feature_browser_rows().len(), 3);
        assert!(!Rc::ptr_eq(&initial, &app.feature_browser_rows()));
    }

    #[test]
    fn search_coalesces_input_and_commits_explicit_actions() {
        let mut app = App::from_args().unwrap();
        app.active_tab = MainTab::Explore;
        app.explore_focus = ColumnFocus::Feature;
        app.project.features = vec![
            teshi_core::gherkin::parse_feature("Feature: Auth", "auth.feature".into()),
            teshi_core::gherkin::parse_feature("Feature: Other", "other.feature".into()),
        ];
        app.git_status = Default::default();
        let initial = app.feature_browser_rows();
        app.feature_browser.searching = true;
        app.paste_feature_browser_search("auth");
        assert_eq!(app.feature_browser.query, "auth");
        assert!(Rc::ptr_eq(&initial, &app.feature_browser_rows()));
        app.poll_feature_browser_search();
        assert!(Rc::ptr_eq(&initial, &app.feature_browser_rows()));
        app.feature_browser.pending_search = Some(Instant::now() - SEARCH_DEBOUNCE);
        app.poll_feature_browser_search();
        assert_eq!(app.feature_browser_rows().len(), 1);
        app.handle_feature_browser_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.feature_browser_rows().len(), 2);
        app.feature_browser.searching = true;
        app.paste_feature_browser_search("other");
        app.handle_feature_browser_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.feature_browser_rows().len(), 1);
        assert_eq!(app.feature_browser_filter_query(), "other");
        assert!(app.feature_browser.pending_search.is_none());
    }

    #[test]
    fn large_tree_reuses_rows_and_invalidates_changed_inputs() {
        let mut app = App::from_args().unwrap();
        app.project.features = (0..10_000)
            .map(|i| {
                teshi_core::gherkin::parse_feature(
                    "Feature: Example",
                    format!("features/group{}/file{i}.feature", i / 10).into(),
                )
            })
            .collect();
        app.git_status = Default::default();
        let initial = app.feature_browser_rows();
        assert_eq!(initial.len(), 1);
        assert!(Rc::ptr_eq(&initial, &app.feature_browser_rows()));
        app.project.features[0].name = "Renamed".into();
        let renamed = app.feature_browser_rows();
        assert!(!Rc::ptr_eq(&initial, &renamed));
        app.feature_browser.query = "Renamed".into();
        let filtered = app.feature_browser_rows();
        assert_eq!(
            filtered
                .iter()
                .filter(|r| matches!(r, Row::File(..)))
                .count(),
            1
        );
        assert!(Rc::ptr_eq(&filtered, &app.feature_browser_rows()));
        app.feature_browser.query.clear();
        app.feature_browser.expanded.remove("features");
        assert_eq!(app.feature_browser_rows().len(), 1);
    }

    #[test]
    fn tree_is_relative_to_selected_discovery_directory() {
        let mut app = App::from_args().unwrap();
        app.feature_discovery_root = Some("features/zh-CN".into());
        app.project.features = vec![
            teshi_core::gherkin::parse_feature(
                "Feature: Root",
                "features/zh-CN/root.feature".into(),
            ),
            teshi_core::gherkin::parse_feature(
                "Feature: Nested",
                "features/zh-CN/auth/login.feature".into(),
            ),
        ];
        app.git_status = Default::default();
        let rows = app.feature_browser_rows();
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[0], Row::Directory(path, 0) if path == "auth"));
        assert!(matches!(&rows[1], Row::File(_, name, false) if name == "root"));
    }

    #[test]
    fn nested_tree_search_and_collapse_preserve_file_indices() {
        let mut app = App::from_args().unwrap();
        app.active_tab = MainTab::Explore;
        app.explore_focus = ColumnFocus::Feature;
        app.project.features = vec![
            teshi_core::gherkin::parse_feature(
                "Feature: Login",
                "features/zh-CN/account/login.feature".into(),
            ),
            teshi_core::gherkin::parse_feature(
                "Feature: Checkout",
                "features/zh-CN/shop/checkout.feature".into(),
            ),
        ];
        app.git_status = Default::default();
        assert_eq!(app.feature_browser_rows().len(), 1);
        app.explore_set_feature_row(0);
        app.handle_feature_browser_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.feature_browser_rows().len(), 2);
        app.feature_browser.expanded.extend([
            "features/zh-CN".into(),
            "features/zh-CN/account".into(),
            "features/zh-CN/shop".into(),
        ]);
        assert_eq!(app.feature_browser_rows().len(), 6);
        app.explore_set_feature_row(2);
        assert!(app.handle_feature_browser_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
        assert_eq!(app.feature_browser_rows().len(), 5);
        assert!(
            app.handle_feature_browser_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        );
        for ch in "LOGIN".chars() {
            assert!(
                app.handle_feature_browser_key(KeyEvent::new(
                    KeyCode::Char(ch),
                    KeyModifiers::NONE
                ))
            );
        }
        app.feature_browser.pending_search = Some(Instant::now() - SEARCH_DEBOUNCE);
        app.poll_feature_browser_search();
        let rows = app.feature_browser_rows();
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows.last(), Some(Row::File(0, _, false))));
        app.explore_set_feature_row(3);
        assert_eq!(app.explore_selected_feature, 0);
        app.handle_feature_browser_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.feature_browser.searching);
        app.handle_feature_browser_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.feature_browser.query.is_empty());
        assert_eq!(app.feature_browser_rows().len(), 5);
    }
}
