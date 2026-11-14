//! Requirements tab state, tree construction, and editing helpers.

mod organize;
pub use organize::{CursorMotion, LineInput, MenuCommand, OrganizationForm, TreeTarget};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::app::{App, MainTab};
use crate::theme;
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use teshi_core::authoring::{
    AuthoringArtifacts, AuthoringDiagnostic, HierarchyPath, RequirementDocumentMeta,
    RequirementGroupMode, RequirementIterationFilter, RequirementPriority, RequirementTagStyle,
    ResolutionState, ReviewState, TestPoint, TextRange, UNASSIGNED_ITERATION_LABEL,
    create_requirement_link, line_col_range_to_char_range, mark_document_links_missing,
    re_resolve_document_links, requirement_title,
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

/// Delay before a typed tree search rebuilds the requirement tree.
const REQUIREMENT_SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);

/// Input mode for the Markdown pane, independent of Explore.
///
/// `Esc` steps back one level: Insert or Visual returns to Browse, and Browse
/// returns to Preview. Only Preview renders the Markdown; every other mode
/// shows the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequirementsEditorMode {
    /// Read-only rendered Markdown, entered whenever a document is opened.
    #[default]
    Preview,
    /// Vim Normal mode over the Markdown source.
    Browse,
    /// Text insertion into the Markdown source.
    Insert,
    /// Characterwise Vim selection.
    VisualChar,
    /// Linewise Vim selection.
    VisualLine,
}

/// Rendered preview of one document body, cached until its inputs change.
#[derive(Debug, Clone, Default)]
pub struct RequirementPreview {
    /// Relative image paths resolve against the document's folder.
    document_id: Option<String>,
    body: String,
    width: usize,
    image_rows: u16,
    /// Display rows, already wrapped to the pane width.
    pub lines: Vec<Line<'static>>,
    /// What each image display row shows, by display row index.
    pub images: HashMap<usize, EditorImageRow>,
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
    ProjectFilter(teshi_core::authoring::RequirementProjectFilter),
    Iteration { document_id: String, value: String },
    Action(crate::keymap::Action),
}

/// Overlay used to pick a filter or edit a document iteration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RequirementsOverlay {
    /// Space menu listing the operations valid for the tree row captured when it opened.
    ActionMenu {
        /// Row the chosen operation applies to.
        target: TreeTarget,
        /// Highlighted menu row.
        selection: usize,
    },
    /// Folder or saved-query text form.
    OrganizationForm(OrganizationForm),
    /// Exact folder membership of one requirement; no checked folder means Inbox.
    FolderPicker {
        /// Stable target captured before any external reload can change selection.
        document_id: String,
        /// Every manual folder as `(id, path)`, sorted by path.
        options: Vec<(String, String)>,
        /// Folder IDs the requirement will belong to.
        checked: BTreeSet<String>,
        /// Highlighted option index.
        selection: usize,
    },
    /// Tag set of one requirement: check existing tags or add a new one.
    TagPicker {
        /// Stable target captured before any external reload can change selection.
        document_id: String,
        /// Every tag in use across the store, sorted.
        options: Vec<String>,
        /// Tags the requirement will carry after saving.
        checked: BTreeSet<String>,
        /// Draft display settings for every option, saved together with `checked`.
        styles: BTreeMap<String, RequirementTagStyle>,
        /// Tags whose settings were changed in this picker; only these are written,
        /// so settings another process changed meanwhile survive the save.
        edited: BTreeSet<String>,
        /// Highlighted row: `options.len()` is the "New tag…" row.
        selection: usize,
        /// Buffer of the inline new-tag field; `Some` while that row accepts text.
        new_tag: Option<String>,
    },
    /// Confirm removal of a folder (with subfolders) or a saved query.
    RemoveCollection {
        /// Stable collection ID.
        collection_id: String,
        /// Display path of the collection.
        name: String,
        /// Whether the collection is a saved query.
        query: bool,
        /// Documents left without any folder after removal.
        inbox_returns: usize,
    },
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
    /// Choose an optional P0-P3 scheduling priority.
    PriorityEdit {
        /// Stable target captured before any external reload can change selection.
        document_id: String,
        /// Highlighted option: 0 = unset, 1-4 = P0-P3.
        selection: usize,
        /// Persisted value observed when the picker opened, retained across reloads.
        expected: Option<RequirementPriority>,
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

/// Widest image box in a requirement document, in cells.
const EDITOR_IMAGE_COLS: usize = 64;

/// Content of one display row below a line that references an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorImageRow {
    /// Row `row` of a `size` cell box showing the file at `path`.
    Pixels {
        path: PathBuf,
        row: u16,
        size: (u16, u16),
    },
    /// A one-row stand-in naming an existing image the terminal cannot draw.
    Label { path: PathBuf, alt: String },
    /// The reference names no existing file.
    Missing { target: String },
}

/// Cheap fingerprint of the shared requirement index. Document bodies change
/// through the index revision on every successful store write.
fn requirement_store_signature(root: &Path) -> Option<(SystemTime, u64)> {
    let metadata = std::fs::metadata(root.join("_teshi.json")).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

/// Appends `markdown` rendered as in AI Chat, wrapped to `width` cells.
///
/// `separate` adds one blank row first, as the renderer itself puts between
/// blocks; it is lost when a document is rendered in pieces around images.
fn push_preview_markdown(
    lines: &mut Vec<Line<'static>>,
    markdown: &str,
    width: usize,
    separate: bool,
) {
    if markdown.trim().is_empty() {
        return;
    }
    if separate {
        lines.push(Line::default());
    }
    let rendered =
        crate::markdown::render_markdown_width(markdown, width.min(u16::MAX as usize) as u16);
    lines.extend(
        rendered
            .into_iter()
            .flat_map(|line| crate::ui::chat::wrap_line_to_width(line, width)),
    );
}

/// UI state for requirement documents and linked test points.
pub struct AuthoringUiState {
    pub artifacts: Option<AuthoringArtifacts>,
    pub discovered: bool,
    pub requirements_root: PathBuf,
    pub project_root: PathBuf,
    /// Identity error shown instead of silently treating a malformed project as empty.
    pub project_identity_error: Option<String>,
    /// Include implemented and verified requirements in project views.
    pub show_completed: bool,
    /// Last loaded project organization; external edits are detected against it.
    organization: Option<teshi_engine::organization::RequirementOrganization>,
    /// Fingerprint of the shared requirement index at the last successful load.
    store_signature: Option<(SystemTime, u64)>,
    /// Throttles background checks of the shared requirement store.
    last_store_check: Option<Instant>,
    /// Draft document and the folder it was created in, filed on its first save.
    draft_folder: Option<(String, String)>,
    /// Styled label of every tree node by identifier, kept beside `tree_items`
    /// because the tree widget does not expose item text and rendering rewraps
    /// labels to the pane width.
    pub tree_labels: HashMap<String, Line<'static>>,
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
    /// First visible row of the rendered preview.
    pub preview_scroll: usize,
    preview: RequirementPreview,
    /// Width used by the most recent editor wrap map.
    pub editor_wrap_width: usize,
    /// Horizontal offsets for the tree, editor, and linked test points. The tree
    /// wraps its labels, so its entry stays `0`.
    pub horizontal_scroll: [usize; 3],
    /// Cursor position at the previous render, used to follow keyboard navigation.
    pub rendered_cursor: Option<(usize, usize)>,
    /// Linked test point viewport start.
    pub linked_scroll: usize,
    /// Last rendered linked selection; wheel scrolling must not follow it again.
    pub rendered_linked_selection: Option<usize>,
    /// Last rendered overflow limits, indexed by pane and axis (horizontal, vertical).
    pub scroll_limits: [[usize; 2]; 3],
    /// Scrollbars drawn last frame, indexed by pane and axis (horizontal, vertical).
    /// `None` when that pane does not overflow, so no stale bar keeps a hitbox.
    pub pane_bars: [[Option<crate::app::ScrollbarBar>; 2]; 3],
    /// Bar being dragged: pane, axis, and the pointer offset inside the thumb.
    pub scrollbar_grab: Option<(usize, usize, u16)>,
    /// Pointer position from the last render, used to highlight a hovered bar.
    pub pointer: Option<(u16, u16)>,
    pub selection_anchor: Option<(usize, usize)>,
    pub selection_end: Option<(usize, usize)>,
    pub iteration_filter: RequirementIterationFilter,
    /// Project window, separate from the iteration filter.
    pub project_filter: teshi_core::authoring::RequirementProjectFilter,
    pub group_mode: RequirementGroupMode,
    pub overlay: RequirementsOverlay,
    /// Live text typed into the tree's `/` search.
    pub search_query: String,
    /// Whether the tree is capturing keys as search input.
    pub search_active: bool,
    /// Query whose filtered tree is displayed; committed after the debounce.
    applied_search_query: String,
    pending_search: Option<Instant>,
    undo_stack: Vec<RequirementsEditSnapshot>,
    redo_stack: Vec<RequirementsEditSnapshot>,
    yank_register: String,
    insert_session_undo: bool,
    vertical_motion: Option<(usize, usize, usize, usize)>,
}

impl AuthoringUiState {
    fn project_filtered_index(&self) -> teshi_core::authoring::RequirementDocumentIndex {
        let mut index = self
            .artifacts
            .as_ref()
            .map(|a| a.index.clone())
            .unwrap_or_default();
        index
            .documents
            .retain(|doc| doc.matches_project_filter(&self.project_filter));
        let query = self.applied_search_query.trim().to_lowercase();
        if !query.is_empty() {
            index
                .documents
                .retain(|doc| requirement_matches_search(doc, &query));
        }
        index
    }

    /// Label for the active project window.
    pub fn project_scope_label(&self) -> &'static str {
        use teshi_core::authoring::RequirementProjectFilter;
        match self.project_filter {
            RequirementProjectFilter::All => "All projects",
            RequirementProjectFilter::Unassigned => "No project",
            RequirementProjectFilter::Project(_) => "Current project",
        }
    }

    /// Changes the project window after resolving any unsaved editor buffer.
    pub fn set_project_filter(&mut self, filter: teshi_core::authoring::RequirementProjectFilter) {
        if self.buffer_dirty {
            self.guard_unsaved(RequirementsPendingAction::ProjectFilter(filter));
            return;
        }
        self.project_filter = filter;
        self.overlay = RequirementsOverlay::None;
        self.rebuild_tree();
    }

    /// Cycles current project, all projects, and documents with no project.
    ///
    /// # Errors
    /// Returns an error when the project's identity file is malformed or unreadable.
    pub fn cycle_project_filter(&mut self) -> Result<(), String> {
        use teshi_core::authoring::RequirementProjectFilter;
        let next = match self.project_filter {
            RequirementProjectFilter::Project(_) => RequirementProjectFilter::All,
            RequirementProjectFilter::All => RequirementProjectFilter::Unassigned,
            RequirementProjectFilter::Unassigned => {
                teshi_engine::requirement_project_filter(&self.project_root)
                    .map_err(|err| err.to_string())?
            }
        };
        self.set_project_filter(next);
        Ok(())
    }

    /// Links or unlinks the selected saved document only for the current project.
    ///
    /// # Errors
    /// Returns an error for dirty buffers, missing selection, invalid identity, or persistence failure.
    pub fn toggle_current_project_link(&mut self) -> Result<bool> {
        if self.buffer_dirty {
            anyhow::bail!(
                "Save or discard the requirement before changing its project association"
            );
        }
        let id = self
            .selected_document_id
            .clone()
            .context("Select a requirement document first")?;
        let project = teshi_engine::read_requirement_project(&self.project_root)?;
        let doc = teshi_engine::read_requirement_document(&self.requirements_root, &id)?;
        let linked = !project.is_some_and(|p| doc.meta.projects.contains_key(&p.id));
        teshi_engine::update_requirement_projects(
            &self.requirements_root,
            &self.project_root,
            &[&id],
            linked,
            None,
        )?;
        self.reload_from_disk()?;
        Ok(linked)
    }

    /// First visible index of one pane's axis, matching [`Self::set_pane_scroll`].
    pub fn pane_scroll(&self, pane: usize, axis: usize) -> usize {
        if axis == 0 {
            return self.horizontal_scroll[pane];
        }
        match pane {
            0 => self.tree_state.get_offset(),
            1 if self.is_previewing() => self.preview_scroll,
            1 => self.scroll_row,
            _ => self.linked_scroll,
        }
    }

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
                1 if self.is_previewing() => self.preview_scroll = value,
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

    /// Renders the buffer as Markdown for the preview pane.
    ///
    /// The result is reused while the body, `width`, and `image_rows` are
    /// unchanged, so unsaved edits appear as soon as the preview is shown.
    /// `image_rows` is the height of a pixel image box; `1` draws each image
    /// as a one-row text stand-in for terminals without a pixel protocol.
    pub fn refresh_preview(&mut self, width: usize, image_rows: u16) -> &RequirementPreview {
        let width = width.max(1);
        let body = self.buffer.as_string();
        if self.preview.width == width
            && self.preview.image_rows == image_rows
            && self.preview.document_id == self.selected_document_id
            && self.preview.body == body
        {
            return &self.preview;
        }
        let document_dir = self.current_document_dir();
        let mut lines = Vec::new();
        let mut images = HashMap::new();
        let mut chunk = String::new();
        let mut in_fence = false;
        let mut after_image = false;
        for line in body.lines() {
            let marker = line.trim_start();
            if marker.starts_with("```") || marker.starts_with("~~~") {
                in_fence = !in_fence;
            }
            chunk.push_str(line);
            chunk.push('\n');
            // Code samples show image syntax literally.
            let refs = if in_fence {
                Vec::new()
            } else {
                crate::doc_images::image_refs(line)
            };
            if refs.is_empty() {
                continue;
            }
            // Markdown renders whole blocks, so the text up to the image line is
            // flushed first and the image boxes land directly below that line.
            push_preview_markdown(&mut lines, &std::mem::take(&mut chunk), width, after_image);
            for image in self.image_rows(refs, width, image_rows, document_dir.as_deref()) {
                images.insert(lines.len(), image);
                lines.push(Line::default());
            }
            after_image = true;
        }
        push_preview_markdown(&mut lines, &chunk, width, after_image);
        self.preview = RequirementPreview {
            document_id: self.selected_document_id.clone(),
            body,
            width,
            image_rows,
            lines,
            images,
        };
        &self.preview
    }

    /// The preview rendered by the last [`Self::refresh_preview`].
    pub fn preview(&self) -> &RequirementPreview {
        &self.preview
    }

    /// Image shown at cell (`col`, `row`) of the preview text area.
    ///
    /// A pixel box only covers its own columns; a one-row stand-in covers its
    /// whole row. Missing images and the source view have nothing to open.
    /// Documents may come from agents or shared stores and Markdown accepts
    /// any target, so only files with an image extension are offered; the
    /// viewer can hand them to the operating system.
    pub fn preview_image_at(&self, col: usize, row: usize) -> Option<PathBuf> {
        if !self.is_previewing() {
            return None;
        }
        let path = match self.preview.images.get(&(self.preview_scroll + row))? {
            EditorImageRow::Pixels { path, size, .. } => {
                (col < usize::from(size.0)).then_some(path)?
            }
            EditorImageRow::Label { path, .. } => path,
            EditorImageRow::Missing { .. } => return None,
        };
        crate::chat_images::has_image_extension(path).then(|| path.clone())
    }

    /// Display rows for the local images referenced on one line.
    fn image_rows(
        &self,
        refs: Vec<crate::doc_images::ImageRef>,
        width: usize,
        box_rows: u16,
        document_dir: Option<&Path>,
    ) -> Vec<EditorImageRow> {
        let size = (width.min(EDITOR_IMAGE_COLS) as u16, box_rows);
        refs.into_iter()
            .flat_map(|reference| {
                match crate::doc_images::resolve(
                    &reference.target,
                    document_dir,
                    &self.project_root,
                ) {
                    Some(path) if box_rows > 1 => (0..box_rows)
                        .map(|row| EditorImageRow::Pixels {
                            path: path.clone(),
                            row,
                            size,
                        })
                        .collect(),
                    Some(path) => vec![EditorImageRow::Label {
                        path,
                        alt: reference.alt,
                    }],
                    None => vec![EditorImageRow::Missing {
                        target: reference.target,
                    }],
                }
            })
            .collect()
    }

    /// Whether the Markdown pane shows the rendered preview.
    pub fn is_previewing(&self) -> bool {
        self.editor_mode == RequirementsEditorMode::Preview
    }

    /// Leaves the preview for Vim Normal mode over the source without starting an edit.
    pub fn enter_edit_view(&mut self) {
        if self.is_previewing() && self.selected_document_id.is_some() {
            self.editor_mode = RequirementsEditorMode::Browse;
            // Bring the cursor back into view, wherever the preview was scrolled.
            self.rendered_cursor = None;
        }
    }

    /// Returns from the source view to the rendered preview.
    ///
    /// The buffer is kept as is: unsaved edits show in the preview and stay
    /// pending until they are saved or discarded.
    pub fn enter_preview(&mut self) {
        self.insert_session_undo = false;
        self.vertical_motion = None;
        self.selection_anchor = None;
        self.selection_end = None;
        self.editor_mode = RequirementsEditorMode::Preview;
    }

    /// Scrolls the preview by `delta` rows within the last rendered limit.
    pub fn scroll_preview(&mut self, delta: isize) {
        self.set_pane_scroll(1, 1, self.preview_scroll.saturating_add_signed(delta));
    }

    /// Folder of the open document, which relative image paths start from.
    fn current_document_dir(&self) -> Option<PathBuf> {
        let meta = self.current_document_meta()?;
        let root = if self.requirements_root.as_os_str().is_empty() {
            &self.project_root
        } else {
            &self.requirements_root
        };
        root.join(&meta.path).parent().map(Path::to_path_buf)
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

    /// Returns to command navigation; the preview stays as it is.
    pub fn exit_insert_mode(&mut self) {
        self.insert_session_undo = false;
        if !self.is_previewing() {
            self.editor_mode = RequirementsEditorMode::Browse;
        }
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
        // Undo runs in the source view. A snapshot taken by an `i` pressed in
        // the preview must not hide the restored text behind the preview again.
        self.editor_mode = match snapshot.mode {
            RequirementsEditorMode::Preview => RequirementsEditorMode::Browse,
            mode => mode,
        };
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
            self.enter_preview();
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
            project_identity_error: None,
            show_completed: false,
            organization: None,
            store_signature: None,
            last_store_check: None,
            draft_folder: None,
            tree_labels: HashMap::new(),
            tree_items: Vec::new(),
            tree_state: TreeState::default(),
            focus: RequirementsFocus::Tree,
            editor_mode: RequirementsEditorMode::Preview,
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
            preview_scroll: 0,
            preview: RequirementPreview::default(),
            editor_wrap_width: 0,
            horizontal_scroll: [0; 3],
            rendered_cursor: None,
            linked_scroll: 0,
            rendered_linked_selection: None,
            scroll_limits: [[0; 2]; 3],
            pane_bars: [[None; 2]; 3],
            scrollbar_grab: None,
            pointer: None,
            selection_anchor: None,
            selection_end: None,
            iteration_filter: RequirementIterationFilter::All,
            project_filter: teshi_core::authoring::RequirementProjectFilter::All,
            group_mode: RequirementGroupMode::Path,
            overlay: RequirementsOverlay::None,
            search_query: String::new(),
            search_active: false,
            applied_search_query: String::new(),
            pending_search: None,
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
        state.project_filter = match teshi_engine::requirement_project_filter(project_root) {
            Ok(filter) => filter,
            Err(error) => {
                state.project_identity_error = Some(error.to_string());
                teshi_core::authoring::RequirementProjectFilter::Project(String::new())
            }
        };
        match load_authoring_artifacts(project_root, requirements_root) {
            Ok(result) => {
                state.discovered = result.discovered;
                state.artifacts = result.artifacts;
                state.rebuild_tree();
                state.restore_view_prefs();
            }
            Err(error) => {
                let _ = error;
            }
        }
        state.store_signature = requirement_store_signature(requirements_root);
        state
    }

    /// Records the store fingerprint after a write from this process so the next
    /// poll does not mistake our own save for an external change.
    fn note_store_write(&mut self) {
        self.store_signature = requirement_store_signature(&self.requirements_root);
        self.last_store_check = Some(Instant::now());
    }

    /// The draft document that exists only in memory, with its unpersisted body.
    fn preserved_draft_document(
        &self,
    ) -> Option<(teshi_core::authoring::RequirementDocumentMeta, String)> {
        let id = self.draft_document_id.as_ref()?;
        let artifacts = self.artifacts.as_ref()?;
        let meta = artifacts
            .index
            .documents
            .iter()
            .find(|doc| doc.id == *id)?
            .clone();
        Some((meta, self.buffer.as_string()))
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
        let focus = self.focus;
        let dirty = self.buffer_dirty;
        let preserved_buffer = dirty.then(|| self.buffer.clone());
        let cursor_row = self.cursor_row;
        let cursor_col = self.cursor_col;
        let desired_col = self.desired_col;
        let scroll_row = self.scroll_row;
        let selection_anchor = self.selection_anchor;
        let selection_end = self.selection_end;

        if matches!(
            self.project_filter,
            teshi_core::authoring::RequirementProjectFilter::Project(_)
        ) {
            self.project_filter = teshi_engine::requirement_project_filter(&project_root)?;
        }
        let loaded = load_authoring_artifacts(&project_root, &requirements_root)?;
        // Requirement Markdown/`_teshi.json` can change on disk; keep in-memory test
        // points so unsaved review-state edits are not discarded at an agent boundary.
        let preserved_test_points = self.artifacts.as_ref().map(|a| a.test_points.clone());
        // An unsaved draft exists only in memory, so a reload from disk would
        // leave the editor on a document id the index no longer contains.
        let preserved_draft = self.preserved_draft_document();
        self.discovered = loaded.discovered;
        self.artifacts = loaded.artifacts;
        if let (Some(artifacts), Some(test_points)) =
            (self.artifacts.as_mut(), preserved_test_points)
        {
            artifacts.test_points = test_points;
        }
        if let Some(artifacts) = self.artifacts.as_mut()
            && let Some((meta, body)) = preserved_draft
            && !artifacts
                .index
                .documents
                .iter()
                .any(|doc| doc.id == meta.id)
        {
            artifacts.index.documents.push(meta.clone());
            artifacts
                .documents
                .push(teshi_core::authoring::RequirementDocumentContent { meta, body });
        }
        self.iteration_filter = filter;
        self.group_mode = group_mode;
        self.overlay = overlay;
        self.rebuild_tree();
        if let Some(id) = selected.as_ref() {
            let still_present = self.artifacts.as_ref().is_some_and(|artifacts| {
                artifacts.index.documents.iter().any(|doc| {
                    doc.id == *id
                        && doc.matches_project_filter(&self.project_filter)
                        && doc.matches_iteration_filter(&self.iteration_filter)
                })
            });
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
        self.focus = focus;
        self.store_signature = requirement_store_signature(&self.requirements_root);
        self.last_store_check = Some(Instant::now());
        Ok(())
    }

    /// Reloads when another Teshi process or the CLI changed the shared store.
    ///
    /// Returns whether a reload happened. A failed read keeps the current
    /// snapshot; the next successful poll can recover.
    pub fn poll_external_store_changes(&mut self) -> Result<bool> {
        if self
            .last_store_check
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return Ok(false);
        }
        self.last_store_check = Some(Instant::now());
        let signature = requirement_store_signature(&self.requirements_root);
        if signature.is_none() || signature == self.store_signature {
            return Ok(false);
        }
        self.reload_from_disk()?;
        Ok(true)
    }

    /// Rebuilds the requirement tree from the current index, filter, and group mode.
    pub fn rebuild_tree(&mut self) {
        let mut index = self.project_filtered_index();
        let project = match &self.project_filter {
            teshi_core::authoring::RequirementProjectFilter::Project(project) => {
                Some(project.clone())
            }
            _ => None,
        };
        if let Some(project) = &project {
            index.documents.retain(|d| {
                self.show_completed
                    || d.projects.get(project)
                        == Some(&teshi_core::authoring::RequirementProjectStatus::Planned)
            });
        }
        let searching = !self.applied_search_query.trim().is_empty();
        if searching {
            (self.tree_items, self.tree_labels) =
                build_search_results(&index, &self.iteration_filter);
        } else if let Some(project) = project {
            // Path grouping in project scope is replaced by the project's own folders.
            if self.group_mode == RequirementGroupMode::Path {
                self.rebuild_folders_tree(&project);
            } else {
                (self.tree_items, self.tree_labels) = build_requirement_tree(
                    &index,
                    &self.iteration_filter,
                    RequirementGroupMode::Iteration,
                );
            }
        } else {
            (self.tree_items, self.tree_labels) =
                build_requirement_tree(&index, &self.iteration_filter, self.group_mode);
        }
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

        // A folder or query row stays selected across rebuilds, e.g. after renaming it.
        let keep_group_row = self
            .tree_state
            .selected()
            .last()
            .is_some_and(|node| !node.starts_with(TREE_DOC_PREFIX))
            && self.selected_tree_path_exists();
        if let Some(id) = target {
            if self.selected_document_id.as_deref() != Some(id.as_str()) {
                self.load_document_state(&id);
            }
            if !keep_group_row {
                self.sync_tree_selection();
            }
        } else if !self.buffer_dirty {
            self.exit_insert_mode();
            self.selected_document_id = None;
            self.buffer = EditorBuffer::from_string(String::new());
            self.buffer_dirty = false;
            if !keep_group_row {
                self.tree_state.select(Vec::new());
            }
        }
    }

    /// Commits the typed query and rebuilds the tree with it applied.
    fn commit_requirement_search(&mut self) {
        self.pending_search = None;
        self.applied_search_query = self.search_query.clone();
        self.rebuild_tree();
    }

    /// Defers a rebuild until typing pauses, or clears it immediately.
    fn schedule_requirement_search(&mut self) {
        if self.search_query.trim().is_empty() {
            self.commit_requirement_search();
        } else {
            self.pending_search = Some(Instant::now());
        }
    }

    /// Applies a debounced query from the main loop.
    pub(crate) fn poll_search(&mut self) {
        if self
            .pending_search
            .is_some_and(|at| at.elapsed() >= REQUIREMENT_SEARCH_DEBOUNCE)
        {
            self.commit_requirement_search();
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

    /// Opens the optional priority picker for the selected document.
    pub fn open_priority_editor(&mut self) {
        let Some(document_id) = self.selected_document_id.clone() else {
            return;
        };
        let expected = self.current_document_meta().and_then(|meta| meta.priority);
        let selection = expected.map(|priority| priority as usize + 1).unwrap_or(0);
        self.overlay = RequirementsOverlay::PriorityEdit {
            document_id,
            selection,
            expected,
        };
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
        let visible = visible_document_ids(&self.project_filtered_index(), &self.iteration_filter);
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
            self.editor_mode = RequirementsEditorMode::Preview;
            self.preview_scroll = 0;
            self.cursor_row = 0;
            self.cursor_col = 0;
            self.desired_col = 0;
            self.scroll_row = 0;
        }
        self.rebuild_tree();
        self.note_store_write();

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
        let visible = visible_document_ids(&self.project_filtered_index(), &filter);
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

    /// Confirms the active filter or metadata overlay.
    ///
    /// # Errors
    ///
    /// Returns an error when saving metadata fails or its expected value changed.
    pub fn confirm_overlay(&mut self) -> Result<(), String> {
        if matches!(self.overlay, RequirementsOverlay::TagPicker { .. }) {
            return self.confirm_tag_picker().map(drop);
        }
        if let Some(result) = self.confirm_organization_overlay() {
            return result.map(drop);
        }
        match self.overlay.clone() {
            RequirementsOverlay::ActionMenu { .. }
            | RequirementsOverlay::OrganizationForm(_)
            | RequirementsOverlay::FolderPicker { .. }
            | RequirementsOverlay::TagPicker { .. }
            | RequirementsOverlay::RemoveCollection { .. } => Ok(()),
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
            RequirementsOverlay::PriorityEdit {
                document_id,
                selection,
                expected,
            } => self.save_current_priority(&document_id, selection, expected),
            RequirementsOverlay::DeleteConfirmation { .. } => Ok(()),
        }
    }

    /// Closes the overlay without applying it.
    ///
    /// While the tag picker's new-tag field is open, Esc only dismisses that field.
    pub fn cancel_overlay(&mut self) {
        if let RequirementsOverlay::TagPicker { new_tag, .. } = &mut self.overlay
            && new_tag.take().is_some()
        {
            return;
        }
        self.overlay = RequirementsOverlay::None;
    }

    /// Moves the highlight of a list overlay, or switches query form fields.
    pub fn overlay_move_selection(&mut self, delta: isize) {
        if self.organization_form_switch_field() {
            return;
        }
        // The new-tag field owns j/k as text, so list navigation pauses while it is open.
        if let RequirementsOverlay::TagPicker {
            new_tag: Some(_), ..
        } = &self.overlay
        {
            return;
        }
        let len = match &self.overlay {
            RequirementsOverlay::FilterPicker { .. } => self.filter_picker_options().len(),
            RequirementsOverlay::PriorityEdit { .. } => 5,
            _ => self.organization_list_len().unwrap_or(0),
        };
        if let RequirementsOverlay::FilterPicker { selection }
        | RequirementsOverlay::PriorityEdit { selection, .. }
        | RequirementsOverlay::ActionMenu { selection, .. }
        | RequirementsOverlay::FolderPicker { selection, .. }
        | RequirementsOverlay::TagPicker { selection, .. } = &mut self.overlay
            && len > 0
        {
            *selection = (*selection as isize + delta).rem_euclid(len as isize) as usize;
        }
    }

    /// Inserts a character into the focused overlay text field.
    pub fn overlay_insert_char(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        if let RequirementsOverlay::TagPicker {
            new_tag: Some(buffer),
            ..
        } = &mut self.overlay
        {
            buffer.push(ch);
        } else if let RequirementsOverlay::IterationEdit { buffer } = &mut self.overlay {
            buffer.push(ch);
        } else if let Some(input) = self.organization_form_input() {
            input.insert(ch);
        }
    }

    /// Deletes the character before the cursor in the focused overlay text field.
    pub fn overlay_backspace(&mut self) {
        if let RequirementsOverlay::TagPicker {
            new_tag: Some(buffer),
            ..
        } = &mut self.overlay
        {
            buffer.pop();
        } else if let RequirementsOverlay::IterationEdit { buffer } = &mut self.overlay {
            buffer.pop();
        } else if let Some(input) = self.organization_form_input() {
            input.backspace();
        }
    }

    /// Returns `true` when a Requirements overlay is capturing keys.
    pub fn overlay_active(&self) -> bool {
        !matches!(self.overlay, RequirementsOverlay::None)
    }

    /// Returns `true` when the open overlay edits text rather than choosing from a list.
    pub fn overlay_accepts_text(&self) -> bool {
        match &self.overlay {
            RequirementsOverlay::IterationEdit { .. }
            | RequirementsOverlay::OrganizationForm(_) => true,
            RequirementsOverlay::TagPicker { new_tag, .. } => new_tag.is_some(),
            _ => false,
        }
    }

    /// Returns `true` when the open overlay is a navigable choice list.
    pub fn overlay_is_list(&self) -> bool {
        matches!(
            self.overlay,
            RequirementsOverlay::FilterPicker { .. }
                | RequirementsOverlay::PriorityEdit { .. }
                | RequirementsOverlay::ActionMenu { .. }
                | RequirementsOverlay::FolderPicker { .. }
                | RequirementsOverlay::TagPicker { .. }
        )
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
        self.note_store_write();
        Ok(())
    }

    fn save_current_priority(
        &mut self,
        doc_id: &str,
        selection: usize,
        expected: Option<RequirementPriority>,
    ) -> Result<(), String> {
        let priority = match selection {
            1 => Some(RequirementPriority::P0),
            2 => Some(RequirementPriority::P1),
            3 => Some(RequirementPriority::P2),
            4 => Some(RequirementPriority::P3),
            _ => None,
        };
        let updated = teshi_engine::update_requirement_document_priority(
            &self.requirements_root,
            doc_id,
            priority,
            Some(expected),
        )
        .map_err(|error| error.to_string())?;
        if let Some(artifacts) = self.artifacts.as_mut() {
            if let Some(index_meta) = artifacts
                .index
                .documents
                .iter_mut()
                .find(|meta| meta.id == doc_id)
            {
                index_meta.priority = updated.priority;
            }
            if let Some(doc) = artifacts
                .documents
                .iter_mut()
                .find(|doc| doc.meta.id == doc_id)
            {
                doc.meta.priority = updated.priority;
            }
        }
        self.overlay = RequirementsOverlay::None;
        self.rebuild_tree();
        self.note_store_write();
        Ok(())
    }

    fn view_preferences_key(&self, store_id: &str) -> String {
        if self.project_root.as_os_str().is_empty() {
            return store_id.to_string();
        }
        let identity = teshi_engine::read_requirement_project(&self.project_root)
            .ok()
            .flatten()
            .map(|project| project.id)
            .unwrap_or_else(|| self.project_root.to_string_lossy().into_owned());
        format!("{store_id}@{identity}")
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
        let Some(prefs) = settings
            .requirements_views
            .get(&self.view_preferences_key(store_id.as_str()))
        else {
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
            self.view_preferences_key(store_id.as_str()),
            teshi_engine::RequirementsViewPrefs {
                filter: self.iteration_filter.clone(),
                group: self.group_mode,
            },
        );
        let _ = teshi_engine::save_settings(&settings);
    }

    /// Selects a document by stable id, synchronizing its tree row and editor buffer.
    pub fn select_document_by_id(&mut self, doc_id: &str) {
        if self.buffer_dirty {
            if self.selected_document_id.as_deref() != Some(doc_id) {
                self.guard_unsaved(RequirementsPendingAction::Document(doc_id.to_string()));
            }
            return;
        }
        self.load_document_state(doc_id);
        // Following an existing test-point link is an explicit request to reveal
        // its source, including legacy documents that have no project membership.
        if self
            .current_document_meta()
            .is_some_and(|meta| !meta.matches_project_filter(&self.project_filter))
        {
            self.project_filter = teshi_core::authoring::RequirementProjectFilter::All;
            self.rebuild_tree();
        } else {
            self.sync_tree_selection();
        }
    }

    fn load_document_state(&mut self, doc_id: &str) {
        self.vertical_motion = None;
        // Undo snapshots belong to one document. Reopening or switching a
        // document must never apply another document's text to this buffer.
        self.undo_stack.clear();
        self.redo_stack.clear();
        // Every opened document starts in the rendered preview.
        self.insert_session_undo = false;
        self.editor_mode = RequirementsEditorMode::Preview;
        self.preview_scroll = 0;
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

    fn selected_tree_path_exists(&self) -> bool {
        let path = self.tree_state.selected();
        let mut items = self.tree_items.as_slice();
        !path.is_empty()
            && path.iter().all(|id| {
                if let Some(item) = items.iter().find(|item| item.identifier() == id) {
                    items = item.children();
                    true
                } else {
                    false
                }
            })
    }

    fn sync_tree_selection(&mut self) {
        let Some(id) = self.selected_document_id.as_ref() else {
            self.tree_state.select(Vec::new());
            return;
        };
        let node = format!("{TREE_DOC_PREFIX}{id}");
        // Documents can be listed under several folders and queries; keep the occurrence
        // the user is on instead of jumping to the first one.
        if self.tree_state.selected().last() == Some(&node) && self.selected_tree_path_exists() {
            self.select_document_tree_path(self.tree_state.selected().to_vec());
            return;
        }
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

    /// Creates a timestamp-named draft while retaining independent stable IDs.
    pub fn create_default_document(&mut self) -> Result<String> {
        let n = self.next_document_number()?;
        self.create_document(
            &teshi_engine::requirement_timestamp_filename(),
            &format!("Document {n}"),
        )
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
        let project = if self.project_root.as_os_str().is_empty() {
            None
        } else {
            Some(teshi_engine::ensure_requirement_project(
                &self.project_root,
            )?)
        };
        if let Some(project) = &project {
            // New documents always belong to the current project and must stay visible.
            self.project_filter =
                teshi_core::authoring::RequirementProjectFilter::Project(project.id.clone());
        }
        if self.artifacts.is_none() {
            self.artifacts = Some(AuthoringArtifacts {
                index: Default::default(),
                documents: Vec::new(),
                test_points: Default::default(),
                diagnostics: Vec::new(),
            });
            self.discovered = true;
        }
        self.draft_folder = self.selected_folder().map(|folder| (id.clone(), folder));
        let artifacts = self.artifacts.as_mut().expect("artifacts initialized");
        let body = format!("# {}\n\n", title.trim());
        let revision = compute_document_revision(&body);
        let mut meta = RequirementDocumentMeta::new(id.clone(), relative_path, title, revision);
        if let Some(project) = project {
            meta.projects.insert(project.id, Default::default());
        }
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
        // A fresh draft starts below its H1 so the first keystroke writes content
        // instead of rewriting the title.
        self.cursor_row = self.buffer.line_count().saturating_sub(1);
        self.cursor_col = self.buffer.line_len_chars(self.cursor_row);
        self.desired_col = self.cursor_col;
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
        if self.draft_document_id.as_deref() == Some(doc_id.as_str()) {
            let project = teshi_engine::ensure_requirement_project(project_root)?;
            if let Some(meta) = artifacts
                .index
                .documents
                .iter_mut()
                .find(|doc| doc.id == doc_id)
            {
                meta.projects.entry(project.id).or_default();
            }
        }
        let meta = artifacts
            .index
            .documents
            .iter()
            .find(|d| d.id == doc_id)
            .context("selected document not in index")?;
        let relative_path = meta.path.clone();
        let save = if self.draft_document_id.as_deref() == Some(doc_id.as_str()) {
            teshi_engine::save_requirement_draft
        } else {
            save_requirement_markdown
        };
        save(
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
        let filing_result = self.file_saved_draft(&doc_id);
        // The saved title must be visible even if linked test-point persistence fails.
        self.rebuild_tree();
        test_points_result?;
        self.buffer_dirty = false;
        self.draft_document_id = None;
        self.note_store_write();
        filing_result
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

    /// Moves focus between the editor and the linked test points stacked
    /// below it; the tree has no vertical neighbor and keeps focus.
    pub fn focus_vertical_pane(&mut self, down: bool) {
        self.exit_insert_mode();
        self.focus = match (self.focus, down) {
            (RequirementsFocus::Editor, true) => RequirementsFocus::LinkedTestPoints,
            (RequirementsFocus::LinkedTestPoints, false) => RequirementsFocus::Editor,
            (focus, _) => focus,
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
            self.selected_linked_index
                .saturating_sub(delta.unsigned_abs())
        } else {
            self.selected_linked_index
                .saturating_add(delta as usize)
                .min(count - 1)
        };
        self.selected_linked_index = next;
        if let Some(tp) = self.filtered_linked_test_points().get(next) {
            self.highlight_test_point_id = Some(tp.id.clone());
        }
    }

    /// Jumps to the first or last document row, skipping folder and query headers.
    ///
    /// Home/End in the tree are document jumps; group rows only matter while
    /// walking the tree one row at a time.
    pub fn select_document_boundary(&mut self, end: bool) {
        let mut rows = Vec::new();
        collect_visible_requirement_rows(
            &self.tree_items,
            &mut Vec::new(),
            self.tree_state.opened(),
            &mut rows,
        );
        let target = if end {
            rows.iter().rev().find(|path| {
                path.last()
                    .is_some_and(|node| node.starts_with(TREE_DOC_PREFIX))
            })
        } else {
            rows.iter().find(|path| {
                path.last()
                    .is_some_and(|node| node.starts_with(TREE_DOC_PREFIX))
            })
        };
        let Some(path) = target.cloned() else {
            return;
        };
        if let Some(node) = path.last()
            && let Some(id) = node.strip_prefix(TREE_DOC_PREFIX)
            && self.selected_document_id.as_deref() != Some(id)
        {
            self.select_tree_node(node);
            if self.selected_document_id.as_deref() == Some(id) {
                self.tree_state.select(path);
                self.tree_state.scroll_selected_into_view();
            }
            return;
        }
        self.tree_state.select(path);
        self.tree_state.scroll_selected_into_view();
    }

    pub fn move_tree_selection(&mut self, delta: isize) {
        let mut rows = Vec::new();
        collect_visible_requirement_rows(
            &self.tree_items,
            &mut Vec::new(),
            self.tree_state.opened(),
            &mut rows,
        );
        if rows.is_empty() {
            return;
        }
        let current = rows
            .iter()
            .position(|path| path == self.tree_state.selected());
        let next = match current {
            None => 0,
            Some(pos) if delta < 0 => pos.saturating_sub(delta.unsigned_abs()),
            Some(pos) => pos.saturating_add(delta as usize).min(rows.len() - 1),
        };
        let path = rows[next].clone();
        if let Some(node) = path.last() {
            self.select_tree_node(node);
            if let Some(id) = node.strip_prefix(TREE_DOC_PREFIX)
                && self.selected_document_id.as_deref() != Some(id)
            {
                return;
            }
        }
        self.tree_state.select(path);
        self.tree_state.scroll_selected_into_view();
    }

    /// Collapses the selected row, then walks to its parent on repeat.
    ///
    /// Mirrors `h` in the MindMap tree: an open folder, query, or iteration row
    /// closes; anything else selects the parent row, so repeated presses climb
    /// back towards the root.
    pub fn collapse_tree_selection(&mut self) {
        let selected = self.tree_state.selected().to_vec();
        if !selected.is_empty() && self.tree_state.close(&selected) {
            return;
        }
        if selected.len() <= 1 {
            return;
        }
        self.tree_state
            .select(selected[..selected.len() - 1].to_vec());
        self.tree_state.scroll_selected_into_view();
    }

    /// Expands the selected row, then steps into its first child on repeat.
    ///
    /// Mirrors `l` in the MindMap tree: a closed folder, query, or iteration row
    /// opens; an open one selects its first child, so repeated presses descend
    /// the tree.
    pub fn expand_tree_selection(&mut self) {
        let selected = self.tree_state.selected().to_vec();
        if selected
            .last()
            .is_some_and(|node| node.starts_with(TREE_DOC_PREFIX))
        {
            return;
        }
        if !self.tree_state.opened().contains(&selected) {
            self.tree_state.open(selected);
            return;
        }
        if tree_children(&self.tree_items, &selected).is_some_and(|children| !children.is_empty()) {
            self.move_tree_selection(1);
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
                (current + 1).min(self.editor_display_rows.len() - 1)
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

/// Case-insensitive substring match over the requirement fields a user can see:
/// title, relative path, and stable document id. `query` is already lowercased.
fn requirement_matches_search(doc: &RequirementDocumentMeta, query: &str) -> bool {
    doc.title.to_lowercase().contains(query)
        || doc.path.to_lowercase().contains(query)
        || doc.id.to_lowercase().contains(query)
}

/// Flat document rows for an active search, so every match stays visible even
/// when its folder or iteration group is collapsed.
fn build_search_results(
    index: &teshi_core::authoring::RequirementDocumentIndex,
    filter: &RequirementIterationFilter,
) -> (
    Vec<TreeItem<'static, String>>,
    HashMap<String, Line<'static>>,
) {
    let styles = &index.tag_styles;
    let mut labels = HashMap::new();
    let items = index
        .documents
        .iter()
        .filter(|doc| doc.matches_iteration_filter(filter))
        .map(|doc| {
            let label = requirement_document_tree_line(doc, styles);
            let id = format!("{TREE_DOC_PREFIX}{}", doc.id);
            labels.insert(id.clone(), label.clone());
            TreeItem::new(id, label, Vec::new()).unwrap()
        })
        .collect();
    (items, labels)
}

/// Finds a complete hierarchy path for a stable tree node identity.
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

/// Children of the node at `path`, or `None` when the path no longer exists.
fn tree_children<'a>(
    items: &'a [TreeItem<'static, String>],
    path: &[String],
) -> Option<&'a [TreeItem<'static, String>]> {
    let (first, rest) = path.split_first()?;
    let item = items.iter().find(|item| item.identifier() == first)?;
    if rest.is_empty() {
        Some(item.children())
    } else {
        tree_children(item.children(), rest)
    }
}

fn collect_visible_requirement_rows(
    items: &[TreeItem<'static, String>],
    parent_path: &mut Vec<String>,
    opened: &std::collections::HashSet<Vec<String>>,
    rows: &mut Vec<Vec<String>>,
) {
    for item in items {
        parent_path.push(item.identifier().clone());
        rows.push(parent_path.clone());
        if opened.contains(parent_path) {
            collect_visible_requirement_rows(item.children(), parent_path, opened, rows);
        }
        parent_path.pop();
    }
}

fn build_path_tree(
    documents: &[RequirementDocumentMeta],
    dir_prefix: &str,
    styles: &BTreeMap<String, RequirementTagStyle>,
    labels: &mut HashMap<String, Line<'static>>,
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
        styles: &BTreeMap<String, RequirementTagStyle>,
        labels: &mut HashMap<String, Line<'static>>,
    ) -> Vec<TreeItem<'static, String>> {
        let mut items = Vec::new();
        for (name, child) in node.children.iter() {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let id = format!("{TREE_DIR_PREFIX}{dir_prefix}{child_path}");
            let child_items = dir_to_items(child_path, child, dir_prefix, styles, labels);
            labels.insert(id.clone(), Line::from(name.clone()));
            items.push(TreeItem::new(id, name.clone(), child_items).unwrap());
        }
        for doc in &node.documents {
            let label = requirement_document_tree_line(doc, styles);
            let id = format!("{TREE_DOC_PREFIX}{}", doc.id);
            labels.insert(id.clone(), label.clone());
            items.push(TreeItem::new(id, label, Vec::new()).unwrap());
        }
        items
    }

    dir_to_items(String::new(), &root, dir_prefix, styles, labels)
}

/// Builds folder/document tree items from indexed paths after applying filter/group.
///
/// Also returns each node's styled label by identifier, which the renderer
/// rewraps to the pane width.
pub fn build_requirement_tree(
    index: &teshi_core::authoring::RequirementDocumentIndex,
    filter: &RequirementIterationFilter,
    group: RequirementGroupMode,
) -> (
    Vec<TreeItem<'static, String>>,
    HashMap<String, Line<'static>>,
) {
    let docs: Vec<RequirementDocumentMeta> = index
        .documents
        .iter()
        .filter(|doc| doc.matches_iteration_filter(filter))
        .cloned()
        .collect();
    let styles = &index.tag_styles;
    let mut labels = HashMap::new();
    let items = match group {
        RequirementGroupMode::Path => build_path_tree(&docs, "", styles, &mut labels),
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
                let child = build_path_tree(&group_docs, &format!("{name}/"), styles, &mut labels);
                let id = format!("{TREE_ITER_PREFIX}{name}");
                labels.insert(id.clone(), Line::from(name.clone()));
                items.push(TreeItem::new(id, name, child).unwrap());
            }
            if !unassigned.is_empty() {
                let child = build_path_tree(&unassigned, "unassigned/", styles, &mut labels);
                let id = format!("{TREE_ITER_PREFIX}{UNASSIGNED_ITERATION_LABEL}");
                labels.insert(id.clone(), Line::from(UNASSIGNED_ITERATION_LABEL));
                items.push(
                    TreeItem::new(id, UNASSIGNED_ITERATION_LABEL.to_string(), child).unwrap(),
                );
            }
            items
        }
    };
    (items, labels)
}

/// Styled tree row for a requirement document.
///
/// Layout: optional priority, prefix-tag chips, title, then plain `#tag`
/// suffixes and a `+N` count of tags that did not fit. Prefix tags come first
/// so the row limit on narrow panes truncates the title, never the badges.
pub fn requirement_document_tree_line(
    doc: &RequirementDocumentMeta,
    styles: &BTreeMap<String, RequirementTagStyle>,
) -> Line<'static> {
    // Two badges fit on each side of a title; the rest stay one keystroke away in Tags….
    const VISIBLE_PREFIX_TAGS: usize = 2;
    const VISIBLE_SUFFIX_TAGS: usize = 2;
    let style_of = |tag: &str| styles.get(tag).copied().unwrap_or_default();
    let title = if doc.title.trim().is_empty() {
        doc.path.as_str()
    } else {
        doc.title.as_str()
    };
    let mut spans = Vec::new();
    if let Some(priority) = doc.priority {
        spans.push(Span::raw(format!("[{priority}] ")));
    }
    // `tags` is a BTreeSet, so both groups are already sorted by name.
    let (prefix, suffix): (Vec<&String>, Vec<&String>) =
        doc.tags.iter().partition(|tag| style_of(tag).prefix);
    for tag in prefix.iter().take(VISIBLE_PREFIX_TAGS) {
        let bg = style_of(tag).color.map_or(theme::ACCENT, theme::tag_color);
        spans.push(theme::chip(format!("#{tag}"), bg));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(title.to_string()));
    let shown_suffix: Vec<&String> = suffix.into_iter().take(VISIBLE_SUFFIX_TAGS).collect();
    let hidden = doc.tags.len() - prefix.len().min(VISIBLE_PREFIX_TAGS) - shown_suffix.len();
    if !shown_suffix.is_empty() || hidden > 0 {
        spans.push(Span::raw("  "));
    }
    for (position, tag) in shown_suffix.iter().enumerate() {
        if position > 0 {
            spans.push(Span::raw(" "));
        }
        let style = style_of(tag).color.map_or_else(Style::default, |color| {
            Style::default().fg(theme::tag_color(color))
        });
        spans.push(Span::styled(format!("#{tag}"), style));
    }
    if hidden > 0 {
        let gap = if shown_suffix.is_empty() { "" } else { " " };
        spans.push(Span::raw(format!("{gap}+{hidden}")));
    }
    Line::from(spans)
}

/// Plain text of [`requirement_document_tree_line`], for assertions.
#[cfg(test)]
pub(crate) fn requirement_document_tree_label(
    doc: &RequirementDocumentMeta,
    styles: &BTreeMap<String, RequirementTagStyle>,
) -> String {
    requirement_document_tree_line(doc, styles).to_string()
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
        Span::raw(" Scope [o] Link [b] "),
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
        Span::styled(" Priority [p] ", Style::default().fg(theme::FOOTER_DESC)),
        Span::raw(" "),
        Span::styled(" Reload [R] ", Style::default().fg(theme::FOOTER_DESC)),
        Span::raw(" "),
        Span::styled(" New doc [n] ", Style::default().fg(theme::FOOTER_DESC)),
        Span::raw(" "),
        Span::styled(" Save [s] ", Style::default().fg(theme::FOOTER_DESC)),
    ])
}

/// Requirements tree text search, mirroring the Explore Feature search state
/// machine: `/` starts typing, Enter keeps the filter, Esc clears it.
impl App {
    pub(crate) fn handle_requirement_search_key(&mut self, event: KeyEvent) -> bool {
        if self.active_tab != MainTab::Requirements
            || self.authoring_ui.focus != RequirementsFocus::Tree
            || self.authoring_ui.overlay_active()
            || self.pending_delete.is_some()
            || self.quit_pending_confirm
            || self.ai_interrupt_confirm
            || self.command_line.is_some()
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
        if self.authoring_ui.search_active {
            // Control chords stay global (Ctrl+Q quits, Ctrl+S saves) rather
            // than becoming query text.
            if event
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                return false;
            }
            match event.code {
                KeyCode::Esc => {
                    self.authoring_ui.search_query.clear();
                    self.authoring_ui.search_active = false;
                }
                KeyCode::Enter => self.authoring_ui.search_active = false,
                KeyCode::Backspace => {
                    self.authoring_ui.search_query.pop();
                }
                KeyCode::Char(ch) => self.authoring_ui.search_query.push(ch),
                KeyCode::Up | KeyCode::Down => return false,
                _ => return true,
            }
            if matches!(event.code, KeyCode::Esc | KeyCode::Enter) {
                self.authoring_ui.commit_requirement_search();
            } else {
                self.authoring_ui.schedule_requirement_search();
            }
            return true;
        }
        match event.code {
            KeyCode::Char('/') if event.modifiers.is_empty() => {
                self.authoring_ui.search_active = true;
                true
            }
            KeyCode::Esc if !self.authoring_ui.search_query.is_empty() => {
                self.authoring_ui.search_query.clear();
                self.authoring_ui.commit_requirement_search();
                true
            }
            _ => false,
        }
    }

    pub(crate) fn paste_requirement_search(&mut self, text: &str) {
        for ch in text.chars() {
            if !self
                .handle_requirement_search_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            {
                break;
            }
        }
    }

    pub(crate) fn poll_requirement_search(&mut self) {
        self.authoring_ui.poll_search();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    // Aliased because the page color budget counts raw color paths by substring.
    use teshi_core::authoring::RequirementTagColor as TagHue;
    use teshi_core::authoring::{AuthoringArtifacts, RequirementDocumentIndex};

    fn preview_text(state: &AuthoringUiState) -> Vec<String> {
        state
            .preview()
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn test_preview_image_at_finds_the_clicked_image_box() {
        let project = tempdir().unwrap();
        fs::write(project.path().join("shot.png"), b"png").unwrap();
        let mut state = AuthoringUiState::empty();
        state.project_root = project.path().to_path_buf();
        state.editor_mode = RequirementsEditorMode::Preview;
        state.buffer = EditorBuffer::from_string("![ui](shot.png) ![x](gone.png)".into());
        state.refresh_preview(80, 3);
        let (&first, path) = state
            .preview()
            .images
            .iter()
            .find_map(|(row, image)| match image {
                EditorImageRow::Pixels { path, row: 0, .. } => Some((row, path.clone())),
                _ => None,
            })
            .expect("the existing image has a pixel box");
        let missing = first + 3;

        assert_eq!(state.preview_image_at(0, first + 2), Some(path.clone()));
        assert_eq!(
            state.preview_image_at(64, first),
            None,
            "columns right of the box are text, not image"
        );
        assert_eq!(state.preview_image_at(0, missing), None, "nothing to open");
        assert_eq!(state.preview_image_at(0, 0), None, "the caption row");

        state.preview_scroll = 1;
        assert_eq!(state.preview_image_at(0, first + 1), Some(path.clone()));
        state.editor_mode = RequirementsEditorMode::Browse;
        assert_eq!(
            state.preview_image_at(0, first + 1),
            None,
            "the source view has no images"
        );

        state.editor_mode = RequirementsEditorMode::Preview;
        state.preview_scroll = 0;
        state.refresh_preview(80, 1);
        let label = state
            .preview()
            .images
            .iter()
            .find_map(|(row, image)| matches!(image, EditorImageRow::Label { .. }).then_some(*row))
            .expect("without pixels the image is a one-row label");
        assert_eq!(state.preview_image_at(70, label), Some(path));
    }

    #[test]
    fn test_preview_places_image_rows_below_their_line() {
        let project = tempdir().unwrap();
        fs::write(project.path().join("shot.png"), b"png").unwrap();
        let mut state = AuthoringUiState::empty();
        state.project_root = project.path().to_path_buf();
        state.buffer =
            EditorBuffer::from_string("# Title\n![ui](shot.png) ![x](gone.png)\n\nafter".into());
        state.refresh_preview(80, 3);
        let text = preview_text(&state);
        assert_eq!(text[0], "Title", "headings render without their marker");
        let image_line = text
            .iter()
            .position(|line| line.contains("🖼 ui"))
            .expect("image line keeps its caption");
        let images = &state.preview().images;
        for row in 0..3 {
            assert!(
                matches!(
                    images.get(&(image_line + 1 + row as usize)),
                    Some(EditorImageRow::Pixels { row: r, .. }) if *r == row
                ),
                "pixel row {row} follows the image line: {images:?}"
            );
        }
        assert_eq!(
            images.get(&(image_line + 4)),
            Some(&EditorImageRow::Missing {
                target: "gone.png".into()
            })
        );
        assert_eq!(images.len(), 4, "three pixel rows and one missing note");
        assert_eq!(text.last().map(String::as_str), Some("after"));

        state.wrap_editor_rows(80);
        assert_eq!(
            state.editor_display_rows.len(),
            4,
            "the source view shows only source lines"
        );
    }

    #[test]
    fn test_preview_keeps_image_syntax_inside_code_fences() {
        let mut state = AuthoringUiState::empty();
        state.buffer = EditorBuffer::from_string("```\n![ui](shot.png)\n```".into());
        state.refresh_preview(80, 3);
        assert!(state.preview().images.is_empty());
        assert!(
            preview_text(&state)
                .iter()
                .any(|line| line.contains("![ui](shot.png)")),
            "{:?}",
            preview_text(&state)
        );
    }

    #[test]
    fn test_preview_follows_unsaved_buffer_edits() {
        let mut state = AuthoringUiState::empty();
        state.buffer = EditorBuffer::from_string("# Title\nfirst".into());
        state.refresh_preview(40, 1);
        assert!(preview_text(&state).contains(&"first".to_string()));
        state.buffer = EditorBuffer::from_string("# Title\n**second**".into());
        state.buffer_dirty = true;
        state.refresh_preview(40, 1);
        let text = preview_text(&state);
        assert!(text.contains(&"second".to_string()), "{text:?}");
        assert!(!text.iter().any(|line| line.contains("**")), "{text:?}");
    }

    #[test]
    fn test_view_transitions_step_through_preview_normal_and_insert() {
        let mut state = AuthoringUiState::empty();
        state.selected_document_id = Some("doc-1".into());
        assert_eq!(state.editor_mode, RequirementsEditorMode::Preview);
        state.exit_insert_mode();
        assert_eq!(
            state.editor_mode,
            RequirementsEditorMode::Preview,
            "leaving insert never opens the source view"
        );
        state.enter_edit_view();
        assert_eq!(state.editor_mode, RequirementsEditorMode::Browse);
        state.enter_insert_mode();
        assert_eq!(state.editor_mode, RequirementsEditorMode::Insert);
        state.insert_char('x');
        state.exit_insert_mode();
        assert_eq!(state.editor_mode, RequirementsEditorMode::Browse);
        state.enter_preview();
        assert_eq!(state.editor_mode, RequirementsEditorMode::Preview);
        assert!(state.buffer_dirty, "switching views keeps unsaved edits");
        assert_eq!(state.buffer.as_string(), "x");
        state.enter_insert_mode();
        state.exit_insert_mode();
        state.undo_edit();
        assert_eq!(
            state.editor_mode,
            RequirementsEditorMode::Browse,
            "undo stays in the source view"
        );
    }

    #[test]
    fn test_edit_view_needs_an_open_document() {
        let mut state = AuthoringUiState::empty();
        state.enter_edit_view();
        assert_eq!(state.editor_mode, RequirementsEditorMode::Preview);
    }

    #[test]
    fn external_document_write_reloads_the_store_without_dropping_edits() {
        let store = tempdir().unwrap();
        let project = tempdir().unwrap();
        initialize_requirement_store(store.path()).unwrap();
        teshi_engine::ensure_requirement_project(project.path()).unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        state.buffer = EditorBuffer::from_string("unsaved draft".into());
        state.buffer_dirty = true;

        teshi_engine::create_requirement_document(
            store.path(),
            "# External\n",
            None,
            None,
            &[],
            None,
        )
        .unwrap();
        state.last_store_check = None;
        assert!(state.poll_external_store_changes().unwrap());
        assert!(
            state
                .artifacts
                .as_ref()
                .unwrap()
                .documents
                .iter()
                .any(|document| document.meta.title == "External")
        );
        assert!(state.buffer_dirty);
        assert_eq!(state.buffer.as_string(), "unsaved draft");
    }

    #[test]
    fn external_tag_style_change_reloads_the_tree_styles() {
        let store = tempdir().unwrap();
        let project = tempdir().unwrap();
        initialize_requirement_store(store.path()).unwrap();
        let meta = teshi_engine::create_requirement_document(
            store.path(),
            "# Login\n",
            None,
            None,
            &["ui".to_string()],
            None,
        )
        .unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());

        teshi_engine::update_requirement_tag_style(
            store.path(),
            "ui",
            teshi_engine::RequirementTagStylePatch {
                color: Some(Some(TagHue::Cyan)),
                prefix: Some(true),
            },
        )
        .unwrap();
        state.last_store_check = None;
        assert!(state.poll_external_store_changes().unwrap());
        let index = &state.artifacts.as_ref().unwrap().index;
        let doc = index.documents.iter().find(|d| d.id == meta.id).unwrap();
        assert_eq!(
            requirement_document_tree_label(doc, &index.tag_styles),
            "#ui Login"
        );
    }

    #[test]
    fn external_organization_refresh_preserves_dirty_markdown_and_reports_invalid_json() {
        use teshi_engine::organization::*;
        let store = tempdir().unwrap();
        let project = tempdir().unwrap();
        initialize_requirement_store(store.path()).unwrap();
        let identity = teshi_engine::ensure_requirement_project(project.path()).unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        state.buffer = EditorBuffer::from_string("unsaved draft".into());
        state.buffer_dirty = true;
        let mut org = RequirementOrganization::default();
        let folder = org.ensure_folder("External").unwrap();
        save_requirement_organization(store.path(), &identity.id, &org).unwrap();
        state.poll_organization_changes();
        assert!(
            find_tree_path(
                &state.tree_items,
                &format!("{TREE_DIR_PREFIX}collection:{folder}")
            )
            .is_some()
        );
        assert!(state.buffer_dirty);
        assert_eq!(state.buffer.as_string(), "unsaved draft");
        let path = store
            .path()
            .join("views")
            .join(format!("{}.json", identity.id));
        fs::write(&path, "invalid JSON").unwrap();
        state.poll_organization_changes();
        assert!(state.project_identity_error.is_some());
        fs::write(&path, serde_json::to_vec(&org).unwrap()).unwrap();
        state.poll_organization_changes();
        assert!(state.project_identity_error.is_none());
        assert_eq!(state.buffer.as_string(), "unsaved draft");
    }

    #[test]
    fn organization_tree_keeps_inbox_and_multiple_references_without_parent_aggregation() {
        use teshi_engine::organization::*;
        let project = "project-a";
        let mut one = RequirementDocumentMeta::new(
            "one",
            "one.md",
            "One",
            teshi_core::authoring::DocumentRevision::new("r1"),
        );
        one.projects.insert(
            project.into(),
            teshi_core::authoring::RequirementProjectStatus::Implemented,
        );
        let two = RequirementDocumentMeta::new(
            "two",
            "two.md",
            "Two",
            teshi_core::authoring::DocumentRevision::new("r2"),
        );
        let index = RequirementDocumentIndex {
            documents: vec![one, two],
            ..Default::default()
        };
        let mut org = RequirementOrganization::default();
        org.ensure_folder("AI/Chat").unwrap();
        org.collections[1].documents.insert("one".into());
        org.collections.push(RequirementCollection {
            id: "query".into(),
            name: "Implemented".into(),
            parent: None,
            documents: Default::default(),
            condition: Some(RequirementCondition::Status(
                teshi_core::authoring::RequirementProjectStatus::Implemented,
            )),
        });
        let (tree, labels) =
            organize::build_organization_tree(&index.documents, &org, project, &index.tag_styles);
        assert_eq!(tree.len(), 3, "Inbox, the AI folder, and Saved queries");
        assert_eq!(tree[0].identifier(), organize::TREE_INBOX_NODE);
        assert_eq!(tree[2].identifier(), organize::TREE_QUERIES_NODE);
        assert_eq!(tree[0].children().len(), 1);
        assert_eq!(tree[0].children()[0].identifier(), "req-doc:two");
        assert_eq!(labels["req-dir:inbox"].to_string(), "Inbox (1)");
        assert_eq!(tree[1].children().len(), 1, "AI only lists its subfolder");
        assert_eq!(
            labels[tree[1].identifier()].to_string(),
            "AI (0)",
            "parents do not aggregate"
        );
        assert_eq!(
            tree[1].children()[0].children()[0].identifier(),
            "req-doc:one"
        );
        let query = &tree[2].children()[0];
        assert_eq!(labels[query.identifier()].to_string(), "Implemented (1)");
        assert_eq!(query.children()[0].identifier(), "req-doc:one");
        assert_eq!(labels["req-doc:one"].to_string(), "✓ One");
        assert_eq!(labels["req-doc:two"].to_string(), "Two");
    }

    #[test]
    fn organization_tree_marks_verified_documents_ahead_of_the_title() {
        use teshi_engine::organization::*;
        let project = "project-a";
        let mut doc = RequirementDocumentMeta::new(
            "one",
            "one.md",
            "One",
            teshi_core::authoring::DocumentRevision::new("r1"),
        );
        doc.projects.insert(
            project.into(),
            teshi_core::authoring::RequirementProjectStatus::Verified,
        );
        let index = RequirementDocumentIndex {
            documents: vec![doc],
            ..Default::default()
        };
        let org = RequirementOrganization::default();
        let (_, labels) =
            organize::build_organization_tree(&index.documents, &org, project, &index.tag_styles);
        assert_eq!(labels["req-doc:one"].to_string(), "✓✓ One");
    }

    fn tagged_doc(tags: &[&str]) -> RequirementDocumentMeta {
        let mut doc = RequirementDocumentMeta::new(
            "doc",
            "doc.md",
            "Login",
            teshi_core::authoring::DocumentRevision::new("r"),
        );
        doc.priority = Some(RequirementPriority::P1);
        doc.tags = tags.iter().map(|tag| tag.to_string()).collect();
        doc
    }

    fn tag_styles(
        entries: &[(&str, Option<TagHue>, bool)],
    ) -> BTreeMap<String, RequirementTagStyle> {
        entries
            .iter()
            .map(|(tag, color, prefix)| {
                (
                    tag.to_string(),
                    RequirementTagStyle {
                        color: *color,
                        prefix: *prefix,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn tree_line_without_styles_keeps_plain_suffix_tags() {
        let doc = tagged_doc(&["a", "b", "c"]);
        assert_eq!(
            requirement_document_tree_label(&doc, &BTreeMap::new()),
            "[P1] Login  #a #b +1"
        );
    }

    #[test]
    fn tree_line_puts_prefix_tags_before_the_title_as_colored_chips() {
        let doc = tagged_doc(&["auth", "login", "ui"]);
        let styles = tag_styles(&[
            ("ui", Some(TagHue::Red), true),
            ("auth", None, true),
            ("login", Some(TagHue::Green), false),
        ]);
        let line = requirement_document_tree_line(&doc, &styles);
        assert_eq!(line.to_string(), "[P1] #auth #ui Login  #login");
        let span = |text: &str| {
            line.spans
                .iter()
                .find(|span| span.content == text)
                .unwrap_or_else(|| panic!("missing span {text}: {line:?}"))
                .style
        };
        assert_eq!(span("#ui").bg, Some(theme::ERROR), "colored prefix chip");
        assert_eq!(
            span("#auth").bg,
            Some(theme::ACCENT),
            "uncolored prefix chip"
        );
        assert_eq!(
            span("#login").fg,
            Some(theme::SUCCESS),
            "colored suffix text"
        );
        assert_eq!(span("#login").bg, None, "suffix tags are not chips");
    }

    #[test]
    fn tree_line_caps_prefix_tags_and_counts_the_overflow() {
        let doc = tagged_doc(&["a", "b", "c", "d"]);
        let styles = tag_styles(&[("a", None, true), ("b", None, true), ("c", None, true)]);
        assert_eq!(
            requirement_document_tree_label(&doc, &styles),
            "[P1] #a #b Login  #d +1",
            "the third prefix tag (sorted by name) joins the +N count"
        );
        let all_prefix = tag_styles(&[
            ("a", None, true),
            ("b", None, true),
            ("c", None, true),
            ("d", None, true),
        ]);
        assert_eq!(
            requirement_document_tree_label(&doc, &all_prefix),
            "[P1] #a #b Login  +2"
        );
    }

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
            ..Default::default()
        };
        let (items, labels) = build_requirement_tree(
            &index,
            &RequirementIterationFilter::All,
            RequirementGroupMode::Path,
        );
        assert!(!items.is_empty());
        assert!(
            items
                .iter()
                .all(|item| labels.contains_key(item.identifier())),
            "every root node has a label to wrap"
        );
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
                ..Default::default()
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
            [
                format!("{TREE_DIR_PREFIX}inbox"),
                format!("{TREE_DOC_PREFIX}doc-3")
            ]
            .as_slice()
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
        let mut reloaded = AuthoringUiState::load_from_project(project.path(), store.path());
        assert_eq!(
            reloaded.artifacts.as_ref().unwrap().test_points.test_points[0],
            *test_point
        );

        assert_eq!(reloaded.selected_document_id.as_deref(), Some(id.as_str()));
        reloaded.open_delete_confirmation().unwrap();
        reloaded.delete_current_document(project.path()).unwrap();
        assert!(reloaded.selected_document_id.is_none());
        assert!(teshi_engine::is_requirement_document_id_reserved(store.path(), &id).unwrap());
        assert!(matches!(
            teshi_engine::read_requirement_document(store.path(), &id),
            Err(teshi_engine::RequirementStoreError::NotFound { .. })
        ));
        let after_delete = AuthoringUiState::load_from_project(project.path(), store.path());
        let deleted_point = &after_delete
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points[0];
        assert_eq!(deleted_point.id, original.id);
        assert_eq!(deleted_point.requirement_links[0].document_id, id);
        assert_eq!(
            deleted_point.requirement_links[0].resolution,
            ResolutionState::Stale
        );
        assert_eq!(deleted_point.review_state, ReviewState::NeedsReview);
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
            assert_eq!(meta.path.len(), 26);
            assert!(meta.path.ends_with("Z.md"));
            assert_eq!(&meta.path[8..9], "T");
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
                ..Default::default()
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
    fn tree_search_filters_by_title_path_and_id_then_clears() {
        with_isolated_app_data(|| {
            let mut state = sample_authoring_state();
            state.search_query = "LOGIN".into();
            state.commit_requirement_search();
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-sprint"));
            assert_eq!(state.tree_items.len(), 1, "flat rows expose nested matches");

            state.search_query = "overview.md".into();
            state.commit_requirement_search();
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-open"));

            state.search_query = "doc-open".into();
            state.commit_requirement_search();
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-open"));

            state.search_query = "nothing-matches".into();
            state.commit_requirement_search();
            assert!(state.tree_items.is_empty());
            assert!(state.selected_document_id.is_none());

            state.search_query.clear();
            state.commit_requirement_search();
            assert_eq!(state.project_filtered_index().documents.len(), 2);
        });
    }

    #[test]
    fn tree_search_key_flow_matches_explore() {
        with_isolated_app_data(|| {
            let mut app = App::from_args().unwrap();
            app.active_tab = MainTab::Requirements;
            app.authoring_ui = sample_authoring_state();

            assert!(app.handle_requirement_search_key(KeyEvent::new(
                KeyCode::Char('/'),
                KeyModifiers::NONE
            )));
            assert!(app.authoring_ui.search_active);
            app.paste_requirement_search("log");
            assert_eq!(app.authoring_ui.search_query, "log");
            // Debounce keeps the previous filter until the poll commits the query.
            assert_eq!(app.authoring_ui.project_filtered_index().documents.len(), 2);
            app.authoring_ui.pending_search = Some(Instant::now() - REQUIREMENT_SEARCH_DEBOUNCE);
            app.poll_requirement_search();
            assert_eq!(app.authoring_ui.tree_items.len(), 1);
            assert_eq!(
                app.authoring_ui.selected_document_id.as_deref(),
                Some("doc-sprint")
            );

            assert!(
                app.handle_requirement_search_key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE
                ))
            );
            assert!(!app.authoring_ui.search_active, "Enter leaves input mode");
            assert_eq!(
                app.authoring_ui.tree_items.len(),
                1,
                "Enter keeps the filter"
            );

            assert!(
                app.handle_requirement_search_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            );
            assert!(
                app.authoring_ui.search_query.is_empty(),
                "Esc clears the filter"
            );
            assert_eq!(app.authoring_ui.project_filtered_index().documents.len(), 2);
        });
    }

    #[test]
    fn tree_search_ignores_keys_outside_the_requirements_tree() {
        with_isolated_app_data(|| {
            let mut app = App::from_args().unwrap();
            app.authoring_ui = sample_authoring_state();

            app.active_tab = MainTab::Explore;
            assert!(!app.handle_requirement_search_key(KeyEvent::new(
                KeyCode::Char('/'),
                KeyModifiers::NONE
            )));
            assert!(!app.authoring_ui.search_active);

            app.active_tab = MainTab::Requirements;
            app.authoring_ui.focus = RequirementsFocus::Editor;
            assert!(!app.handle_requirement_search_key(KeyEvent::new(
                KeyCode::Char('/'),
                KeyModifiers::NONE
            )));
            assert!(!app.authoring_ui.search_active);
        });
    }

    #[test]
    fn tree_status_title_shows_query_and_idle_hint() {
        let mut state = AuthoringUiState::empty();
        assert!(state.tree_status_title().contains("[/] Search"));
        state.search_active = true;
        state.search_query = "login".into();
        assert!(state.tree_status_title().starts_with("/login"));
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
            assert!(
                state
                    .tree_state
                    .selected()
                    .last()
                    .unwrap()
                    .starts_with(TREE_ITER_PREFIX)
            );
            state.tree_state.toggle_selected();
            state.move_tree_selection(1);
            assert_eq!(state.selected_document_id.as_deref(), Some("doc-open"));
            assert_eq!(
                state.tree_state.selected().last().map(String::as_str),
                Some("req-doc:doc-open")
            );

            state.move_tree_selection(-1);
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
    fn two_projects_share_storage_but_default_to_their_own_requirement_tree() {
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
        let mut b = AuthoringUiState::load_from_project(project_b.path(), store.path());
        assert!(!a.tree_items.is_empty());
        assert!(b.tree_items.is_empty());
        assert!(b.selected_document_id.is_none());
        b.cycle_project_filter().unwrap();
        assert_eq!(b.selected_document_id.as_deref(), Some("doc-1"));
        b.toggle_current_project_link().unwrap();
        b.cycle_project_filter().unwrap();
        assert!(b.tree_items.is_empty());
        b.cycle_project_filter().unwrap();
        assert_eq!(b.selected_document_id.as_deref(), Some("doc-1"));
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
    fn priority_picker_keeps_its_target_when_external_deletion_changes_selection() {
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let project = tempdir().unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        let first = state.create_document("first.md", "First").unwrap();
        state.save_current_document(project.path()).unwrap();
        let second = state.create_document("second.md", "Second").unwrap();
        state.save_current_document(project.path()).unwrap();
        state.select_document_by_id(&first);
        state.open_priority_editor();
        state.overlay_move_selection(2);
        teshi_engine::delete_requirement_document(store.path(), &first, None).unwrap();
        state.reload_from_disk().unwrap();
        assert_eq!(state.selected_document_id.as_deref(), Some(second.as_str()));
        assert!(state.confirm_overlay().is_err());
        assert!(state.overlay_active());
        assert_eq!(
            teshi_engine::read_requirement_document(store.path(), &second)
                .unwrap()
                .meta
                .priority,
            None
        );
    }

    #[test]
    fn priority_picker_rejects_external_change_even_after_reload() {
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let project = tempdir().unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        let id = state.create_document("priority.md", "Priority").unwrap();
        state.save_current_document(project.path()).unwrap();
        state.open_priority_editor();
        state.overlay_move_selection(2);
        teshi_engine::update_requirement_document_priority(
            store.path(),
            &id,
            Some(RequirementPriority::P3),
            Some(None),
        )
        .unwrap();
        state.reload_from_disk().unwrap();
        assert!(
            state
                .confirm_overlay()
                .unwrap_err()
                .contains("changed priority")
        );
        assert!(state.overlay_active());
        assert_eq!(
            teshi_engine::read_requirement_document(store.path(), &id)
                .unwrap()
                .meta
                .priority,
            Some(RequirementPriority::P3)
        );
        state.cancel_overlay();
        state.open_priority_editor();
        state.overlay_move_selection(1);
        state.confirm_overlay().unwrap();
        assert_eq!(
            teshi_engine::read_requirement_document(store.path(), &id)
                .unwrap()
                .meta
                .priority,
            None
        );
    }

    #[test]
    fn priority_picker_updates_metadata_and_tree_label() {
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let project = tempdir().unwrap();
        let mut state = AuthoringUiState::load_from_project(project.path(), store.path());
        let id = state.create_document("priority.md", "Priority").unwrap();
        state.save_current_document(project.path()).unwrap();
        state.open_priority_editor();
        state.overlay_move_selection(2);
        state.confirm_overlay().unwrap();

        let meta = state
            .artifacts
            .as_ref()
            .unwrap()
            .index
            .documents
            .iter()
            .find(|doc| doc.id == id)
            .unwrap();
        assert_eq!(meta.priority, Some(RequirementPriority::P1));
        // The priority badge is its own span, so prefix-tag chips can sit before the title.
        let tree = format!("{:?}", state.tree_items);
        assert!(
            tree.contains("[P1] ") && tree.contains("Priority"),
            "{tree}"
        );
        assert_eq!(
            teshi_engine::read_requirement_document(store.path(), &id)
                .unwrap()
                .meta
                .priority,
            Some(RequirementPriority::P1)
        );
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
