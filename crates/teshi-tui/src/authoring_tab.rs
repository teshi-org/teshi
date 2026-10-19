//! Requirements tab state, tree construction, and editing helpers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::theme;
use anyhow::{Context, Result};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use teshi_core::authoring::{
    AuthoringArtifacts, AuthoringDiagnostic, HierarchyPath, RequirementDocumentMeta,
    RequirementGroupMode, RequirementIterationFilter, ResolutionState, ReviewState, TestPoint,
    TextRange, UNASSIGNED_ITERATION_LABEL, create_requirement_link, line_col_range_to_char_range,
    mark_document_links_missing, re_resolve_document_links, requirement_title,
};
use teshi_engine::{
    compute_document_revision, delete_requirement_document, initialize_requirement_store,
    is_requirement_document_id_reserved, load_authoring_artifacts, save_requirement_markdown,
    save_test_points,
};
use tui_tree_widget::{TreeItem, TreeState};

use crate::editor_buffer::EditorBuffer;

/// Tree node id prefix for folder directories.
pub const TREE_DIR_PREFIX: &str = "req-dir:";
/// Tree node id prefix for requirement documents.
pub const TREE_DOC_PREFIX: &str = "req-doc:";
/// Tree node id prefix for iteration group nodes.
pub const TREE_ITER_PREFIX: &str = "req-iter:";

/// Independent input mode for the Markdown editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequirementsEditorMode {
    #[default]
    Browse,
    Insert,
    VisualChar,
    VisualLine,
}

#[derive(Debug, Clone)]
struct RequirementsEditSnapshot {
    buffer: EditorBuffer,
    cursor_row: usize,
    cursor_col: usize,
    desired_col: usize,
    selection_anchor: Option<(usize, usize)>,
    selection_end: Option<(usize, usize)>,
    mode: RequirementsEditorMode,
}

/// Deferred navigation resumed after resolving unsaved Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementsPendingAction {
    Document(String),
    Filter(RequirementIterationFilter),
    Iteration { document_id: String, value: String },
    Action(crate::keymap::Action),
}

/// Overlay used to pick a filter or edit a document iteration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RequirementsOverlay {
    /// Resolve unsaved changes before navigation.
    Unsaved { pending: RequirementsPendingAction },
    /// No overlay.
    #[default]
    None,
    /// Choose All / Unassigned / a named iteration.
    FilterPicker {
        /// Highlighted option index.
        selection: usize,
    },
    /// Edit the selected document's iteration name.
    IterationEdit {
        /// Current input buffer.
        buffer: String,
    },
    /// Confirm removal of the selected document from the global library.
    DeleteConfirmation {
        /// Stable id being removed.
        document_id: String,
        /// Display title shown in the confirmation.
        title: String,
    },
}

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
    pub requirements_root: PathBuf,
    pub project_root: PathBuf,
    pub tree_items: Vec<TreeItem<'static, String>>,
    pub tree_state: TreeState<String>,
    pub focus: RequirementsFocus,
    /// Input mode independent of Explore.
    pub editor_mode: RequirementsEditorMode,
    /// Last rendered pane areas for mouse navigation and text selection.
    pub pane_areas: [ratatui::layout::Rect; 3],
    /// Whether a Markdown selection drag is in progress.
    pub selection_dragging: bool,
    pub selected_document_id: Option<String>,
    pub selected_linked_index: usize,
    pub highlight_test_point_id: Option<String>,
    pub buffer: EditorBuffer,
    pub buffer_dirty: bool,
    draft_document_id: Option<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub desired_col: usize,
    /// First visible soft-wrapped display row in the Markdown editor.
    pub scroll_row: usize,
    /// Display rows mapped to document row and character range, including an end cursor cell.
    pub editor_display_rows: Vec<(usize, usize, usize)>,
    /// Width used by the most recent editor wrap map.
    pub editor_wrap_width: usize,
    /// Horizontal offsets for the tree, editor, and linked test points.
    pub horizontal_scroll: [usize; 3],
    /// Cursor position at the previous render, used to follow keyboard navigation.
    pub rendered_cursor: Option<(usize, usize)>,
    /// Linked test point viewport start.
    pub linked_scroll: usize,
    /// Last rendered linked selection; wheel scrolling must not follow it again.
    pub rendered_linked_selection: Option<usize>,
    /// Last rendered overflow limits, indexed by pane and axis (horizontal, vertical).
    pub scroll_limits: [[usize; 2]; 3],
    /// Scrollbar being dragged (pane, axis).
    pub scrollbar_drag: Option<(usize, usize)>,
    pub selection_anchor: Option<(usize, usize)>,
    pub selection_end: Option<(usize, usize)>,
    pub iteration_filter: RequirementIterationFilter,
    pub group_mode: RequirementGroupMode,
    pub overlay: RequirementsOverlay,
    undo_stack: Vec<RequirementsEditSnapshot>,
    redo_stack: Vec<RequirementsEditSnapshot>,
    yank_register: String,
    insert_session_undo: bool,
    vertical_motion: Option<(usize, usize, usize, usize)>,
}

impl AuthoringUiState {
    /// Moves a pane viewport, clamping to the last rendered overflow limits.
    pub fn set_pane_scroll(&mut self, pane: usize, axis: usize, value: usize) {
        let value = value.min(self.scroll_limits[pane][axis]);
        if axis == 0 {
            self.horizontal_scroll[pane] = value;
        } else {
            match pane {
                0 => {
                    let old = self.tree_state.get_offset();
                    if value < old {
                        self.tree_state.scroll_up(old - value);
                    } else {
                        self.tree_state.scroll_down(value - old);
                    }
                }
                1 => {
                    self.scroll_row = value;
                    self.rendered_cursor = Some((self.cursor_row, self.cursor_col));
                }
                _ => {
                    self.linked_scroll = value;
                    self.selected_linked_index = value;
                }
            }
        }
    }

    /// Rebuilds soft-wrap coordinates without changing the document or its source ranges.
    pub fn wrap_editor_rows(&mut self, width: usize) {
        self.editor_display_rows.clear();
        let width = width.max(1);
        self.editor_wrap_width = width;
        for row in 0..self.buffer.line_count() {
            let text = self.buffer.line(row);
            let mut start = 0;
            let mut cells = 0;
            let mut end = 0;
            // Reserve a trailing cell so the insertion cursor remains visible at line end.
            for (col, ch) in text.chars().chain(std::iter::once(' ')).enumerate() {
                let size = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if cells + size > width && col > start {
                    self.editor_display_rows.push((row, start, col));
                    start = col;
                    cells = 0;
                }
                cells += size;
                end = col + 1;
            }
            self.editor_display_rows.push((row, start, end));
        }
    }

    /// Maps a viewport cell to document character coordinates for clicking and selection.
    pub fn editor_position_at(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        let &(row, start, end) = self.editor_display_rows.get(self.scroll_row + y)?;
        let text = self.buffer.line(row);
        let mut cells = 0;
        let mut col = start;
        for ch in text.chars().skip(start).take(end - start) {
            let size = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if cells + size > x {
                break;
            }
            cells += size;
            col += 1;
        }
        Some((row, col.min(text.chars().count())))
    }

    /// Enters text input when a document is selected.
    pub fn enter_insert_mode(&mut self) {
        if self.selected_document_id.is_some() {
            if !self.insert_session_undo {
                self.push_edit_undo();
                self.insert_session_undo = true;
            }
            self.editor_mode = RequirementsEditorMode::Insert;
        }
    }

    /// Returns to command navigation.
    pub fn exit_insert_mode(&mut self) {
        self.insert_session_undo = false;
        self.editor_mode = RequirementsEditorMode::Browse;
    }

    fn snapshot(&self) -> RequirementsEditSnapshot {
        RequirementsEditSnapshot {
            buffer: self.buffer.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            desired_col: self.desired_col,
            selection_anchor: self.selection_anchor,
            selection_end: self.selection_end,
            mode: self.editor_mode,
        }
    }

    fn restore_snapshot(&mut self, snapshot: RequirementsEditSnapshot) {
        self.vertical_motion = None;
        self.buffer = snapshot.buffer;
        self.cursor_row = snapshot.cursor_row;
        self.cursor_col = snapshot.cursor_col;
        self.desired_col = snapshot.desired_col;
        self.selection_anchor = snapshot.selection_anchor;
        self.selection_end = snapshot.selection_end;
        self.editor_mode = snapshot.mode;
        // A save can occur after the snapshot was taken. Its old dirty bit is
        // no longer evidence that this restored body matches persisted text.
        self.buffer_dirty = self.draft_document_id.is_some()
            || self.selected_document_id.as_ref().is_none_or(|id| {
                self.artifacts
                    .as_ref()
                    .and_then(|artifacts| artifacts.documents.iter().find(|doc| doc.meta.id == *id))
                    .is_none_or(|doc| doc.body != self.buffer.as_string())
            });
        self.insert_session_undo = false;
        self.clamp_cursor();
    }

    /// Record a document-local edit boundary and invalidate redo history.
    pub fn push_edit_undo(&mut self) {
        self.undo_stack.push(self.snapshot());
        self.redo_stack.clear();
    }

    /// Restore the previous document snapshot and recompute its saved-state difference.
    pub fn undo_edit(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            return;
        };
        let current = self.snapshot();
        self.redo_stack.push(current);
        self.restore_snapshot(snapshot);
    }

    /// Restore the next document-local snapshot after an undo.
    pub fn redo_edit(&mut self) {
        let Some(snapshot) = self.redo_stack.pop() else {
            return;
        };
        let current = self.snapshot();
        self.undo_stack.push(current);
        self.restore_snapshot(snapshot);
    }

    /// Start an inclusive character selection at the source cursor.
    pub fn enter_visual_char(&mut self) {
        self.exit_insert_mode();
        self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        let end = self
            .cursor_col
            .saturating_add(1)
            .min(self.buffer.line_len_chars(self.cursor_row));
        self.selection_end = Some((self.cursor_row, end));
        self.editor_mode = RequirementsEditorMode::VisualChar;
    }

    /// Start a selection of complete logical source lines.
    pub fn enter_visual_line(&mut self) {
        self.exit_insert_mode();
        self.selection_anchor = Some((self.cursor_row, 0));
        self.selection_end = Some((self.cursor_row, 0));
        self.editor_mode = RequirementsEditorMode::VisualLine;
    }

    /// Clear the visual selection and return to Browse mode.
    pub fn exit_visual_mode(&mut self) {
        self.selection_anchor = None;
        self.selection_end = None;
        self.editor_mode = RequirementsEditorMode::Browse;
    }

    /// Extend the active visual selection after a cursor motion.
    pub fn extend_visual_selection(&mut self) {
        if matches!(
            self.editor_mode,
            RequirementsEditorMode::VisualChar | RequirementsEditorMode::VisualLine
        ) {
            let col = if self.editor_mode == RequirementsEditorMode::VisualLine {
                0
            } else {
                self.cursor_col
                    .saturating_add(1)
                    .min(self.buffer.line_len_chars(self.cursor_row))
            };
            self.selection_end = Some((self.cursor_row, col));
        }
    }

    /// Copy the selected source characters or lines into the local register.
    pub fn yank_visual_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_anchor.zip(self.selection_end) else {
            return false;
        };
        self.yank_register = if self.editor_mode == RequirementsEditorMode::VisualLine {
            let lo = start.0.min(end.0);
            let hi = start.0.max(end.0);
            (lo..=hi)
                .map(|row| self.buffer.line(row))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        } else {
            self.buffer.text_range(start, end)
        };
        true
    }

    /// Delete the selected source range as one undoable edit.
    pub fn delete_visual_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_anchor.zip(self.selection_end) else {
            return false;
        };
        self.push_edit_undo();
        if self.editor_mode == RequirementsEditorMode::VisualLine {
            let lo = start.0.min(end.0);
            let hi = start.0.max(end.0);
            for row in (lo..=hi).rev() {
                self.buffer.delete_line(row);
            }
            self.cursor_row = lo.min(self.buffer.line_count().saturating_sub(1));
            self.cursor_col = 0;
        } else {
            let (lo, hi) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            self.buffer.replace_range(lo, hi, "");
            self.cursor_row = lo.0;
            self.cursor_col = lo.1;
        }
        self.desired_col = self.cursor_col;
        self.buffer_dirty = true;
        self.exit_visual_mode();
        true
    }

    /// Paste the local register as logical lines before or after the cursor line.
    pub fn paste_register(&mut self, before: bool) -> bool {
        if self.yank_register.is_empty() {
            return false;
        }
        let register = self.yank_register.clone();
        self.push_edit_undo();
        let content = register.trim_end_matches('\n');
        if before {
            if self.cursor_row == 0 {
                self.buffer.insert_str(0, 0, &format!("{content}\n"));
            } else {
                self.buffer.insert_line(self.cursor_row - 1, content);
            }
        } else {
            self.buffer.insert_line(self.cursor_row, content);
            self.cursor_row = self.cursor_row.saturating_add(1);
        }
        self.cursor_row = self
            .cursor_row
            .min(self.buffer.line_count().saturating_sub(1));
        self.cursor_col = 0;
        self.desired_col = 0;
        self.buffer_dirty = true;
        true
    }

    /// Restores saved Markdown, or removes a document that was never saved.
    pub fn discard_current_document(&mut self) {
        self.buffer_dirty = false;
        if let Some(id) = self.draft_document_id.take() {
            if let Some(artifacts) = self.artifacts.as_mut() {
                artifacts.index.documents.retain(|doc| doc.id != id);
                artifacts.documents.retain(|doc| doc.meta.id != id);
            }
            self.selected_document_id = None;
            self.buffer = EditorBuffer::from_string(String::new());
            self.exit_insert_mode();
            self.rebuild_tree();
        } else if let Some(id) = self.selected_document_id.clone() {
            self.select_document_by_id(&id);
        }
    }

    /// Defers navigation while the buffer has unsaved changes.
    pub fn guard_unsaved(&mut self, pending: RequirementsPendingAction) -> bool {
        if self.buffer_dirty {
            self.overlay = RequirementsOverlay::Unsaved { pending };
            true
        } else {
            false
        }
    }

    /// Creates an empty state with no loaded artifacts.
    pub fn empty() -> Self {
        Self {
            artifacts: None,
            discovered: false,
            requirements_root: PathBuf::new(),
            project_root: PathBuf::new(),
            tree_items: Vec::new(),
            tree_state: TreeState::default(),
            focus: RequirementsFocus::Tree,
            editor_mode: RequirementsEditorMode::Browse,
            pane_areas: [ratatui::layout::Rect::default(); 3],
            selection_dragging: false,
            selected_document_id: None,
            selected_linked_index: 0,
            highlight_test_point_id: None,
            buffer: EditorBuffer::from_string(String::new()),
            buffer_dirty: false,
            draft_document_id: None,
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            editor_display_rows: Vec::new(),
            editor_wrap_width: 0,
            horizontal_scroll: [0; 3],
            rendered_cursor: None,
            linked_scroll: 0,
            rendered_linked_selection: None,
            scroll_limits: [[0; 2]; 3],
            scrollbar_drag: None,
            selection_anchor: None,
            selection_end: None,
            iteration_filter: RequirementIterationFilter::All,
            group_mode: RequirementGroupMode::Path,
            overlay: RequirementsOverlay::None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            yank_register: String::new(),
            insert_session_undo: false,
            vertical_motion: None,
        }
    }

    /// Loads authoring artifacts from the global requirement store and project test points.
    pub fn load_from_project(project_root: &Path, requirements_root: &Path) -> Self {
        let mut state = Self::empty();
        state.project_root = project_root.to_path_buf();
        state.requirements_root = requirements_root.to_path_buf();
        match load_authoring_artifacts(project_root, requirements_root) {
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
                state.restore_view_prefs();
            }
            Err(error) => {
                let _ = error;
            }
        }
        state
    }

    /// Reloads the requirement store from disk, preserving view state.
    ///
    /// Filter, group mode, overlay, and a dirty editor buffer are kept so an
    /// agent tool boundary can observe the current store without discarding
    /// in-progress UI edits.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read or parsed.
    pub fn reload_from_disk(&mut self) -> Result<()> {
        let project_root = self.project_root.clone();
        let requirements_root = self.requirements_root.clone();
        let selected = self.selected_document_id.clone();
        let filter = self.iteration_filter.clone();
        let group_mode = self.group_mode;
        let overlay = self.overlay.clone();
        let editor_mode = self.editor_mode;
        let dirty = self.buffer_dirty;
        let preserved_buffer = dirty.then(|| self.buffer.clone());
        let cursor_row = self.cursor_row;
        let cursor_col = self.cursor_col;
        let desired_col = self.desired_col;
        let scroll_row = self.scroll_row;
        let selection_anchor = self.selection_anchor;
        let selection_end = self.selection_end;

        let loaded = load_authoring_artifacts(&project_root, &requirements_root)?;
        // Requirement Markdown/`_teshi.json` can change on disk; keep in-memory test
        // points so unsaved review-state edits are not discarded at an agent boundary.
        let preserved_test_points = self.artifacts.as_ref().map(|a| a.test_points.clone());
        self.discovered = loaded.discovered;
        self.artifacts = loaded.artifacts;
        if let (Some(artifacts), Some(test_points)) =
            (self.artifacts.as_mut(), preserved_test_points)
        {
            artifacts.test_points = test_points;
        }
        self.iteration_filter = filter;
        self.group_mode = group_mode;
        self.overlay = overlay;
        self.rebuild_tree();
        if let Some(id) = selected.as_ref() {
            let still_present = self
                .artifacts
                .as_ref()
                .is_some_and(|artifacts| artifacts.index.documents.iter().any(|doc| doc.id == *id));
            if still_present {
                self.select_document_by_id(id);
            }
        }
        if let Some(buffer) = preserved_buffer {
            self.buffer = buffer;
            self.buffer_dirty = true;
            self.cursor_row = cursor_row;
            self.cursor_col = cursor_col;
            self.desired_col = desired_col;
            self.scroll_row = scroll_row;
            self.selection_anchor = selection_anchor;
            self.selection_end = selection_end;
        }
        self.editor_mode = editor_mode;
        Ok(())
    }

    /// Rebuilds the requirement tree from the current index, filter, and group mode.
    pub fn rebuild_tree(&mut self) {
        let index = self
            .artifacts
            .as_ref()
            .map(|a| &a.index)
            .cloned()
            .unwrap_or_default();
        self.tree_items = build_requirement_tree(&index, &self.iteration_filter, self.group_mode);
        let visible = visible_document_ids(&index, &self.iteration_filter);
        let target = self
            .selected_document_id
            .as_ref()
            .filter(|selected| visible.iter().any(|id| id == *selected))
            .cloned()
            .or_else(|| {
                if self.buffer_dirty {
                    None
                } else {
                    visible.first().cloned()
                }
            });

        if let Some(id) = target {
            if self.selected_document_id.as_deref() != Some(id.as_str()) {
                self.load_document_state(&id);
            }
            self.sync_tree_selection();
        } else if !self.buffer_dirty {
            self.exit_insert_mode();
            self.selected_document_id = None;
            self.buffer = EditorBuffer::from_string(String::new());
            self.buffer_dirty = false;
            self.tree_state.select(Vec::new());
        }
    }

    fn select_document_tree_path(&mut self, path: Vec<String>) {
        // A selected child is not rendered as highlighted while one of its
        // ancestors is collapsed. Open the ancestors whenever selection is
        // changed programmatically (keyboard navigation, reload, or click).
        for depth in 1..path.len() {
            self.tree_state.open(path[..depth].to_vec());
        }
        self.tree_state.select(path);
        self.tree_state.scroll_selected_into_view();
    }

    /// Iteration names currently present in the loaded index.
    pub fn discovered_iteration_names(&self) -> Vec<String> {
        self.artifacts
            .as_ref()
            .map(|a| a.index.discovered_iteration_names())
            .unwrap_or_default()
    }

    /// Filter picker rows: All, Unassigned, then named iterations.
    pub fn filter_picker_options(&self) -> Vec<RequirementIterationFilter> {
        let mut options = vec![
            RequirementIterationFilter::All,
            RequirementIterationFilter::Unassigned,
        ];
        options.extend(
            self.discovered_iteration_names()
                .into_iter()
                .map(RequirementIterationFilter::Named),
        );
        options
    }

    /// Opens the iteration filter overlay.
    pub fn open_filter_picker(&mut self) {
        self.overlay = RequirementsOverlay::FilterPicker { selection: 0 };
    }

    /// Opens the document iteration editor for the current document.
    pub fn open_iteration_editor(&mut self) {
        let current = self
            .current_document_meta()
            .and_then(|m| m.iteration.clone())
            .unwrap_or_default();
        self.overlay = RequirementsOverlay::IterationEdit { buffer: current };
    }

    /// Opens the explicit global-library deletion confirmation.
    pub fn open_delete_confirmation(&mut self) -> Result<(), String> {
        let document_id = self
            .selected_document_id
            .clone()
            .ok_or_else(|| "Select a requirement document to delete".to_string())?;
        let title = self
            .current_document_meta()
            .map(|meta| meta.title.clone())
            .unwrap_or_else(|| document_id.clone());
        self.overlay = RequirementsOverlay::DeleteConfirmation { document_id, title };
        Ok(())
    }

    /// Removes the selected saved document and moves selection to the next
    /// visible document, then the previous one, or the empty state.
    pub fn delete_current_document(&mut self, project_root: &Path) -> Result<(), String> {
        let document_id = self
            .selected_document_id
            .clone()
            .ok_or_else(|| "Select a requirement document to delete".to_string())?;
        let index_before = self
            .artifacts
            .as_ref()
            .map(|artifacts| artifacts.index.clone())
            .ok_or_else(|| "Requirement library is not loaded".to_string())?;
        let meta = index_before
            .documents
            .iter()
            .find(|doc| doc.id == document_id)
            .cloned()
            .ok_or_else(|| format!("Requirement document '{document_id}' is not indexed"))?;
        let visible = visible_document_ids(&index_before, &self.iteration_filter);
        let current_position = visible
            .iter()
            .position(|id| id == &document_id)
            .ok_or_else(|| "Selected requirement is hidden by the active filter".to_string())?;
        let next_document_id = visible.get(current_position + 1).cloned().or_else(|| {
            current_position
                .checked_sub(1)
                .and_then(|i| visible.get(i).cloned())
        });

        delete_requirement_document(
            &self.requirements_root,
            &document_id,
            Some(meta.revision.as_str()),
        )
        .map_err(|error| error.to_string())?;

        let test_points_result = {
            let artifacts = self
                .artifacts
                .as_mut()
                .ok_or_else(|| "Requirement library is not loaded".to_string())?;
            artifacts
                .index
                .documents
                .retain(|doc| doc.id != document_id);
            artifacts.documents.retain(|doc| doc.meta.id != document_id);
            artifacts
                .diagnostics
                .retain(|diagnostic| diagnostic.record_id.as_deref() != Some(&document_id));
            let before = artifacts.test_points.clone();
            mark_document_links_missing(&document_id, &mut artifacts.test_points.test_points);
            if artifacts.test_points != before {
                Some(
                    save_test_points(project_root, &artifacts.test_points)
                        .map_err(|error| error.to_string()),
                )
            } else {
                None
            }
        };

        self.overlay = RequirementsOverlay::None;
        self.buffer_dirty = false;
        self.draft_document_id = None;
        self.selection_anchor = None;
        self.selection_end = None;
        self.selected_linked_index = 0;
        self.highlight_test_point_id = None;
        self.focus = RequirementsFocus::Tree;
        if let Some(next_id) = next_document_id {
            self.selected_document_id = Some(next_id.clone());
            self.select_document_by_id(&next_id);
        } else {
            self.selected_document_id = None;
            self.buffer = EditorBuffer::from_string(String::new());
            self.editor_mode = RequirementsEditorMode::Browse;
            self.cursor_row = 0;
            self.cursor_col = 0;
            self.desired_col = 0;
            self.scroll_row = 0;
        }
        self.rebuild_tree();

        if let Some(result) = test_points_result {
            result?;
        }
        Ok(())
    }

    /// Applies `filter`, or asks to resolve unsaved changes before hiding the document.
    ///
    /// # Errors
    ///
    /// Reserved for filter validation failures.
    pub fn try_set_iteration_filter(
        &mut self,
        filter: RequirementIterationFilter,
    ) -> Result<(), String> {
        let index = self
            .artifacts
            .as_ref()
            .map(|a| a.index.clone())
            .unwrap_or_default();
        let visible = visible_document_ids(&index, &filter);
        let would_hide = self
            .selected_document_id
            .as_ref()
            .is_some_and(|id| !visible.iter().any(|visible_id| visible_id == id));
        if would_hide && self.buffer_dirty {
            self.guard_unsaved(RequirementsPendingAction::Filter(filter));
            return Ok(());
        }
        self.iteration_filter = filter;
        self.overlay = RequirementsOverlay::None;
        self.rebuild_tree();
        self.persist_view_prefs();
        Ok(())
    }

    /// Toggles path vs iteration grouping. Grouping never hides documents.
    pub fn toggle_group_mode(&mut self) {
        self.group_mode = match self.group_mode {
            RequirementGroupMode::Path => RequirementGroupMode::Iteration,
            RequirementGroupMode::Iteration => RequirementGroupMode::Path,
        };
        self.rebuild_tree();
        self.persist_view_prefs();
    }

    /// Confirms the active overlay (filter pick or iteration edit).
    ///
    /// # Errors
    ///
    /// Returns an error when saving iteration metadata fails.
    pub fn confirm_overlay(&mut self) -> Result<(), String> {
        match self.overlay.clone() {
            RequirementsOverlay::Unsaved { .. } => Ok(()),
            RequirementsOverlay::None => Ok(()),
            RequirementsOverlay::FilterPicker { selection } => {
                let options = self.filter_picker_options();
                let filter = options
                    .get(selection)
                    .cloned()
                    .unwrap_or(RequirementIterationFilter::All);
                self.try_set_iteration_filter(filter)
            }
            RequirementsOverlay::IterationEdit { buffer } => self.save_current_iteration(&buffer),
            RequirementsOverlay::DeleteConfirmation { .. } => Ok(()),
        }
    }

    /// Closes the overlay without applying it.
    pub fn cancel_overlay(&mut self) {
        self.overlay = RequirementsOverlay::None;
    }

    /// Moves the filter-picker highlight.
    pub fn overlay_move_selection(&mut self, delta: isize) {
        let len = self.filter_picker_options().len();
        if len == 0 {
            return;
        }
        if let RequirementsOverlay::FilterPicker { selection } = &mut self.overlay {
            let next = (*selection as isize + delta).rem_euclid(len as isize) as usize;
            *selection = next;
        }
    }

    /// Inserts a character into the iteration editor buffer.
    pub fn overlay_insert_char(&mut self, ch: char) {
        if let RequirementsOverlay::IterationEdit { buffer } = &mut self.overlay
            && !ch.is_control()
        {
            buffer.push(ch);
        }
    }

    /// Deletes the last character from the iteration editor buffer.
    pub fn overlay_backspace(&mut self) {
        if let RequirementsOverlay::IterationEdit { buffer } = &mut self.overlay {
            buffer.pop();
        }
    }

    /// Returns `true` when a Requirements overlay is capturing keys.
    pub fn overlay_active(&self) -> bool {
        !matches!(self.overlay, RequirementsOverlay::None)
    }

    /// Saves iteration metadata, prompting first if this would hide dirty Markdown.
    pub(crate) fn save_current_iteration(&mut self, raw: &str) -> Result<(), String> {
        let doc_id = self
            .selected_document_id
            .clone()
            .ok_or_else(|| "no requirement document selected".to_string())?;
        let would_hide = match &self.iteration_filter {
            RequirementIterationFilter::All => false,
            RequirementIterationFilter::Unassigned => !raw.trim().is_empty(),
            RequirementIterationFilter::Named(name) => name != raw.trim(),
        };
        if self.buffer_dirty && would_hide {
            self.guard_unsaved(RequirementsPendingAction::Iteration {
                document_id: doc_id,
                value: raw.to_string(),
            });
            return Ok(());
        }
        let iteration = if raw.trim().is_empty() {
            None
        } else {
            Some(raw)
        };
        let index = teshi_engine::set_requirement_document_iteration(
            &self.requirements_root,
            &doc_id,
            iteration,
        )
        .map_err(|err| err.to_string())?;
        if let Some(artifacts) = self.artifacts.as_mut() {
            artifacts.index = index;
            if let Some(doc) = artifacts.documents.iter_mut().find(|d| d.meta.id == doc_id) {
                doc.meta.iteration = artifacts
                    .index
                    .documents
                    .iter()
                    .find(|d| d.id == doc_id)
                    .and_then(|d| d.iteration.clone());
            }
        }
        self.overlay = RequirementsOverlay::None;
        self.rebuild_tree();
        Ok(())
    }

    fn restore_view_prefs(&mut self) {
        let Some(store_id) = self
            .artifacts
            .as_ref()
            .and_then(|a| a.index.store_id.clone())
        else {
            return;
        };
        let Ok(settings) = teshi_engine::load_settings() else {
            return;
        };
        let Some(prefs) = settings.requirements_views.get(store_id.as_str()) else {
            return;
        };
        self.group_mode = prefs.group;
        self.iteration_filter = match &prefs.filter {
            RequirementIterationFilter::Named(name)
                if !self.discovered_iteration_names().iter().any(|n| n == name) =>
            {
                RequirementIterationFilter::All
            }
            other => other.clone(),
        };
        self.rebuild_tree();
    }

    fn persist_view_prefs(&self) {
        let Some(store_id) = self
            .artifacts
            .as_ref()
            .and_then(|a| a.index.store_id.as_ref())
        else {
            return;
        };
        let Ok(mut settings) = teshi_engine::load_settings() else {
            return;
        };
        settings.requirements_views.insert(
            store_id.as_str().to_string(),
            teshi_engine::RequirementsViewPrefs {
                filter: self.iteration_filter.clone(),
                group: self.group_mode,
            },
        );
        let _ = teshi_engine::save_settings(&settings);
    }

    /// Selects a document by stable id and loads its Markdown into the editor buffer.
    pub fn select_document_by_id(&mut self, doc_id: &str) {
        if self.buffer_dirty {
            if self.selected_document_id.as_deref() != Some(doc_id) {
                self.guard_unsaved(RequirementsPendingAction::Document(doc_id.to_string()));
            }
            return;
        }
        self.load_document_state(doc_id);
        self.sync_tree_selection();
    }

    fn load_document_state(&mut self, doc_id: &str) {
        self.vertical_motion = None;
        // Undo snapshots belong to one document. Reopening or switching a
        // document must never apply another document's text to this buffer.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.exit_insert_mode();
        self.selected_document_id = Some(doc_id.to_string());
        self.selected_linked_index = 0;
        self.highlight_test_point_id = None;
        self.selection_anchor = None;
        self.selection_end = None;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.scroll_row = 0;
        self.horizontal_scroll[1] = 0;
        self.rendered_cursor = None;
        self.linked_scroll = 0;

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

    fn sync_tree_selection(&mut self) {
        let Some(id) = self.selected_document_id.as_ref() else {
            self.tree_state.select(Vec::new());
            return;
        };
        if let Some(path) = find_tree_path(&self.tree_items, &format!("{TREE_DOC_PREFIX}{id}")) {
            self.select_document_tree_path(path);
        }
    }

    /// Selects a document from a tree node id (`req-doc:<id>` or `req-dir:...`).
    pub fn select_tree_node(&mut self, node_id: &str) {
        if let Some(doc_id) = node_id.strip_prefix(TREE_DOC_PREFIX) {
            self.select_document_by_id(doc_id);
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
        let store_id = self.artifacts.as_ref()?.index.store_id.clone()?;
        let link = create_requirement_link(
            store_id,
            doc_meta.id,
            doc_meta.revision.as_str(),
            &body,
            range,
        )?;
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

    /// Creates a default document using one available number for its ID, path, and title.
    pub fn create_default_document(&mut self) -> Result<String> {
        let n = self.next_document_number()?;
        self.create_document(&format!("doc-{n}.md"), &format!("Document {n}"))
    }

    fn next_document_number(&self) -> Result<u32> {
        let mut n = 1;
        loop {
            let id = format!("doc-{n}");
            let path = format!("doc-{n}.md");
            let indexed = self.artifacts.as_ref().is_some_and(|artifacts| {
                artifacts
                    .index
                    .documents
                    .iter()
                    .any(|doc| doc.id == id || doc.path == path)
            });
            if !indexed
                && !self.requirements_root.join(&path).exists()
                && !is_requirement_document_id_reserved(&self.requirements_root, &id)?
            {
                return Ok(n);
            }
            n += 1;
        }
    }

    /// Creates a new requirement document under `relative_path` with the given title.
    pub fn create_document(&mut self, relative_path: &str, title: &str) -> Result<String> {
        let id = format!("doc-{}", self.next_document_number()?);
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
        let body = format!("# {}\n\n", title.trim());
        let revision = compute_document_revision(&body);
        let mut meta = RequirementDocumentMeta::new(id.clone(), relative_path, title, revision);
        if let RequirementIterationFilter::Named(name) = &self.iteration_filter
            && !name.trim().is_empty()
        {
            meta.iteration = Some(name.clone());
        }

        artifacts.index.documents.push(meta.clone());
        artifacts
            .documents
            .push(teshi_core::authoring::RequirementDocumentContent {
                meta,
                body: body.clone(),
            });
        self.rebuild_tree();
        self.select_document_by_id(&id);
        self.buffer_dirty = true;
        self.draft_document_id = Some(id.clone());
        Ok(id)
    }

    /// Saves the current requirement document and refreshes link resolutions.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid first-line H1 title or failed persistence.
    pub fn save_current_document(&mut self, project_root: &Path) -> Result<()> {
        let body = self.buffer.as_string();
        requirement_title(&body)?;
        let requirements_root = if self.requirements_root.as_os_str().is_empty() {
            project_root.to_path_buf()
        } else {
            self.requirements_root.clone()
        };
        let doc_id = self
            .selected_document_id
            .clone()
            .context("no requirement document selected")?;
        let artifacts = self
            .artifacts
            .as_mut()
            .context("authoring artifacts not loaded")?;
        if artifacts.index.store_id.is_none() {
            let initialized = initialize_requirement_store(&requirements_root)?;
            artifacts.index.store_id = initialized.store_id;
            artifacts.index.version = initialized.version;
        }
        let meta = artifacts
            .index
            .documents
            .iter()
            .find(|d| d.id == doc_id)
            .context("selected document not in index")?;
        let relative_path = meta.path.clone();
        save_requirement_markdown(
            &requirements_root,
            &mut artifacts.index,
            &relative_path,
            &body,
        )?;
        // Markdown now exists on disk even if saving linked test points fails.
        self.draft_document_id = None;
        if let Some(doc) = artifacts.documents.iter_mut().find(|d| d.meta.id == doc_id) {
            doc.body = body.clone();
            if let Some(index_meta) = artifacts.index.documents.iter().find(|d| d.id == doc_id) {
                doc.meta = index_meta.clone();
            }
        }
        let store_id = artifacts.index.store_id.clone();
        let revision = artifacts
            .index
            .documents
            .iter()
            .find(|d| d.id == doc_id)
            .map(|d| d.revision.as_str().to_string())
            .unwrap_or_default();
        re_resolve_document_links(
            &body,
            store_id.as_ref(),
            &doc_id,
            &revision,
            &mut artifacts.test_points.test_points,
        );
        let test_points_result = save_test_points(project_root, &artifacts.test_points);
        // The saved title must be visible even if linked test-point persistence fails.
        self.rebuild_tree();
        test_points_result?;
        self.buffer_dirty = false;
        self.draft_document_id = None;
        Ok(())
    }

    pub fn focus_next_column(&mut self) {
        self.exit_insert_mode();
        self.focus = match self.focus {
            RequirementsFocus::Tree => RequirementsFocus::Editor,
            RequirementsFocus::Editor => RequirementsFocus::LinkedTestPoints,
            RequirementsFocus::LinkedTestPoints => RequirementsFocus::LinkedTestPoints,
        };
    }

    pub fn focus_prev_column(&mut self) {
        self.exit_insert_mode();
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
        let mut documents = Vec::new();
        collect_tree_documents(&self.tree_items, &mut Vec::new(), &mut documents);
        if documents.is_empty() {
            return;
        }
        let pos = self
            .selected_document_id
            .as_ref()
            .and_then(|id| documents.iter().position(|(_, doc_id)| doc_id == id))
            .unwrap_or(0);
        let next = if delta < 0 {
            pos.saturating_sub(1)
        } else {
            pos.saturating_add(1).min(documents.len().saturating_sub(1))
        };
        if let Some((path, doc_id)) = documents.get(next) {
            self.select_document_by_id(doc_id);
            if self.selected_document_id.as_deref() == Some(doc_id) {
                self.select_document_tree_path(path.clone());
            }
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        if !self.insert_session_undo {
            self.push_edit_undo();
        }
        self.buffer
            .insert_char(self.cursor_row, self.cursor_col, ch);
        self.cursor_col += 1;
        self.desired_col = self.cursor_col;
        self.buffer_dirty = true;
    }

    pub fn insert_newline(&mut self) {
        if !self.insert_session_undo {
            self.push_edit_undo();
        }
        self.buffer
            .insert_char(self.cursor_row, self.cursor_col, '\n');
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.buffer_dirty = true;
    }

    pub fn backspace(&mut self) {
        if !self.insert_session_undo {
            self.push_edit_undo();
        }
        let (row, col, changed) = self.buffer.backspace(self.cursor_row, self.cursor_col);
        if changed {
            self.cursor_row = row;
            self.cursor_col = col;
            self.desired_col = col;
            self.buffer_dirty = true;
        }
    }

    pub fn delete_forward(&mut self) {
        if !self.insert_session_undo {
            self.push_edit_undo();
        }
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
        if let Some(current) = self
            .editor_display_rows
            .iter()
            .position(|&(row, start, end)| {
                row == self.cursor_row && self.cursor_col >= start && self.cursor_col < end
            })
        {
            let (_, start, _) = self.editor_display_rows[current];
            let cells = self
                .vertical_motion
                .filter(|&(row, col, width, _)| {
                    row == self.cursor_row
                        && col == self.cursor_col
                        && width == self.editor_wrap_width
                })
                .map(|(_, _, _, cells)| cells)
                .unwrap_or_else(|| {
                    self.buffer
                        .line(self.cursor_row)
                        .chars()
                        .skip(start)
                        .take(self.cursor_col - start)
                        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                        .sum()
                });
            let target = if delta < 0 {
                current.saturating_sub(1)
            } else {
                current
                    .saturating_add(1)
                    .min(self.editor_display_rows.len().saturating_sub(1))
            };
            let (row, start, end) = self.editor_display_rows[target];
            let mut column = start;
            let mut consumed = 0;
            for ch in self.buffer.line(row).chars().skip(start).take(end - start) {
                let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if consumed + width > cells {
                    break;
                }
                consumed += width;
                column += 1;
            }
            self.cursor_row = row;
            self.cursor_col = column.min(self.buffer.line_len_chars(row));
            self.desired_col = self.cursor_col;
            self.vertical_motion = Some((row, self.cursor_col, self.editor_wrap_width, cells));
            return;
        }
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

    fn source_positions(&self) -> Vec<((usize, usize), char)> {
        let mut positions = Vec::new();
        for row in 0..self.buffer.line_count() {
            let line = self.buffer.line(row);
            for (col, ch) in line.chars().enumerate() {
                positions.push(((row, col), ch));
            }
            if row + 1 < self.buffer.line_count() {
                positions.push(((row, line.chars().count()), '\n'));
            }
        }
        positions
    }

    fn position_index(&self) -> usize {
        let positions = self.source_positions();
        positions
            .iter()
            .position(|(position, _)| *position == (self.cursor_row, self.cursor_col))
            .unwrap_or_else(|| positions.len().saturating_sub(1))
    }

    fn set_position_index(&mut self, index: usize) {
        if let Some(((row, col), _)) = self.source_positions().get(index).copied() {
            self.cursor_row = row;
            self.cursor_col = col;
            self.desired_col = col;
        }
    }

    /// Move to the next word or punctuation run in logical source text.
    pub fn vim_word_forward(&mut self) {
        let positions = self.source_positions();
        if positions.is_empty() {
            return;
        }
        let mut index = self.position_index();
        // Words and punctuation runs are separate tokens; whitespace only
        // separates them. Starting within a token must skip that whole run.
        let current = vim_word_class(positions[index].1);
        if current != 0 {
            while index < positions.len() && vim_word_class(positions[index].1) == current {
                index += 1;
            }
        }
        while index < positions.len() && positions[index].1.is_whitespace() {
            index += 1;
        }
        self.set_position_index(index.min(positions.len().saturating_sub(1)));
        self.extend_visual_selection();
    }

    /// Move to the preceding word or punctuation run in logical source text.
    pub fn vim_word_backward(&mut self) {
        let positions = self.source_positions();
        if positions.is_empty() {
            return;
        }
        let mut index = self.position_index().saturating_sub(1);
        while index > 0 && positions[index].1.is_whitespace() {
            index -= 1;
        }
        let class = vim_word_class(positions[index].1);
        while index > 0 && vim_word_class(positions[index - 1].1) == class {
            index -= 1;
        }
        self.set_position_index(index);
        self.extend_visual_selection();
    }

    /// Move to the end of the next word or punctuation run.
    pub fn vim_word_end(&mut self) {
        let positions = self.source_positions();
        if positions.is_empty() {
            return;
        }
        let mut index = self.position_index().saturating_add(1);
        while index < positions.len() && positions[index].1.is_whitespace() {
            index += 1;
        }
        let class = positions.get(index).map(|(_, ch)| vim_word_class(*ch));
        while index + 1 < positions.len() && Some(vim_word_class(positions[index + 1].1)) == class {
            index += 1;
        }
        self.set_position_index(index.min(positions.len().saturating_sub(1)));
        self.extend_visual_selection();
    }

    /// Move to the first character of the logical source line.
    pub fn vim_line_start(&mut self) {
        self.cursor_col = 0;
        self.desired_col = 0;
        self.extend_visual_selection();
    }

    /// Move to the first nonblank source character on the current line.
    pub fn vim_first_nonblank(&mut self) {
        let line = self.buffer.line(self.cursor_row);
        self.cursor_col = line.chars().position(|ch| !ch.is_whitespace()).unwrap_or(0);
        self.desired_col = self.cursor_col;
        self.extend_visual_selection();
    }

    /// Move to the insertion position at the logical source line end.
    pub fn vim_line_end(&mut self) {
        self.cursor_col = self.buffer.line_len_chars(self.cursor_row);
        self.desired_col = self.cursor_col;
        self.extend_visual_selection();
    }

    /// Move to the first nonblank character of the first source line.
    pub fn vim_go_top(&mut self) {
        self.cursor_row = 0;
        self.vim_first_nonblank();
    }

    /// Move to the first nonblank character of the last source line.
    pub fn vim_go_bottom(&mut self) {
        self.cursor_row = self.buffer.line_count().saturating_sub(1);
        self.vim_first_nonblank();
    }

    /// Delete the source character under the cursor as one undoable edit.
    pub fn vim_delete_char(&mut self) -> bool {
        if self.cursor_col >= self.buffer.line_len_chars(self.cursor_row) {
            return false;
        }
        self.push_edit_undo();
        let changed = self.buffer.delete(self.cursor_row, self.cursor_col);
        if changed {
            self.buffer_dirty = true;
            self.clamp_cursor();
        }
        changed
    }

    /// Delete the logical source line as one undoable edit.
    pub fn vim_delete_line(&mut self) -> bool {
        self.push_edit_undo();
        let changed = self.buffer.delete_line(self.cursor_row);
        if changed {
            self.buffer_dirty = true;
            self.clamp_cursor();
        }
        changed
    }

    /// Copy the logical source line into the local line register.
    pub fn vim_yank_line(&mut self) {
        self.yank_register = format!("{}\n", self.buffer.line(self.cursor_row));
    }

    /// Open a logical source line and group its insertion into one undo operation.
    pub fn vim_open_line(&mut self, above: bool) {
        self.push_edit_undo();
        if above {
            self.buffer.insert_str(self.cursor_row, 0, "\n");
        } else {
            let line_len = self.buffer.line_len_chars(self.cursor_row);
            self.buffer.insert_str(self.cursor_row, line_len, "\n");
            self.cursor_row = self.cursor_row.saturating_add(1);
        }
        self.cursor_col = 0;
        self.desired_col = 0;
        self.buffer_dirty = true;
        self.editor_mode = RequirementsEditorMode::Insert;
        self.insert_session_undo = true;
    }

    pub fn clamp_cursor(&mut self) {
        let last_row = self.buffer.line_count().saturating_sub(1);
        self.cursor_row = self.cursor_row.min(last_row);
        self.cursor_col = self.buffer.clamp_col(self.cursor_row, self.cursor_col);
        self.desired_col = self
            .desired_col
            .min(self.buffer.line_len_chars(self.cursor_row));
    }

    /// Returns validation and load diagnostics for display in the tree pane.
    pub fn visible_diagnostics(&self) -> Vec<&AuthoringDiagnostic> {
        self.artifacts
            .as_ref()
            .map(|a| a.diagnostics.iter().collect())
            .unwrap_or_default()
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn vim_word_class(ch: char) -> u8 {
    if ch.is_whitespace() {
        0
    } else if is_word_char(ch) {
        1
    } else {
        2
    }
}

fn ranges_overlap(a: TextRange, b: TextRange) -> bool {
    a.start.offset() < b.end.offset() && b.start.offset() < a.end.offset()
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

fn visible_document_ids(
    index: &teshi_core::authoring::RequirementDocumentIndex,
    filter: &RequirementIterationFilter,
) -> Vec<String> {
    index
        .documents
        .iter()
        .filter(|doc| doc.matches_iteration_filter(filter))
        .map(|doc| doc.id.clone())
        .collect()
}

/// Finds the complete ancestor path for a stable tree row identity.
pub(crate) fn find_tree_path(
    items: &[TreeItem<'static, String>],
    target: &str,
) -> Option<Vec<String>> {
    for item in items {
        let mut path = vec![item.identifier().clone()];
        if item.identifier() == target {
            return Some(path);
        }
        if let Some(mut child_path) = find_tree_path(item.children(), target) {
            path.append(&mut child_path);
            return Some(path);
        }
    }
    None
}

fn collect_tree_documents(
    items: &[TreeItem<'static, String>],
    parent_path: &mut Vec<String>,
    documents: &mut Vec<(Vec<String>, String)>,
) {
    for item in items {
        parent_path.push(item.identifier().clone());
        if let Some(doc_id) = item.identifier().strip_prefix(TREE_DOC_PREFIX) {
            documents.push((parent_path.clone(), doc_id.to_string()));
        } else {
            collect_tree_documents(item.children(), parent_path, documents);
        }
        parent_path.pop();
    }
}

fn build_path_tree(
    documents: &[RequirementDocumentMeta],
    dir_prefix: &str,
) -> Vec<TreeItem<'static, String>> {
    #[derive(Default)]
    struct DirNode {
        children: HashMap<String, DirNode>,
        documents: Vec<RequirementDocumentMeta>,
    }

    let mut root = DirNode::default();
    for doc in documents {
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

    fn dir_to_items(
        path: String,
        node: &DirNode,
        dir_prefix: &str,
    ) -> Vec<TreeItem<'static, String>> {
        let mut items = Vec::new();
        for (name, child) in node.children.iter() {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let id = format!("{TREE_DIR_PREFIX}{dir_prefix}{child_path}");
            let child_items = dir_to_items(child_path, child, dir_prefix);
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

    dir_to_items(String::new(), &root, dir_prefix)
}

/// Builds folder/document tree items from indexed paths after applying filter/group.
pub fn build_requirement_tree(
    index: &teshi_core::authoring::RequirementDocumentIndex,
    filter: &RequirementIterationFilter,
    group: RequirementGroupMode,
) -> Vec<TreeItem<'static, String>> {
    let docs: Vec<RequirementDocumentMeta> = index
        .documents
        .iter()
        .filter(|doc| doc.matches_iteration_filter(filter))
        .cloned()
        .collect();
    match group {
        RequirementGroupMode::Path => build_path_tree(&docs, ""),
        RequirementGroupMode::Iteration => {
            let mut named: std::collections::BTreeMap<String, Vec<RequirementDocumentMeta>> =
                std::collections::BTreeMap::new();
            let mut unassigned = Vec::new();
            for doc in docs {
                match doc.iteration.clone() {
                    Some(name) => named.entry(name).or_default().push(doc),
                    None => unassigned.push(doc),
                }
            }
            let mut items = Vec::new();
            for (name, group_docs) in named {
                let child = build_path_tree(&group_docs, &format!("{name}/"));
                items
                    .push(TreeItem::new(format!("{TREE_ITER_PREFIX}{name}"), name, child).unwrap());
            }
            if !unassigned.is_empty() {
                let child = build_path_tree(&unassigned, "unassigned/");
                items.push(
                    TreeItem::new(
                        format!("{TREE_ITER_PREFIX}{UNASSIGNED_ITERATION_LABEL}"),
                        UNASSIGNED_ITERATION_LABEL.to_string(),
                        child,
                    )
                    .unwrap(),
                );
            }
            items
        }
    }
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
        ReviewState::Proposed => Style::default().fg(theme::WARNING),
        ReviewState::Approved => Style::default().fg(theme::SUCCESS),
        ReviewState::Rejected => Style::default().fg(theme::ERROR),
        ReviewState::NeedsReview => Style::default().fg(theme::ACCENT),
    }
}

/// Footer hint line for the Requirements tab.
pub fn requirements_footer_line(
    store_path: &Path,
    filter: &RequirementIterationFilter,
) -> Line<'static> {
    let filter_label = match filter {
        RequirementIterationFilter::All => "All".to_string(),
        RequirementIterationFilter::Unassigned => UNASSIGNED_ITERATION_LABEL.to_string(),
        RequirementIterationFilter::Named(name) => name.clone(),
    };
    theme::footer_line(vec![
        Span::styled(
            " Delete [d/Delete] ",
            Style::default().fg(theme::FOOTER_DESC),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" Store {} ", store_path.display()),
            Style::default().fg(theme::FOOTER_DESC),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" Filter {filter_label} [i] "),
            Style::default().fg(theme::FOOTER_DESC),
        ),
        Span::raw(" "),
        Span::styled(" Group [g] ", Style::default().fg(theme::FOOTER_DESC)),
        Span::raw(" "),
        Span::styled(" Iteration [I] ", Style::default().fg(theme::FOOTER_DESC)),
        Span::raw(" "),
        Span::styled(
            " New doc [Ctrl+n] ",
            Style::default().fg(theme::FOOTER_DESC),
        ),
        Span::raw(" "),
        Span::styled(" Save [s] ", Style::default().fg(theme::FOOTER_DESC)),
    ])
}

/// Title shown on the Requirements tree pane.
pub fn requirements_tree_title(filter: &RequirementIterationFilter) -> String {
    match filter {
        RequirementIterationFilter::All => "Requirements · All".into(),
        RequirementIterationFilter::Unassigned => {
            format!("Requirements · {UNASSIGNED_ITERATION_LABEL}")
        }
        RequirementIterationFilter::Named(name) => format!("Requirements · {name}"),
    }
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
            version: 2,
            store_id: Some(
                teshi_core::authoring::RequirementStoreId::parse("reqstore-test").unwrap(),
            ),
            documents: vec![
                RequirementDocumentMeta::new(
                    "d1",
                    "auth/login.md",
                    "Login",
                    teshi_core::authoring::DocumentRevision::new("r1"),
                ),
                RequirementDocumentMeta::new(
                    "d2",
                    "overview.md",
                    "Overview",
                    teshi_core::authoring::DocumentRevision::new("r2"),
                ),
            ],
        };
        let items = build_requirement_tree(
            &index,
            &RequirementIterationFilter::All,
            RequirementGroupMode::Path,
        );
        assert!(!items.is_empty());
        assert_eq!(index.documents.len(), 2);
    }

    #[test]
    fn create_test_point_from_selection_persists_proposed() {
        let mut state = AuthoringUiState::empty();
        let body = "user login required";
        let store_id = teshi_core::authoring::RequirementStoreId::parse("reqstore-test").unwrap();
        state.artifacts = Some(AuthoringArtifacts {
            index: RequirementDocumentIndex {
                version: 2,
                store_id: Some(store_id),
                documents: vec![RequirementDocumentMeta::new(
                    "doc-1",
                    "req.md",
                    "Req",
                    teshi_core::authoring::DocumentRevision::new("rev"),
                )],
            },
            documents: vec![teshi_core::authoring::RequirementDocumentContent {
                meta: RequirementDocumentMeta::new(
                    "doc-1",
                    "req.md",
                    "Req",
                    teshi_core::authoring::DocumentRevision::new("rev"),
                ),
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
        let store = tempdir().unwrap();
        let mut state = AuthoringUiState::empty();
        state.project_root = root.to_path_buf();
        state.requirements_root = store.path().to_path_buf();
        state.create_document("sample.md", "Sample").unwrap();
        state.buffer = EditorBuffer::from_string("# Sample\n\nedited".into());
        state.save_current_document(root).expect("save");
        let reloaded = AuthoringUiState::load_from_project(root, store.path());
        assert!(reloaded.discovered);
        let body = reloaded.buffer.as_string();
        assert!(body.contains("edited"));
    }

    #[test]
    fn delete_document_marks_links_missing_selects_next_and_reserves_id() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        initialize_requirement_store(store.path()).unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        for (path, title) in [("one.md", "One"), ("two.md", "Two"), ("three.md", "Three")] {
            state.create_document(path, title).unwrap();
            state.save_current_document(project.path()).unwrap();
        }

        state.select_document_by_id("doc-2");
        state.buffer = EditorBuffer::from_string("# Two\n\nText".into());
        state.buffer_dirty = true;
        state.selection_anchor = Some((2, 0));
        state.selection_end = Some((2, 4));
        state.create_test_point_from_selection().unwrap();
        state.save_current_document(project.path()).unwrap();
        state.artifacts.as_mut().unwrap().test_points.test_points[0].review_state =
            ReviewState::Approved;
        state.open_delete_confirmation().unwrap();
        assert!(matches!(
            state.overlay,
            RequirementsOverlay::DeleteConfirmation { ref document_id, .. }
                if document_id == "doc-2"
        ));

        state.delete_current_document(project.path()).unwrap();

        assert_eq!(state.selected_document_id.as_deref(), Some("doc-3"));
        assert_eq!(state.buffer.as_string(), "# Three\n\n");
        assert_eq!(
            state.tree_state.selected(),
            [format!("{TREE_DOC_PREFIX}doc-3")].as_slice()
        );
        let test_point = &state.artifacts.as_ref().unwrap().test_points.test_points[0];
        assert_eq!(test_point.review_state, ReviewState::NeedsReview);
        assert_eq!(
            test_point.requirement_links[0].resolution,
            ResolutionState::Stale
        );
        assert!(teshi_engine::is_requirement_document_id_reserved(store.path(), "doc-2").unwrap());
        assert_eq!(state.create_default_document().unwrap(), "doc-4");
    }

    #[test]
    fn delete_first_last_and_only_documents_choose_next_previous_and_empty() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        initialize_requirement_store(store.path()).unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        for (path, title) in [("one.md", "One"), ("two.md", "Two"), ("three.md", "Three")] {
            state.create_document(path, title).unwrap();
            state.save_current_document(project.path()).unwrap();
        }

        state.select_document_by_id("doc-1");
        state.open_delete_confirmation().unwrap();
        state.delete_current_document(project.path()).unwrap();
        assert_eq!(state.selected_document_id.as_deref(), Some("doc-2"));

        state.select_document_by_id("doc-3");
        state.open_delete_confirmation().unwrap();
        state.delete_current_document(project.path()).unwrap();
        assert_eq!(state.selected_document_id.as_deref(), Some("doc-2"));

        state.open_delete_confirmation().unwrap();
        state.delete_current_document(project.path()).unwrap();
        assert!(state.selected_document_id.is_none());
        assert!(state.buffer.as_string().is_empty());
        assert!(state.tree_state.selected().is_empty());
        assert!(teshi_engine::is_requirement_document_id_reserved(store.path(), "doc-1").unwrap());
        assert!(teshi_engine::is_requirement_document_id_reserved(store.path(), "doc-2").unwrap());
        assert!(teshi_engine::is_requirement_document_id_reserved(store.path(), "doc-3").unwrap());
    }

    #[test]
    fn title_rename_preserves_linked_test_point_identity_and_resolution() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        let mut state = AuthoringUiState::empty();
        state.requirements_root = store.path().to_path_buf();
        let id = state.create_default_document().unwrap();
        state.buffer = EditorBuffer::from_string("# Original\n\nStable text\n".into());
        state.save_current_document(project.path()).unwrap();
        state.selection_anchor = Some((2, 0));
        state.selection_end = Some((2, 11));
        state.create_test_point_from_selection().unwrap();
        let artifacts = state.artifacts.as_mut().unwrap();
        artifacts.test_points.test_points[0].review_state = ReviewState::Approved;
        let original = artifacts.test_points.test_points[0].clone();

        state.buffer =
            EditorBuffer::from_string("# A longer renamed requirement\n\nStable text\n".into());
        state.buffer_dirty = true;
        state.save_current_document(project.path()).unwrap();
        let artifacts = state.artifacts.as_ref().unwrap();
        assert_eq!(
            artifacts.documents[0].meta.title,
            "A longer renamed requirement"
        );
        let test_point = &artifacts.test_points.test_points[0];
        assert_eq!(test_point.id, original.id);
        assert_eq!(test_point.title, original.title);
        assert_eq!(test_point.review_state, ReviewState::Approved);
        assert_eq!(test_point.requirement_links[0].document_id, id);
        assert_eq!(
            test_point.requirement_links[0].store_id,
            original.requirement_links[0].store_id
        );
        assert_eq!(
            test_point.requirement_links[0].resolution,
            ResolutionState::Resolved
        );
        let reloaded = AuthoringUiState::load_from_project(project.path(), store.path());
        assert_eq!(
            reloaded.artifacts.as_ref().unwrap().test_points.test_points[0],
            *test_point
        );
    }

    #[test]
    fn default_document_number_avoids_id_path_and_unindexed_file_collisions() {
        for (reserved_path, unindexed_file, expected) in
            [(false, false, 2), (true, false, 4), (false, true, 4)]
        {
            let store = tempdir().unwrap();
            let mut state = AuthoringUiState::empty();
            state.requirements_root = store.path().to_path_buf();
            let mut index = RequirementDocumentIndex::default();
            for n in [1, 3] {
                index.documents.push(RequirementDocumentMeta::new(
                    format!("doc-{n}"),
                    format!("doc-{n}.md"),
                    format!("Document {n}"),
                    compute_document_revision(""),
                ));
            }
            if reserved_path {
                index.documents.push(RequirementDocumentMeta::new(
                    "custom-id",
                    "doc-2.md",
                    "Custom",
                    compute_document_revision(""),
                ));
            }
            if unindexed_file {
                fs::write(store.path().join("doc-2.md"), "keep me").unwrap();
            }
            state.artifacts = Some(AuthoringArtifacts {
                index,
                documents: vec![],
                test_points: Default::default(),
                diagnostics: vec![],
            });
            let id = state.create_default_document().unwrap();
            assert_eq!(id, format!("doc-{expected}"));
            let meta = &state
                .artifacts
                .as_ref()
                .unwrap()
                .index
                .documents
                .last()
                .unwrap();
            assert_eq!(meta.path, format!("doc-{expected}.md"));
            assert_eq!(meta.title, format!("Document {expected}"));
            assert_eq!(
                state.buffer.as_string(),
                format!("# Document {expected}\n\n")
            );
            assert_eq!(state.selected_document_id.as_deref(), Some(id.as_str()));
            if unindexed_file {
                assert_eq!(
                    fs::read_to_string(store.path().join("doc-2.md")).unwrap(),
                    "keep me"
                );
            }
        }
    }

    #[test]
    fn create_document_keeps_document_tree_and_editor_selection_in_sync() {
        let mut state = sample_authoring_state();

        let id = state.create_document("draft.md", "Draft").unwrap();
        let node_id = format!("{TREE_DOC_PREFIX}{id}");
        let path = find_tree_path(&state.tree_items, &node_id).expect("new document in tree");

        assert_eq!(state.selected_document_id.as_deref(), Some(id.as_str()));
        assert_eq!(state.tree_state.selected(), path.as_slice());
        assert!(
            state
                .tree_items
                .iter()
                .any(|item| item.identifier() == &node_id)
        );
        assert_eq!(state.buffer.as_string(), "# Draft\n\n");
        assert!(state.buffer_dirty);
    }

    #[test]
    fn create_document_inherits_named_filter_and_reopens_collapsed_group() {
        let mut state = sample_authoring_state();
        state
            .try_set_iteration_filter(RequirementIterationFilter::Named("Sprint 1".into()))
            .unwrap();
        state.toggle_group_mode();
        let group_path = vec![format!("{TREE_ITER_PREFIX}Sprint 1")];
        state.tree_state.close(&group_path);

        let id = state.create_document("draft.md", "Draft").unwrap();
        let node_id = format!("{TREE_DOC_PREFIX}{id}");
        let path = find_tree_path(&state.tree_items, &node_id).expect("new document in tree");
        let meta = state
            .artifacts
            .as_ref()
            .and_then(|artifacts| artifacts.index.documents.iter().find(|doc| doc.id == id))
            .expect("new document metadata");

        assert_eq!(meta.iteration.as_deref(), Some("Sprint 1"));
        assert_eq!(state.selected_document_id.as_deref(), Some(id.as_str()));
        assert_eq!(state.tree_state.selected(), path.as_slice());
        assert!(state.tree_state.opened().contains(&group_path));
        assert_eq!(state.buffer.as_string(), "# Draft\n\n");
    }

    #[test]
    fn narrow_normal_wide_layout_constants_exist() {
        // Rendering tests live in ui; ensure footer helper is non-empty.
        let line = requirements_footer_line(
            Path::new("%APPDATA%/teshi/requirements"),
            &RequirementIterationFilter::All,
        );
        assert!(!line.spans.is_empty());
    }

    #[test]
    fn dirty_tree_navigation_preserves_buffer_and_selection_until_confirmed() {
        let mut state = sample_authoring_state();
        state.enter_insert_mode();
        state.insert_char('中');
        let body = state.buffer.as_string();
        let tree_selection = state.tree_state.selected().to_vec();
        state.move_tree_selection(1);
        assert_eq!(state.buffer.as_string(), body);
        assert_eq!(state.selected_document_id.as_deref(), Some("doc-sprint"));
        assert_eq!(state.tree_state.selected(), tree_selection);
        assert!(matches!(state.overlay, RequirementsOverlay::Unsaved { .. }));
        state.cancel_overlay();
        assert_eq!(state.editor_mode, RequirementsEditorMode::Insert);
        state.select_document_by_id("doc-sprint");
        assert_eq!(state.buffer.as_string(), body);
        assert!(!state.overlay_active());
    }

    #[test]
    fn discard_new_document_removes_unsaved_draft() {
        let mut state = sample_authoring_state();
        let id = state.create_document("draft.md", "Draft").unwrap();
        state.discard_current_document();
        assert!(!state.buffer_dirty);
        assert!(
            !state
                .artifacts
                .as_ref()
                .unwrap()
                .index
                .documents
                .iter()
                .any(|doc| doc.id == id)
        );
        assert_eq!(state.buffer.as_string(), "login");
    }

    fn sample_authoring_state() -> AuthoringUiState {
        let mut state = AuthoringUiState::empty();
        let store_id = teshi_core::authoring::RequirementStoreId::parse("reqstore-shared").unwrap();
        let mut sprint = RequirementDocumentMeta::new(
            "doc-sprint",
            "auth/login.md",
            "Login",
            teshi_core::authoring::DocumentRevision::new("r1"),
        );
        sprint.iteration = Some("Sprint 1".into());
        let unassigned = RequirementDocumentMeta::new(
            "doc-open",
            "overview.md",
            "Overview",
            teshi_core::authoring::DocumentRevision::new("r2"),
        );
        state.artifacts = Some(AuthoringArtifacts {
            index: RequirementDocumentIndex {
                version: 2,
                store_id: Some(store_id),
                documents: vec![sprint.clone(), unassigned.clone()],
            },
            documents: vec![
                teshi_core::authoring::RequirementDocumentContent {
                    meta: sprint,
                    body: "login".into(),
                },
                teshi_core::authoring::RequirementDocumentContent {
                    meta: unassigned,
                    body: "overview".into(),
                },
            ],
            test_points: Default::default(),
            diagnostics: Vec::new(),
        });
        state.rebuild_tree();
        state
    }

    #[test]
    fn reload_from_disk_preserves_dirty_buffer_after_external_delete() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        initialize_requirement_store(store.path()).unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        state.create_document("req.md", "Requirement").unwrap();
        state.save_current_document(project.path()).unwrap();
        state.buffer = EditorBuffer::from_string("# Local draft\n\nKeep this edit\n".into());
        state.buffer_dirty = true;

        let meta = state.current_document_meta().cloned().unwrap();
        teshi_engine::delete_requirement_document(
            store.path(),
            &meta.id,
            Some(meta.revision.as_str()),
        )
        .unwrap();
        state.reload_from_disk().unwrap();

        assert!(state.buffer_dirty);
        assert_eq!(
            state.buffer.as_string(),
            "# Local draft\n\nKeep this edit\n"
        );
        assert_eq!(
            state.selected_document_id.as_deref(),
            Some(meta.id.as_str())
        );
        assert!(state.current_document_meta().is_none());
    }

    #[test]
    fn selecting_document_by_id_reveals_its_collapsed_tree_path() {
        let mut state = sample_authoring_state();
        let current_path = find_tree_path(&state.tree_items, "req-doc:doc-sprint")
            .expect("selected document path");
        state
            .tree_state
            .close(&current_path[..current_path.len().saturating_sub(1)]);

        state.select_document_by_id("doc-sprint");

        assert_eq!(state.tree_state.selected(), current_path.as_slice());
        assert!(
            state
                .tree_state
                .opened()
                .contains(&current_path[..current_path.len() - 1])
        );

        state.select_document_by_id("doc-open");

        let selected_path = find_tree_path(&state.tree_items, "req-doc:doc-open")
            .expect("selected document in tree");
        assert_eq!(state.selected_document_id.as_deref(), Some("doc-open"));
        assert_eq!(state.tree_state.selected(), selected_path.as_slice());
        assert_eq!(state.buffer.as_string(), "overview");
    }

    #[test]
    fn create_document_rebuilds_before_selecting_new_nested_document() {
        let mut state = sample_authoring_state();
        let current_path = find_tree_path(&state.tree_items, "req-doc:doc-sprint")
            .expect("selected document path");
        state
            .tree_state
            .close(&current_path[..current_path.len().saturating_sub(1)]);

        let id = state.create_document("auth/draft.md", "Draft").unwrap();
        let node_id = format!("{TREE_DOC_PREFIX}{id}");
        let selected_path = find_tree_path(&state.tree_items, &node_id).expect("new document path");

        assert_eq!(state.selected_document_id.as_deref(), Some(id.as_str()));
        assert_eq!(state.tree_state.selected(), selected_path.as_slice());
        assert_eq!(state.buffer.as_string(), "# Draft\n\n");
        assert!(
            state
                .tree_state
                .opened()
                .contains(&current_path[..current_path.len() - 1])
        );
    }

    #[test]
    fn filter_named_and_unassigned_and_empty_results() {
        with_isolated_app_data(|| {
            let mut state = sample_authoring_state();
            state
                .try_set_iteration_filter(RequirementIterationFilter::Named("Sprint 1".into()))
                .unwrap();
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-sprint"));

            state
                .try_set_iteration_filter(RequirementIterationFilter::Unassigned)
                .unwrap();
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-open"));

            state
                .try_set_iteration_filter(RequirementIterationFilter::Named("Missing".into()))
                .unwrap();
            assert!(state.selected_document_id.is_none());
            assert!(state.tree_items.is_empty());
        });
    }

    #[test]
    fn group_by_iteration_inserts_iteration_nodes() {
        with_isolated_app_data(|| {
            let mut state = sample_authoring_state();
            state.toggle_group_mode();
            assert_eq!(state.group_mode, RequirementGroupMode::Iteration);
            let encoded = format!("{:?}", state.tree_items);
            assert!(encoded.contains(TREE_ITER_PREFIX));
        });
    }

    #[test]
    fn tree_navigation_selects_documents_inside_iteration_groups() {
        with_isolated_app_data(|| {
            let mut state = sample_authoring_state();
            state.toggle_group_mode();
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-sprint"));

            state.move_tree_selection(1);
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-open"));
            assert_eq!(
                state.tree_state.selected().last().map(String::as_str),
                Some("req-doc:doc-open")
            );

            state.move_tree_selection(-1);
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-sprint"));
        });
    }

    #[test]
    fn dirty_buffer_blocks_filter_that_would_hide_document() {
        let mut state = sample_authoring_state();
        state.select_document_by_id("doc-sprint");
        state.buffer_dirty = true;
        state
            .try_set_iteration_filter(RequirementIterationFilter::Unassigned)
            .unwrap();
        assert!(matches!(state.overlay, RequirementsOverlay::Unsaved { .. }));
        assert_eq!(state.iteration_filter, RequirementIterationFilter::All);
    }

    #[test]
    fn two_projects_share_the_same_requirement_tree() {
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let project_a = tempdir().unwrap();
        let project_b = tempdir().unwrap();
        let mut writer = AuthoringUiState::load_from_project(project_a.path(), store.path());
        writer.create_document("shared.md", "Shared").unwrap();
        writer
            .save_current_document(project_a.path())
            .expect("save");
        let a = AuthoringUiState::load_from_project(project_a.path(), store.path());
        let b = AuthoringUiState::load_from_project(project_b.path(), store.path());
        assert_eq!(
            a.artifacts.as_ref().unwrap().index.documents.len(),
            b.artifacts.as_ref().unwrap().index.documents.len()
        );
        assert_eq!(
            a.artifacts.as_ref().unwrap().index.store_id,
            b.artifacts.as_ref().unwrap().index.store_id
        );
    }

    #[test]
    fn create_document_on_invalid_nonempty_store_does_not_initialize() {
        let store = tempdir().unwrap();
        fs::write(store.path().join("notes.md"), "keep me").unwrap();
        let project = tempdir().unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        assert!(state.artifacts.as_ref().unwrap().index.store_id.is_none());
        state.create_document("doc-1.md", "Doc").unwrap();
        assert!(state.artifacts.as_ref().unwrap().index.store_id.is_none());
        let err = state
            .save_current_document(project.path())
            .expect_err("save must fail closed");
        assert!(err.to_string().contains("not empty"));
        assert_eq!(
            fs::read_to_string(store.path().join("notes.md")).unwrap(),
            "keep me"
        );
        assert!(!store.path().join("doc-1.md").exists());
        assert!(!store.path().join("_teshi.json").exists());
    }

    fn with_isolated_app_data<T>(f: impl FnOnce() -> T) -> T {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();
        let prev = std::env::var("TESHI_APP_DATA_DIR").ok();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("TESHI_APP_DATA_DIR", tmp.path());
        }
        let result = f();
        match prev {
            Some(value) => unsafe { std::env::set_var("TESHI_APP_DATA_DIR", value) },
            None => unsafe { std::env::remove_var("TESHI_APP_DATA_DIR") },
        }
        result
    }

    #[test]
    fn view_prefs_restore_and_invalid_named_filter_falls_back_to_all() {
        with_isolated_app_data(|| {
            let mut state = sample_authoring_state();
            state
                .try_set_iteration_filter(RequirementIterationFilter::Named("Sprint 1".into()))
                .unwrap();
            state.toggle_group_mode();

            let mut restored = sample_authoring_state();
            restored.restore_view_prefs();
            assert_eq!(
                restored.iteration_filter,
                RequirementIterationFilter::Named("Sprint 1".into())
            );
            assert_eq!(restored.group_mode, RequirementGroupMode::Iteration);

            restored.artifacts.as_mut().unwrap().index.documents[0].iteration = None;
            restored.restore_view_prefs();
            assert_eq!(restored.iteration_filter, RequirementIterationFilter::All);
            assert_eq!(restored.group_mode, RequirementGroupMode::Iteration);
        });
    }

    #[test]
    fn overlay_edit_and_filter_picker_navigation() {
        let mut state = sample_authoring_state();
        state.open_filter_picker();
        state.overlay_move_selection(1);
        if let RequirementsOverlay::FilterPicker { selection } = state.overlay {
            assert_eq!(selection, 1);
        } else {
            panic!("expected filter picker");
        }
        state.select_document_by_id("doc-sprint");
        state.open_iteration_editor();
        state.overlay_insert_char('X');
        state.overlay_backspace();
        state.cancel_overlay();
        assert_eq!(state.overlay, RequirementsOverlay::None);
    }
    #[test]
    fn vim_word_motions_cross_tokens_and_extend_visual_selection() {
        let mut state = AuthoringUiState::empty();
        state.buffer = EditorBuffer::from_string("alpha, beta\n甲乙 🙂 尾".into());
        state.cursor_col = 1;
        state.vim_word_forward();
        assert_eq!((state.cursor_row, state.cursor_col), (0, 5));
        state.vim_word_forward();
        assert_eq!((state.cursor_row, state.cursor_col), (0, 7));
        state.vim_word_backward();
        assert_eq!(state.cursor_col, 5);
        state.vim_word_backward();
        assert_eq!(state.cursor_col, 0);
        state.vim_word_end();
        assert_eq!(state.cursor_col, 4);
        state.vim_word_end();
        assert_eq!(state.cursor_col, 5);
        state.vim_word_forward();
        state.vim_word_forward();
        assert_eq!((state.cursor_row, state.cursor_col), (1, 0));
        state.enter_visual_char();
        state.vim_word_forward();
        assert_eq!(state.cursor_col, 3);
        assert_eq!(state.selection_end, Some((1, 4)));
    }

    #[test]
    fn undo_after_save_is_dirty_and_cannot_cross_documents() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        initialize_requirement_store(store.path()).unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        let a = state.create_document("a.md", "A").unwrap();
        state.save_current_document(project.path()).unwrap();
        let original = state.buffer.as_string();
        state.cursor_col = state.buffer.line_len_chars(0);
        state.enter_insert_mode();
        state.insert_char('X');
        state.exit_insert_mode();
        let changed = state.buffer.as_string();
        state.save_current_document(project.path()).unwrap();
        state.undo_edit();
        assert_eq!(state.buffer.as_string(), original);
        assert!(
            state.buffer_dirty,
            "undo must compare against the latest saved body"
        );
        state.save_current_document(project.path()).unwrap();
        assert_eq!(
            fs::read_to_string(store.path().join("a.md")).unwrap(),
            original
        );
        state.redo_edit();
        assert_eq!(state.buffer.as_string(), changed);
        assert!(state.buffer_dirty);
        state.save_current_document(project.path()).unwrap();
        let b = state.create_document("b.md", "B").unwrap();
        state.save_current_document(project.path()).unwrap();
        let body_b = state.buffer.as_string();
        state.undo_edit();
        assert_eq!(state.selected_document_id.as_deref(), Some(b.as_str()));
        assert_eq!(state.buffer.as_string(), body_b);
        assert!(!state.buffer_dirty);
        state.select_document_by_id(&a);
        state.undo_edit();
        assert_eq!(state.buffer.as_string(), changed);
        assert!(!state.buffer_dirty);
    }

    #[test]
    fn bounded_vim_editing_keeps_unicode_and_local_undo() {
        let mut state = AuthoringUiState::empty();
        state.selected_document_id = Some("doc-local".into());
        state.buffer = EditorBuffer::from_string("# 标题\n第二行 🙂".into());
        state.enter_insert_mode();
        state.cursor_row = 0;
        state.cursor_col = 3;
        state.insert_char('X');
        state.exit_insert_mode();
        assert_eq!(state.buffer.line(0), "# 标X题");
        state.undo_edit();
        assert_eq!(state.buffer.line(0), "# 标题");
        state.redo_edit();
        assert_eq!(state.buffer.line(0), "# 标X题");

        state.cursor_row = 1;
        state.cursor_col = 0;
        state.enter_visual_char();
        state.move_cursor_right();
        state.extend_visual_selection();
        assert!(state.yank_visual_selection());
        assert_eq!(state.yank_register, "第二");
        assert!(state.delete_visual_selection());
        assert_eq!(state.buffer.line(1), "行 🙂");
    }

    #[test]
    fn vim_paste_before_and_after_have_bounded_line_semantics() {
        let mut state = AuthoringUiState::empty();
        state.buffer = EditorBuffer::from_string("a\nb".into());
        state.yank_register = "x\n".into();
        state.cursor_row = 1;
        assert!(state.paste_register(true));
        assert_eq!(state.buffer.as_string(), "a\nx\nb");
        state.cursor_row = 0;
        assert!(state.paste_register(false));
        assert_eq!(state.buffer.as_string(), "a\nx\nx\nb");
    }
}
