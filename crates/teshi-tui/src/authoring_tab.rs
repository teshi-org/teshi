//! Requirements tab state, tree construction, and editing helpers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use teshi_core::authoring::{
    AuthoringArtifacts, AuthoringDiagnostic, HierarchyPath, RequirementDocumentMeta,
    RequirementGroupMode, RequirementIterationFilter, ResolutionState, ReviewState, TestPoint,
    TextRange, UNASSIGNED_ITERATION_LABEL, create_requirement_link, line_col_range_to_char_range,
    re_resolve_document_links,
};
use teshi_engine::{
    compute_document_revision, initialize_requirement_store, load_authoring_artifacts,
    save_requirement_markdown, save_test_points,
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
    pub scroll_row: usize,
    pub selection_anchor: Option<(usize, usize)>,
    pub selection_end: Option<(usize, usize)>,
    pub iteration_filter: RequirementIterationFilter,
    pub group_mode: RequirementGroupMode,
    pub overlay: RequirementsOverlay,
}

impl AuthoringUiState {
    /// Enters text input when a document is selected.
    pub fn enter_insert_mode(&mut self) {
        if self.selected_document_id.is_some() {
            self.editor_mode = RequirementsEditorMode::Insert;
        }
    }

    /// Returns to command navigation.
    pub fn exit_insert_mode(&mut self) {
        self.editor_mode = RequirementsEditorMode::Browse;
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
            selection_anchor: None,
            selection_end: None,
            iteration_filter: RequirementIterationFilter::All,
            group_mode: RequirementGroupMode::Path,
            overlay: RequirementsOverlay::None,
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
        if let Some(selected) = self.selected_document_id.clone() {
            if !visible.iter().any(|id| id == &selected) {
                if let Some(first) = visible.first() {
                    self.select_document_by_id(first);
                } else {
                    if self.buffer_dirty {
                        return;
                    }
                    self.exit_insert_mode();
                    self.selected_document_id = None;
                    self.buffer = EditorBuffer::from_string(String::new());
                    self.buffer_dirty = false;
                }
            }
        } else if let Some(first) = visible.first() {
            self.select_document_by_id(first);
        }
        if let Some(id) = &self.selected_document_id {
            self.tree_state
                .select(vec![format!("{TREE_DOC_PREFIX}{id}")]);
        }
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
            if self.selected_document_id.as_deref() == Some(doc_id) {
                self.tree_state.select(vec![node_id.to_string()]);
            }
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
        let meta = RequirementDocumentMeta::new(id.clone(), relative_path, title, revision);

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
        self.draft_document_id = Some(id.clone());
        id
    }

    /// Saves the current requirement document and refreshes link resolutions.
    pub fn save_current_document(&mut self, project_root: &Path) -> Result<()> {
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
        let body = self.buffer.as_string();
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
                doc.meta.revision = index_meta.revision.clone();
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
        save_test_points(project_root, &artifacts.test_points)?;
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
        let doc_ids: Vec<String> = self
            .artifacts
            .as_ref()
            .map(|a| {
                visible_document_ids(&a.index, &self.iteration_filter)
                    .into_iter()
                    .map(|id| format!("{TREE_DOC_PREFIX}{id}"))
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
        ReviewState::Proposed => Style::default().fg(Color::Yellow),
        ReviewState::Approved => Style::default().fg(Color::Green),
        ReviewState::Rejected => Style::default().fg(Color::Red),
        ReviewState::NeedsReview => Style::default().fg(Color::Magenta),
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
    Line::from(vec![
        Span::styled(
            format!(" Store {} ", store_path.display()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" Filter {filter_label} [i] "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled(" Group [g] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Iteration [I] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" New doc [Ctrl+n] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Save [s] ", Style::default().fg(Color::DarkGray)),
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
        state.create_document("sample.md", "Sample");
        state.buffer = EditorBuffer::from_string("# Sample\n\nedited".into());
        state.save_current_document(root).expect("save");
        let reloaded = AuthoringUiState::load_from_project(root, store.path());
        assert!(reloaded.discovered);
        let body = reloaded.buffer.as_string();
        assert!(body.contains("edited"));
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
        let id = state.create_document("draft.md", "Draft");
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
        writer.create_document("shared.md", "Shared");
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
        state.create_document("doc-1.md", "Doc");
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
}
