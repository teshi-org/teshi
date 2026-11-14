use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::{Duration, Instant};

const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, ColumnFocus, MainTab};
use crate::authoring_tab::{CursorMotion, LineInput};

#[derive(Default)]
pub(crate) struct FeatureBrowser {
    pub(crate) query: LineInput,
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

/// One visible tree row; the trailing `usize` is the row's tree depth.
#[derive(Clone)]
pub(crate) enum Row {
    Directory(String, usize),
    File(usize, String, bool, usize),
}

impl Row {
    fn depth(&self) -> usize {
        match self {
            Row::Directory(_, depth) | Row::File(_, _, _, depth) => *depth,
        }
    }
}

impl App {
    fn feature_browser_filter_query(&self) -> &str {
        if self.feature_browser.pending_search.is_some() {
            &self.feature_browser.applied_query
        } else if self.feature_browser.query.text().trim().is_empty() {
            ""
        } else {
            self.feature_browser.query.text()
        }
    }

    fn commit_feature_browser_search(&mut self) {
        self.feature_browser.pending_search = None;
        self.feature_browser.applied_query = self.feature_browser.query.text().to_string();
        if self.explore_selected_feature_row().is_none() {
            self.explore_set_feature_row(0);
        }
    }

    fn schedule_feature_browser_search(&mut self) {
        if self.feature_browser.query.text().trim().is_empty() {
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
        for ch in text.chars().filter(|ch| !ch.is_control()) {
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
                Row::File(index, _, deleted, _) => {
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
                    Row::File(_, label, _, _) => 4 + label.width(),
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
                    parts.len().saturating_sub(1),
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
            // Control chords are commands, not query text. Leave input mode
            // before handing them to the keymap so a following chord key or a
            // focus change never sees a stale search state.
            if event
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                self.feature_browser.searching = false;
                self.commit_feature_browser_search();
                return false;
            }
            match event.code {
                KeyCode::Esc => {
                    self.feature_browser.query = LineInput::default();
                    self.feature_browser.searching = false;
                }
                KeyCode::Enter => self.feature_browser.searching = false,
                KeyCode::Backspace => self.feature_browser.query.backspace(),
                KeyCode::Delete => self.feature_browser.query.delete(),
                KeyCode::Left => self.feature_browser.query.move_cursor(CursorMotion::Left),
                KeyCode::Right => self.feature_browser.query.move_cursor(CursorMotion::Right),
                KeyCode::Home => self.feature_browser.query.move_cursor(CursorMotion::Home),
                KeyCode::End => self.feature_browser.query.move_cursor(CursorMotion::End),
                KeyCode::Char(ch) => self.feature_browser.query.insert(ch),
                KeyCode::Up | KeyCode::Down => return false,
                _ => return true,
            }
            match event.code {
                KeyCode::Esc | KeyCode::Enter => self.commit_feature_browser_search(),
                KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_) => {
                    self.schedule_feature_browser_search()
                }
                // Cursor motion only moves the caret; the query is unchanged.
                _ => {}
            }
            return true;
        }
        match event.code {
            KeyCode::Char('/') if event.modifiers.is_empty() => {
                self.feature_browser.searching = true;
                self.feature_browser.query.move_cursor(CursorMotion::End);
                true
            }
            KeyCode::Esc if !self.feature_browser.query.text().is_empty() => {
                self.feature_browser.query = LineInput::default();
                self.commit_feature_browser_search();
                true
            }
            KeyCode::Enter => {
                let Some(path) = self.feature_browser.directory.clone() else {
                    return false;
                };
                if !self.feature_browser.expanded.remove(&path) {
                    self.feature_browser.expanded.insert(path);
                }
                true
            }
            // The second key of a Ctrl+W chord changes panes, not tree rows.
            _ if self.pending_char == Some('w') => false,
            KeyCode::Left => self.feature_browser_step_out(),
            KeyCode::Char('h') if event.modifiers.is_empty() => self.feature_browser_step_out(),
            KeyCode::Right => self.feature_browser_step_in(),
            KeyCode::Char('l') if event.modifiers.is_empty() => self.feature_browser_step_in(),
            _ => false,
        }
    }

    /// Expands a collapsed directory, or selects the first child of an open one.
    ///
    /// Returns false on Feature rows so the shared keymap still moves focus to
    /// the Scenario column.
    fn feature_browser_step_in(&mut self) -> bool {
        let Some(path) = self.feature_browser.directory.clone() else {
            return false;
        };
        self.pending_char = None;
        if self.feature_directory_collapsed(&path) {
            self.feature_browser.expanded.insert(path);
            return true;
        }
        let rows = self.feature_browser_rows();
        if let Some(row) = self.feature_browser_selected_row()
            && rows
                .get(row + 1)
                .is_some_and(|child| child.depth() > rows[row].depth())
        {
            self.explore_set_feature_row(row + 1);
        }
        true
    }

    /// Collapses an open directory, or selects the parent of the current row.
    ///
    /// Returns false for top-level collapsed rows so the key keeps its column
    /// navigation meaning.
    fn feature_browser_step_out(&mut self) -> bool {
        if let Some(path) = self.feature_browser.directory.clone()
            && self.feature_browser_filter_query().is_empty()
            && self.feature_browser.expanded.remove(&path)
        {
            self.pending_char = None;
            return true;
        }
        let Some(row) = self.feature_browser_selected_row() else {
            return false;
        };
        let rows = self.feature_browser_rows();
        let depth = rows[row].depth();
        // Rows are emitted depth-first, so every row between a directory and
        // one of its descendants is deeper than that directory.
        let Some(parent) = rows[..row].iter().rposition(|item| item.depth() < depth) else {
            return false;
        };
        self.pending_char = None;
        self.explore_set_feature_row(parent);
        true
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
        app.feature_browser.query = LineInput::new("ONE");
        assert_eq!(app.feature_browser_selected_row(), None);
        app.feature_browser.directory = Some("a".into());
        assert_eq!(app.feature_browser_selected_row(), Some(0));
        app.feature_browser.directory = None;
        app.feature_browser.query = LineInput::default();
        let catalog = app.lazy_features.as_mut().unwrap();
        catalog.paths.insert(0, root.path().join("a/added.feature"));
        catalog.revision += 1;
        assert_eq!(app.feature_browser_selected_row(), Some(3));
        app.feature_browser.query = LineInput::new("added");
        let rows = app.feature_browser_rows();
        assert!(matches!(rows.last(), Some(Row::File(0, _, false, _))));
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
        assert_eq!(app.feature_browser.query.text(), "auth");
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
        app.feature_browser.query = LineInput::new("Renamed");
        let filtered = app.feature_browser_rows();
        assert_eq!(
            filtered
                .iter()
                .filter(|r| matches!(r, Row::File(..)))
                .count(),
            1
        );
        assert!(Rc::ptr_eq(&filtered, &app.feature_browser_rows()));
        app.feature_browser.query = LineInput::default();
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
        assert!(matches!(&rows[1], Row::File(_, name, false, 0) if name == "root"));
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
        assert!(matches!(rows.last(), Some(Row::File(0, _, false, _))));
        app.explore_set_feature_row(3);
        assert_eq!(app.explore_selected_feature, 0);
        app.handle_feature_browser_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.feature_browser.searching);
        app.handle_feature_browser_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.feature_browser.query.text().is_empty());
        assert_eq!(app.feature_browser_rows().len(), 5);
    }

    #[test]
    fn test_hl_and_arrows_walk_tree_levels_identically() {
        for (out, into) in [
            (KeyCode::Char('h'), KeyCode::Char('l')),
            (KeyCode::Left, KeyCode::Right),
        ] {
            let mut app = App::from_args().unwrap();
            app.active_tab = MainTab::Explore;
            app.explore_focus = ColumnFocus::Feature;
            app.project.features = vec![
                teshi_core::gherkin::parse_feature("Feature: Alpha", "suite/alpha.feature".into()),
                teshi_core::gherkin::parse_feature(
                    "Feature: Beta",
                    "suite/nested/beta.feature".into(),
                ),
            ];
            app.git_status = Default::default();
            let press = |app: &mut App, code| {
                app.handle_feature_browser_key(KeyEvent::new(code, KeyModifiers::NONE))
            };
            app.explore_set_feature_row(0);

            assert!(
                press(&mut app, into),
                "{into:?} expands a collapsed directory"
            );
            assert_eq!(app.feature_browser_rows().len(), 3);
            assert!(
                press(&mut app, into),
                "{into:?} steps into an open directory"
            );
            assert_eq!(app.feature_browser_selected_row(), Some(1));
            assert_eq!(app.explore_selected_feature, 0);
            assert!(
                !press(&mut app, into),
                "{into:?} on a Feature falls through to the Scenario column"
            );

            assert!(press(&mut app, out), "{out:?} selects the parent directory");
            assert_eq!(app.feature_browser.directory.as_deref(), Some("suite"));

            app.explore_set_feature_row(2);
            assert!(press(&mut app, into), "{into:?} expands a nested directory");
            assert!(
                press(&mut app, into),
                "{into:?} steps into the nested directory"
            );
            assert_eq!(app.explore_selected_feature, 1);
            assert!(press(&mut app, out));
            assert_eq!(
                app.feature_browser.directory.as_deref(),
                Some("suite/nested")
            );
            assert!(press(&mut app, out), "{out:?} collapses an open directory");
            assert_eq!(app.feature_browser_rows().len(), 3);
            assert!(press(&mut app, out));
            assert_eq!(app.feature_browser.directory.as_deref(), Some("suite"));
            assert!(press(&mut app, out));
            assert_eq!(app.feature_browser_rows().len(), 1);
            assert!(
                !press(&mut app, out),
                "{out:?} on a collapsed top-level row keeps column navigation"
            );

            app.pending_char = Some('w');
            assert!(
                !press(&mut app, into),
                "{into:?} completes the Ctrl+W chord instead of expanding"
            );
        }
    }
}
