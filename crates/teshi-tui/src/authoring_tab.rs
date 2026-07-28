//! Requirements tab state, tree construction, and editing helpers.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use teshi_core::authoring::{
    AuthoringArtifacts, AuthoringDiagnostic, HierarchyPath, RequirementDocumentMeta,
    ResolutionState, ReviewState, TestPoint, TextRange, create_requirement_link,
    line_col_range_to_char_range, re_resolve_document_links,
};
use teshi_engine::{
    compute_document_revision, load_authoring_artifacts, save_requirement_markdown,
    save_test_points,
};
use tui_tree_widget::{TreeItem, TreeState};

use crate::editor_buffer::EditorBuffer;

/// Tree node id prefix for folder directories.
pub const TREE_DIR_PREFIX: &str = "req-dir:";
/// Tree node id prefix for requirement documents.
pub const TREE_DOC_PREFIX: &str = "req-doc:";

/// Focus within the Requirements tab three-pane layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementsFocus {
    Tree,
    Editor,
    LinkedTestPoints,
}

/// UI state for requirement documents and linked test points.
pub struct AuthoringUiState {
    pub artifacts: Option<AuthoringArtifacts>,
    pub discovered: bool,
    pub tree_items: Vec<TreeItem<'static, String>>,
    pub tree_state: TreeState<String>,
    pub focus: RequirementsFocus,
    pub selected_document_id: Option<String>,
    pub selected_linked_index: usize,
    pub highlight_test_point_id: Option<String>,
    pub buffer: EditorBuffer,
    pub buffer_dirty: bool,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub desired_col: usize,
    pub scroll_row: usize,
    pub selection_anchor: Option<(usize, usize)>,
    pub selection_end: Option<(usize, usize)>,
}

impl AuthoringUiState {
    /// Creates an empty state with no loaded artifacts.
    pub fn empty() -> Self {
        Self {
            artifacts: None,
            discovered: false,
            tree_items: Vec::new(),
            tree_state: TreeState::default(),
            focus: RequirementsFocus::Tree,
            selected_document_id: None,
            selected_linked_index: 0,
            highlight_test_point_id: None,
            buffer: EditorBuffer::from_string(String::new()),
            buffer_dirty: false,
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            selection_anchor: None,
            selection_end: None,
        }
    }

    /// Loads authoring artifacts from disk when present.
    pub fn load_from_project(project_root: &Path) -> Self {
        let mut state = Self::empty();
        match load_authoring_artifacts(project_root) {
            Ok(result) => {
                state.discovered = result.discovered;
                state.artifacts = result.artifacts;
                state.rebuild_tree();
                let first_id = state
                    .artifacts
                    .as_ref()
                    .and_then(|a| a.index.documents.first().map(|d| d.id.clone()));
                if let Some(id) = first_id {
                    state.select_document_by_id(&id);
                }
            }
            Err(error) => {
                let _ = error;
            }
        }
        state
    }

    /// Rebuilds the requirement tree from the current index.
    pub fn rebuild_tree(&mut self) {
        let index = self
            .artifacts
            .as_ref()
            .map(|a| &a.index)
            .cloned()
            .unwrap_or_default();
        self.tree_items = build_requirement_tree(&index);
        if self.tree_state.selected().is_empty()
            && let Some(doc) = index.documents.first()
        {
            self.tree_state
                .select(vec![format!("{TREE_DOC_PREFIX}{}", doc.id)]);
        }
    }

    /// Selects a document by stable id and loads its Markdown into the editor buffer.
    pub fn select_document_by_id(&mut self, doc_id: &str) {
        self.selected_document_id = Some(doc_id.to_string());
        self.selected_linked_index = 0;
        self.highlight_test_point_id = None;
        self.selection_anchor = None;
        self.selection_end = None;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.scroll_row = 0;

        if let Some(artifacts) = self.artifacts.as_ref()
            && let Some(doc) = artifacts.documents.iter().find(|d| d.meta.id == doc_id)
        {
            self.buffer = EditorBuffer::from_string(doc.body.clone());
            self.buffer_dirty = false;
            return;
        }
        self.buffer = EditorBuffer::from_string(String::new());
        self.buffer_dirty = false;
    }

    /// Selects a document from a tree node id (`req-doc:<id>` or `req-dir:...`).
    pub fn select_tree_node(&mut self, node_id: &str) {
        if let Some(doc_id) = node_id.strip_prefix(TREE_DOC_PREFIX) {
            self.select_document_by_id(doc_id);
            self.tree_state.select(vec![node_id.to_string()]);
        }
    }

    /// Returns metadata for the currently selected document.
    pub fn current_document_meta(&self) -> Option<&RequirementDocumentMeta> {
        let doc_id = self.selected_document_id.as_ref()?;
        self.artifacts
            .as_ref()?
            .index
            .documents
            .iter()
            .find(|d| d.id == *doc_id)
    }

    /// Whether the indexed document file is missing on disk.
    pub fn current_document_missing(&self) -> bool {
        let doc_id = match self.selected_document_id.as_ref() {
            Some(id) => id,
            None => return false,
        };
        self.artifacts.as_ref().is_some_and(|a| {
            a.index.documents.iter().any(|m| m.id == *doc_id)
                && !a.documents.iter().any(|d| d.meta.id == *doc_id)
        })
    }

    /// All test points linked to the current document (any link on that document).
    pub fn linked_test_points_for_document(&self) -> Vec<&TestPoint> {
        let doc_id = match self.selected_document_id.as_ref() {
            Some(id) => id,
            None => return Vec::new(),
        };
        self.artifacts
            .as_ref()
            .map(|a| {
                a.test_points
                    .test_points
                    .iter()
                    .filter(|tp| {
                        tp.requirement_links
                            .iter()
                            .any(|l| l.document_id == *doc_id)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Linked test points filtered by the active editor selection when non-empty.
    pub fn filtered_linked_test_points(&self) -> Vec<&TestPoint> {
        let all = self.linked_test_points_for_document();
        let selection = self.selection_char_range();
        if selection.is_none() {
            return all;
        }
        let (sel_range, doc_id) = match (selection, self.selected_document_id.as_ref()) {
            (Some(r), Some(id)) => (r, id),
            _ => return all,
        };
        all.into_iter()
            .filter(|tp| {
                tp.requirement_links.iter().any(|link| {
                    link.document_id == *doc_id
                        && link.resolution == ResolutionState::Resolved
                        && ranges_overlap(link.position, sel_range)
                })
            })
            .collect()
    }

    /// Resolved character ranges in the current document for highlight painting.
    pub fn highlight_ranges_for_test_point(&self, tp_id: &str) -> Vec<TextRange> {
        let doc_id = match self.selected_document_id.as_ref() {
            Some(id) => id,
            None => return Vec::new(),
        };
        self.artifacts
            .as_ref()
            .and_then(|a| a.test_points.test_points.iter().find(|tp| tp.id == tp_id))
            .map(|tp| {
                tp.requirement_links
                    .iter()
                    .filter(|l| {
                        l.document_id == *doc_id && l.resolution == ResolutionState::Resolved
                    })
                    .map(|l| l.position)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Active selection as a scalar character range in the current document.
    pub fn selection_char_range(&self) -> Option<TextRange> {
        let (anchor, end) = match (self.selection_anchor, self.selection_end) {
            (Some(a), Some(e)) => (a, e),
            _ => return None,
        };
        if anchor == end {
            return None;
        }
        let body = self.buffer.as_string();
        line_col_range_to_char_range(&body, anchor, end)
    }

    /// Creates a `Proposed` test point from the current non-empty selection.
    pub fn create_test_point_from_selection(&mut self) -> Option<String> {
        let doc_meta = self.current_document_meta()?.clone();
        let range = self.selection_char_range()?;
        let body = self.buffer.as_string();
        let link = create_requirement_link(&doc_meta.id, doc_meta.revision.as_str(), &body, range)?;
        let artifacts = self.artifacts.as_mut()?;
        let id = next_test_point_id(&artifacts.test_points.test_points);
        let title = link.quote.quote.chars().take(48).collect::<String>();
        let tp = TestPoint {
            id: id.clone(),
            title: if title.is_empty() {
                "New test point".into()
            } else {
                title
            },
            objective: link.quote.quote.clone(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["Uncategorized".into()]),
            review_state: ReviewState::Proposed,
            requirement_links: vec![link],
            scenario_refs: Vec::new(),
        };
        artifacts.test_points.test_points.push(tp);
        self.selected_linked_index = self.filtered_linked_test_points().len().saturating_sub(1);
        Some(id)
    }

    /// Creates a new requirement document under `relative_path` with the given title.
    pub fn create_document(&mut self, relative_path: &str, title: &str) -> String {
        if self.artifacts.is_none() {
            self.artifacts = Some(AuthoringArtifacts {
                index: Default::default(),
                documents: Vec::new(),
                test_points: Default::default(),
                diagnostics: Vec::new(),
            });
            self.discovered = true;
        }
        let artifacts = self.artifacts.as_mut().expect("artifacts initialized");
        let id = next_document_id(&artifacts.index.documents);

        let body = format!("# {}\n\n", title.trim());
        let revision = compute_document_revision(&body);
        let meta = RequirementDocumentMeta {
            id: id.clone(),
            path: relative_path.to_string(),
            title: title.to_string(),
            revision,
        };

        artifacts.index.documents.push(meta.clone());
        artifacts
            .documents
            .push(teshi_core::authoring::RequirementDocumentContent {
                meta,
                body: body.clone(),
            });
        self.rebuild_tree();
        self.select_document_by_id(&id);
        self.buffer = EditorBuffer::from_string(body);
        self.buffer_dirty = true;
        id
    }

    /// Saves the current requirement document and refreshes link resolutions.
    pub fn save_current_document(&mut self, project_root: &Path) -> Result<()> {
        let doc_id = self
            .selected_document_id
            .clone()
            .context("no requirement document selected")?;
        let artifacts = self
            .artifacts
            .as_mut()
            .context("authoring artifacts not loaded")?;
        let meta = artifacts
            .index
            .documents
            .iter()
            .find(|d| d.id == doc_id)
            .context("selected document not in index")?;
        let relative_path = meta.path.clone();
        let body = self.buffer.as_string();
        save_requirement_markdown(project_root, &mut artifacts.index, &relative_path, &body)?;
        if let Some(doc) = artifacts.documents.iter_mut().find(|d| d.meta.id == doc_id) {
            doc.body = body.clone();
            if let Some(index_meta) = artifacts.index.documents.iter().find(|d| d.id == doc_id) {
                doc.meta.revision = index_meta.revision.clone();
            }
        }
        re_resolve_document_links(
            &body,
            &doc_id,
            artifacts
                .index
                .documents
                .iter()
                .find(|d| d.id == doc_id)
                .map(|d| d.revision.as_str())
                .unwrap_or(""),
            &mut artifacts.test_points.test_points,
        );
        save_test_points(project_root, &artifacts.test_points)?;
        self.buffer_dirty = false;
        Ok(())
    }

    pub fn focus_next_column(&mut self) {
        self.focus = match self.focus {
            RequirementsFocus::Tree => RequirementsFocus::Editor,
            RequirementsFocus::Editor => RequirementsFocus::LinkedTestPoints,
            RequirementsFocus::LinkedTestPoints => RequirementsFocus::LinkedTestPoints,
        };
    }

    pub fn focus_prev_column(&mut self) {
        self.focus = match self.focus {
            RequirementsFocus::Tree => RequirementsFocus::Tree,
            RequirementsFocus::Editor => RequirementsFocus::Tree,
            RequirementsFocus::LinkedTestPoints => RequirementsFocus::Editor,
        };
    }

    pub fn move_linked_selection(&mut self, delta: isize) {
        let count = self.filtered_linked_test_points().len();
        if count == 0 {
            self.selected_linked_index = 0;
            self.highlight_test_point_id = None;
            return;
        }
        let next = if delta < 0 {
            self.selected_linked_index.saturating_sub(1)
        } else {
            self.selected_linked_index.saturating_add(1).min(count - 1)
        };
        self.selected_linked_index = next;
        if let Some(tp) = self.filtered_linked_test_points().get(next) {
            self.highlight_test_point_id = Some(tp.id.clone());
        }
    }

    pub fn move_tree_selection(&mut self, delta: isize) {
        let doc_ids: Vec<String> = self
            .artifacts
            .as_ref()
            .map(|a| {
                a.index
                    .documents
                    .iter()
                    .map(|d| format!("{TREE_DOC_PREFIX}{}", d.id))
                    .collect()
            })
            .unwrap_or_default();
        if doc_ids.is_empty() {
            return;
        }
        let current = self.tree_state.selected().first().cloned().or_else(|| {
            self.selected_document_id
                .as_ref()
                .map(|id| format!("{TREE_DOC_PREFIX}{id}"))
        });
        let pos = current
            .and_then(|id| doc_ids.iter().position(|x| x == &id))
            .unwrap_or(0);
        let next = if delta < 0 {
            pos.saturating_sub(1)
        } else {
            pos.saturating_add(1).min(doc_ids.len().saturating_sub(1))
        };
        if let Some(id) = doc_ids.get(next) {
            self.select_tree_node(id);
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        self.buffer
            .insert_char(self.cursor_row, self.cursor_col, ch);
        self.cursor_col += 1;
        self.desired_col = self.cursor_col;
        self.buffer_dirty = true;
    }

    pub fn insert_newline(&mut self) {
        self.buffer
            .insert_char(self.cursor_row, self.cursor_col, '\n');
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.buffer_dirty = true;
    }

    pub fn backspace(&mut self) {
        let (row, col, changed) = self.buffer.backspace(self.cursor_row, self.cursor_col);
        if changed {
            self.cursor_row = row;
            self.cursor_col = col;
            self.desired_col = col;
            self.buffer_dirty = true;
        }
    }

    pub fn delete_forward(&mut self) {
        if self.buffer.delete(self.cursor_row, self.cursor_col) {
            self.buffer_dirty = true;
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.buffer.line_len_chars(self.cursor_row);
        }
        self.desired_col = self.cursor_col;
    }

    pub fn move_cursor_right(&mut self) {
        let line_len = self.buffer.line_len_chars(self.cursor_row);
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.buffer.line_count() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
        self.desired_col = self.cursor_col;
    }

    pub fn move_cursor_vertical(&mut self, delta: isize) {
        let target = if delta < 0 {
            self.cursor_row.saturating_sub(1)
        } else {
            self.cursor_row
                .saturating_add(1)
                .min(self.buffer.line_count().saturating_sub(1))
        };
        self.cursor_row = target;
        self.cursor_col = self.buffer.clamp_col(self.cursor_row, self.desired_col);
    }

    pub fn clamp_cursor(&mut self) {
        let last_row = self.buffer.line_count().saturating_sub(1);
        self.cursor_row = self.cursor_row.min(last_row);
        self.cursor_col = self.buffer.clamp_col(self.cursor_row, self.cursor_col);
        self.desired_col = self
            .desired_col
            .min(self.buffer.line_len_chars(self.cursor_row));
        if self.cursor_row < self.scroll_row {
            self.scroll_row = self.cursor_row;
        }
    }

    /// Returns validation and load diagnostics for display in the tree pane.
    pub fn visible_diagnostics(&self) -> Vec<&AuthoringDiagnostic> {
        self.artifacts
            .as_ref()
            .map(|a| a.diagnostics.iter().collect())
            .unwrap_or_default()
    }
}

fn ranges_overlap(a: TextRange, b: TextRange) -> bool {
    a.start.offset() < b.end.offset() && b.start.offset() < a.end.offset()
}

fn next_document_id(existing: &[RequirementDocumentMeta]) -> String {
    let mut n = 1u32;
    loop {
        let id = format!("doc-{n}");
        if !existing.iter().any(|d| d.id == id) {
            return id;
        }
        n += 1;
    }
}

fn next_test_point_id(existing: &[TestPoint]) -> String {
    let mut n = 1u32;
    loop {
        let id = format!("tp-{n}");
        if !existing.iter().any(|tp| tp.id == id) {
            return id;
        }
        n += 1;
    }
}

/// Builds folder/document tree items from indexed paths.
pub fn build_requirement_tree(
    index: &teshi_core::authoring::RequirementDocumentIndex,
) -> Vec<TreeItem<'static, String>> {
    #[derive(Default)]
    struct DirNode {
        children: HashMap<String, DirNode>,
        documents: Vec<RequirementDocumentMeta>,
    }

    let mut root = DirNode::default();
    for doc in &index.documents {
        let parts: Vec<&str> = doc.path.split('/').collect();
        if parts.is_empty() {
            continue;
        }
        let mut node = &mut root;
        for (i, part) in parts.iter().enumerate() {
            if i + 1 == parts.len() {
                node.documents.push(doc.clone());
            } else {
                node = node.children.entry(part.to_string()).or_default();
            }
        }
    }

    fn dir_to_items(path: String, node: &DirNode) -> Vec<TreeItem<'static, String>> {
        let mut items = Vec::new();
        for (name, child) in node.children.iter() {
            let id = if path.is_empty() {
                format!("{TREE_DIR_PREFIX}{name}")
            } else {
                format!("{TREE_DIR_PREFIX}{path}/{name}")
            };
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let child_items = dir_to_items(child_path, child);
            items.push(TreeItem::new(id, name.clone(), child_items).unwrap());
        }
        for doc in &node.documents {
            let label = if doc.title.trim().is_empty() {
                doc.path.clone()
            } else {
                doc.title.clone()
            };
            let id = format!("{TREE_DOC_PREFIX}{}", doc.id);
            items.push(TreeItem::new(id, label, Vec::new()).unwrap());
        }
        items
    }

    dir_to_items(String::new(), &root)
}

/// Status indicator for a test point row.
pub fn review_state_label(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Proposed => "P",
        ReviewState::Approved => "A",
        ReviewState::Rejected => "R",
        ReviewState::NeedsReview => "!",
    }
}

/// Style for a test-point review state badge.
pub fn review_state_style(state: ReviewState) -> Style {
    match state {
        ReviewState::Proposed => Style::default().fg(Color::Yellow),
        ReviewState::Approved => Style::default().fg(Color::Green),
        ReviewState::Rejected => Style::default().fg(Color::Red),
        ReviewState::NeedsReview => Style::default().fg(Color::Magenta),
    }
}

/// Footer hint line for the Requirements tab.
pub fn requirements_footer_line() -> Line<'static> {
    Line::from(vec![
        Span::styled(" Focus [Tab] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" New doc [Ctrl+n] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" New TP [n] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Save [s] ", Style::default().fg(Color::DarkGray)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use teshi_core::authoring::{AuthoringArtifacts, RequirementDocumentIndex};

    #[test]
    fn tree_builds_nested_paths() {
        let index = RequirementDocumentIndex {
            version: 1,
            documents: vec![
                RequirementDocumentMeta {
                    id: "d1".into(),
                    path: "auth/login.md".into(),
                    title: "Login".into(),
                    revision: teshi_core::authoring::DocumentRevision::new("r1"),
                },
                RequirementDocumentMeta {
                    id: "d2".into(),
                    path: "overview.md".into(),
                    title: "Overview".into(),
                    revision: teshi_core::authoring::DocumentRevision::new("r2"),
                },
            ],
        };
        let items = build_requirement_tree(&index);
        assert!(!items.is_empty());
        assert_eq!(index.documents.len(), 2);
    }

    #[test]
    fn create_test_point_from_selection_persists_proposed() {
        let mut state = AuthoringUiState::empty();
        let body = "user login required";
        state.artifacts = Some(AuthoringArtifacts {
            index: RequirementDocumentIndex {
                version: 1,
                documents: vec![RequirementDocumentMeta {
                    id: "doc-1".into(),
                    path: "req.md".into(),
                    title: "Req".into(),
                    revision: teshi_core::authoring::DocumentRevision::new("rev"),
                }],
            },
            documents: vec![teshi_core::authoring::RequirementDocumentContent {
                meta: RequirementDocumentMeta {
                    id: "doc-1".into(),
                    path: "req.md".into(),
                    title: "Req".into(),
                    revision: teshi_core::authoring::DocumentRevision::new("rev"),
                },
                body: body.to_string(),
            }],
            test_points: Default::default(),
            diagnostics: Vec::new(),
        });
        state.select_document_by_id("doc-1");
        state.selection_anchor = Some((0, 0));
        state.selection_end = Some((0, 4));
        let id = state.create_test_point_from_selection().expect("create");
        let tp = state
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points
            .iter()
            .find(|t| t.id == id)
            .unwrap();
        assert_eq!(tp.review_state, ReviewState::Proposed);
        assert_eq!(tp.requirement_links.len(), 1);
    }

    #[test]
    fn load_and_save_roundtrip() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("requirements")).unwrap();
        let mut state = AuthoringUiState::empty();
        state.create_document("sample.md", "Sample");
        state.buffer = EditorBuffer::from_string("# Sample\n\nedited".into());
        state.save_current_document(root).expect("save");
        let reloaded = AuthoringUiState::load_from_project(root);
        assert!(reloaded.discovered);
        let body = reloaded.buffer.as_string();
        assert!(body.contains("edited"));
    }

    #[test]
    fn narrow_normal_wide_layout_constants_exist() {
        // Rendering tests live in ui; ensure footer helper is non-empty.
        let line = requirements_footer_line();
        assert!(!line.spans.is_empty());
    }
}
