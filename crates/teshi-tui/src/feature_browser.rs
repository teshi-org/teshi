use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, ColumnFocus, MainTab};

#[derive(Default)]
pub(crate) struct FeatureBrowser {
    pub(crate) query: String,
    pub(crate) searching: bool,
    pub(crate) directory: Option<String>,
    collapsed: BTreeSet<String>,
}

#[derive(Clone)]
pub(crate) enum Row {
    Directory(String, usize),
    File(usize, String, bool),
}

impl App {
    pub(crate) fn feature_browser_rows(&self) -> Vec<Row> {
        let mut files = BTreeMap::new();
        let display_root = self
            .feature_discovery_root
            .as_deref()
            .unwrap_or(&self.project.root_dir);
        let query = self.feature_browser.query.to_lowercase();
        for (index, feature) in self.project.features.iter().enumerate() {
            let path = feature
                .file_path
                .strip_prefix(display_root)
                .unwrap_or(&feature.file_path)
                .to_string_lossy()
                .replace('\\', "/");
            if query.is_empty()
                || path.to_lowercase().contains(&query)
                || feature.name.to_lowercase().contains(&query)
            {
                files.insert((path, false), index);
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
            if query.is_empty() || path.to_lowercase().contains(&query) {
                files.insert((path, true), index);
            }
        }
        let mut rows = Vec::new();
        let mut emitted = BTreeSet::new();
        for ((path, deleted), index) in files {
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
                if query.is_empty() && self.feature_browser.collapsed.contains(&directory) {
                    hidden = true;
                    break;
                }
            }
            if !hidden {
                rows.push(Row::File(
                    index,
                    format!(
                        "{}{}",
                        "  ".repeat(parts.len().saturating_sub(1)),
                        parts
                            .last()
                            .unwrap_or(&"")
                            .strip_suffix(".feature")
                            .unwrap_or(parts.last().unwrap_or(&""))
                    ),
                    deleted,
                ));
            }
        }
        rows
    }

    pub(crate) fn feature_directory_collapsed(&self, path: &str) -> bool {
        self.feature_browser.query.is_empty() && self.feature_browser.collapsed.contains(path)
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
            if self.explore_selected_feature_row().is_none() {
                self.explore_set_feature_row(0);
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
                true
            }
            KeyCode::Enter | KeyCode::Left | KeyCode::Right => {
                let Some(path) = self.feature_browser.directory.clone() else {
                    return false;
                };
                match event.code {
                    KeyCode::Left => {
                        self.feature_browser.collapsed.insert(path);
                    }
                    KeyCode::Right => {
                        self.feature_browser.collapsed.remove(&path);
                    }
                    _ => {
                        if !self.feature_browser.collapsed.remove(&path) {
                            self.feature_browser.collapsed.insert(path);
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
        assert_eq!(rows.len(), 3);
        assert!(matches!(&rows[0], Row::Directory(path, 0) if path == "auth"));
        assert!(matches!(&rows[2], Row::File(_, name, false) if name == "root"));
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
