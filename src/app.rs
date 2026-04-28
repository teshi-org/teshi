use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use crate::bdd_nav::{
    bdd_step_rows, current_step_keyword_index, delete_scenario_block, delete_step,
    insert_scenario_after_current, insert_step_above, insert_step_below,
    line_body_edit_min_col_in_buffer, next_node_row, prev_node_row, replace_step_keyword_line,
    scenario_content_rows, scenario_header_for_row, scenario_step_rows, swap_step_with_next,
    swap_step_with_prev,
};
use crate::editor_buffer::EditorBuffer;
use crate::gherkin::{self, BddProject};
use crate::keymap::Action;
use crate::mindmap;
use crate::runner::{self, RunCase, RunEvent, RunRequest, RunnerConfig};
use crate::step_index::StepIndex;

/// Step keywords in cycle order (re-exported for UI pickers).
pub use crate::bdd_nav::STEP_KEYWORDS_CYCLE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    MindMap,
    Explore,
    Help,
    Ai,
}

/// A single message in the AI chat history.
#[derive(Debug, Clone)]
pub struct AiChatMessage {
    pub role: AiRole,
    pub content: String,
    /// Tool calls included in an assistant message (for function calling).
    pub tool_calls: Option<Vec<crate::llm::ToolCall>>,
    /// The tool call ID this message responds to (for `Tool` role).
    pub tool_call_id: Option<String>,
    /// DeepSeek V4 thinking chain — preserved across tool-call turns.
    pub reasoning_content: Option<String>,
    /// Optional source tag for UI display (e.g., `"MindMap"`).
    pub source: Option<String>,
}

/// Who sent the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRole {
    User,
    Assistant,
    /// A tool result message fed back to the LLM.
    Tool,
}

/// Current state of the AI interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiStatus {
    Idle,
    Waiting,
    Error,
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

/// Focused column in the Explore tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFocus {
    Feature,
    Scenario,
    Step,
}

/// Focus target within the MindMap tab when the AI preview panel is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MindMapFocus {
    /// Tree has keyboard focus.
    Main,
    /// AI preview panel has keyboard focus.
    AiPanel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Idle,
    Running,
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct CaseDetail {
    pub case_id: String,
    pub status: RunStatus,
    pub duration_ms: Option<u64>,
    pub message: Option<String>,
    pub stack: Option<String>,
    pub attachments: Vec<runner::RunAttachment>,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// UI state for the step-keyword list shown after Space on the keyword prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepKeywordPicker {
    /// Buffer line index for the step being edited.
    pub buffer_row: usize,
    /// Index into [`STEP_KEYWORDS_CYCLE`] for the highlighted item.
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    fn capture(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

#[derive(Debug, Clone)]
struct ExternalChangePrompt {
    feature_idx: usize,
    path: PathBuf,
    disk_stamp: Option<FileStamp>,
}

/// A pending text modification queued by an AI agent tool waiting for user approval.
#[derive(Debug, Clone)]
pub struct AgentPendingChange {
    /// The tool name that initiated this change (e.g. `"insert_scenario"`).
    pub tool_name: String,
    /// Human-readable description for the confirmation prompt.
    pub description: String,
    /// Target file path (matches `BddFeature::file_path`).
    pub file_path: String,
    /// 1-based line number *after which* the text should be inserted.
    pub insertion_line_1based: usize,
    /// The full text to insert (including trailing newline).
    pub text_to_insert: String,
    /// Short scenario name for status messages.
    pub scenario_name: String,
    /// The tool call ID this change responds to (for feeding back to the LLM).
    pub tool_call_id: String,
}

pub struct App {
    // ── Multi-file project ──────────────────────────────────────────
    pub project: BddProject,
    pub step_index: StepIndex,
    pub mindmap_index: mindmap::MindMapIndex,
    pub mindmap_location_selection: HashMap<String, usize>,
    /// One `EditorBuffer` per feature file; order matches `project.features`.
    pub buffers: Vec<EditorBuffer>,
    buffer_dirty: Vec<bool>,
    disk_stamps: Vec<Option<FileStamp>>,
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
    // Stage-2 preview buffer (scenario-only slice)
    pub preview_buffer: Option<EditorBuffer>,
    pub preview_title: String,
    pub preview_cursor_row: usize,
    pub preview_scroll_row: usize,

    // ── Global UI ───────────────────────────────────────────────────
    pub should_quit: bool,
    pub active_tab: MainTab,
    pub dirty: bool,
    pub status: String,
    pub step_input_active: bool,
    step_input_row: usize,
    step_input_min_col: usize,
    pub step_keyword_picker: Option<StepKeywordPicker>,
    pub pending_char: Option<char>,
    pub clipboard: Option<String>,
    pub scenario_fold: HashSet<usize>,
    undo_stack: Vec<(EditorBuffer, usize, usize)>,
    redo_stack: Vec<(EditorBuffer, usize, usize)>,
    pub runner_config: Option<RunnerConfig>,
    runner_rx: Option<Receiver<RunEvent>>,
    last_external_check: Instant,
    external_change_prompt: Option<ExternalChangePrompt>,
    /// Pending text modifications requested by AI agent tools, awaiting user confirmation.
    pending_agent_changes: Vec<AgentPendingChange>,
    // ── Explore tab state ───────────────────────────────────────────
    pub explore_focus: ColumnFocus,
    pub explore_selected_feature: usize,
    pub explore_selected_scenario: usize,
    pub explore_selected_step: usize,
    pub explore_edit_mode: bool,
    pub explore_feature_scenario_memory: HashMap<usize, usize>,
    pub explore_scenario_step_memory: HashMap<(usize, usize), usize>,
    pub explore_case_map: HashMap<String, (usize, usize)>,
    pub explore_case_status: HashMap<(usize, usize), RunStatus>,
    pub explore_case_details: HashMap<(usize, usize), CaseDetail>,
    pub explore_detail_open: bool,
    pub explore_detail_case: Option<(usize, usize)>,
    pub explore_run_summary: Option<RunSummary>,
    // ── MindMap AI panel state ────────────────────────────────────────
    pub mindmap_focus: MindMapFocus,
    pub mindmap_ai_panel_visible: bool,
    // ── AI tab state ───────────────────────────────────────────────
    pub ai_messages: Vec<AiChatMessage>,
    pub ai_input: String,
    pub ai_status: AiStatus,
    pub ai_partial_response: String,
    pub ai_llm_handle: Option<crate::llm::LlmHandle>,
    pub ai_llm_rx: Option<std::sync::mpsc::Receiver<crate::llm::LlmEvent>>,
    /// Human-readable status text shown while the agent executes a tool.
    pub ai_tool_status: Option<String>,
    /// Bounds the agent loop to prevent infinite tool-call cycles.
    agent_loop_count: u32,
    quit_pending_confirm: bool,
    /// Temporary one-shot status message (e.g. "AI applied filter: @smoke").
    pub status_message: Option<String>,
    /// When the status message should be cleared (3-second lifespan).
    status_message_deadline: Option<Instant>,
}

impl App {
    fn capture_disk_stamps(project: &BddProject) -> Vec<Option<FileStamp>> {
        project
            .features
            .iter()
            .map(|feature| FileStamp::capture(&feature.file_path))
            .collect()
    }

    /// Builds the editor state from process arguments.
    ///
    /// Accepts a directory path (recursive `.feature` scan) or a single file path.
    /// When both a directory and a `.feature` file path are given (e.g.
    /// `cargo run -- . path/to/demo.feature`), the specific file takes priority.
    pub fn from_args() -> Result<Self> {
        let paths: Vec<PathBuf> = std::env::args()
            .skip(1)
            .filter(|arg| !arg.starts_with('-'))
            .map(PathBuf::from)
            .collect();

        // Prefer an explicit .feature file path over a directory path
        let feature_file = paths
            .iter()
            .find(|p| p.extension().is_some_and(|ext| ext == "feature"));
        if let Some(p) = feature_file {
            return Self::from_file(p);
        }

        match paths.iter().find(|p| p.is_dir()) {
            Some(p) => Self::from_directory(p),
            None => Ok(Self::empty()),
        }
    }

    fn from_directory(dir: &Path) -> Result<Self> {
        let project = gherkin::parse_project(dir);
        let step_index = StepIndex::build(&project);
        let mindmap_index = mindmap::build_index(&project);
        let disk_stamps = Self::capture_disk_stamps(&project);
        let buffers: Vec<EditorBuffer> = project
            .features
            .iter()
            .map(|f| {
                let content = fs::read_to_string(&f.file_path).unwrap_or_default();
                EditorBuffer::from_string(content)
            })
            .collect();
        let buffer_dirty = vec![false; buffers.len()];
        let tree_state = mindmap::init_tree_state(&mindmap_index);
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
            mindmap_index,
            mindmap_location_selection: HashMap::new(),
            buffers,
            buffer_dirty,
            disk_stamps,
            active_buffer_idx: active_idx,
            view_stage: ViewStage::TreeOnly,
            tree_state,
            buffer,
            file_path,
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Body,
            preview_buffer: None,
            preview_title: String::new(),
            preview_cursor_row: 0,
            preview_scroll_row: 0,
            should_quit: false,
            active_tab: MainTab::Explore,
            dirty: false,
            status: format!(
                "Opened directory with {} feature file(s)",
                active_idx
                    .map_or(0, |_| 1)
                    .max(if active_idx.is_some() { 1 } else { 0 })
            ),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            pending_char: None,
            clipboard: None,
            scenario_fold: HashSet::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_case_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: true,
            ai_messages: Vec::new(),
            ai_input: String::new(),
            ai_status: AiStatus::Idle,
            ai_partial_response: String::new(),
            ai_llm_handle: None,
            ai_llm_rx: None,
            ai_tool_status: None,
            agent_loop_count: 0,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
        };
        app.spawn_llm_if_configured();
        let n = app.buffers.len();
        app.status = format!("Opened directory with {n} feature file(s)");
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
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
        let mindmap_index = mindmap::build_index(&project);
        let buffers = vec![EditorBuffer::from_string(content.clone())];
        let buffer_dirty = vec![false; buffers.len()];
        let disk_stamps = Self::capture_disk_stamps(&project);
        let tree_state = mindmap::init_tree_state(&mindmap_index);
        let mut app = Self {
            project,
            step_index,
            mindmap_index,
            mindmap_location_selection: HashMap::new(),
            buffers,
            buffer_dirty,
            disk_stamps,
            active_buffer_idx: Some(0),
            view_stage: ViewStage::TreeOnly,
            tree_state,
            buffer: EditorBuffer::from_string(content),
            file_path: Some(path.clone()),
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Body,
            preview_buffer: None,
            preview_title: String::new(),
            preview_cursor_row: 0,
            preview_scroll_row: 0,
            should_quit: false,
            active_tab: MainTab::Explore,
            dirty: false,
            status: "Opened file".to_string(),
            ai_messages: Vec::new(),
            ai_input: String::new(),
            ai_status: AiStatus::Idle,
            ai_partial_response: String::new(),
            ai_llm_handle: None,
            ai_llm_rx: None,
            ai_tool_status: None,
            agent_loop_count: 0,
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            pending_char: None,
            clipboard: None,
            scenario_fold: HashSet::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_case_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: true,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
        };
        app.spawn_llm_if_configured();
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        Ok(app)
    }

    fn empty() -> Self {
        let project = BddProject {
            root_dir: PathBuf::from("."),
            features: Vec::new(),
        };
        let step_index = StepIndex::build(&project);
        let mindmap_index = mindmap::build_index(&project);
        let tree_state = mindmap::init_tree_state(&mindmap_index);
        let mut app = Self {
            project,
            step_index,
            mindmap_index,
            mindmap_location_selection: HashMap::new(),
            buffers: Vec::new(),
            buffer_dirty: Vec::new(),
            disk_stamps: Vec::new(),
            active_buffer_idx: None,
            view_stage: ViewStage::TreeOnly,
            tree_state,
            buffer: EditorBuffer::from_string(String::new()),
            file_path: None,
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Body,
            preview_buffer: None,
            preview_title: String::new(),
            preview_cursor_row: 0,
            preview_scroll_row: 0,
            should_quit: false,
            active_tab: MainTab::Explore,
            dirty: false,
            status: "New buffer".to_string(),
            ai_messages: Vec::new(),
            ai_input: String::new(),
            ai_status: AiStatus::Idle,
            ai_partial_response: String::new(),
            ai_llm_handle: None,
            ai_llm_rx: None,
            ai_tool_status: None,
            agent_loop_count: 0,
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            pending_char: None,
            clipboard: None,
            scenario_fold: HashSet::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_case_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: true,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
        };
        app.spawn_llm_if_configured();
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        app
    }

    /// Positions the navigation row on the first BDD node, or keeps row `0` when there are none.
    fn sync_cursor_to_first_node(&mut self) {
        let rows = bdd_step_rows(&self.buffer);
        if let Some(&r) = rows.first() {
            self.cursor_row = r;
            self.cursor_col = 0;
            self.desired_col = 0;
        }
        self.focus_slot = BddFocusSlot::Body;
    }

    fn normalize_explore_selection(&mut self) {
        let feature_len = self.project.features.len();
        if feature_len == 0 {
            self.explore_selected_feature = 0;
            self.explore_selected_scenario = 0;
            self.explore_selected_step = 0;
            return;
        }
        if self.explore_selected_feature >= feature_len {
            self.explore_selected_feature = feature_len - 1;
        }
        let scenarios = &self.project.features[self.explore_selected_feature].scenarios;
        if scenarios.is_empty() {
            self.explore_selected_scenario = 0;
            self.explore_selected_step = 0;
            return;
        }
        if self.explore_selected_scenario >= scenarios.len() {
            self.explore_selected_scenario = scenarios.len() - 1;
        }
        let steps = &scenarios[self.explore_selected_scenario].steps;
        if steps.is_empty() {
            self.explore_selected_step = 0;
            return;
        }
        if self.explore_selected_step >= steps.len() {
            self.explore_selected_step = steps.len() - 1;
        }
    }

    fn sync_dirty_flag_with_active_buffer(&mut self) {
        self.dirty = self
            .active_buffer_idx
            .and_then(|idx| self.buffer_dirty.get(idx).copied())
            .unwrap_or(false);
    }

    fn set_buffer_dirty(&mut self, idx: usize, dirty: bool) {
        if let Some(slot) = self.buffer_dirty.get_mut(idx) {
            *slot = dirty;
        }
        if self.active_buffer_idx == Some(idx) {
            self.dirty = dirty;
        }
    }

    fn mark_current_buffer_dirty(&mut self) {
        if let Some(idx) = self.active_buffer_idx {
            self.set_buffer_dirty(idx, true);
        } else {
            self.dirty = true;
        }
    }

    pub fn has_external_change_prompt(&self) -> bool {
        self.external_change_prompt.is_some()
    }

    pub fn external_change_prompt_title(&self) -> Option<&'static str> {
        self.external_change_prompt
            .as_ref()
            .map(|_| "Feature changed on disk")
    }

    pub fn external_change_prompt_path(&self) -> Option<String> {
        self.external_change_prompt.as_ref().map(|prompt| {
            prompt
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| prompt.path.display().to_string())
        })
    }

    pub fn poll_runner_events(&mut self) {
        let Some(rx) = self.runner_rx.take() else {
            return;
        };
        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(event) => {
                    let end = matches!(
                        event,
                        RunEvent::RunnerExit { .. } | RunEvent::RunnerError { .. }
                    );
                    self.apply_run_event(event);
                    if end {
                        keep_rx = false;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    keep_rx = false;
                    break;
                }
            }
        }
        if keep_rx {
            self.runner_rx = Some(rx);
        }
    }

    /// Spawn the LLM worker thread if `TESHI_LLM_API_KEY` is set and no handle exists yet.
    pub fn spawn_llm_if_configured(&mut self) {
        if self.ai_llm_handle.is_some() {
            return;
        }
        match crate::llm::LlmConfig::from_env() {
            Ok(config) => {
                self.status = format!(
                    "LLM configured: model={}, base_url={}",
                    config.model, config.base_url
                );
                let (handle, rx) = crate::llm::spawn_llm(config);
                self.ai_llm_handle = Some(handle);
                self.ai_llm_rx = Some(rx);
            }
            Err(e) => {
                self.status = format!("LLM not configured: {e}");
            }
        }
    }

    /// Poll the LLM response channel and push completed responses into chat history.
    ///
    /// When the LLM requests tool calls, this method executes them and
    /// re-invokes the LLM with the results (the "agent loop") until a plain
    /// text response is received or the iteration limit is reached.
    pub fn poll_llm_events(&mut self) {
        let Some(rx) = self.ai_llm_rx.take() else {
            return;
        };
        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(crate::llm::LlmEvent::Done {
                    full_text,
                    reasoning_content,
                    model,
                    ..
                }) => {
                    // If we already have a partial response from streaming,
                    // use that instead; otherwise store the full text.
                    if self.ai_partial_response.is_empty() {
                        self.ai_messages.push(AiChatMessage {
                            role: AiRole::Assistant,
                            content: full_text,
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content,
                            source: None,
                        });
                    } else {
                        let content = std::mem::take(&mut self.ai_partial_response);
                        self.ai_messages.push(AiChatMessage {
                            role: AiRole::Assistant,
                            content,
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content,
                            source: None,
                        });
                    }
                    self.ai_partial_response.clear();
                    self.ai_status = AiStatus::Idle;
                    self.ai_tool_status = None;
                    self.agent_loop_count = 0;
                    self.status = format!("AI response received ({model})");
                }
                Ok(crate::llm::LlmEvent::ToolCallRequest {
                    tool_calls,
                    reasoning_content,
                }) => {
                    // Store the assistant message with its tool calls
                    let partial_text = std::mem::take(&mut self.ai_partial_response);
                    self.ai_messages.push(AiChatMessage {
                        role: AiRole::Assistant,
                        content: partial_text,
                        tool_calls: Some(tool_calls.clone()),
                        tool_call_id: None,
                        reasoning_content,
                        source: None,
                    });

                    // Execute each requested tool and append results
                    let mut pending_queued = false;
                    for tc in &tool_calls {
                        self.ai_tool_status = Some(format!("AI is calling {}...", tc.name));
                        let pending_before = self.pending_agent_changes.len();
                        match crate::agent::execute_tool(self, &tc.name, &tc.arguments, &tc.id) {
                            Ok(result) => {
                                let pending_after = self.pending_agent_changes.len();
                                if pending_after > pending_before {
                                    // The tool queued a pending change — don't feed the
                                    // placeholder result; user confirmation will do it.
                                    pending_queued = true;
                                } else {
                                    self.ai_messages.push(AiChatMessage {
                                        role: AiRole::Tool,
                                        content: result,
                                        tool_calls: None,
                                        tool_call_id: Some(tc.id.clone()),
                                        reasoning_content: None,
                                        source: None,
                                    });
                                }
                            }
                            Err(e) => {
                                self.ai_messages.push(AiChatMessage {
                                    role: AiRole::Tool,
                                    content: format!("Error: {e}"),
                                    tool_calls: None,
                                    tool_call_id: Some(tc.id.clone()),
                                    reasoning_content: None,
                                    source: None,
                                });
                            }
                        }
                    }

                    // Continue the conversation: re-invoke LLM with results,
                    // unless a tool queued a pending change that needs user confirmation.
                    if pending_queued {
                        self.ai_partial_response.clear();
                        self.ai_status = AiStatus::Idle;
                        self.ai_tool_status = None;
                        // The agent loop will resume when the user confirms/rejects.
                    } else {
                        self.agent_loop_count += 1;
                        if self.agent_loop_count > 5 {
                            self.ai_status = AiStatus::Error;
                            self.ai_tool_status = None;
                            self.agent_loop_count = 0;
                            self.status = "AI error: too many tool call iterations".to_string();
                        } else if let Some(ref handle) = self.ai_llm_handle {
                            let messages = self.build_chat_messages_for_llm();
                            let tools = Some(crate::agent::get_tools());
                            let _ = handle.send(crate::llm::LlmRequest::Chat {
                                system: Some(Self::ai_system_prompt().into()),
                                messages,
                                tools,
                            });
                        }
                    }
                }
                Ok(crate::llm::LlmEvent::Error { message }) => {
                    self.ai_partial_response.clear();
                    self.ai_status = AiStatus::Error;
                    self.ai_tool_status = None;
                    self.agent_loop_count = 0;
                    self.status = format!("AI error: {message}");
                }
                Ok(crate::llm::LlmEvent::Chunk { content }) => {
                    self.ai_partial_response.push_str(&content);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    keep_rx = false;
                    self.ai_partial_response.clear();
                    self.ai_status = AiStatus::Error;
                    self.ai_tool_status = None;
                    self.agent_loop_count = 0;
                    self.status = "AI error: background LLM thread has exited".to_string();
                    break;
                }
            }
        }
        if keep_rx {
            self.ai_llm_rx = Some(rx);
        }
    }

    /// Build `ChatMessage` list from the current AI chat history for LLM
    /// requests.
    fn build_chat_messages_for_llm(&self) -> Vec<crate::llm::ChatMessage> {
        self.ai_messages
            .iter()
            .map(|m| crate::llm::ChatMessage {
                role: match m.role {
                    AiRole::User => "user".into(),
                    AiRole::Assistant => "assistant".into(),
                    AiRole::Tool => "tool".into(),
                },
                content: m.content.clone(),
                tool_calls: m.tool_calls.clone(),
                tool_call_id: m.tool_call_id.clone(),
                reasoning_content: m.reasoning_content.clone(),
            })
            .collect()
    }

    /// The system prompt used for all AI chat requests.
    fn ai_system_prompt() -> &'static str {
        "You are a BDD/Gherkin assistant. You have tools to inspect and edit feature files.\n\
         \n\
         Workflow when the user mentions a specific file (e.g. \"add a scenario to X.feature\"):\n\
         1. Call get_feature_content first to see the file's current scenarios and line numbers.\n\
         2. Then call insert_scenario with the right steps and insert_after_line.\n\
         \n\
         Use highlight_mindmap_nodes / apply_mindmap_filter only for visual exploration — \
         they do NOT return text content. Use get_project_info for general project stats, \
         and get_feature_content for specific file details.\n\
         \n\
         Keep tool calls to a minimum. One get_feature_content is enough before editing."
    }

    /// After a pending agent change is accepted or rejected, feed the result back
    /// to the LLM as a tool result message and continue the agent loop.
    fn feed_agent_tool_result(&mut self, tool_call_id: String, result: String) {
        // Append tool result message
        self.ai_messages.push(AiChatMessage {
            role: AiRole::Tool,
            content: result,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            reasoning_content: None,
            source: None,
        });

        // Re-invoke the LLM to continue the agent loop
        self.agent_loop_count += 1;
        if self.agent_loop_count > 5 {
            self.ai_status = AiStatus::Error;
            self.ai_tool_status = None;
            self.agent_loop_count = 0;
            self.status = "AI error: too many tool call iterations".to_string();
        } else if let Some(ref handle) = self.ai_llm_handle {
            let messages = self.build_chat_messages_for_llm();
            let tools = Some(crate::agent::get_tools());
            let _ = handle.send(crate::llm::LlmRequest::Chat {
                system: Some(Self::ai_system_prompt().into()),
                messages,
                tools,
            });
        }
    }

    pub fn poll_external_feature_changes(&mut self) {
        if self.project.features.is_empty() || self.external_change_prompt.is_some() {
            return;
        }
        if self.last_external_check.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_external_check = Instant::now();

        for idx in 0..self.project.features.len() {
            let path = self.project.features[idx].file_path.clone();
            let current_stamp = FileStamp::capture(&path);
            let known_stamp = self.disk_stamps.get(idx).cloned().unwrap_or(None);
            if current_stamp == known_stamp {
                continue;
            }

            if self.buffer_dirty.get(idx).copied().unwrap_or(false) {
                self.external_change_prompt = Some(ExternalChangePrompt {
                    feature_idx: idx,
                    path: path.clone(),
                    disk_stamp: current_stamp,
                });
                self.status = format!(
                    "Feature changed on disk: {}. Reload [Enter/r] or keep local [Esc/k].",
                    path.display()
                );
            } else if let Err(err) = self.reload_feature_from_disk(idx, current_stamp) {
                self.status = format!("Failed to reload {}: {err}", path.display());
            }
            self.quit_pending_confirm = false;
            break;
        }
    }

    /// Clear the temporary status message if its 3-second lifespan has elapsed.
    pub fn poll_status_message_expiry(&mut self) {
        if let Some(deadline) = self.status_message_deadline
            && Instant::now() >= deadline
        {
            self.status_message = None;
            self.status_message_deadline = None;
        }
    }

    /// Set a temporary status message that auto-clears after 3 seconds.
    fn set_status_message(&mut self, msg: String) {
        self.status_message = Some(msg);
        self.status_message_deadline = Some(Instant::now() + Duration::from_secs(3));
    }

    // ── Agent MindMap modification helpers ────────────────────────────

    /// Apply highlight rules to the MindMap tree (called by Agent tools).
    pub fn apply_mindmap_highlights(&mut self, rules: Vec<mindmap::HighlightRule>) {
        let count = rules.len();
        self.mindmap_index.apply_highlights(rules);
        self.set_status_message(format!("AI applied {count} highlight rule(s)"));
    }

    /// Apply a filter to the MindMap tree (called by Agent tools).
    pub fn apply_mindmap_filter(&mut self, filter: mindmap::MindMapFilter) {
        let desc = match &filter {
            mindmap::MindMapFilter::NameContains(text) => format!("@\u{200B}{text}"),
        };
        self.mindmap_index.apply_filter(filter);
        self.set_status_message(format!("AI applied filter: {desc}"));
    }

    /// Clear all MindMap highlights (called by Agent tools).
    pub fn clear_mindmap_highlights(&mut self) {
        self.mindmap_index.clear_highlights();
        self.set_status_message("AI cleared highlights".into());
    }

    /// Clear the MindMap filter (called by Agent tools).
    pub fn clear_mindmap_filter(&mut self) {
        self.mindmap_index.clear_filter();
        self.set_status_message("AI cleared filter".into());
    }

    // ── Agent editor modification helpers ───────────────────────────────

    /// Finds the feature index in `project.features` whose file path matches
    /// `file_path` (compared by file name and/or full path suffix).
    pub fn find_feature_idx_for_file(&self, file_path: &str) -> Option<usize> {
        self.project.features.iter().position(|f| {
            let p = f.file_path.to_string_lossy();
            p == file_path || p.ends_with(file_path) || file_path.ends_with(p.as_ref())
        })
    }

    /// Insert text into the buffer for `file_path` after the given 1-based line number.
    ///
    /// Updates the active editor view if the target file is currently displayed.
    /// Does not write to disk; the buffer is marked dirty.
    pub fn insert_text_into_buffer(
        &mut self,
        file_path: &str,
        after_line_1based: usize,
        text: &str,
    ) -> Result<()> {
        let feature_idx = self
            .find_feature_idx_for_file(file_path)
            .with_context(|| format!("feature file not found: {file_path}"))?;

        // Insert into the persistent buffer
        self.buffers[feature_idx].insert_line(after_line_1based.saturating_sub(1), text);
        self.set_buffer_dirty(feature_idx, true);

        // Update active editor view if this is the current buffer
        if self.active_buffer_idx == Some(feature_idx) {
            self.buffer = self.buffers[feature_idx].clone();
        }

        Ok(())
    }

    /// Returns the content of the buffer for a given file path.
    pub fn buffer_content_for_file(&self, file_path: &str) -> Option<String> {
        let idx = self.find_feature_idx_for_file(file_path)?;
        Some(self.buffers[idx].as_string())
    }

    /// Returns the line count of the buffer for a given file path.
    pub fn line_count_for_file(&self, file_path: &str) -> Option<usize> {
        let idx = self.find_feature_idx_for_file(file_path)?;
        Some(self.buffers[idx].line_count())
    }

    /// Re-parse the project from the current buffer contents (applies pending
    /// text edits to the Gherkin AST, MindMap, and step index).
    pub fn refresh_project_from_buffers(&mut self) {
        let selected = self.selected_tree_location();
        for (idx, buffer) in self.buffers.iter().enumerate() {
            if idx < self.project.features.len() {
                let content = buffer.as_string();
                let path = self.project.features[idx].file_path.clone();
                self.project.features[idx] = gherkin::parse_feature(&content, path);
            }
        }
        self.rebuild_project_views(selected);
    }

    // ── Agent pending change queue ──────────────────────────────────────

    /// Whether an agent change is waiting for user confirmation.
    pub fn has_agent_change_prompt(&self) -> bool {
        !self.pending_agent_changes.is_empty()
    }

    /// Queue a pending change from the agent, show confirmation prompt.
    pub fn queue_agent_change(&mut self, change: AgentPendingChange) {
        let desc = change.description.clone();
        self.pending_agent_changes.push(change);
        self.status = format!("AI wants to {}. [Y] accept [N] reject [D] view diff", desc);
    }

    /// Accept and apply the first pending agent change.
    ///
    /// Returns `(tool_call_id, result_text)` for feeding back to the LLM.
    pub fn accept_agent_change(&mut self) -> Result<(String, String)> {
        let change = self.pending_agent_changes.remove(0);
        self.insert_text_into_buffer(
            &change.file_path,
            change.insertion_line_1based,
            &change.text_to_insert,
        )?;

        // Re-parse the project to update Gherkin AST and MindMap
        self.refresh_project_from_buffers();

        // Move cursor to the newly inserted scenario area
        if let Some(idx) = self.find_feature_idx_for_file(&change.file_path) {
            if self.active_buffer_idx != Some(idx) {
                // Switch to the modified buffer
                self.switch_to_buffer(idx);
            }
        }
        // Position cursor at the start of the inserted text (first line after
        // insertion point)
        let new_cursor_row = change.insertion_line_1based;
        self.cursor_row = new_cursor_row.min(self.buffer.line_count().saturating_sub(1));
        self.cursor_col = 0;
        self.desired_col = 0;
        self.scroll_row = self.cursor_row.saturating_sub(4).max(0);

        self.set_status_message(format!(
            "AI inserted scenario \"{}\" in {}",
            change.scenario_name, change.file_path
        ));

        let file_name = change.file_path.clone();
        let scenario_name = change.scenario_name.clone();
        let result = format!(
            "Successfully inserted scenario \"{scenario_name}\" into {file_name} at line {}.",
            change.insertion_line_1based + 1
        );
        Ok((change.tool_call_id, result))
    }

    /// Reject and discard the first pending agent change.
    ///
    /// Returns `(tool_call_id, result_text)` for feeding back to the LLM.
    pub fn reject_agent_change(&mut self) -> (String, String) {
        let change = self.pending_agent_changes.remove(0);
        let desc = change.description.clone();
        self.status = format!("Rejected AI change: {desc}");
        self.quit_pending_confirm = false;
        let result = format!("User rejected the change: {desc}");
        (change.tool_call_id, result)
    }

    fn reload_feature_from_disk(&mut self, idx: usize, stamp: Option<FileStamp>) -> Result<()> {
        if idx >= self.project.features.len() || idx >= self.buffers.len() {
            return Ok(());
        }

        let selected_tree_location = self.selected_tree_location();
        let path = self.project.features[idx].file_path.clone();
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let reloaded_buffer = EditorBuffer::from_string(content.clone());
        let feature = gherkin::parse_feature(&content, path.clone());

        self.project.features[idx] = feature;
        self.buffers[idx] = reloaded_buffer.clone();
        self.set_buffer_dirty(idx, false);
        if let Some(slot) = self.disk_stamps.get_mut(idx) {
            *slot = stamp.or_else(|| FileStamp::capture(&path));
        }

        if self.active_buffer_idx == Some(idx) {
            self.buffer = reloaded_buffer;
            self.file_path = Some(path.clone());
            self.clear_step_input_state();
            self.clear_step_keyword_picker();
            self.pending_char = None;
            self.scenario_fold.clear();
            self.clamp_cursor();
        }

        self.rebuild_project_views(selected_tree_location);
        self.external_change_prompt = None;
        self.status = format!("Reloaded from disk: {}", path.display());
        self.quit_pending_confirm = false;
        Ok(())
    }

    fn rebuild_project_views(&mut self, selected_tree_location: Option<(usize, usize)>) {
        self.step_index = StepIndex::build(&self.project);
        self.mindmap_index = mindmap::build_index(&self.project);
        self.tree_state = mindmap::init_tree_state(&self.mindmap_index);
        self.mindmap_location_selection.clear();
        self.normalize_explore_selection();

        if let Some((feature_idx, line_number)) = selected_tree_location {
            self.restore_tree_selection_from_line(feature_idx, line_number);
        }

        if self.active_tab == MainTab::MindMap && self.view_stage == ViewStage::TreeAndEditor {
            self.rebuild_preview();
        }
    }

    fn restore_tree_selection_from_line(&mut self, feature_idx: usize, line_number: usize) {
        let Some(node_match) =
            mindmap::find_closest_node(&self.mindmap_index, feature_idx, line_number)
        else {
            return;
        };
        let Some(path) = mindmap::node_id_to_path(&node_match.node_id, &self.mindmap_index) else {
            return;
        };
        self.tree_state.select(path);
        self.mindmap_location_selection
            .insert(node_match.node_id, node_match.location_index);
    }

    fn accept_external_reload(&mut self) -> Result<()> {
        let Some(prompt) = self.external_change_prompt.clone() else {
            return Ok(());
        };
        self.reload_feature_from_disk(prompt.feature_idx, prompt.disk_stamp)
    }

    fn keep_local_external_version(&mut self) {
        let Some(prompt) = self.external_change_prompt.take() else {
            return;
        };
        if let Some(slot) = self.disk_stamps.get_mut(prompt.feature_idx) {
            *slot = prompt.disk_stamp;
        }
        self.status = format!("Kept local buffer for {}", prompt.path.display());
        self.quit_pending_confirm = false;
    }

    fn apply_run_event(&mut self, event: RunEvent) {
        match event {
            RunEvent::StartRun { total, .. } => {
                self.explore_run_summary = Some(RunSummary {
                    total: total.unwrap_or(0),
                    passed: 0,
                    failed: 0,
                    skipped: 0,
                });
            }
            RunEvent::StartCase { case_id, .. } => {
                if let Some(key) = self.explore_case_map.get(&case_id).copied() {
                    self.explore_case_status.insert(key, RunStatus::Running);
                    self.explore_case_details
                        .entry(key)
                        .or_insert_with(|| CaseDetail {
                            case_id: case_id.clone(),
                            status: RunStatus::Running,
                            duration_ms: None,
                            message: None,
                            stack: None,
                            attachments: Vec::new(),
                            logs: Vec::new(),
                        });
                }
            }
            RunEvent::CasePassed {
                case_id,
                duration_ms,
            } => {
                if let Some(key) = self.explore_case_map.get(&case_id).copied() {
                    self.explore_case_status.insert(key, RunStatus::Passed);
                    let detail =
                        self.explore_case_details
                            .entry(key)
                            .or_insert_with(|| CaseDetail {
                                case_id: case_id.clone(),
                                status: RunStatus::Passed,
                                duration_ms: None,
                                message: None,
                                stack: None,
                                attachments: Vec::new(),
                                logs: Vec::new(),
                            });
                    detail.status = RunStatus::Passed;
                    detail.duration_ms = duration_ms;
                    detail.message = None;
                    detail.stack = None;
                    if let Some(summary) = self.explore_run_summary.as_mut() {
                        summary.passed = summary.passed.saturating_add(1);
                    }
                }
            }
            RunEvent::CaseFailed {
                case_id,
                duration_ms,
                error,
            } => {
                if let Some(key) = self.explore_case_map.get(&case_id).copied() {
                    self.explore_case_status.insert(key, RunStatus::Failed);
                    let detail =
                        self.explore_case_details
                            .entry(key)
                            .or_insert_with(|| CaseDetail {
                                case_id: case_id.clone(),
                                status: RunStatus::Failed,
                                duration_ms: None,
                                message: None,
                                stack: None,
                                attachments: Vec::new(),
                                logs: Vec::new(),
                            });
                    detail.status = RunStatus::Failed;
                    detail.duration_ms = duration_ms;
                    detail.message = Some(error.message);
                    detail.stack = error.stack;
                    if !error.attachments.is_empty() {
                        detail.attachments.extend(error.attachments);
                    }
                    if let Some(summary) = self.explore_run_summary.as_mut() {
                        summary.failed = summary.failed.saturating_add(1);
                    }
                }
            }
            RunEvent::CaseSkipped { case_id, reason } => {
                if let Some(key) = self.explore_case_map.get(&case_id).copied() {
                    self.explore_case_status.insert(key, RunStatus::Skipped);
                    let detail =
                        self.explore_case_details
                            .entry(key)
                            .or_insert_with(|| CaseDetail {
                                case_id: case_id.clone(),
                                status: RunStatus::Skipped,
                                duration_ms: None,
                                message: None,
                                stack: None,
                                attachments: Vec::new(),
                                logs: Vec::new(),
                            });
                    detail.status = RunStatus::Skipped;
                    detail.duration_ms = None;
                    detail.message = reason;
                    detail.stack = None;
                    if let Some(summary) = self.explore_run_summary.as_mut() {
                        summary.skipped = summary.skipped.saturating_add(1);
                    }
                }
            }
            RunEvent::Log { case_id, message } => {
                if let Some(case_id) = case_id
                    && let Some(key) = self.explore_case_map.get(&case_id).copied()
                {
                    let detail =
                        self.explore_case_details
                            .entry(key)
                            .or_insert_with(|| CaseDetail {
                                case_id: case_id.clone(),
                                status: RunStatus::Running,
                                duration_ms: None,
                                message: None,
                                stack: None,
                                attachments: Vec::new(),
                                logs: Vec::new(),
                            });
                    if detail.logs.len() >= 200 {
                        detail.logs.remove(0);
                    }
                    detail.logs.push(message);
                }
            }
            RunEvent::Artifact {
                case_id,
                kind,
                path,
            } => {
                if let Some(case_id) = case_id
                    && let Some(key) = self.explore_case_map.get(&case_id).copied()
                {
                    let detail =
                        self.explore_case_details
                            .entry(key)
                            .or_insert_with(|| CaseDetail {
                                case_id: case_id.clone(),
                                status: RunStatus::Running,
                                duration_ms: None,
                                message: None,
                                stack: None,
                                attachments: Vec::new(),
                                logs: Vec::new(),
                            });
                    detail
                        .attachments
                        .push(runner::RunAttachment { kind, path });
                }
            }
            RunEvent::EndRun {
                passed,
                failed,
                skipped,
            } => {
                let total = passed + failed + skipped;
                self.explore_run_summary = Some(RunSummary {
                    total,
                    passed,
                    failed,
                    skipped,
                });
                self.status = format!("Run complete: {passed} passed, {failed} failed");
            }
            RunEvent::RunnerExit { success, .. } => {
                if !success {
                    self.status = "Runner exited with error".to_string();
                }
                self.runner_rx = None;
            }
            RunEvent::RunnerError { message } => {
                self.status = format!("Runner error: {message}");
                self.runner_rx = None;
            }
        }
        self.quit_pending_confirm = false;
    }

    fn reset_explore_run_state(&mut self) {
        self.explore_case_map.clear();
        self.explore_case_status.clear();
        self.explore_case_details.clear();
        self.explore_run_summary = None;
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn start_explore_run(&mut self) {
        if self.runner_rx.is_some() {
            self.status = "Runner already active".to_string();
            return;
        }
        let Some(config) = self.runner_config.clone() else {
            self.status = "Runner not configured (teshi.toml or TESHI_RUNNER_CMD)".to_string();
            return;
        };
        let cases = self.build_explore_cases();
        if cases.is_empty() {
            self.status = "No scenarios to run".to_string();
            return;
        }
        self.reset_explore_run_state();
        self.explore_run_summary = Some(RunSummary {
            total: cases.len(),
            passed: 0,
            failed: 0,
            skipped: 0,
        });
        for case in &cases {
            if let Some((fi, si)) = parse_case_key(&case.id) {
                self.explore_case_map.insert(case.id.clone(), (fi, si));
                self.explore_case_status
                    .insert((fi, si), RunStatus::Running);
            }
        }
        let request = RunRequest {
            command: "run".to_string(),
            cases,
            meta: HashMap::new(),
        };
        match runner::spawn_runner(config, request) {
            Ok(rx) => {
                self.runner_rx = Some(rx);
                self.status = "Run started".to_string();
            }
            Err(err) => {
                self.status = format!("Failed to start runner: {err}");
            }
        }
    }

    fn build_explore_cases(&self) -> Vec<RunCase> {
        let mut cases = Vec::new();
        let Some(feature) = self.project.features.get(self.explore_selected_feature) else {
            return cases;
        };
        match self.explore_focus {
            ColumnFocus::Feature => {
                for (si, scenario) in feature.scenarios.iter().enumerate() {
                    cases.push(build_case(
                        self.explore_selected_feature,
                        si,
                        feature,
                        scenario,
                    ));
                }
            }
            ColumnFocus::Scenario | ColumnFocus::Step => {
                if let Some(scenario) = feature.scenarios.get(self.explore_selected_scenario) {
                    cases.push(build_case(
                        self.explore_selected_feature,
                        self.explore_selected_scenario,
                        feature,
                        scenario,
                    ));
                }
            }
        }
        cases
    }

    fn toggle_failure_detail(&mut self) {
        if self.explore_detail_open {
            self.explore_detail_open = false;
            self.explore_detail_case = None;
            return;
        }
        let key = (
            self.explore_selected_feature,
            self.explore_selected_scenario,
        );
        if let Some(detail) = self.explore_case_details.get(&key)
            && detail.status == RunStatus::Failed
        {
            self.explore_detail_open = true;
            self.explore_detail_case = Some(key);
        } else {
            self.status = "No failure details for selection".to_string();
        }
    }

    fn persist_explore_memory(&mut self) {
        self.explore_feature_scenario_memory.insert(
            self.explore_selected_feature,
            self.explore_selected_scenario,
        );
        self.explore_scenario_step_memory.insert(
            (
                self.explore_selected_feature,
                self.explore_selected_scenario,
            ),
            self.explore_selected_step,
        );
    }

    fn restore_explore_memory(&mut self) {
        if let Some(&scenario_idx) = self
            .explore_feature_scenario_memory
            .get(&self.explore_selected_feature)
        {
            self.explore_selected_scenario = scenario_idx;
        } else {
            self.explore_selected_scenario = 0;
        }
        if let Some(&step_idx) = self.explore_scenario_step_memory.get(&(
            self.explore_selected_feature,
            self.explore_selected_scenario,
        )) {
            self.explore_selected_step = step_idx;
        } else {
            self.explore_selected_step = 0;
        }
        self.normalize_explore_selection();
    }

    fn explore_set_feature(&mut self, idx: usize) {
        self.persist_explore_memory();
        self.explore_selected_feature = idx;
        self.restore_explore_memory();
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_set_scenario(&mut self, idx: usize) {
        self.persist_explore_memory();
        self.explore_selected_scenario = idx;
        if let Some(&step_idx) = self.explore_scenario_step_memory.get(&(
            self.explore_selected_feature,
            self.explore_selected_scenario,
        )) {
            self.explore_selected_step = step_idx;
        } else {
            self.explore_selected_step = 0;
        }
        self.normalize_explore_selection();
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_move_selection(&mut self, delta: isize) {
        let clamp_idx = |idx: isize, len: usize| -> usize {
            if len == 0 {
                return 0;
            }
            idx.clamp(0, len as isize - 1) as usize
        };
        match self.explore_focus {
            ColumnFocus::Feature => {
                let len = self.project.features.len();
                let next = clamp_idx(self.explore_selected_feature as isize + delta, len);
                if next != self.explore_selected_feature {
                    self.explore_set_feature(next);
                }
            }
            ColumnFocus::Scenario => {
                let scenarios = self
                    .project
                    .features
                    .get(self.explore_selected_feature)
                    .map(|f| f.scenarios.len())
                    .unwrap_or(0);
                let next = clamp_idx(self.explore_selected_scenario as isize + delta, scenarios);
                if next != self.explore_selected_scenario {
                    self.explore_set_scenario(next);
                }
            }
            ColumnFocus::Step => {
                let steps = self
                    .project
                    .features
                    .get(self.explore_selected_feature)
                    .and_then(|f| f.scenarios.get(self.explore_selected_scenario))
                    .map(|s| s.steps.len())
                    .unwrap_or(0);
                let next = clamp_idx(self.explore_selected_step as isize + delta, steps);
                self.explore_selected_step = next;
                self.persist_explore_memory();
            }
        }
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_move_home(&mut self) {
        match self.explore_focus {
            ColumnFocus::Feature => self.explore_set_feature(0),
            ColumnFocus::Scenario => self.explore_set_scenario(0),
            ColumnFocus::Step => {
                self.explore_selected_step = 0;
                self.persist_explore_memory();
            }
        }
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_move_end(&mut self) {
        match self.explore_focus {
            ColumnFocus::Feature => {
                if !self.project.features.is_empty() {
                    self.explore_set_feature(self.project.features.len() - 1);
                }
            }
            ColumnFocus::Scenario => {
                if let Some(f) = self.project.features.get(self.explore_selected_feature)
                    && !f.scenarios.is_empty()
                {
                    self.explore_set_scenario(f.scenarios.len() - 1);
                }
            }
            ColumnFocus::Step => {
                if let Some(s) = self
                    .project
                    .features
                    .get(self.explore_selected_feature)
                    .and_then(|f| f.scenarios.get(self.explore_selected_scenario))
                    && !s.steps.is_empty()
                {
                    self.explore_selected_step = s.steps.len() - 1;
                    self.persist_explore_memory();
                }
            }
        }
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_focus_next(&mut self) {
        self.explore_focus = match self.explore_focus {
            ColumnFocus::Feature => ColumnFocus::Scenario,
            ColumnFocus::Scenario => ColumnFocus::Step,
            ColumnFocus::Step => ColumnFocus::Step,
        };
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_focus_prev(&mut self) {
        self.explore_focus = match self.explore_focus {
            ColumnFocus::Feature => ColumnFocus::Feature,
            ColumnFocus::Scenario => ColumnFocus::Feature,
            ColumnFocus::Step => ColumnFocus::Scenario,
        };
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_selected_step_line(&self) -> Option<usize> {
        let feature = self.project.features.get(self.explore_selected_feature)?;
        let scenario = feature.scenarios.get(self.explore_selected_scenario)?;
        let step = scenario.steps.get(self.explore_selected_step)?;
        Some(step.line_number)
    }

    fn explore_enter_edit(&mut self) {
        let Some(line) = self.explore_selected_step_line() else {
            self.status = "No step to edit".to_string();
            return;
        };
        if self.active_buffer_idx != Some(self.explore_selected_feature) {
            self.switch_to_buffer(self.explore_selected_feature);
        }
        self.editor_goto_line(line);
        self.clear_step_input_state();
        self.clear_step_keyword_picker();
        self.explore_edit_mode = true;
        self.explore_detail_open = false;
        self.explore_detail_case = None;
        self.status = "Explore edit mode".to_string();
    }

    fn explore_exit_edit(&mut self) {
        self.clear_step_input_state();
        self.clear_step_keyword_picker();
        self.pending_char = None;
        self.explore_edit_mode = false;
        self.status = "Explore mode".to_string();
    }

    // ── Stage transitions ───────────────────────────────────────────

    /// Switch the active editor buffer to the feature file at `idx`.
    fn switch_to_buffer(&mut self, idx: usize) {
        if idx >= self.buffers.len() {
            return;
        }
        // Persist current editor buffer back
        if let Some(cur) = self.active_buffer_idx
            && cur < self.buffers.len()
        {
            self.buffers[cur] = self.buffer.clone();
        }
        self.active_buffer_idx = Some(idx);
        self.buffer = self.buffers[idx].clone();
        self.sync_dirty_flag_with_active_buffer();
        self.file_path = self.project.features.get(idx).map(|f| f.file_path.clone());
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.scroll_row = 0;
        self.focus_slot = BddFocusSlot::Body;
        self.pending_char = None;
        self.scenario_fold.clear();
    }

    /// Scroll the editor to show `line_number` (1-based) centered in view.
    fn editor_goto_line(&mut self, line_1based: usize) {
        let row = line_1based.saturating_sub(1);
        let last = self.buffer.line_count().saturating_sub(1);
        self.cursor_row = row.min(last);
        self.cursor_col = 0;
        self.desired_col = 0;
        self.focus_slot = BddFocusSlot::Body;
    }

    /// Returns the selected node's concrete source location.
    fn selected_node_location(&mut self) -> Option<mindmap::NodeLocation> {
        let id = mindmap::selected_node_id(&self.tree_state)?;
        let locations = self.mindmap_index.locations_for(id)?;
        if locations.is_empty() {
            return None;
        }
        let entry = self
            .mindmap_location_selection
            .entry(id.to_string())
            .or_insert(0);
        if *entry >= locations.len() {
            *entry = 0;
        }
        locations.get(*entry).copied()
    }

    /// Returns `(feature_idx, line_number)` for the currently selected tree node.
    fn selected_tree_location(&mut self) -> Option<(usize, usize)> {
        let loc = self.selected_node_location()?;
        Some((loc.feature_idx, loc.line_number))
    }

    /// Build the stage-2 preview buffer containing only the selected Scenario (or Background).
    fn rebuild_preview(&mut self) {
        let Some(loc) = self.selected_node_location() else {
            self.set_empty_preview();
            return;
        };

        if self.active_buffer_idx != Some(loc.feature_idx) {
            self.switch_to_buffer(loc.feature_idx);
        }

        let Some(feature) = self.project.features.get(loc.feature_idx) else {
            self.set_empty_preview();
            return;
        };

        let buffer = &self.buffer;
        let buffer_lines = buffer.line_count().max(1);

        let (mut start_line, mut end_line, title) = match loc.context {
            mindmap::LocationContext::Scenario(sci) => {
                let Some(scenario) = feature.scenarios.get(sci) else {
                    self.set_empty_preview();
                    return;
                };
                let mut start = scenario.line_number.max(1);
                // Include contiguous @tag lines immediately above the scenario.
                let mut row = start.saturating_sub(1);
                while row > 0 {
                    let prev_row = row - 1;
                    let line = buffer.line(prev_row);
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && trimmed.starts_with('@') {
                        start = prev_row + 1;
                        row = prev_row;
                    } else {
                        break;
                    }
                }

                let mut end = buffer_lines;
                if let Some(next_sc) = feature.scenarios.get(sci + 1) {
                    end = next_sc.line_number.saturating_sub(1).max(1);
                }
                if end < start {
                    end = start;
                }

                let title = match scenario.kind {
                    gherkin::ScenarioKind::Scenario => {
                        format!("Scenario: {}", scenario.name)
                    }
                    gherkin::ScenarioKind::ScenarioOutline => {
                        format!("Scenario Outline: {}", scenario.name)
                    }
                };
                (start, end, title)
            }
            mindmap::LocationContext::Background => {
                let Some(bg) = feature.background.as_ref() else {
                    self.set_empty_preview();
                    return;
                };
                let start = bg.line_number.max(1);
                let mut end = buffer_lines;
                if let Some(first_sc) = feature.scenarios.first() {
                    end = first_sc.line_number.saturating_sub(1).max(1);
                }
                if end < start {
                    end = start;
                }
                (start, end, "Background".to_string())
            }
        };

        if start_line == 0 || end_line == 0 {
            self.set_empty_preview();
            return;
        }

        start_line = start_line.min(buffer_lines);
        end_line = end_line.min(buffer_lines).max(start_line);

        let mut out = String::new();
        for row in (start_line - 1)..=end_line - 1 {
            out.push_str(&buffer.line(row));
            if row < end_line - 1 {
                out.push('\n');
            }
        }

        let rel_cursor = loc
            .line_number
            .saturating_sub(start_line)
            .min(out.lines().count().saturating_sub(1));

        self.preview_buffer = Some(EditorBuffer::from_string(out));
        self.preview_title = title;
        self.preview_cursor_row = rel_cursor;
        self.preview_scroll_row = 0;
    }

    fn set_empty_preview(&mut self) {
        self.preview_buffer = Some(EditorBuffer::from_string(String::new()));
        self.preview_title = "Preview".to_string();
        self.preview_cursor_row = 0;
        self.preview_scroll_row = 0;
    }

    /// Transition back one stage.
    fn stage_back(&mut self) {
        match self.view_stage {
            ViewStage::EditorAndPanel => {
                self.sync_editor_to_project();
                // Sync tree selection to editor cursor
                if let Some(fi) = self.active_buffer_idx {
                    let line_1based = self.cursor_row + 1;
                    if let Some(node_match) =
                        mindmap::find_closest_node(&self.mindmap_index, fi, line_1based)
                        && let Some(path) =
                            mindmap::node_id_to_path(&node_match.node_id, &self.mindmap_index)
                    {
                        self.tree_state.select(path);
                        self.mindmap_location_selection
                            .insert(node_match.node_id, node_match.location_index);
                    }
                }
                self.view_stage = ViewStage::TreeAndEditor;
                self.clear_step_input_state();
                self.clear_step_keyword_picker();
                self.rebuild_preview();
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
        let selected_tree_location = self.selected_tree_location();
        self.rebuild_project_views(selected_tree_location);
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
        self.rebuild_preview();
    }

    fn tree_toggle_or_expand(&mut self) {
        // Tree-only mode: expand tree nodes
        self.tree_state.key_right();
        self.quit_pending_confirm = false;
    }

    fn tree_cycle_location(&mut self, delta: isize) {
        if self.view_stage != ViewStage::TreeAndEditor {
            return;
        }
        let Some(id) = mindmap::selected_node_id(&self.tree_state) else {
            return;
        };
        let Some(locations) = self.mindmap_index.locations_for(id) else {
            return;
        };
        if locations.len() <= 1 {
            return;
        }
        let entry = self
            .mindmap_location_selection
            .entry(id.to_string())
            .or_insert(0);
        let len = locations.len() as isize;
        let mut next = *entry as isize + delta;
        if next < 0 {
            next = len - 1;
        } else if next >= len {
            next = 0;
        }
        *entry = next as usize;

        if let Some((fi, line)) = mindmap::parse_node_line_number(id, &self.mindmap_index, *entry) {
            if self.active_buffer_idx != Some(fi) {
                self.switch_to_buffer(fi);
            }
            self.editor_goto_line(line);
        }
        self.rebuild_preview();
        self.status = "Location switched".to_string();
        self.quit_pending_confirm = false;
    }

    fn tree_collapse(&mut self) {
        self.tree_state.key_left();
        self.quit_pending_confirm = false;
    }

    fn tree_toggle(&mut self) {
        self.tree_state.toggle_selected();
        if self.view_stage == ViewStage::TreeAndEditor {
            self.rebuild_preview();
        }
        self.quit_pending_confirm = false;
    }

    // ── Action handler ──────────────────────────────────────────────

    pub fn handle_action(&mut self, action: Action) -> Result<()> {
        if self.external_change_prompt.is_some() {
            return match action {
                Action::ExternalChangeReload => self.accept_external_reload(),
                Action::ExternalChangeKeepLocal => {
                    self.keep_local_external_version();
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        if self.has_agent_change_prompt() {
            return match action {
                Action::AgentChangeAccept => {
                    let (tool_call_id, result) = self.accept_agent_change()?;
                    self.feed_agent_tool_result(tool_call_id, result);
                    Ok(())
                }
                Action::AgentChangeReject => {
                    let (tool_call_id, result) = self.reject_agent_change();
                    self.feed_agent_tool_result(tool_call_id, result);
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        if !matches!(action, Action::PendingChar(_)) {
            self.pending_char = None;
        }
        match action {
            // Explore tab navigation
            Action::FocusNextColumn => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_focus_next();
                    self.quit_pending_confirm = false;
                }
            }
            Action::FocusPrevColumn => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_focus_prev();
                    self.quit_pending_confirm = false;
                }
            }
            Action::ExploreRight => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    if self.explore_focus == ColumnFocus::Step {
                        self.explore_enter_edit();
                    } else {
                        self.explore_focus_next();
                    }
                    self.quit_pending_confirm = false;
                }
            }
            Action::RunScenario => {
                if self.active_tab == MainTab::Explore {
                    self.start_explore_run();
                    self.quit_pending_confirm = false;
                }
            }
            Action::AiSuggest => {
                if self.active_tab == MainTab::Explore {
                    self.status = "AI suggest: not implemented".to_string();
                    self.quit_pending_confirm = false;
                }
            }
            Action::MindMapSendToAi => {
                if self.active_tab == MainTab::MindMap
                    && let Some(ctx) =
                        crate::mindmap::selected_node_context(&self.tree_state, &self.mindmap_index)
                {
                    let path_str = ctx.path_labels.join(" > ");
                    let msg = format!(
                        "[MindMap] Selected step: \"{}\"\nPath: {}\nAppears in {} location(s)",
                        ctx.step_text, path_str, ctx.location_count
                    );
                    self.ai_messages.push(AiChatMessage {
                        role: AiRole::User,
                        content: msg,
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                        source: Some("MindMap".into()),
                    });
                    self.active_tab = MainTab::Ai;
                    self.ai_status = AiStatus::Waiting;
                    self.ai_partial_response.clear();
                    self.agent_loop_count = 0;
                    self.status = "Sending MindMap context to AI...".to_string();

                    if !crate::llm::LlmConfig::is_configured() {
                        self.ai_messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content: "AI is not configured. Set TESHI_LLM_API_KEY in your environment to enable AI responses.".to_string(),
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: None,
                                source: None,
                            });
                        self.ai_status = AiStatus::Idle;
                        self.ai_partial_response.clear();
                        self.status = "AI not configured".to_string();
                    } else if let Some(ref handle) = self.ai_llm_handle {
                        let messages = self.build_chat_messages_for_llm();
                        let tools = Some(crate::agent::get_tools());
                        if handle
                            .send(crate::llm::LlmRequest::Chat {
                                system: Some(Self::ai_system_prompt().into()),
                                messages,
                                tools,
                            })
                            .is_err()
                        {
                            self.ai_status = AiStatus::Error;
                            self.ai_partial_response.clear();
                            self.status = "AI error: background LLM thread has exited".to_string();
                        }
                    } else {
                        self.ai_status = AiStatus::Error;
                        self.ai_partial_response.clear();
                        self.status = "AI error: LLM handle not available".to_string();
                    }
                }
                self.quit_pending_confirm = false;
            }
            Action::ToggleMindMapAiPanel => {
                self.mindmap_ai_panel_visible = !self.mindmap_ai_panel_visible;
                if !self.mindmap_ai_panel_visible {
                    self.mindmap_focus = MindMapFocus::Main;
                }
                self.quit_pending_confirm = false;
            }
            Action::MindMapFocusAiPanel => {
                if self.active_tab == MainTab::MindMap && self.mindmap_ai_panel_visible {
                    self.mindmap_focus = MindMapFocus::AiPanel;
                }
                self.quit_pending_confirm = false;
            }
            Action::EnterEdit => {
                if self.active_tab == MainTab::Explore {
                    self.explore_enter_edit();
                    self.quit_pending_confirm = false;
                }
            }
            Action::ToggleFailureDetail => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.toggle_failure_detail();
                    self.quit_pending_confirm = false;
                }
            }

            // Tree navigation (MindMap stages 1 & 2)
            Action::TreeUp => self.tree_move_up(),
            Action::TreeDown => self.tree_move_down(),
            Action::TreeExpand => self.tree_toggle_or_expand(),
            Action::TreeCollapse => self.tree_collapse(),
            Action::TreeToggle => self.tree_toggle(),
            Action::TreeOpen => {
                self.status = "MindMap is display-only".to_string();
                self.quit_pending_confirm = false;
            }
            Action::TreeHome => self.tree_home(),
            Action::TreeEnd => self.tree_end(),
            Action::TreeLocationPrev => self.tree_cycle_location(-1),
            Action::TreeLocationNext => self.tree_cycle_location(1),

            // Editor navigation (MindMap stage 3 & legacy)
            Action::MoveUp => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_selection(-1);
                    self.quit_pending_confirm = false;
                } else {
                    self.move_up();
                }
            }
            Action::MoveDown => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_selection(1);
                    self.quit_pending_confirm = false;
                } else {
                    self.move_down();
                }
            }
            Action::MoveLeft => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    // No-op in Explore browse mode
                } else {
                    self.move_left();
                }
            }
            Action::MoveRight => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    // No-op in Explore browse mode
                } else {
                    self.move_right();
                }
            }
            Action::MoveHome => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_home();
                    self.quit_pending_confirm = false;
                } else {
                    self.move_home();
                }
            }
            Action::MoveEnd => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_end();
                    self.quit_pending_confirm = false;
                } else {
                    self.move_end();
                }
            }
            Action::PageUp => self.page_up(),
            Action::PageDown => self.page_down(),
            Action::MoveStepUp => self.move_step_block(false),
            Action::MoveStepDown => self.move_step_block(true),
            Action::SwitchKeyword(keyword) => self.switch_step_keyword(keyword),
            Action::InsertStepBelow => self.insert_step(false),
            Action::InsertStepAbove => self.insert_step(true),
            Action::NewScenario => self.insert_scenario(),
            Action::DeleteNode => self.delete_current_node(),
            Action::CopyStep => self.copy_current_step(),
            Action::PasteStep => self.paste_step(),
            Action::ToggleScenarioFold => self.toggle_current_scenario_fold(),
            Action::FoldAllScenarios => self.fold_all_scenarios(),
            Action::RunBackground => self.run_background(),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::PendingChar(ch) => {
                self.pending_char = Some(ch);
                self.status = match ch {
                    'd' => "`dd` to delete".to_string(),
                    'y' => "`yy` to copy".to_string(),
                    _ => "Pending command".to_string(),
                };
                self.quit_pending_confirm = false;
            }
            Action::Insert(ch) => {
                if !self.step_input_active {
                    return Ok(());
                }
                self.push_undo();
                self.buffer
                    .insert_char(self.cursor_row, self.cursor_col, ch);
                self.cursor_col += 1;
                self.desired_col = self.cursor_col;
                self.mark_current_buffer_dirty();
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
                self.push_undo();
                let (row, col, changed) = self.buffer.backspace(self.cursor_row, self.cursor_col);
                self.cursor_row = row;
                self.cursor_col = col;
                self.desired_col = col;
                if changed {
                    self.mark_current_buffer_dirty();
                    self.quit_pending_confirm = false;
                }
            }
            Action::Delete => {
                if !self.step_input_active {
                    return Ok(());
                }
                self.push_undo();
                if self.buffer.delete(self.cursor_row, self.cursor_col) {
                    self.mark_current_buffer_dirty();
                    self.quit_pending_confirm = false;
                }
            }
            Action::InsertNewline => {
                if !self.step_input_active {
                    return Ok(());
                }
                let row = self.step_input_row;
                let line = self.buffer.line(row);
                if current_step_keyword_index(&line).is_none() {
                    self.status = "New line is available on step lines only".to_string();
                    self.quit_pending_confirm = false;
                    return Ok(());
                }
                let prefix: String = line.chars().take(self.step_input_min_col).collect();
                self.push_undo();
                self.buffer.insert_char(row, self.cursor_col, '\n');
                self.buffer.insert_str(row + 1, 0, &prefix);
                self.cursor_row = row + 1;
                self.cursor_col = prefix.chars().count();
                self.desired_col = self.cursor_col;
                self.step_input_row = self.cursor_row;
                self.step_input_min_col = self.cursor_col;
                self.focus_slot = BddFocusSlot::Body;
                self.mark_current_buffer_dirty();
                self.status = "Inserted new step line".to_string();
                self.quit_pending_confirm = false;
            }
            Action::Save => self.save()?,
            Action::Quit => self.quit(),
            Action::SelectTab(tab) => self.select_tab(tab),
            Action::ActivateStepInput => self.begin_step_or_title_edit()?,
            Action::StepKeywordPickerUp => self.step_keyword_picker_move(-1),
            Action::StepKeywordPickerDown => self.step_keyword_picker_move(1),
            Action::StepKeywordPickerConfirm => self.confirm_step_keyword_picker(),
            Action::StepKeywordPickerCancel => {
                self.clear_step_keyword_picker();
                self.status = "Step keyword selection canceled".to_string();
                self.quit_pending_confirm = false;
            }
            Action::AiSendChar(ch) => {
                self.ai_input.push(ch);
                self.quit_pending_confirm = false;
            }
            Action::AiSendMessage => {
                if self.ai_input.trim().is_empty() || self.ai_status == AiStatus::Waiting {
                    return Ok(());
                }
                let user_msg = std::mem::take(&mut self.ai_input);
                self.ai_messages.push(AiChatMessage {
                    role: AiRole::User,
                    content: user_msg.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                    source: None,
                });
                self.ai_status = AiStatus::Waiting;
                self.ai_partial_response.clear();
                self.agent_loop_count = 0;
                self.status = "Sending message to AI...".to_string();

                // If the LLM is not configured, add a mock response
                if !crate::llm::LlmConfig::is_configured() {
                    self.ai_messages.push(AiChatMessage {
                        role: AiRole::Assistant,
                        content: "AI is not configured. Set TESHI_LLM_API_KEY in your environment to enable AI responses.".to_string(),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                        source: None,
                    });
                    self.ai_status = AiStatus::Idle;
                    self.ai_partial_response.clear();
                    self.status = "AI not configured".to_string();
                } else if let Some(ref handle) = self.ai_llm_handle {
                    use crate::llm::LlmRequest;
                    let messages = self.build_chat_messages_for_llm();
                    let tools = Some(crate::agent::get_tools());
                    if handle
                        .send(LlmRequest::Chat {
                            system: Some(Self::ai_system_prompt().into()),
                            messages,
                            tools,
                        })
                        .is_err()
                    {
                        self.ai_status = AiStatus::Error;
                        self.ai_partial_response.clear();
                        self.status = "AI error: background LLM thread has exited".to_string();
                    }
                } else {
                    // LLM is configured but the handle is None — shouldn't happen normally.
                    self.ai_status = AiStatus::Error;
                    self.ai_partial_response.clear();
                    self.status = "AI error: LLM handle not available".to_string();
                }
                self.quit_pending_confirm = false;
            }
            Action::AiBackspace => {
                self.ai_input.pop();
                self.quit_pending_confirm = false;
            }
            Action::ExternalChangeReload | Action::ExternalChangeKeepLocal => {}
            // Handled in early-return guard above; unreachable here.
            Action::AgentChangeAccept | Action::AgentChangeReject => {}
            Action::ClearInputState => {
                if self.active_tab == MainTab::MindMap
                    && self.mindmap_focus == MindMapFocus::AiPanel
                {
                    self.mindmap_focus = MindMapFocus::Main;
                    self.status = "Focus returned to tree".to_string();
                } else if self.active_tab == MainTab::MindMap
                    && self.mindmap_ai_panel_visible
                    && self.mindmap_focus == MindMapFocus::Main
                {
                    self.mindmap_ai_panel_visible = false;
                    self.status = "AI preview panel closed".to_string();
                } else if self.active_tab == MainTab::Ai {
                    self.ai_input.clear();
                    self.ai_partial_response.clear();
                    self.ai_status = AiStatus::Idle;
                    self.status = "Input cleared".to_string();
                } else {
                    if self.step_input_active || self.step_keyword_picker.is_some() {
                        self.clear_step_input_state();
                        self.clear_step_keyword_picker();
                        self.status = "Input state cleared".to_string();
                    } else if self.explore_detail_open {
                        self.explore_detail_open = false;
                        self.explore_detail_case = None;
                    } else if self.active_tab == MainTab::Explore && self.explore_edit_mode {
                        self.explore_exit_edit();
                    } else if self.view_stage != ViewStage::TreeOnly {
                        self.stage_back();
                    }
                }
                self.quit_pending_confirm = false;
            }
        }
        self.clamp_cursor();
        Ok(())
    }

    fn save(&mut self) -> Result<()> {
        if let Some(path) = self.file_path.clone() {
            fs::write(&path, self.buffer.as_string())
                .with_context(|| format!("failed to write {}", path.display()))?;
            self.status = format!("Saved {}", path.display());
            if let Some(idx) = self.active_buffer_idx {
                self.set_buffer_dirty(idx, false);
                if let Some(slot) = self.disk_stamps.get_mut(idx) {
                    *slot = FileStamp::capture(&path);
                }
            } else {
                self.dirty = false;
            }
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
        self.pending_char = None;
        if self.active_tab == MainTab::Explore {
            self.explore_edit_mode = false;
            self.explore_detail_open = false;
            self.explore_detail_case = None;
        }
        self.quit_pending_confirm = false;
        self.active_tab = tab;
        if self.active_tab == MainTab::MindMap {
            self.view_stage = ViewStage::TreeOnly;
            self.mindmap_focus = MindMapFocus::Main;
        }
        self.status = match tab {
            MainTab::MindMap => "Switched to MindMap tab",
            MainTab::Explore => "Switched to Explore tab",
            MainTab::Help => "Switched to Help tab",
            MainTab::Ai => "Switched to AI tab",
        }
        .to_string();
    }

    fn clear_step_input_state(&mut self) {
        self.step_input_active = false;
    }

    fn clear_step_keyword_picker(&mut self) {
        self.step_keyword_picker = None;
    }

    fn push_undo(&mut self) {
        self.undo_stack
            .push((self.buffer.clone(), self.cursor_row, self.cursor_col));
        self.redo_stack.clear();
    }

    fn restore_snapshot(&mut self, snapshot: (EditorBuffer, usize, usize)) {
        self.buffer = snapshot.0;
        self.cursor_row = snapshot.1;
        self.cursor_col = snapshot.2;
        self.desired_col = self.cursor_col;
        self.clear_step_input_state();
        self.clear_step_keyword_picker();
        self.pending_char = None;
        self.scenario_fold.clear();
    }

    fn hidden_editor_rows(&self) -> HashSet<usize> {
        self.scenario_fold
            .iter()
            .flat_map(|&scenario_row| scenario_content_rows(&self.buffer, scenario_row))
            .collect()
    }

    pub fn visible_editor_rows(&self) -> Vec<usize> {
        let hidden = self.hidden_editor_rows();
        let last_row = self.buffer.line_count().saturating_sub(1);
        let mut rows: Vec<usize> = (0..self.buffer.line_count())
            .filter(|row| !hidden.contains(row))
            .filter(|&row| !(row == last_row && self.buffer.line(row).is_empty()))
            .collect();
        if rows.is_empty() {
            rows.push(0);
        }
        rows
    }

    pub fn folded_step_count(&self, scenario_row: usize) -> Option<usize> {
        self.scenario_fold
            .contains(&scenario_row)
            .then(|| scenario_step_rows(&self.buffer, scenario_row).len())
    }

    fn clear_structural_state(&mut self) {
        self.pending_char = None;
        self.scenario_fold.clear();
    }

    fn current_editor_scenario(&self) -> Option<(usize, usize)> {
        let feature_idx = self.active_buffer_idx?;
        let feature = self.project.features.get(feature_idx)?;
        let line_number = self.cursor_row + 1;
        let mut selected = None;
        for (scenario_idx, scenario) in feature.scenarios.iter().enumerate() {
            if scenario.line_number <= line_number {
                selected = Some(scenario_idx);
            } else {
                break;
            }
        }
        selected.map(|scenario_idx| (feature_idx, scenario_idx))
    }

    fn begin_step_or_title_edit(&mut self) -> Result<()> {
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
                    self.status = "Step keyword list is available on step lines only".to_string();
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
        self.pending_char = None;
        self.quit_pending_confirm = false;
        Ok(())
    }

    fn switch_step_keyword(&mut self, keyword: &'static str) {
        let line = self.buffer.line(self.cursor_row);
        if let Some(new_line) = replace_step_keyword_line(&line, keyword) {
            self.push_undo();
            self.buffer.replace_line(self.cursor_row, &new_line);
            self.focus_slot = BddFocusSlot::Keyword;
            self.mark_current_buffer_dirty();
            self.pending_char = None;
            self.quit_pending_confirm = false;
            self.status = format!("Step keyword set to {keyword}");
        } else {
            self.status = "Step keyword shortcuts work on step lines only".to_string();
        }
    }

    fn insert_step(&mut self, above: bool) {
        if !self.is_editor_active() {
            self.status = "Enter editor mode first".to_string();
            return;
        }
        self.push_undo();
        let inserted_row = if above {
            insert_step_above(&mut self.buffer, self.cursor_row)
        } else {
            insert_step_below(&mut self.buffer, self.cursor_row)
        };
        let Some(row) = inserted_row else {
            let _ = self.undo_stack.pop();
            self.status = "No scenario selected for step insertion".to_string();
            return;
        };
        self.cursor_row = row;
        self.focus_slot = BddFocusSlot::Body;
        self.mark_current_buffer_dirty();
        self.clear_structural_state();
        let _ = self.begin_step_or_title_edit();
        self.status = if above {
            "Inserted step above".to_string()
        } else {
            "Inserted step below".to_string()
        };
    }

    fn insert_scenario(&mut self) {
        if !self.is_editor_active() {
            self.status = "Enter editor mode first".to_string();
            return;
        }
        self.push_undo();
        let Some(row) = insert_scenario_after_current(&mut self.buffer, self.cursor_row) else {
            let _ = self.undo_stack.pop();
            self.status = "No scenario selected".to_string();
            return;
        };
        self.cursor_row = row;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.focus_slot = BddFocusSlot::Body;
        self.mark_current_buffer_dirty();
        self.clear_structural_state();
        let _ = self.begin_step_or_title_edit();
        self.status = "Inserted scenario".to_string();
    }

    fn delete_current_node(&mut self) {
        if !self.is_editor_active() {
            self.status = "Enter editor mode first".to_string();
            return;
        }
        let line = self.buffer.line(self.cursor_row);
        self.push_undo();
        let target_row = if scenario_header_for_row(&self.buffer, self.cursor_row)
            == Some(self.cursor_row)
            && line.trim_start().starts_with("Scenario")
        {
            delete_scenario_block(&mut self.buffer, self.cursor_row)
        } else {
            delete_step(&mut self.buffer, self.cursor_row)
        };
        let Some(row) = target_row else {
            let _ = self.undo_stack.pop();
            self.status = "Delete works on steps or scenario headers".to_string();
            return;
        };
        self.cursor_row = row;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.focus_slot = BddFocusSlot::Body;
        self.mark_current_buffer_dirty();
        self.clear_structural_state();
        self.status = "Deleted node".to_string();
    }

    fn copy_current_step(&mut self) {
        let Some(lines) = crate::bdd_nav::step_block_lines(&self.buffer, self.cursor_row) else {
            self.status = "Copy works on steps only".to_string();
            return;
        };
        self.clipboard = Some(lines.join("\n"));
        self.pending_char = None;
        self.quit_pending_confirm = false;
        self.status = "Step copied".to_string();
    }

    fn paste_step(&mut self) {
        if !self.is_editor_active() {
            self.status = "Enter editor mode first".to_string();
            return;
        }
        let Some(clipboard) = self.clipboard.clone() else {
            self.status = "Clipboard is empty".to_string();
            return;
        };
        let scenario_row = scenario_header_for_row(&self.buffer, self.cursor_row);
        let Some(scenario_row) = scenario_row else {
            self.status = "Paste works inside a scenario".to_string();
            return;
        };
        let block_lines = clipboard
            .lines()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let insert_at = if crate::bdd_nav::step_block_lines(&self.buffer, self.cursor_row).is_some()
        {
            let step_rows = scenario_step_rows(&self.buffer, scenario_row);
            if step_rows.contains(&self.cursor_row) {
                let mut end_row = self.cursor_row + 1;
                while end_row < self.buffer.line_count() {
                    let line = self.buffer.line(end_row);
                    let trimmed = line.trim_start();
                    if crate::bdd_nav::step_edit_start_col(&line).is_some()
                        || trimmed.starts_with("Scenario:")
                        || trimmed.starts_with("Scenario Outline:")
                        || trimmed.starts_with("Background:")
                        || trimmed.starts_with("Feature:")
                    {
                        break;
                    }
                    end_row += 1;
                }
                end_row
            } else {
                scenario_row + 1
            }
        } else {
            scenario_row + 1
        };
        self.push_undo();
        let (mut lines, trailing_newline) = {
            let text = self.buffer.as_string();
            let trailing_newline = text.ends_with('\n');
            let mut lines = (0..self.buffer.line_count())
                .map(|row| self.buffer.line(row))
                .collect::<Vec<_>>();
            if trailing_newline && lines.last().is_some_and(|line| line.is_empty()) {
                lines.pop();
            }
            (lines, trailing_newline)
        };
        let insert_at = insert_at.min(lines.len());
        lines.splice(insert_at..insert_at, block_lines.clone());
        let mut text = if lines.is_empty() {
            String::new()
        } else {
            lines.join("\n")
        };
        if trailing_newline {
            text.push('\n');
        }
        self.buffer = EditorBuffer::from_string(text);
        self.cursor_row = insert_at;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.focus_slot = BddFocusSlot::Body;
        self.mark_current_buffer_dirty();
        self.clear_structural_state();
        self.status = "Step pasted".to_string();
    }

    fn move_step_block(&mut self, down: bool) {
        if !self.is_editor_active() {
            self.status = "Enter editor mode first".to_string();
            return;
        }
        self.push_undo();
        let moved_to = if down {
            swap_step_with_next(&mut self.buffer, self.cursor_row)
        } else {
            swap_step_with_prev(&mut self.buffer, self.cursor_row)
        };
        let Some(row) = moved_to else {
            let _ = self.undo_stack.pop();
            self.status = if down {
                "Step cannot move further down".to_string()
            } else {
                "Step cannot move further up".to_string()
            };
            return;
        };
        self.cursor_row = row;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.focus_slot = BddFocusSlot::Body;
        self.mark_current_buffer_dirty();
        self.clear_structural_state();
        self.status = if down {
            "Moved step down".to_string()
        } else {
            "Moved step up".to_string()
        };
    }

    fn toggle_current_scenario_fold(&mut self) {
        let Some(scenario_row) = scenario_header_for_row(&self.buffer, self.cursor_row) else {
            self.status = "Fold works inside a scenario".to_string();
            return;
        };
        if self.scenario_fold.insert(scenario_row) {
            self.status = "Scenario folded".to_string();
        } else {
            self.scenario_fold.remove(&scenario_row);
            self.status = "Scenario expanded".to_string();
        }
        if self.hidden_editor_rows().contains(&self.cursor_row) {
            self.cursor_row = scenario_row;
            self.cursor_col = 0;
            self.desired_col = 0;
            self.focus_slot = BddFocusSlot::Body;
        }
        self.pending_char = None;
        self.quit_pending_confirm = false;
    }

    fn fold_all_scenarios(&mut self) {
        self.scenario_fold = (0..self.buffer.line_count())
            .filter(|&row| {
                let line = self.buffer.line(row);
                let trimmed = line.trim_start();
                trimmed.starts_with("Scenario:") || trimmed.starts_with("Scenario Outline:")
            })
            .collect();
        if self.hidden_editor_rows().contains(&self.cursor_row)
            && let Some(scenario_row) = scenario_header_for_row(&self.buffer, self.cursor_row)
        {
            self.cursor_row = scenario_row;
            self.cursor_col = 0;
            self.desired_col = 0;
            self.focus_slot = BddFocusSlot::Body;
        }
        self.pending_char = None;
        self.quit_pending_confirm = false;
        self.status = "All scenarios folded".to_string();
    }

    fn run_background(&mut self) {
        if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
            self.start_explore_run();
            return;
        }
        let Some((feature_idx, scenario_idx)) = self.current_editor_scenario() else {
            self.status = "No scenario selected to run".to_string();
            return;
        };
        self.explore_selected_feature = feature_idx;
        self.explore_selected_scenario = scenario_idx;
        self.explore_selected_step = 0;
        self.start_explore_run();
        self.status = "Background run started".to_string();
    }

    fn undo(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            self.status = "Nothing to undo".to_string();
            return;
        };
        self.redo_stack
            .push((self.buffer.clone(), self.cursor_row, self.cursor_col));
        self.restore_snapshot(snapshot);
        self.mark_current_buffer_dirty();
        self.status = "Undo".to_string();
        self.quit_pending_confirm = false;
    }

    fn redo(&mut self) {
        let Some(snapshot) = self.redo_stack.pop() else {
            self.status = "Nothing to redo".to_string();
            return;
        };
        self.undo_stack
            .push((self.buffer.clone(), self.cursor_row, self.cursor_col));
        self.restore_snapshot(snapshot);
        self.mark_current_buffer_dirty();
        self.status = "Redo".to_string();
        self.quit_pending_confirm = false;
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
            self.push_undo();
            self.buffer.replace_line(picker.buffer_row, &new_line);
            self.cursor_row = picker.buffer_row;
            self.cursor_col = 0;
            self.desired_col = 0;
            self.focus_slot = BddFocusSlot::Body;
            self.mark_current_buffer_dirty();
            self.status = "Step keyword updated".to_string();
        }
        self.step_keyword_picker = None;
        self.pending_char = None;
        self.quit_pending_confirm = false;
    }

    /// Returns `true` when the editor panel is active and accepts editing operations.
    pub fn is_editor_active(&self) -> bool {
        (self.active_tab == MainTab::MindMap && self.view_stage == ViewStage::EditorAndPanel)
            || (self.active_tab == MainTab::Explore && self.explore_edit_mode)
    }

    pub fn is_editor_nav_mode(&self) -> bool {
        self.is_editor_active() && !self.step_input_active && self.step_keyword_picker.is_none()
    }

    fn toggle_focus_slot_horizontal(&mut self) {
        self.focus_slot = BddFocusSlot::Body;
    }

    fn vertical_nav_rows(&self) -> (Vec<usize>, bool) {
        let body_chain_nav = self.focus_slot == BddFocusSlot::Body;
        let hidden = self.hidden_editor_rows();
        let rows = bdd_step_rows(&self.buffer)
            .into_iter()
            .filter(|row| !hidden.contains(row))
            .collect();
        (rows, body_chain_nav)
    }

    fn apply_vertical_nav_jump(&mut self, new_row: usize, body_chain_nav: bool) {
        self.cursor_row = new_row;
        self.cursor_col = 0;
        self.desired_col = 0;
        if body_chain_nav {
            return;
        }
        self.focus_slot = BddFocusSlot::Body;
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
            if self.active_tab == MainTab::Explore && self.explore_edit_mode {
                self.explore_exit_edit();
                self.quit_pending_confirm = false;
                return;
            }
            if self.active_tab == MainTab::MindMap && self.view_stage == ViewStage::EditorAndPanel {
                self.stage_back();
                return;
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

fn build_case(
    feature_idx: usize,
    scenario_idx: usize,
    feature: &gherkin::BddFeature,
    scenario: &gherkin::BddScenario,
) -> RunCase {
    RunCase {
        id: format!("f{feature_idx}:s{scenario_idx}"),
        feature_path: feature.file_path.to_string_lossy().to_string(),
        scenario: scenario.name.clone(),
        line_number: Some(scenario.line_number),
    }
}

fn parse_case_key(id: &str) -> Option<(usize, usize)> {
    let mut parts = id.split(':');
    let f = parts.next()?;
    let s = parts.next()?;
    let f_idx = f.strip_prefix('f')?.parse::<usize>().ok()?;
    let s_idx = s.strip_prefix('s')?.parse::<usize>().ok()?;
    Some((f_idx, s_idx))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use super::{
        AiStatus, App, BddFocusSlot, ColumnFocus, MainTab, MindMapFocus, ViewStage,
        current_step_keyword_index, replace_step_keyword_line,
    };
    use crate::bdd_nav::step_edit_start_col;
    use crate::editor_buffer::EditorBuffer;
    use crate::keymap::Action;

    /// Helper: create an app pre-set to editor-active mode (stage 3) for existing editor tests.
    fn editor_test_app() -> App {
        let mut app = App::from_args().expect("app init should work");
        app.active_tab = MainTab::MindMap;
        app.view_stage = ViewStage::EditorAndPanel;
        app
    }

    fn temp_feature_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "teshi-{name}-{}-{unique}.feature",
            std::process::id()
        ))
    }

    fn feature_file_app(name: &str, content: &str) -> (App, PathBuf) {
        let path = temp_feature_path(name);
        fs::write(&path, content).expect("feature fixture should be written");
        let app = App::from_file(&path).expect("app should open fixture file");
        (app, path)
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
    fn test_tab_inserts_new_step_line() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        app.handle_action(Action::InsertNewline)
            .expect("insert newline should work");
        assert!(app.step_input_active);
        assert_eq!(app.buffer.line(0), "Given hello");
        assert_eq!(app.buffer.line(1), "Given ");
        assert_eq!(app.cursor_row, 1);
        assert_eq!(app.cursor_col, 6);
    }

    #[test]
    fn test_tab_splits_step_line_and_carries_suffix() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello world".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        app.cursor_col = 11;
        app.desired_col = 11;
        app.handle_action(Action::InsertNewline)
            .expect("insert newline should work");
        assert_eq!(app.buffer.line(0), "Given hello");
        assert_eq!(app.buffer.line(1), "Given  world");
        assert_eq!(app.cursor_row, 1);
        assert_eq!(app.cursor_col, 6);
    }

    #[test]
    fn test_space_on_feature_keyword_does_not_open_step_picker() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Feature: X\n".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        assert!(app.step_keyword_picker.is_none());
        assert!(app.step_input_active);
        assert_eq!(app.step_input_min_col, 9);
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
        assert_eq!(app.cursor_row, 0);
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        assert!(!crate::bdd_nav::is_feature_narrative_row(&app.buffer, 0));
        app.handle_action(Action::ActivateStepInput)
            .expect("edit should work");
        assert!(app.step_input_active);
        assert_eq!(app.step_input_min_col, 9);
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
    fn test_explore_focus_clamps_at_edges() {
        let mut app = App::from_args().expect("app init should work");
        app.active_tab = MainTab::Explore;

        app.explore_focus = ColumnFocus::Feature;
        app.explore_focus_prev();
        assert_eq!(app.explore_focus, ColumnFocus::Feature);

        app.explore_focus = ColumnFocus::Step;
        app.explore_focus_next();
        assert_eq!(app.explore_focus, ColumnFocus::Step);
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
        assert_eq!(app.cursor_row, 1);
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        assert_eq!(app.cursor_row, 1);
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
    }

    #[test]
    fn test_nav_move_down_skips_non_step_rows() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n  Given a\n  Scenario: T\n  When b\n".to_string(),
        );
        app.sync_cursor_to_first_node();
        assert_eq!(app.cursor_row, 2);
        app.handle_action(Action::MoveDown)
            .expect("step move should work");
        assert_eq!(app.cursor_row, 4);
        assert!(app.buffer.line(4).trim_start().starts_with("When"));
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
    }

    #[test]
    fn test_nav_sync_starts_at_first_step() {
        let mut app = editor_test_app();
        app.buffer =
            EditorBuffer::from_string("Feature: A\n  Scenario: S\n  Given a\n".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.cursor_row, 2);
        assert!(app.buffer.line(2).trim_start().starts_with("Given"));
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
    }

    #[test]
    fn test_nav_move_up_stays_on_step_rows() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Feature: F\nScenario: S\n  When x\n".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.cursor_row, 2);
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveUp)
            .expect("step move should work");
        assert_eq!(app.cursor_row, 2);
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
    }

    #[test]
    fn test_nav_left_right_keeps_body_focus() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("  When hello".to_string());
        app.sync_cursor_to_first_node();
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveRight)
            .expect("right should work");
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
        app.handle_action(Action::MoveLeft)
            .expect("left should work");
        assert_eq!(app.focus_slot, BddFocusSlot::Body);
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
    fn test_tree_open_is_noop_in_display_only_mode() {
        let mut app = App::from_args().expect("app init should work");
        assert_eq!(app.view_stage, ViewStage::TreeOnly);

        app.handle_action(Action::TreeOpen)
            .expect("tree open should be ignored");
        assert_eq!(app.view_stage, ViewStage::TreeOnly);
    }

    #[test]
    fn test_tree_expand_does_not_enter_editor() {
        let mut app = App::from_args().expect("app init should work");
        app.handle_action(Action::TreeExpand)
            .expect("expand should work");
        assert_eq!(app.view_stage, ViewStage::TreeOnly);
    }

    #[test]
    fn test_explore_right_enters_and_left_exits_edit() {
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
        let mindmap_index = crate::mindmap::build_index(&project);
        let tree_state = crate::mindmap::init_tree_state(&mindmap_index);

        let mut app = App {
            project,
            step_index,
            mindmap_index,
            mindmap_location_selection: HashMap::new(),
            buffers,
            buffer_dirty: vec![false],
            disk_stamps: vec![None],
            active_buffer_idx: Some(0),
            view_stage: ViewStage::TreeOnly,
            tree_state,
            buffer: EditorBuffer::from_string(content.to_string()),
            file_path: Some(PathBuf::from("test.feature")),
            cursor_row: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            focus_slot: BddFocusSlot::Keyword,
            preview_buffer: None,
            preview_title: String::new(),
            preview_cursor_row: 0,
            preview_scroll_row: 0,
            should_quit: false,
            active_tab: MainTab::Explore,
            dirty: false,
            status: String::new(),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            pending_char: None,
            clipboard: None,
            scenario_fold: HashSet::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: None,
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            explore_focus: ColumnFocus::Step,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_case_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: true,
            ai_messages: Vec::new(),
            ai_input: String::new(),
            ai_status: AiStatus::Idle,
            ai_partial_response: String::new(),
            ai_llm_handle: None,
            ai_llm_rx: None,
            ai_tool_status: None,
            agent_loop_count: 0,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
        };

        app.handle_action(Action::ExploreRight)
            .expect("right should enter edit");
        assert!(app.explore_edit_mode);

        app.handle_action(Action::MoveLeft)
            .expect("left on keyword should exit edit");
        assert!(!app.explore_edit_mode);
    }

    #[test]
    fn test_explore_memory_restores_scenario_and_step() {
        use crate::gherkin;
        use std::path::PathBuf;

        let fa = gherkin::parse_feature(
            "\
Feature: A
  Scenario: S1
    Given a1
    When a2
    Then a3
  Scenario: S2
    Given b1
    When b2
    Then b3
",
            PathBuf::from("a.feature"),
        );
        let fb = gherkin::parse_feature(
            "\
Feature: B
  Scenario: T1
    Given c1
",
            PathBuf::from("b.feature"),
        );
        let project = crate::gherkin::BddProject {
            root_dir: PathBuf::from("."),
            features: vec![fa, fb],
        };
        let step_index = crate::step_index::StepIndex::build(&project);
        let mindmap_index = crate::mindmap::build_index(&project);
        let tree_state = crate::mindmap::init_tree_state(&mindmap_index);

        let mut app = App {
            project,
            step_index,
            mindmap_index,
            mindmap_location_selection: HashMap::new(),
            buffers: Vec::new(),
            buffer_dirty: Vec::new(),
            disk_stamps: vec![None, None],
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
            preview_buffer: None,
            preview_title: String::new(),
            preview_cursor_row: 0,
            preview_scroll_row: 0,
            should_quit: false,
            active_tab: MainTab::Explore,
            dirty: false,
            status: String::new(),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            pending_char: None,
            clipboard: None,
            scenario_fold: HashSet::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: None,
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_case_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: true,
            ai_messages: Vec::new(),
            ai_input: String::new(),
            ai_status: AiStatus::Idle,
            ai_partial_response: String::new(),
            ai_llm_handle: None,
            ai_llm_rx: None,
            ai_tool_status: None,
            agent_loop_count: 0,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
        };

        app.explore_selected_feature = 0;
        app.explore_selected_scenario = 1;
        app.explore_selected_step = 2;
        app.persist_explore_memory();

        app.explore_selected_feature = 1;
        app.explore_selected_scenario = 0;
        app.explore_selected_step = 0;
        app.persist_explore_memory();

        app.explore_set_feature(0);
        assert_eq!(app.explore_selected_scenario, 1);
        assert_eq!(app.explore_selected_step, 2);
    }

    #[test]
    fn test_undo_and_redo_restore_buffer_state() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string("Given hello".to_string());
        app.sync_cursor_to_first_node();
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        app.handle_action(Action::Insert('!'))
            .expect("insert should work");
        assert_eq!(app.buffer.line(0), "Given hello!");

        app.handle_action(Action::Undo).expect("undo should work");
        assert_eq!(app.buffer.line(0), "Given hello");

        app.handle_action(Action::Redo).expect("redo should work");
        assert_eq!(app.buffer.line(0), "Given hello!");
    }

    #[test]
    fn test_pending_delete_sequence_deletes_current_step() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n    Then two\n".to_string(),
        );
        app.cursor_row = 2;
        app.focus_slot = BddFocusSlot::Keyword;

        app.handle_action(Action::PendingChar('d'))
            .expect("first d should work");
        assert_eq!(app.pending_char, Some('d'));

        app.handle_action(Action::DeleteNode)
            .expect("second d should delete");
        assert_eq!(app.buffer.line(2), "    Then two");
        assert!(app.pending_char.is_none());
    }

    #[test]
    fn test_copy_and_paste_step_duplicate_block() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n      | a |\n    Then two\n".to_string(),
        );
        app.cursor_row = 2;
        app.focus_slot = BddFocusSlot::Keyword;

        app.handle_action(Action::CopyStep)
            .expect("copy should work");
        app.handle_action(Action::MoveDown)
            .expect("move should work");
        app.handle_action(Action::PasteStep)
            .expect("paste should work");

        assert_eq!(app.buffer.line(4), "    Then two");
        assert_eq!(app.buffer.line(5), "    Given one");
        assert_eq!(app.buffer.line(6), "      | a |");
    }

    #[test]
    fn test_toggle_fold_hides_scenario_rows_from_visible_editor_rows() {
        let mut app = editor_test_app();
        app.buffer = EditorBuffer::from_string(
            "Feature: A\n  Scenario: S\n    Given one\n    Then two\n  Scenario: T\n    When next\n"
                .to_string(),
        );
        app.cursor_row = 2;
        app.focus_slot = BddFocusSlot::Keyword;

        app.handle_action(Action::ToggleScenarioFold)
            .expect("fold should work");

        assert_eq!(app.cursor_row, 1);
        assert_eq!(app.folded_step_count(1), Some(2));
        assert_eq!(app.visible_editor_rows(), vec![0, 1, 4, 5]);
    }

    #[test]
    fn test_external_change_clean_buffer_auto_reloads() {
        let original = "Feature: T\n  Scenario: S\n    Given one\n";
        let updated = "Feature: T\n  Scenario: S\n    Given updated step text\n";
        let (mut app, path) = feature_file_app("external-clean", original);

        fs::write(&path, updated).expect("updated feature should be written");
        app.last_external_check = Instant::now() - Duration::from_secs(1);
        app.poll_external_feature_changes();

        assert_eq!(app.buffer.as_string(), updated);
        assert_eq!(
            app.project.features[0].scenarios[0].steps[0].text,
            "updated step text"
        );
        assert!(app.external_change_prompt.is_none());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_external_change_dirty_buffer_prompts_without_overwrite() {
        let original = "Feature: T\n  Scenario: S\n    Given one\n";
        let updated = "Feature: T\n  Scenario: S\n    Given disk version\n";
        let (mut app, path) = feature_file_app("external-dirty", original);

        app.buffer
            .replace_line(2, "    Given local unsaved version");
        app.mark_current_buffer_dirty();
        fs::write(&path, updated).expect("updated feature should be written");

        app.last_external_check = Instant::now() - Duration::from_secs(1);
        app.poll_external_feature_changes();

        assert_eq!(app.buffer.line(2), "    Given local unsaved version");
        assert!(app.external_change_prompt.is_some());
        assert!(app.dirty);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_external_change_reload_choice_updates_project_and_mindmap() {
        let original = "Feature: T\n  Scenario: S\n    Given one\n";
        let updated = "Feature: T\n  Scenario: S\n    Given disk version\n    Then synced change\n";
        let (mut app, path) = feature_file_app("external-reload-choice", original);

        app.buffer
            .replace_line(2, "    Given local unsaved version");
        app.mark_current_buffer_dirty();
        fs::write(&path, updated).expect("updated feature should be written");

        app.last_external_check = Instant::now() - Duration::from_secs(1);
        app.poll_external_feature_changes();
        assert!(app.external_change_prompt.is_some());

        app.handle_action(Action::ExternalChangeReload)
            .expect("reload choice should succeed");

        assert_eq!(app.buffer.as_string(), updated);
        assert_eq!(app.project.features[0].scenarios[0].steps.len(), 2);
        assert_eq!(
            app.project.features[0].scenarios[0].steps[0].text,
            "disk version"
        );
        assert!(
            crate::mindmap::find_closest_node(&app.mindmap_index, 0, 3).is_some(),
            "mind map index should rebuild after reloading from disk"
        );
        assert!(app.external_change_prompt.is_none());
        assert!(!app.dirty);

        let _ = fs::remove_file(path);
    }
}
