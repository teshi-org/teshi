use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::bdd_nav::{
    bdd_node_rows, bdd_step_and_header_title_rows, body_char_range, current_step_keyword_index,
    header_title_edit_start_col, is_feature_narrative_row, keyword_char_range,
    line_body_edit_min_col_in_buffer, next_node_row, prev_node_row, replace_step_keyword_line,
};
use crate::editor_buffer::EditorBuffer;
use crate::gherkin::{self, BddProject};
use crate::keymap::Action;
use crate::mindmap;
use crate::step_index::StepIndex;

/// Step keywords in cycle order (re-exported for UI pickers).
pub use crate::bdd_nav::STEP_KEYWORDS_CYCLE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    MindMap,
    Help,
}

/// Three-stage layout state machine for the MindMap tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewStage {
    /// Stage 1: tree occupies full width for navigation.
    TreeOnly,
    /// Stage 2: tree left (~45%) + editor preview right (~55%).
    TreeAndEditor,
    /// Stage 3: editor left (~65%) + reserved panel right (~35%). Tree hidden.
    EditorAndPanel,
}

/// Navigation focus on the current line: Gherkin keyword/token vs editable trailing text (step body or header title).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BddFocusSlot {
    Keyword,
    Body,
}

/// UI state for the step-keyword list shown after Space on the keyword prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepKeywordPicker {
    /// Buffer line index for the step being edited.
    pub buffer_row: usize,
    /// Index into [`STEP_KEYWORDS_CYCLE`] for the highlighted item.
    pub selected: usize,
}

pub struct App {
    // ── Multi-file project ──────────────────────────────────────────
    pub project: BddProject,
    pub step_index: StepIndex,
    /// One `EditorBuffer` per feature file; order matches `project.features`.
    pub buffers: Vec<EditorBuffer>,
    /// Which buffer is shown in the editor panel (`None` when no file is loaded).
    pub active_buffer_idx: Option<usize>,
    pub view_stage: ViewStage,
    pub tree_state: tui_tree_widget::TreeState<String>,

    // ── Active editor state (operates on `buffer`) ──────────────────
    /// The editor buffer currently displayed in the editor panel.
    pub buffer: EditorBuffer,
    pub file_path: Option<PathBuf>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub desired_col: usize,
    pub scroll_row: usize,
    pub focus_slot: BddFocusSlot,

    // ── Global UI ───────────────────────────────────────────────────
    pub should_quit: bool,
    pub active_tab: MainTab,
    pub dirty: bool,
    pub status: String,
    pub step_input_active: bool,
    step_input_row: usize,
    step_input_min_col: usize,
    pub step_keyword_picker: Option<StepKeywordPicker>,
    quit_pending_confirm: bool,
}

impl App {
    /// Builds the editor state from process arguments.
    ///
    /// Accepts a directory path (recursive `.feature` scan) or a single file path.
    pub fn from_args() -> Result<Self> {
        let path = std::env::args()
            .skip(1)
            .find(|arg| !arg.starts_with('-'))
            .map(PathBuf::from);

        match path {
            Some(p) if p.is_dir() => Self::from_directory(&p),
            Some(p) => Self::from_file(&p),
            None => Ok(Self::empty()),
        }
    }

    fn from_directory(dir: &PathBuf) -> Result<Self> {
        let project = gherkin::parse_project(dir);
        let step_index = StepIndex::build(&project);
        let buffers: Vec<EditorBuffer> = project
            .features
            .iter()
            .map(|f| {
                let content = fs::read_to_string(&f.file_path).unwrap_or_default();
                EditorBuffer::from_string(content)
            })
            .collect();
        let tree_state = mindmap::init_tree_state(&project);
        let (buffer, file_path, active_idx) = if buffers.is_empty() {
            (EditorBuffer::from_string(String::new()), None, None)
        } else {
            (
                buffers[0].clone(),
                Some(project.features[0].file_path.clone()),
                Some(0),
            )
        };
        let mut app = Self {
            project,
            step_index,
            buffers,
            active_buffer_idx: active_idx,
            view_stage: ViewStage::TreeOnly,
            tree_state,
            buffer,
            file_path,
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Keyword,
            should_quit: false,
            active_tab: MainTab::MindMap,
            dirty: false,
            status: format!("Opened directory with {} feature file(s)", active_idx.map_or(0, |_| 1).max(if active_idx.is_some() { 1 } else { 0 })),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            quit_pending_confirm: false,
        };
        let n = app.buffers.len();
        app.status = format!("Opened directory with {n} feature file(s)");
        app.sync_cursor_to_first_node();
        Ok(app)
    }

    fn from_file(path: &PathBuf) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let feature = gherkin::parse_feature(&content, path.clone());
        let root_dir = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        let project = BddProject {
            root_dir,
            features: vec![feature],
        };
        let step_index = StepIndex::build(&project);
        let buffers = vec![EditorBuffer::from_string(content.clone())];
        let tree_state = mindmap::init_tree_state(&project);
        let mut app = Self {
            project,
            step_index,
            buffers,
            active_buffer_idx: Some(0),
            view_stage: ViewStage::TreeOnly,
            tree_state,
            buffer: EditorBuffer::from_string(content),
            file_path: Some(path.clone()),
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Keyword,
            should_quit: false,
            active_tab: MainTab::MindMap,
            dirty: false,
            status: "Opened file".to_string(),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            quit_pending_confirm: false,
        };
        app.sync_cursor_to_first_node();
        Ok(app)
    }

    fn empty() -> Self {
        let project = BddProject {
            root_dir: PathBuf::from("."),
            features: Vec::new(),
        };
        let step_index = StepIndex::build(&project);
        let tree_state = mindmap::init_tree_state(&project);
        let mut app = Self {
            project,
            step_index,
            buffers: Vec::new(),
            active_buffer_idx: None,
            view_stage: ViewStage::TreeOnly,
            tree_state,
            buffer: EditorBuffer::from_string(String::new()),
            file_path: None,
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Keyword,
            should_quit: false,
            active_tab: MainTab::MindMap,
            dirty: false,
            status: "New buffer".to_string(),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            quit_pending_confirm: false,
        };
        app.sync_cursor_to_first_node();
        app
    }

    /// Positions the navigation row on the first BDD node, or keeps row `0` when there are none.
    fn sync_cursor_to_first_node(&mut self) {
        let rows = bdd_node_rows(&self.buffer);
        if let Some(&r) = rows.first() {
            self.cursor_row = r;
            self.cursor_col = 0;
            self.desired_col = 0;
        }
        self.focus_slot = BddFocusSlot::Keyword;
    }

    // ── Stage transitions ───────────────────────────────────────────

    /// Switch the active editor buffer to the feature file at `idx`.
    fn switch_to_buffer(&mut self, idx: usize) {
        if idx >= self.buffers.len() {
            return;
        }
        // Persist current editor buffer back
        if let Some(cur) = self.active_buffer_idx {
            if cur < self.buffers.len() {
                self.buffers[cur] = self.buffer.clone();
            }
        }
        self.active_buffer_idx = Some(idx);
        self.buffer = self.buffers[idx].clone();
        self.file_path = self.project.features.get(idx).map(|f| f.file_path.clone());
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.scroll_row = 0;
        self.focus_slot = BddFocusSlot::Keyword;
    }

    /// Scroll the editor to show `line_number` (1-based) centered in view.
    fn editor_goto_line(&mut self, line_1based: usize) {
        let row = line_1based.saturating_sub(1);
        let last = self.buffer.line_count().saturating_sub(1);
        self.cursor_row = row.min(last);
        self.cursor_col = 0;
        self.desired_col = 0;
        self.focus_slot = BddFocusSlot::Keyword;
    }

    /// Returns `(feature_idx, line_number)` for the currently selected tree node.
    fn selected_tree_location(&self) -> Option<(usize, usize)> {
        let id = mindmap::selected_node_id(&self.tree_state)?;
        mindmap::parse_node_line_number(id, &self.project)
    }

    /// Transition from Stage 1 → Stage 2: open editor preview for the selected tree node.
    fn stage_open_editor_preview(&mut self) {
        if let Some((fi, line)) = self.selected_tree_location() {
            if self.active_buffer_idx != Some(fi) {
                self.switch_to_buffer(fi);
            }
            self.editor_goto_line(line);
        }
        self.view_stage = ViewStage::TreeAndEditor;
        self.status = "Preview opened".to_string();
    }

    /// Transition from Stage 2 → Stage 3: activate editor for full editing.
    fn stage_enter_editor(&mut self) {
        if let Some((fi, line)) = self.selected_tree_location() {
            if self.active_buffer_idx != Some(fi) {
                self.switch_to_buffer(fi);
            }
            self.editor_goto_line(line);
        }
        self.view_stage = ViewStage::EditorAndPanel;
        self.status = "Editor active".to_string();
    }

    /// Transition back one stage.
    fn stage_back(&mut self) {
        match self.view_stage {
            ViewStage::EditorAndPanel => {
                self.sync_editor_to_project();
                // Sync tree selection to editor cursor
                if let Some(fi) = self.active_buffer_idx {
                    let line_1based = self.cursor_row + 1;
                    if let Some(node_id) =
                        mindmap::find_closest_node_id(&self.project, fi, line_1based)
                    {
                        let path = mindmap::node_id_to_path(&node_id);
                        self.tree_state.select(path);
                    }
                }
                self.view_stage = ViewStage::TreeAndEditor;
                self.clear_step_input_state();
                self.clear_step_keyword_picker();
                self.status = "Back to tree + preview".to_string();
            }
            ViewStage::TreeAndEditor => {
                self.view_stage = ViewStage::TreeOnly;
                self.status = "Preview closed".to_string();
            }
            ViewStage::TreeOnly => {}
        }
        self.quit_pending_confirm = false;
    }

    /// Re-parse the current editor buffer into the project AST and rebuild the step index.
    fn sync_editor_to_project(&mut self) {
        let Some(idx) = self.active_buffer_idx else {
            return;
        };
        if idx >= self.buffers.len() {
            return;
        }
        // Persist current buffer
        self.buffers[idx] = self.buffer.clone();
        // Re-parse
        let path = self.project.features[idx].file_path.clone();
        let content = self.buffer.as_string();
        self.project.features[idx] = gherkin::parse_feature(&content, path);
        self.step_index = StepIndex::build(&self.project);
    }

    // ── Tree navigation ─────────────────────────────────────────────

    fn tree_move_up(&mut self) {
        self.tree_state.key_up();
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_move_down(&mut self) {
        self.tree_state.key_down();
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_home(&mut self) {
        self.tree_state.select_first();
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_end(&mut self) {
        self.tree_state.select_last();
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    /// In Stage 2, keep editor preview in sync with tree selection.
    fn tree_follow_editor(&mut self) {
        if self.view_stage != ViewStage::TreeAndEditor {
            return;
        }
        if let Some((fi, line)) = self.selected_tree_location() {
            if self.active_buffer_idx != Some(fi) {
                self.switch_to_buffer(fi);
            }
            self.editor_goto_line(line);
        }
    }

    fn tree_toggle_or_expand(&mut self) {
        let id = match mindmap::selected_node_id(&self.tree_state) {
            Some(id) => id.to_string(),
            None => return,
        };
        if mindmap::is_leaf_node(&id) {
            // On leaf in stage 2, advance to stage 3
            if self.view_stage == ViewStage::TreeAndEditor {
                self.stage_enter_editor();
            }
        } else {
            self.tree_state.key_right();
        }
        self.quit_pending_confirm = false;
    }

    fn tree_collapse(&mut self) {
        self.tree_state.key_left();
        self.quit_pending_confirm = false;
    }

    // ── Action handler ──────────────────────────────────────────────

    pub fn handle_action(&mut self, action: Action) -> Result<()> {
        match action {
            // Tree navigation (MindMap stages 1 & 2)
            Action::TreeUp => self.tree_move_up(),
            Action::TreeDown => self.tree_move_down(),
            Action::TreeExpand => self.tree_toggle_or_expand(),
            Action::TreeCollapse => self.tree_collapse(),
            Action::TreeOpen => {
                match self.view_stage {
                    ViewStage::TreeOnly => self.stage_open_editor_preview(),
                    ViewStage::TreeAndEditor => self.stage_enter_editor(),
                    _ => {}
                }
            }
            Action::TreeHome => self.tree_home(),
            Action::TreeEnd => self.tree_end(),
            Action::StageBack => self.stage_back(),

            // Editor navigation (MindMap stage 3 & legacy)
            Action::MoveUp => self.move_up(),
            Action::MoveDown => self.move_down(),
            Action::MoveLeft => self.move_left(),
            Action::MoveRight => self.move_right(),
            Action::MoveHome => self.move_home(),
            Action::MoveEnd => self.move_end(),
            Action::PageUp => self.page_up(),
            Action::PageDown => self.page_down(),
            Action::Insert(ch) => {
                if !self.step_input_active {
                    return Ok(());
                }
                self.buffer
                    .insert_char(self.cursor_row, self.cursor_col, ch);
                self.cursor_col += 1;
                self.desired_col = self.cursor_col;
                self.dirty = true;
                self.quit_pending_confirm = false;
            }
            Action::Enter => {
                if self.step_input_active {
                    self.step_input_active = false;
                    self.focus_slot = BddFocusSlot::Body;
                    self.status = "Edit committed".to_string();
                }
            }
            Action::Backspace => {
                if !self.step_input_active {
                    return Ok(());
                }
                if self.cursor_col <= self.step_input_min_col {
                    return Ok(());
                }
                let (row, col, changed) = self.buffer.backspace(self.cursor_row, self.cursor_col);
                self.cursor_row = row;
                self.cursor_col = col;
                self.desired_col = col;
                if changed {
                    self.dirty = true;
                    self.quit_pending_confirm = false;
                }
            }
            Action::Delete => {
                if !self.step_input_active {
                    return Ok(());
                }
                if self.buffer.delete(self.cursor_row, self.cursor_col) {
                    self.dirty = true;
                    self.quit_pending_confirm = false;
                }
            }
            Action::Save => self.save()?,
            Action::Quit => self.quit(),
            Action::SelectTab(tab) => self.select_tab(tab),
            Action::ActivateStepInput => {
                if !self.is_editor_active() {
                    self.status = "Enter editor mode first".to_string();
                    return Ok(());
                }
                let line = self.buffer.line(self.cursor_row);
                match self.focus_slot {
                    BddFocusSlot::Keyword => {
                        self.clear_step_input_state();
                        if let Some(idx) = current_step_keyword_index(&line) {
                            self.step_keyword_picker = Some(StepKeywordPicker {
                                buffer_row: self.cursor_row,
                                selected: idx,
                            });
                            self.status = "Select step keyword (↑↓ Enter, Esc cancel)".to_string();
                        } else {
                            self.status =
                                "Step keyword list is available on step lines only".to_string();
                        }
                    }
                    BddFocusSlot::Body => {
                        self.clear_step_keyword_picker();
                        let Some(body_start) =
                            line_body_edit_min_col_in_buffer(&self.buffer, self.cursor_row)
                        else {
                            self.status = "No editable text region on this line".to_string();
                            self.quit_pending_confirm = false;
                            return Ok(());
                        };
                        self.step_input_active = true;
                        self.step_input_row = self.cursor_row;
                        self.step_input_min_col = body_start;
                        let end = self.buffer.line_len_chars(self.cursor_row);
                        self.cursor_col = end;
                        self.desired_col = end;
                        self.status = "Editing active".to_string();
                    }
                }
                self.quit_pending_confirm = false;
            }
            Action::StepKeywordPickerUp => self.step_keyword_picker_move(-1),
            Action::StepKeywordPickerDown => self.step_keyword_picker_move(1),
            Action::StepKeywordPickerConfirm => self.confirm_step_keyword_picker(),
            Action::StepKeywordPickerCancel => {
                self.clear_step_keyword_picker();
                self.status = "Step keyword selection canceled".to_string();
                self.quit_pending_confirm = false;
            }
            Action::ClearInputState => {
                if self.step_input_active || self.step_keyword_picker.is_some() {
                    self.clear_step_input_state();
                    self.clear_step_keyword_picker();
                    self.status = "Input state cleared".to_string();
                } else if self.view_stage != ViewStage::TreeOnly {
                    self.stage_back();
                }
                self.quit_pending_confirm = false;
            }
        }
        self.clamp_cursor();
        Ok(())
    }

    fn save(&mut self) -> Result<()> {
        if let Some(path) = &self.file_path {
            fs::write(path, self.buffer.as_string())
                .with_context(|| format!("failed to write {}", path.display()))?;
            self.status = format!("Saved {}", path.display());
            self.dirty = false;
            self.sync_editor_to_project();
        } else {
            self.status = "No file path: run with `cargo run -- path/to/file.feature`".to_string();
        }
        Ok(())
    }

    fn quit(&mut self) {
        if self.dirty && !self.quit_pending_confirm {
            self.status = "Unsaved changes. Press q again to quit.".to_string();
            self.quit_pending_confirm = true;
            return;
        }
        self.should_quit = true;
    }

    fn clamp_cursor(&mut self) {
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

    fn select_tab(&mut self, tab: MainTab) {
        if self.active_tab == tab {
            return;
        }
        if self.step_input_active {
            self.clear_step_input_state();
        }
        self.clear_step_keyword_picker();
        self.quit_pending_confirm = false;
        self.active_tab = tab;
        self.status = match tab {
            MainTab::MindMap => "Switched to MindMap tab",
            MainTab::Help => "Switched to Help tab",
        }
        .to_string();
    }

    fn clear_step_input_state(&mut self) {
        self.step_input_active = false;
    }

    fn clear_step_keyword_picker(&mut self) {
        self.step_keyword_picker = None;
    }

    fn step_keyword_picker_move(&mut self, delta: isize) {
        let Some(ref mut p) = self.step_keyword_picker else {
            return;
        };
        let len = STEP_KEYWORDS_CYCLE.len();
        let i = p.selected as isize + delta;
        p.selected = i.clamp(0, len as isize - 1) as usize;
        self.quit_pending_confirm = false;
    }

    fn confirm_step_keyword_picker(&mut self) {
        let Some(picker) = self.step_keyword_picker else {
            return;
        };
        let line = self.buffer.line(picker.buffer_row);
        let new_kw = STEP_KEYWORDS_CYCLE[picker.selected];
        if let Some(new_line) = replace_step_keyword_line(&line, new_kw) {
            self.buffer.replace_line(picker.buffer_row, &new_line);
            self.cursor_row = picker.buffer_row;
            self.cursor_col = 0;
            self.desired_col = 0;
            self.focus_slot = BddFocusSlot::Keyword;
            self.dirty = true;
            self.status = "Step keyword updated".to_string();
        }
        self.step_keyword_picker = None;
        self.quit_pending_confirm = false;
    }

    /// Returns `true` when the editor panel is active and accepts editing operations.
    fn is_editor_active(&self) -> bool {
        self.active_tab == MainTab::MindMap && self.view_stage == ViewStage::EditorAndPanel
    }

    fn is_editor_nav_mode(&self) -> bool {
        self.is_editor_active()
            && !self.step_input_active
            && self.step_keyword_picker.is_none()
    }

    fn toggle_focus_slot_horizontal(&mut self) {
        let line = self.buffer.line(self.cursor_row);
        if line_body_edit_min_col_in_buffer(&self.buffer, self.cursor_row).is_none() {
            return;
        }
        match self.focus_slot {
            BddFocusSlot::Keyword => {
                self.focus_slot = BddFocusSlot::Body;
            }
            BddFocusSlot::Body => {
                if keyword_char_range(&line).is_some() {
                    self.focus_slot = BddFocusSlot::Keyword;
                }
            }
        }
    }

    fn vertical_nav_rows(&self) -> (Vec<usize>, bool) {
        let line = self.buffer.line(self.cursor_row);
        let body_chain_nav = self.focus_slot == BddFocusSlot::Body
            && (body_char_range(&line).is_some() || header_title_edit_start_col(&line).is_some())
            && !is_feature_narrative_row(&self.buffer, self.cursor_row);
        let rows = if body_chain_nav {
            bdd_step_and_header_title_rows(&self.buffer)
        } else {
            bdd_node_rows(&self.buffer)
        };
        (rows, body_chain_nav)
    }

    fn apply_vertical_nav_jump(&mut self, new_row: usize, body_chain_nav: bool) {
        self.cursor_row = new_row;
        self.cursor_col = 0;
        self.desired_col = 0;
        if body_chain_nav {
            return;
        }
        self.focus_slot = if is_feature_narrative_row(&self.buffer, new_row) {
            BddFocusSlot::Body
        } else {
            BddFocusSlot::Keyword
        };
    }

    #[allow(dead_code)]
    pub fn feature_outline_lines(&self) -> Vec<String> {
        let mut rows = Vec::new();
        for row in 0..self.buffer.line_count() {
            let line = self.buffer.line(row);
            let trimmed = line.trim_start();
            if ["Feature:", "Scenario:", "Scenario Outline:", "Examples:"]
                .iter()
                .any(|prefix| trimmed.starts_with(prefix))
            {
                rows.push(trimmed.to_string());
            }
        }
        rows
    }

    fn move_up(&mut self) {
        if self.step_input_active || self.step_keyword_picker.is_some() {
            return;
        }
        if self.is_editor_nav_mode() {
            let (rows, body_chain_nav) = self.vertical_nav_rows();
            if let Some(r) = prev_node_row(&rows, self.cursor_row) {
                self.apply_vertical_nav_jump(r, body_chain_nav);
            }
            self.quit_pending_confirm = false;
        }
    }

    fn move_down(&mut self) {
        if self.step_input_active || self.step_keyword_picker.is_some() {
            return;
        }
        if self.is_editor_nav_mode() {
            let (rows, body_chain_nav) = self.vertical_nav_rows();
            if let Some(r) = next_node_row(&rows, self.cursor_row) {
                self.apply_vertical_nav_jump(r, body_chain_nav);
            }
            self.quit_pending_confirm = false;
        }
    }

    fn move_left(&mut self) {
        if self.step_keyword_picker.is_some() {
            return;
        }
        if self.step_input_active {
            if self.cursor_col > self.step_input_min_col {
                self.cursor_col -= 1;
            }
            self.cursor_row = self.step_input_row;
            self.desired_col = self.cursor_col;
            self.quit_pending_confirm = false;
            return;
        }
        if self.is_editor_nav_mode() {
            // In stage 3 with keyword focus: go back to stage 2
            if self.focus_slot == BddFocusSlot::Keyword {
                let line = self.buffer.line(self.cursor_row);
                if keyword_char_range(&line).is_some() {
                    self.stage_back();
                    return;
                }
            }
            self.toggle_focus_slot_horizontal();
            self.quit_pending_confirm = false;
        }
    }

    fn move_right(&mut self) {
        if self.step_keyword_picker.is_some() {
            return;
        }
        if self.step_input_active {
            let line_len = self.buffer.line_len_chars(self.cursor_row);
            if self.cursor_col < line_len {
                self.cursor_col += 1;
            }
            self.cursor_row = self.step_input_row;
            self.desired_col = self.cursor_col;
            self.quit_pending_confirm = false;
            return;
        }
        if self.is_editor_nav_mode() {
            self.toggle_focus_slot_horizontal();
            self.quit_pending_confirm = false;
        }
    }

    fn move_home(&mut self) {
        if self.step_keyword_picker.is_some() {
            return;
        }
        if self.step_input_active {
            self.cursor_col = self.step_input_min_col;
            self.desired_col = self.cursor_col;
            self.quit_pending_confirm = false;
            return;
        }
        if self.is_editor_nav_mode() {
            let (rows, body_chain_nav) = self.vertical_nav_rows();
            if let Some(&r) = rows.first() {
                self.apply_vertical_nav_jump(r, body_chain_nav);
            }
            self.quit_pending_confirm = false;
        }
    }

    fn move_end(&mut self) {
        if self.step_keyword_picker.is_some() {
            return;
        }
        if self.step_input_active {
            self.cursor_col = self.buffer.line_len_chars(self.cursor_row);
            self.desired_col = self.cursor_col;
            self.quit_pending_confirm = false;
            return;
        }
        if self.is_editor_nav_mode() {
            let (rows, body_chain_nav) = self.vertical_nav_rows();
            if let Some(&r) = rows.last() {
                self.apply_vertical_nav_jump(r, body_chain_nav);
            }
            self.quit_pending_confirm = false;
        }
    }

    fn page_up(&mut self) {
        if self.step_input_active || self.step_keyword_picker.is_some() {
            return;
        }
        if !self.is_editor_nav_mode() {
            return;
        }
        let (rows, body_chain_nav) = self.vertical_nav_rows();
        let mut r = self.cursor_row;
        for _ in 0..10 {
            match prev_node_row(&rows, r) {
                Some(pr) => r = pr,
                None => break,
            }
        }
        if r != self.cursor_row {
            self.apply_vertical_nav_jump(r, body_chain_nav);
        }
        self.quit_pending_confirm = false;
    }

    fn page_down(&mut self) {
        if self.step_input_active || self.step_keyword_picker.is_some() {
            return;
        }
        if !self.is_editor_nav_mode() {
            return;
        }
        let (rows, body_chain_nav) = self.vertical_nav_rows();
        let mut r = self.cursor_row;
        for _ in 0..10 {
            match next_node_row(&rows, r) {
                Some(nr) => r = nr,
                None => break,
            }
        }
        if r != self.cursor_row {
            self.apply_vertical_nav_jump(r, body_chain_nav);
        }
        self.quit_pending_confirm = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        App, BddFocusSlot, MainTab, ViewStage, current_step_keyword_index,
        replace_step_keyword_line,
    };
    use crate::bdd_nav::step_edit_start_col;
    use crate::editor_buffer::EditorBuffer;
    use crate::keymap::Action;

    /// Helper: create an app pre-set to editor-active mode (stage 3) for existing editor tests.
    fn editor_test_app() -> App {
        let mut app = App::from_args().expect("app init should work");
        app.view_stage = ViewStage::EditorAndPanel;
        app
    }

    #[test]
    fn test_step_edit_boundary_detection() {
        assert_eq!(step_edit_start_col("  Given I log in"), Some(8));
        assert_eq!(step_edit_start_col("When x"), Some(5));
        assert_eq!(step_edit_start_col("Feature: x"), None);
    }

    #[test]
    fn test_activate_step_input_and_block_prefix_backspace() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        assert!(app.step_input_active);
        assert_eq!(app.cursor_col, 11);
        app.handle_action(Action::Backspace)
            .expect("backspace should work");
        assert_eq!(app.buffer.as_string(), "Given hell");
    }

    #[test]
    fn test_space_on_prefix_opens_step_keyword_picker() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello\n".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Keyword;
        app.handle_action(Action::ActivateStepInput)
            .expect("open picker should work");
        assert_eq!(app.buffer.line(0), "Given hello");
        assert!(!app.step_input_active);
        let picker = app.step_keyword_picker.expect("picker should be open");
        assert_eq!(picker.buffer_row, 0);
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn test_step_keyword_picker_confirm_updates_line() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Keyword;
        app.handle_action(Action::ActivateStepInput)
            .expect("open picker should work");
        app.handle_action(Action::StepKeywordPickerDown)
            .expect("move selection should work");
        app.handle_action(Action::StepKeywordPickerConfirm)
            .expect("confirm should work");
        assert_eq!(app.buffer.line(0), "When hello");
        assert!(app.step_keyword_picker.is_none());
    }

    #[test]
    fn test_step_keyword_picker_cancel_leaves_buffer() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Keyword;
        app.handle_action(Action::ActivateStepInput)
            .expect("open picker should work");
        app.handle_action(Action::StepKeywordPickerCancel)
            .expect("cancel should work");
        assert_eq!(app.buffer.line(0), "Given hello");
        assert!(app.step_keyword_picker.is_none());
    }

    #[test]
    fn test_space_in_body_activates_at_line_end() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        assert!(app.step_input_active);
        assert_eq!(app.cursor_col, 11);
    }

    #[test]
    fn test_space_on_feature_keyword_does_not_open_step_picker() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Feature: X\n".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.focus_slot, BddFocusSlot::Keyword);
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        assert!(app.step_keyword_picker.is_none());
        assert!(!app.step_input_active);
    }

    #[test]
    fn test_feature_title_body_edit() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Feature: My title\n".to_string());
        app.sync_cursor_to_first_node();
        app.handle_action(Action::MoveRight).expect("toggle body");
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::ActivateStepInput)
            .expect("edit should work");
        assert!(app.step_input_active);
        app.handle_action(Action::Insert('!'))
            .expect("insert should work");
        assert_eq!(app.buffer.line(0), "Feature: My title!");
    }

    #[test]
    fn test_feature_description_nav_and_edit() {
        let mut app = editor_test_app();
        app.buffer =
            EditorBuffer::from_string("Feature: T\n  Desc line\nBackground:\n".to_string());
        app.sync_cursor_to_first_node();
        app.handle_action(Action::MoveDown)
            .expect("move to description should work");
        assert_eq!(app.cursor_row, 1);
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        assert!(crate::bdd_nav::is_feature_narrative_row(&app.buffer, 1));
        app.handle_action(Action::ActivateStepInput)
            .expect("edit should work");
        assert!(app.step_input_active);
        assert_eq!(app.step_input_min_col, 0);
        app.handle_action(Action::Insert('!'))
            .expect("insert should work");
        assert_eq!(app.buffer.line(1), "  Desc line!");
    }

    #[test]
    fn test_replace_step_keyword_line_order() {
        assert_eq!(
            replace_step_keyword_line("  Given x", "When").as_deref(),
            Some("  When x")
        );
        assert_eq!(
            replace_step_keyword_line("But last", "Given").as_deref(),
            Some("Given last")
        );
        assert_eq!(current_step_keyword_index("  Given x"), Some(0));
        assert_eq!(current_step_keyword_index("But last"), Some(4));
    }

    #[test]
    fn test_quit_needs_confirmation_when_dirty() {
        let mut app = App::from_args().expect("app init should work");
        app.dirty = true;
        app.handle_action(Action::Quit).expect("quit should work");
        assert!(!app.should_quit);
        app.handle_action(Action::Quit).expect("quit should work");
        assert!(app.should_quit);
    }

    #[test]
    fn test_switching_tab_clears_step_input() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        assert!(app.step_input_active);
        app.handle_action(Action::SelectTab(MainTab::Help))
            .expect("tab switch should work");
        assert!(!app.step_input_active);
        assert_eq!(app.active_tab, MainTab::Help);
    }

    #[test]
    fn test_switching_tab_clears_step_keyword_picker() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Keyword;
        app.handle_action(Action::ActivateStepInput)
            .expect("open picker should work");
        assert!(app.step_keyword_picker.is_some());
        app.handle_action(Action::SelectTab(MainTab::Help))
            .expect("tab switch should work");
        assert!(app.step_keyword_picker.is_none());
        assert_eq!(app.active_tab, MainTab::Help);
    }

    #[test]
    fn test_feature_outline_lines_extracts_expected_rows() {
        let mut app = App::from_args().expect("app init should work");
        app.buffer = crate::editor_buffer::EditorBuffer::from_string(
            "Feature: Login\n  Scenario: ok\nGiven noop\n  Examples:\n".to_string(),
        );
        let outline = app.feature_outline_lines();
        assert_eq!(
            outline,
            vec![
                "Feature: Login".to_string(),
                "Scenario: ok".to_string(),
                "Examples:".to_string()
            ]
        );
    }

    #[test]
    fn test_nav_move_down_goes_to_next_node() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Feature: A\n  Given x\n".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.cursor_row, 0);
        assert_eq!(app.focus_slot, BddFocusSlot::Keyword);
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        assert_eq!(app.cursor_row, 1);
        assert_eq!(app.focus_slot, BddFocusSlot::Keyword);
    }

    #[test]
    fn test_nav_body_move_down_chain_includes_scenario_title() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n  Given a\n  Scenario: T\n  When b\n".to_string(),
        );
        app.sync_cursor_to_first_node();
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        assert_eq!(app.cursor_row, 2);
        app.handle_action(Action::MoveRight)
            .expect("body focus should work");
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveDown)
            .expect("body-chain move should work");
        assert_eq!(app.cursor_row, 3);
        assert!(app.buffer.line(3).trim_start().starts_with("Scenario"));
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveDown)
            .expect("body-chain move should work");
        assert_eq!(app.cursor_row, 4);
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
    }

    #[test]
    fn test_nav_body_on_feature_title_uses_body_chain() {
        let mut app = editor_test_app();
        app.buffer =
            EditorBuffer::from_string("Feature: A\n  Scenario: S\n  Given a\n".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.cursor_row, 0);
        app.handle_action(Action::MoveRight)
            .expect("body focus on feature title should work");
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        assert_eq!(app.cursor_row, 1);
        assert!(app.buffer.line(1).trim_start().starts_with("Scenario"));
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        assert_eq!(app.cursor_row, 2);
        assert!(app.buffer.line(2).trim_start().starts_with("Given"));
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
    }

    #[test]
    fn test_nav_body_move_up_from_step_to_scenario_title() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Feature: F\nScenario: S\n  When x\n".to_string());
        app.sync_cursor_to_first_node();
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        assert!(
            app.buffer
                .line(app.cursor_row)
                .trim_start()
                .starts_with("When")
        );
        app.handle_action(Action::MoveRight)
            .expect("body focus should work");
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveUp)
            .expect("body-chain move should work");
        assert_eq!(app.cursor_row, 1);
        assert!(app.buffer.line(1).trim_start().starts_with("Scenario"));
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
    }

    #[test]
    fn test_nav_left_right_toggles_keyword_and_body() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("  When hello".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.focus_slot, BddFocusSlot::Keyword);
        app.handle_action(Action::MoveRight)
            .expect("toggle should work");
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        // MoveLeft from Body toggles back to Keyword
        app.handle_action(Action::MoveLeft)
            .expect("toggle should work");
        assert_eq!(app.focus_slot, BddFocusSlot::Keyword);
    }

    #[test]
    fn test_space_respects_focus_slot_keyword_vs_body() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given ok\n".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Keyword;
        app.handle_action(Action::ActivateStepInput)
            .expect("picker open should work");
        assert!(app.step_keyword_picker.is_some());
        app.handle_action(Action::StepKeywordPickerCancel)
            .expect("cancel should work");
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("body edit should work");
        assert!(app.step_input_active);
    }

    #[test]
    fn test_stage_transitions() {
        let mut app = App::from_args().expect("app init should work");
        assert_eq!(app.view_stage, ViewStage::TreeOnly);

        // TreeOpen → Stage 2
        app.handle_action(Action::TreeOpen).expect("open should work");
        assert_eq!(app.view_stage, ViewStage::TreeAndEditor);

        // StageBack → Stage 1
        app.handle_action(Action::StageBack)
            .expect("back should work");
        assert_eq!(app.view_stage, ViewStage::TreeOnly);
    }

    #[test]
    fn test_edit_sync_on_stage_back() {
        use crate::gherkin;
        use std::path::PathBuf;

        let content = "Feature: Test\n  Scenario: S1\n    Given original step\n";
        let feature = gherkin::parse_feature(content, PathBuf::from("test.feature"));
        let project = crate::gherkin::BddProject {
            root_dir: PathBuf::from("."),
            features: vec![feature],
        };
        let step_index = crate::step_index::StepIndex::build(&project);
        let buffers = vec![EditorBuffer::from_string(content.to_string())];
        let tree_state = crate::mindmap::init_tree_state(&project);

        let mut app = App {
            project,
            step_index,
            buffers,
            active_buffer_idx: Some(0),
            view_stage: ViewStage::EditorAndPanel,
            tree_state,
            buffer: EditorBuffer::from_string(content.to_string()),
            file_path: Some(PathBuf::from("test.feature")),
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Keyword,
            should_quit: false,
            active_tab: MainTab::MindMap,
            dirty: false,
            status: String::new(),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            quit_pending_confirm: false,
        };

        // Simulate editing the buffer
        app.buffer.replace_line(2, "    Given modified step");
        app.dirty = true;

        // Going back from Stage 3 → Stage 2 triggers sync
        app.handle_action(Action::StageBack)
            .expect("stage back should work");
        assert_eq!(app.view_stage, ViewStage::TreeAndEditor);

        // Verify the project AST was re-parsed
        assert_eq!(app.project.features[0].scenarios[0].steps[0].text, "modified step");
        // Verify the buffer was persisted to the buffers vec
        assert!(app.buffers[0].line(2).contains("modified"));
    }
}
