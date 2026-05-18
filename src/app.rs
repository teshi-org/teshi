use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::bdd_nav::{
    bdd_step_rows, current_step_keyword_index, delete_scenario_block, delete_step,
    insert_scenario_after_current, insert_step_above, insert_step_below,
    line_body_edit_min_col_in_buffer, next_node_row, prev_node_row, replace_step_keyword_line,
    scenario_content_rows, scenario_header_for_row, scenario_step_rows, swap_step_with_next,
    swap_step_with_prev,
};
use crate::config::AppConfig;
use crate::editor_buffer::EditorBuffer;

pub use crate::diff::{ChangeKind, DiffLine};
use crate::gherkin::{self, BddProject};
use crate::gherkin_lang::StructuralType;
use crate::keymap::Action;
use crate::mindmap;
use crate::runner::{self, RunCase, RunEvent, RunRequest, RunnerConfig};
use crate::step_index::StepIndex;
use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

/// Available slash commands: (name, description)
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("new", "Start a new session"),
    ("exit", "Exit Teshi"),
    ("resume", "Resume the most recent session"),
    ("copy", "Copy last N assistant responses to clipboard"),
    ("models", "Open model settings"),
    ("sessions", "Browse saved sessions"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    MindMap,
    Explore,
    Ai,
}

/// A single message in the AI chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    AwaitingApproval,
    Error,
}

/// A single independent agent conversation with its own state and LLM connection.
#[derive(Debug)]
pub struct AgentThread {
    #[allow(dead_code)]
    pub id: usize,
    pub title: String,
    pub status: AiStatus,
    pub messages: Vec<AiChatMessage>,
    pub input: String,
    pub input_cursor: usize,
    pub partial_response: String,
    pub tool_status: Option<String>,
    pub scroll_offset: usize,
    pub horizontal_scroll: usize,
    pub llm_handle: Option<crate::llm::LlmHandle>,
    pub llm_rx: Option<std::sync::mpsc::Receiver<crate::llm::LlmEvent>>,
    agent_loop_count: u32,
    /// Cumulative input tokens across all requests for this agent.
    pub total_input_tokens: u64,
    /// Cumulative output tokens across all requests for this agent.
    pub total_output_tokens: u64,
    /// Last reported input token count (used for context trimming).
    pub last_input_tokens: Option<u32>,
}

impl AgentThread {
    pub fn new(id: usize, title: &str) -> Self {
        Self {
            id,
            title: title.to_string(),
            status: AiStatus::Idle,
            messages: Vec::new(),
            input: String::new(),
            input_cursor: 0,
            partial_response: String::new(),
            tool_status: None,
            scroll_offset: 0,
            horizontal_scroll: 0,
            llm_handle: None,
            llm_rx: None,
            agent_loop_count: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            last_input_tokens: None,
        }
    }
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

/// Mode of the model profile management overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPanelMode {
    /// Show the list of profiles.
    List,
    /// Show the "Add model" form.
    Adding,
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
    /// Index into the buffer's language `all_step_keywords()` for the highlighted item.
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

/// A concrete buffer mutation to apply when the user accepts the change.
#[derive(Debug, Clone)]
pub enum AgentMutation {
    /// Insert text after a given 1-based line number.
    InsertAfterLine {
        after_line_1based: usize,
        text: String,
    },
    /// Replace the contents of a single line (0-based row in the buffer).
    ReplaceLine { row_0based: usize, new_text: String },
    /// Create a new feature file with given name and content.
    CreateFile { file_name: String, text: String },
    /// Delete a range of lines (0-based, inclusive start, exclusive end).
    DeleteRange {
        start_row_0based: usize,
        end_row_0based: usize,
    },
    /// Replace a range of lines with new text.
    ReplaceRange {
        start_row_0based: usize,
        end_row_0based: usize,
        new_text: String,
    },
}

/// A single changed node in the Change Summary panel (MindMap view).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChangeSummaryNode {
    pub kind: ChangeKind,
    /// Matches a MindMap node ID for click-to-jump navigation.
    pub node_id: String,
    pub feature_idx: usize,
    pub scenario_name: String,
    pub step_text: String,
    pub old_step_text: Option<String>,
    pub line_number_1based: usize,
}

/// A pending text modification queued by an AI agent tool waiting for user approval.
#[derive(Debug, Clone)]
pub struct AgentPendingChange {
    /// Human-readable description for the confirmation prompt.
    pub description: String,
    /// Target file path (matches `BddFeature::file_path`).
    pub file_path: String,
    /// The buffer mutation to apply on acceptance.
    pub mutation: AgentMutation,
    /// Short scenario name for status messages.
    pub scenario_name: String,
    /// The tool call ID this change responds to (for feeding back to the LLM).
    pub tool_call_id: String,
    /// Snapshot of buffer content before the change (for diff computation).
    pub old_buffer_snapshot: String,
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
    /// Anchor position (row, col) where mouse drag started; `None` means no selection.
    pub selection_anchor: Option<(usize, usize)>,
    /// Current drag position (row, col); updated on every drag event.
    pub selection_end: Option<(usize, usize)>,
    /// Screen rectangle of the editor panel, updated each render frame.
    pub editor_panel_rect: Option<ratatui::layout::Rect>,
    /// Clickable regions registered during the last render frame.
    pub clickable_regions: Vec<ClickableRegion>,
    /// Screen rectangle of the tree panel, updated each render frame.
    pub tree_panel_rect: Option<ratatui::layout::Rect>,
    /// Screen rectangle of the preview panel, updated each render frame.
    pub preview_panel_rect: Option<ratatui::layout::Rect>,
    undo_stack: Vec<(EditorBuffer, usize, usize)>,
    redo_stack: Vec<(EditorBuffer, usize, usize)>,
    pub runner_config: Option<RunnerConfig>,
    runner_rx: Option<Receiver<RunEvent>>,
    last_external_check: Instant,
    external_change_prompt: Option<ExternalChangePrompt>,
    /// Pending text modifications requested by AI agent tools, awaiting user confirmation.
    pending_agent_changes: Vec<AgentPendingChange>,
    /// Computed diffs for each pending agent change (indexed parallel to `pending_agent_changes`).
    pub pending_change_diffs: Vec<Vec<DiffLine>>,
    /// Change summary nodes for the MindMap Change Summary panel.
    pub pending_change_summary: Vec<ChangeSummaryNode>,
    /// When set, the Explore tab steps column renders in diff mode.
    pub explore_diff_lines: Option<Vec<DiffLine>>,
    /// Whether the Change Summary overlay is visible on the MindMap tab.
    pub change_summary_visible: bool,
    /// Selected index within the Change Summary list.
    pub change_summary_selection: usize,
    // ── Explore tab state ───────────────────────────────────────────
    pub explore_focus: ColumnFocus,
    pub explore_selected_feature: usize,
    pub explore_selected_scenario: usize,
    pub explore_selected_step: usize,
    pub explore_edit_mode: bool,
    /// When set, the editor dims steps in non-focused scenarios.
    /// Stores the buffer row of the focused scenario's header line.
    pub editor_focus_scenario_row: Option<usize>,
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
    // ── Scenario location dropdown state ─────────────
    pub scenario_dropdown_open: bool,
    pub scenario_dropdown_selection: usize,
    // ── Multi-agent state ──────────────────────────────
    pub agents: Vec<AgentThread>,
    pub selected_agent: usize,
    pub next_agent_id: usize,
    pub slash_suggestion_active: bool,
    pub slash_suggestion_selection: usize,
    /// Whether the AI tab input bar has keyboard focus (Esc toggles this off).
    pub ai_input_focused: bool,
    quit_pending_confirm: bool,
    /// Temporary one-shot status message (e.g. "AI applied filter: @smoke").
    pub status_message: Option<String>,
    /// When the status message should be cleared (3-second lifespan).
    status_message_deadline: Option<Instant>,
    // ── Config ────────────────────────────────────────────────────────
    /// Resolved application configuration from layered sources.
    pub config: AppConfig,
    /// Whether the auth credential management overlay is active.
    pub auth_panel_active: bool,
    // ── Model profile state ─────────────────────────────────
    pub model_profiles: Vec<crate::profiles::ModelProfile>,
    pub model_active_id: Option<String>,
    pub model_panel_active: bool,
    pub model_panel_selection: usize,
    pub active_model_label: Option<String>,
    // ── Model profile "Add" form state ────────────────
    pub model_panel_mode: ModelPanelMode,
    pub model_form_focus: usize,
    pub model_form_name: String,
    pub model_form_provider: String,
    pub model_form_model: String,
    pub model_form_base_url: String,
    pub model_form_api_key: String,
    pub model_form_max_tokens: String,
    pub model_form_temperature: String,
    // ── Session browser state ──────────────────────────
    pub session_panel_active: bool,
    pub session_panel_selection: usize,
    pub session_list: Vec<crate::session::Session>,
    // ── Skill/template registry ─────────────────────────
    pub skill_registry: crate::agent::skills::SkillRegistry,
    // ── Generation pipeline state ───────────────────────
    pub generation_stage: crate::agent::pipeline::GenerationStage,
    pub pipeline_requirement: Option<crate::agent::pipeline::Requirement>,
    pub pipeline_plan: Option<crate::agent::pipeline::GenerationPlan>,
}

/// Convert a character index to the corresponding byte offset in a UTF-8 string.
fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// A clickable region registered during rendering for mouse hit-testing.
#[derive(Debug, Clone)]
pub enum ClickableRegion {
    Tab(MainTab),
    Tree,
    ExploreFeature {
        feature_idx: usize,
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
    ExploreScenario {
        scenario_idx: usize,
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
    ExploreStep {
        step_idx: usize,
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
    /// Editor panel area — reserved for click-to-focus.
    #[allow(dead_code)]
    EditorPanel,
    /// Preview panel area — reserved for click-to-focus.
    #[allow(dead_code)]
    PreviewPanel,
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
    #[allow(dead_code)]
    pub fn from_args() -> Result<Self> {
        let config = crate::config::load_config()?;
        let paths: Vec<PathBuf> = std::env::args()
            .skip(1)
            .filter(|arg| !arg.starts_with('-'))
            .map(PathBuf::from)
            .collect();

        let feature_file = paths
            .iter()
            .find(|p| p.extension().is_some_and(|ext| ext == "feature"));
        if let Some(p) = feature_file {
            return Self::from_file(p, config);
        }

        match paths.iter().find(|p| p.is_dir()) {
            Some(p) => Self::from_directory(p, config),
            None => Ok(Self::empty(config)),
        }
    }

    /// Builds the editor state from parsed CLI arguments.
    pub fn from_cli(cli: &crate::cli::Cli) -> Result<Self> {
        let config = crate::config::load_config()?;
        let paths: Vec<PathBuf> = cli.paths.iter().map(PathBuf::from).collect();

        let feature_file = paths
            .iter()
            .find(|p| p.extension().is_some_and(|ext| ext == "feature"));
        if let Some(p) = feature_file {
            return Self::from_file(p, config);
        }

        match paths.iter().find(|p| p.is_dir()) {
            Some(p) => Self::from_directory(p, config),
            None => Ok(Self::empty(config)),
        }
    }

    fn from_directory(dir: &Path, config: AppConfig) -> Result<Self> {
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
            selection_anchor: None,
            selection_end: None,
            editor_panel_rect: None,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            editor_focus_scenario_row: None,
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
            scenario_dropdown_open: false,
            scenario_dropdown_selection: 0,
            agents: vec![AgentThread::new(0, "Agent 1")],
            selected_agent: 0,
            next_agent_id: 1,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
            config,
            auth_panel_active: false,
            model_profiles: crate::profiles::ModelProfile::load_all(),
            model_active_id: crate::profiles::ModelProfile::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            model_panel_mode: ModelPanelMode::List,
            model_form_focus: 0,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            active_model_label: None,
            session_panel_active: false,
            session_panel_selection: 0,
            session_list: Vec::new(),
            skill_registry: Self::load_skill_registry(dir),
            generation_stage: crate::agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
        };
        app.spawn_llm_if_configured();
        app.activate_active_profile();
        app.mindmap_index.apply_highlight_categories("root");
        let n = app.buffers.len();
        app.status = format!("Opened directory with {n} feature file(s)");
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        Ok(app)
    }

    fn from_file(path: &PathBuf, config: AppConfig) -> Result<Self> {
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
            agents: vec![AgentThread::new(0, "Agent 1")],
            selected_agent: 0,
            next_agent_id: 1,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            pending_char: None,
            clipboard: None,
            scenario_fold: HashSet::new(),
            selection_anchor: None,
            selection_end: None,
            editor_panel_rect: None,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            editor_focus_scenario_row: None,
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
            scenario_dropdown_open: false,
            scenario_dropdown_selection: 0,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
            config,
            auth_panel_active: false,
            model_profiles: crate::profiles::ModelProfile::load_all(),
            model_active_id: crate::profiles::ModelProfile::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            model_panel_mode: ModelPanelMode::List,
            model_form_focus: 0,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            active_model_label: None,
            session_panel_active: false,
            session_panel_selection: 0,
            session_list: Vec::new(),
            skill_registry: {
                let root_dir = path.parent().unwrap_or(Path::new("."));
                Self::load_skill_registry(root_dir)
            },
            generation_stage: crate::agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
        };
        app.spawn_llm_if_configured();
        app.activate_active_profile();
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        app.mindmap_index.apply_highlight_categories("root");
        Ok(app)
    }

    fn empty(config: AppConfig) -> Self {
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
            agents: vec![AgentThread::new(0, "Agent 1")],
            selected_agent: 0,
            next_agent_id: 1,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            pending_char: None,
            clipboard: None,
            scenario_fold: HashSet::new(),
            selection_anchor: None,
            selection_end: None,
            editor_panel_rect: None,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            editor_focus_scenario_row: None,
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
            scenario_dropdown_open: false,
            scenario_dropdown_selection: 0,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
            config,
            auth_panel_active: false,
            model_profiles: crate::profiles::ModelProfile::load_all(),
            model_active_id: crate::profiles::ModelProfile::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            model_panel_mode: ModelPanelMode::List,
            model_form_focus: 0,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            active_model_label: None,
            session_panel_active: false,
            session_panel_selection: 0,
            session_list: Vec::new(),
            skill_registry: crate::agent::skills::SkillRegistry::new(),
            generation_stage: crate::agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
        };
        app.spawn_llm_if_configured();
        app.activate_active_profile();
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        app.mindmap_index.apply_highlight_categories("root");
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

    pub fn agent_mut(&mut self) -> &mut AgentThread {
        let idx = self.selected_agent.min(self.agents.len().saturating_sub(1));
        &mut self.agents[idx]
    }

    pub fn agent(&self) -> &AgentThread {
        let idx = self.selected_agent.min(self.agents.len().saturating_sub(1));
        &self.agents[idx]
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
        if self.agent_mut().llm_handle.is_some() {
            return;
        }
        // Try the new config-based approach first
        if let Some((name, provider)) = self.config.default_provider_config() {
            match crate::llm::LlmConfig::from_provider_config(name, provider) {
                Ok(config) => {
                    self.active_model_label = Some(format!("{name} ({})", config.model));
                    self.status = format!(
                        "LLM configured: model={}, base_url={}",
                        config.model, config.base_url
                    );
                    let (handle, rx) = crate::llm::spawn_llm(config);
                    self.agent_mut().llm_handle = Some(handle);
                    self.agent_mut().llm_rx = Some(rx);
                    return;
                }
                Err(e) => {
                    self.status = format!("LLM not configured: {e}");
                    return;
                }
            }
        }
        // Fall back to legacy env-var config
        match crate::llm::LlmConfig::from_env() {
            Ok(config) => {
                self.active_model_label = Some(format!("env: {}", config.model));
                self.status = format!(
                    "LLM configured: model={}, base_url={}",
                    config.model, config.base_url
                );
                let (handle, rx) = crate::llm::spawn_llm(config);
                self.agent_mut().llm_handle = Some(handle);
                self.agent_mut().llm_rx = Some(rx);
            }
            Err(e) => {
                self.status = format!("LLM not configured: {e}");
            }
        }
    }

    /// Activate the saved active profile (if any) that has a matching file on disk.
    pub fn activate_active_profile(&mut self) {
        let active_id = match self.model_active_id.clone() {
            Some(id) => id,
            None => return,
        };
        let profile = match self.model_profiles.iter().find(|p| p.id == active_id) {
            Some(p) => p.clone(),
            None => return,
        };
        self.activate_model_profile(&profile);
    }

    /// Respawn the LLM worker thread using the given model profile's configuration.
    fn activate_model_profile(&mut self, profile: &crate::profiles::ModelProfile) {
        let config = crate::llm::LlmConfig {
            api_key: profile.api_key.clone(),
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            max_tokens: profile.max_tokens,
            temperature: profile.temperature,
            context_window: None,
        };
        let (handle, rx) = crate::llm::spawn_llm(config);
        self.agent_mut().llm_handle = Some(handle);
        self.agent_mut().llm_rx = Some(rx);
        self.active_model_label = Some(format!("{} ({})", profile.name, profile.model));
        self.status = format!("Switched to model: {}", profile.name);
    }

    /// Poll the LLM response channel and push completed responses into chat history.
    ///
    /// When the LLM requests tool calls, this method executes them and
    /// re-invokes the LLM with the results (the "agent loop") until a plain
    /// text response is received or the iteration limit is reached.
    pub fn poll_llm_events(&mut self) {
        for i in 0..self.agents.len() {
            let Some(rx) = self.agents[i].llm_rx.take() else {
                continue;
            };
            let mut keep_rx = true;
            loop {
                match rx.try_recv() {
                    Ok(crate::llm::LlmEvent::Done {
                        full_text,
                        reasoning_content,
                        model,
                        input_tokens,
                        output_tokens,
                    }) => {
                        if self.agents[i].partial_response.is_empty() {
                            self.agents[i].messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content: full_text,
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content,
                                source: None,
                            });
                        } else {
                            let content = std::mem::take(&mut self.agents[i].partial_response);
                            self.agents[i].messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content,
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content,
                                source: None,
                            });
                        }
                        self.agents[i].partial_response.clear();
                        self.agents[i].status = AiStatus::Idle;
                        self.agents[i].tool_status = None;
                        self.agents[i].agent_loop_count = 0;
                        self.agents[i].total_input_tokens += input_tokens.unwrap_or(0) as u64;
                        self.agents[i].last_input_tokens = input_tokens;
                        self.agents[i].total_output_tokens += output_tokens.unwrap_or(0) as u64;
                        if i == self.selected_agent {
                            self.status = format!("AI response received ({model})");
                        }
                    }
                    Ok(crate::llm::LlmEvent::ToolCallRequest {
                        tool_calls,
                        reasoning_content,
                    }) => {
                        let partial_text = std::mem::take(&mut self.agents[i].partial_response);
                        self.agents[i].messages.push(AiChatMessage {
                            role: AiRole::Assistant,
                            content: partial_text,
                            tool_calls: Some(tool_calls.clone()),
                            tool_call_id: None,
                            reasoning_content,
                            source: None,
                        });
                        let mut pending_queued = false;
                        for tc in &tool_calls {
                            self.agents[i].tool_status =
                                Some(format!("AI is calling {}...", tc.name));
                            let pending_before = self.pending_agent_changes.len();
                            match crate::agent::execute_tool(self, &tc.name, &tc.arguments, &tc.id)
                            {
                                Ok(result) => {
                                    let pending_after = self.pending_agent_changes.len();
                                    if pending_after > pending_before {
                                        pending_queued = true;
                                    } else {
                                        self.agents[i].messages.push(AiChatMessage {
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
                                    self.agents[i].messages.push(AiChatMessage {
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
                        if pending_queued {
                            self.agents[i].partial_response.clear();
                            self.agents[i].status = AiStatus::AwaitingApproval;
                            self.agents[i].tool_status = None;
                        } else if self.project.features.is_empty() {
                            self.agents[i].partial_response.clear();
                            self.agents[i].status = AiStatus::Idle;
                            self.agents[i].tool_status = None;
                            self.agents[i].agent_loop_count = 0;
                            if i == self.selected_agent {
                                self.status = "The project directory has no .feature files. Add one to begin.".to_string();
                            }
                        } else {
                            self.agents[i].agent_loop_count += 1;
                            if self.agents[i].agent_loop_count > 5 {
                                self.agents[i].status = AiStatus::Error;
                                self.agents[i].tool_status = None;
                                self.agents[i].agent_loop_count = 0;
                                if i == self.selected_agent {
                                    self.status =
                                        "AI error: too many tool call iterations".to_string();
                                }
                            } else if self.agents[i].llm_handle.is_some() {
                                // Compact context before re-invocation
                                self.compact_context_if_needed(i);
                                let messages = self.build_chat_messages_for_agent(i);
                                let tools = Some(crate::agent::get_tools());
                                let handle = self.agents[i].llm_handle.as_ref().unwrap();
                                let _ = handle.send(crate::llm::LlmRequest::Chat {
                                    system: Some(self.ai_system_prompt(None)),
                                    messages,
                                    tools,
                                });
                            }
                        }
                    }
                    Ok(crate::llm::LlmEvent::Error { message }) => {
                        self.agents[i].partial_response.clear();
                        self.agents[i].status = AiStatus::Error;
                        self.agents[i].tool_status = None;
                        self.agents[i].agent_loop_count = 0;
                        if i == self.selected_agent {
                            self.status = format!("AI error: {message}");
                        }
                    }
                    Ok(crate::llm::LlmEvent::Chunk { content }) => {
                        self.agents[i].partial_response.push_str(&content);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        keep_rx = false;
                        self.agents[i].partial_response.clear();
                        self.agents[i].status = AiStatus::Error;
                        self.agents[i].tool_status = None;
                        self.agents[i].agent_loop_count = 0;
                        if i == self.selected_agent {
                            self.status = "AI error: background LLM thread has exited".to_string();
                        }
                        break;
                    }
                }
            }
            if keep_rx {
                self.agents[i].llm_rx = Some(rx);
            }
        }
    }

    /// Build a project-context summary to inject ahead of every LLM request.
    /// This lets the LLM see the full project structure without extra tool calls.
    fn build_project_context_summary(&self) -> String {
        let mut ctx = String::from("[Project Context]\n");
        ctx.push_str(&format!(
            "Features: {} file(s)\n",
            self.project.features.len()
        ));

        for f in &self.project.features {
            let path = f.file_path.to_string_lossy();
            let sc_count = f.scenarios.len();
            let st_count: usize = f.scenarios.iter().map(|s| s.steps.len()).sum::<usize>()
                + f.background.as_ref().map(|bg| bg.steps.len()).unwrap_or(0);
            ctx.push_str(&format!(
                "  {path}: {sc_count} scenario(s), {st_count} step(s)"
            ));
            if f.language != "en" {
                ctx.push_str(&format!("     Language: {}", f.language));
            }
            if !f.tags.is_empty() {
                ctx.push_str(&format!(" [{}]", f.tags.join(" ")));
            }
            if !f.description.is_empty() {
                let desc_preview: String = f
                    .description
                    .iter()
                    .flat_map(|l| l.chars())
                    .take(80)
                    .collect();
                ctx.push_str(&format!("     Description: {}...\n", desc_preview));
            } else {
                ctx.push('\n');
            }
        }

        // Most-reused step patterns
        if !self.step_index.is_empty() {
            ctx.push_str("\nFrequent step patterns:\n");
            for (text, count) in self.step_index.most_common(8) {
                ctx.push_str(&format!("  ({count}x) \"{text}\"\n"));
            }
        }

        // Active file
        if let Some(ref active_path) = self.file_path {
            ctx.push_str(&format!(
                "\nActive file: {}\n",
                active_path.to_string_lossy()
            ));
        }

        ctx
    }

    /// Check whether a user message is requesting feature/scenario generation.
    fn is_generation_request(text: &str) -> bool {
        let keywords = [
            "create",
            "generate",
            "make",
            "new feature",
            "new scenario",
            "add feature",
            "add scenario",
            "write a",
            "write an",
            "写",
            "创建",
            "生成",
            "添加",
            "新增",
        ];
        let lower = text.to_lowercase();
        keywords.iter().any(|k| lower.contains(k))
    }

    /// Build `ChatMessage` list from the current AI chat history for LLM
    /// requests.  Prepends a project-context summary as a system message.
    fn build_chat_messages_for_agent(&self, agent_idx: usize) -> Vec<crate::llm::ChatMessage> {
        let mut msgs: Vec<crate::llm::ChatMessage> = Vec::new();

        // Inject project context as a system message at the front
        if self.agents[agent_idx]
            .messages
            .iter()
            .any(|m| matches!(m.role, AiRole::User))
        {
            msgs.push(crate::llm::ChatMessage {
                role: "system".into(),
                content: self.build_project_context_summary(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            });
        }

        // Original 1:1 mapping
        for m in &self.agents[agent_idx].messages {
            msgs.push(crate::llm::ChatMessage {
                role: match m.role {
                    AiRole::User => "user".into(),
                    AiRole::Assistant => "assistant".into(),
                    AiRole::Tool => "tool".into(),
                },
                content: m.content.clone(),
                tool_calls: m.tool_calls.clone(),
                tool_call_id: m.tool_call_id.clone(),
                reasoning_content: m.reasoning_content.clone(),
            });
        }
        msgs
    }

    /// Load the skill registry from the project directory or parent directories.
    fn load_skill_registry(project_dir: &Path) -> crate::agent::skills::SkillRegistry {
        // Try several paths for skill files
        for dir in &[
            project_dir.join(".teshi/skills"),
            project_dir.join("skills"),
        ] {
            if dir.exists() {
                return crate::agent::skills::SkillRegistry::load_from_dir(dir);
            }
        }
        // Also check parent directories (useful when project is a subdirectory)
        if let Some(parent) = project_dir.parent() {
            let parent_skills = parent.join(".teshi/skills");
            if parent_skills.exists() {
                return crate::agent::skills::SkillRegistry::load_from_dir(&parent_skills);
            }
        }
        crate::agent::skills::SkillRegistry::new()
    }

    /// The system prompt used for all AI chat requests.
    /// When `request` contains generation keywords, additional guidance is appended.
    fn ai_system_prompt(&self, request: Option<&str>) -> String {
        let mut prompt = String::from(
            "You are a BDD/Gherkin assistant embedded in Teshi, a TUI editor for .feature files.\n\
             \n\
             ## Your Role\n\
             You help users write, edit, organize, and validate Gherkin feature files using\n\
             automated tools. You have access to files, scenarios, steps, test runners, and\n\
             visual aids (MindMap). Always think before acting: inspect the project structure\n\
             first, then make precise changes.\n\
             \n\
             ## Core Principles\n\
             - **Understand first, then act**: Before making any changes, inspect the\n\
               project context and existing files using get_project_info or get_feature_content.\n\
             - **Prefer simplicity**: Start with the simplest approach. Do not create\n\
               unnecessary scenarios or complex Scenario Outlines when a basic Scenario suffices.\n\
             - **Do exactly what was asked**: Generate what the user requested. Do not\n\
               add extra scenarios, tags, or features unless explicitly requested.\n\
             - **Verify your work**: After creating a feature, call validate_feature to\n\
               check for common issues.\n\
             - **Respect project conventions**: Match the existing style, keyword language,\n\
               indentation, tag format, and naming patterns from [Project Context].\n\
             \n\
             ## Generated Content Standards\n\
             - Every scenario must have at least one **Given** and one **Then** step.\n\
             - Scenario names should be descriptive and follow the pattern of existing scenarios.\n\
             - Use @tags consistently with the project's tag conventions.\n\
             - When the project uses non-English keywords (e.g. 中文), generate new steps\n\
               using the same language.\n\
             - Use Scenario Outline + Examples when the same steps apply to 3+ data variations,\n\
               not for just 1-2 variations.\n\
             - Each feature file should focus on one feature area.\n\
             \n\
             ## Available Tools\n\
             - **get_project_info**: Get project directory, file list, scenario/step counts.\n\
               Use this FIRST when the user asks about the project.\n\
             - **get_feature_content**: Get parsed content of a specific .feature file (names,\n\
               steps, line numbers, tags, background, examples). Use this BEFORE editing any file.\n\
             - **search_features**: Search all features for scenarios matching tag, step content,\n\
               or scenario name. Use this when the user asks 'find scenarios that...'.\n\
             - **create_feature_file**: Create a brand new .feature file with a feature name,\n\
               optional description, tags, and background steps. Requires user approval.\n\
             - **insert_scenario**: Insert a new Scenario or Scenario Outline into an existing\n\
               feature file. Requires user approval. Always call get_feature_content first to\n\
               determine the correct insert_after_line.\n\
             - **update_step**: Replace the body text of a specific step in a scenario while\n\
               preserving its keyword and indentation. Requires user approval.\n\
             - **delete_scenario**: Delete an entire scenario from a feature file by name.\n\
               Requires user approval.\n\
             - **rename_scenario**: Rename a scenario. Requires user approval.\n\
             - **reorder_steps**: Reorder the steps inside a scenario (providing a permutation\n\
               of step indices). Requires user approval.\n\
             - **run_tests**: Execute the external test runner for all or filtered scenarios.\n\
               Returns pass/fail/skip summary with details. Use this when the user asks to\n\
               'run the tests' or 'check if these scenarios pass'.\n\
             - **highlight_mindmap_nodes**: Visually highlight MindMap tree nodes matching a\n\
               condition. Use for visual exploration only — it does NOT return text content.\n\
             - **apply_mindmap_filter**: Filter the MindMap tree to show only matching nodes.\n\
               Use 'clear' to remove the active filter.\n\
             \n\
             ## Workflow Guidelines\n\
             1. When the user mentions a specific file, ALWAYS call get_feature_content first.\n\
             2. When creating a new file, call create_feature_file.\n\
             3. After viewing content, make ONE editing tool call at a time. Do not batch.\n\
             4. When editing, provide accurate line numbers from get_feature_content.\n\
             5. When the user asks to search or find, use search_features.\n\
             6. When the user asks to run or test, use run_tests.\n\
             7. Use highlight_mindmap_nodes and apply_mindmap_filter only for visual\n\
                exploration — never as a substitute for reading file content.\n\
             \n\
             ## Gherkin Conventions\n\
             - Use standard keywords: **Given**, **When**, **Then**, **And**, **But**.\n\
             - Indentation: Feature at column 0, Scenario at 2 spaces, Steps at 4 spaces.\n\
             - Tags start with @ and appear before the element they annotate.\n\
             - **Background** blocks contain steps common to all scenarios in a feature.\n\
             - **Scenario Outline** uses `<placeholders>` and **Examples** tables.\n\
             - Examples tables use pipe-delimited format: `| header1 | header2 |`.\n\
             - Keep scenarios focused: one behavior per scenario.\n\
             - Steps should be declarative, not imperative: describe WHAT, not HOW.\n\
             \n\
             ## Example Gherkin Structure\n\
             ```gherkin\n\
             @smoke @login\n\
             Feature: User Login\n\
               As a registered user\n\
               I want to log in\n\
               So that I can access my account\n\
             \n\
               Background:\n\
                 Given a registered user with email \"test@example.com\"\n\
             \n\
               Scenario: Successful login with valid credentials\n\
                 Given I am on the login page\n\
                 When I enter valid credentials\n\
                 Then I should see the dashboard\n\
             \n\
               Scenario Outline: Login with various roles\n\
                 Given I am on the login page\n\
                 When I log in as <role>\n\
                 Then I should see the <landing_page>\n\
             \n\
                 Examples:\n\
                   | role    | landing_page |\n\
                   | admin   | Admin Panel  |\n\
                   | user    | Dashboard    |\n\
               ```\n\
             \n\
             ## Feature Generation Process\n\
             When the user asks to create, generate, or add a feature or scenario:\n\
             1. FIRST look at [Project Context] (sent alongside your system prompt)\n\
                to understand existing files, scenarios, and step patterns.\n\
             2. THEN use get_feature_content to inspect the file you will edit.\n\
             3. Plan before generating: consider what scenarios are needed.\n\
             Always try to cover:\n\
               - Happy path (the expected successful flow)\n\
               - Error / validation paths (what happens when things go wrong)\n\
               - Edge cases (empty inputs, boundary values, permissions, roles)\n\
             Use Scenario Outline + Examples tables for data-driven variations.\n\
             Reuse existing step patterns from [Project Context] to keep style consistent.\n\
             \n\
             ## Error Recovery\n\
             - If a tool call fails because a file or scenario was not found, re-read the\n\
               project state with get_project_info or get_feature_content and try again.\n\
             - If you are unsure about line numbers, call get_feature_content to verify.\n\
             - If the project is empty, suggest creating a feature file with create_feature_file.\n\
             - Do NOT call the same tool repeatedly in a loop if it keeps failing.\n\
             \n\
             ## Interaction Guidelines\n\
             - Be concise. Tool results speak louder than words.\n\
             - Explain what you are about to do before making file-modifying tool calls.\n\
             - When a change is queued for approval, tell the user to press [Y] to accept\n\
               or [N] to reject.\n\
             - Respect the user's existing file structure, indentation, and naming style.\n\
             - Do not invent file names — use the ones the user provides or that exist.",
        );

        // Add skill catalog
        if !self.skill_registry.is_empty() {
            prompt.push_str("\n\n## Available Generation Templates\n");
            prompt.push_str(&self.skill_registry.catalog());
            prompt.push_str("Use the `load_skill` tool to load the full template content.");
        }

        // Generation pipeline guidance
        prompt.push_str(
            "\n\n## Feature Generation Pipeline\n\
             When the user asks to create or generate a feature, follow this pipeline:\n\
             1. **Requirements Gathering** — Ask questions about what they need. Call `submit_requirements` when done.\n\
             2. **Planning** — Design the scenario structure. Call `generate_plan` to submit your plan.\n\
             3. **Writing** — Execute the plan using `create_feature_file` and `insert_scenario`.\n\
             4. **Validation** — Use `validate_feature` to check for issues.\n\
             \n\
             Do NOT skip steps. Start by gathering requirements.\n\
             If the user's request is already detailed, you can ask 1-2 clarifying questions then proceed.\n\
             Always check the [Project Context] to understand existing files before generating.",
        );

        // Inject pipeline stage guidance if a generation is in progress
        if !matches!(
            self.generation_stage,
            crate::agent::pipeline::GenerationStage::Idle
                | crate::agent::pipeline::GenerationStage::Complete
        ) {
            prompt.push_str("\n## Generation Pipeline Status\n");
            prompt.push_str(&format!(
                "Current stage: **{}**\n",
                self.generation_stage.label()
            ));
            prompt.push_str(self.generation_stage.prompt_guidance());
        }

        // Append extra guidance for generation requests
        if let Some(req) = request
            && Self::is_generation_request(req)
        {
            prompt.push_str(
                "\n\n## Additional Guidance (Generation Request Detected)\n\
                     The user is asking you to create or generate content. Follow the\n\
                     Feature Generation Process above carefully. Before creating files,\n\
                     scan the [Project Context] to understand existing patterns and reuse\n\
                     step text where appropriate. Prioritize data-driven Scenario Outlines\n\
                     over repetitive Basic scenarios when the same flow applies to\n\
                     multiple input variations.",
            );
        }

        prompt
    }

    /// After a pending agent change is accepted or rejected, feed the result back
    /// to the LLM as a tool result message and continue the agent loop.
    fn feed_agent_tool_result(&mut self, tool_call_id: String, result: String) {
        // Append tool result message
        self.agent_mut().messages.push(AiChatMessage {
            role: AiRole::Tool,
            content: result,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            reasoning_content: None,
            source: None,
        });

        // If the project is empty, terminate gracefully
        if self.project.features.is_empty() {
            self.agent_mut().partial_response.clear();
            self.agent_mut().status = AiStatus::Idle;
            self.agent_mut().tool_status = None;
            self.agent_mut().agent_loop_count = 0;
            self.status =
                "The project directory has no .feature files. Add one to begin.".to_string();
            return;
        }

        // Re-invoke the LLM to continue the agent loop
        // Compact context before sending to avoid exceeding token limits
        self.compact_context_if_needed(self.selected_agent);
        let messages = self.build_chat_messages_for_agent(self.selected_agent);
        let tools = Some(crate::agent::get_tools());
        let system_prompt = self.ai_system_prompt(None);
        let agent = self.agent_mut();
        agent.agent_loop_count += 1;
        if agent.agent_loop_count > 5 {
            agent.status = AiStatus::Error;
            agent.tool_status = None;
            agent.agent_loop_count = 0;
            self.status = "AI error: too many tool call iterations".to_string();
        } else if let Some(ref handle) = agent.llm_handle {
            agent.status = AiStatus::Waiting;
            agent.tool_status = Some("Teshi is thinking...".into());
            let _ = handle.send(crate::llm::LlmRequest::Chat {
                system: Some(system_prompt),
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

    /// Get the context window size from the active provider config.
    /// Falls back to 128000 if not configured.
    fn active_context_window(&self) -> u32 {
        self.config
            .default_provider_config()
            .and_then(|(_, p)| p.context_window)
            .unwrap_or(128000)
    }

    /// Compact oldest User+Assistant messages into a summary when the estimated
    /// token count exceeds 70 % of the context window.  Tool messages are kept
    /// intact (they belong to the current agent loop).
    fn compact_context_if_needed(&mut self, agent_idx: usize) {
        let last_input = match self.agents[agent_idx].last_input_tokens {
            Some(v) => v,
            None => return,
        };
        let context_window = self.active_context_window();
        let threshold = (context_window as f64 * 0.7) as u32;
        if last_input <= threshold {
            return;
        }
        // Estimate tokens at ~4 chars per token
        let target_chars = (threshold.saturating_sub(2000).max(1000) as usize) * 4;

        let messages = &mut self.agents[agent_idx].messages;

        // Find oldest compactable block (User + Assistant pairs, keep Tool messages)
        let mut current_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let mut compact_end: usize = 0;
        let mut compact_topics: Vec<String> = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            if current_chars <= target_chars {
                break;
            }
            if matches!(msg.role, AiRole::Tool) {
                continue; // keep tool results
            }
            if matches!(msg.role, AiRole::User | AiRole::Assistant) {
                // Record first few words as "topic" for the summary
                let preview: String = msg.content.chars().take(60).collect();
                if !preview.is_empty() {
                    compact_topics.push(preview);
                }
                current_chars = current_chars.saturating_sub(msg.content.len());
                compact_end = i + 1;
            }
        }

        if compact_end > 1 {
            let keep_min = 4.min(messages.len());
            let max_remove = messages.len().saturating_sub(keep_min);
            compact_end = compact_end.min(max_remove).max(1);

            let drained: Vec<_> = messages.drain(0..compact_end).collect();
            let removed_count = drained.len();

            // Insert a compacted summary message instead of losing context entirely
            let summary = format!(
                "[Compressed: {} earlier message(s) about: {}]",
                removed_count,
                compact_topics
                    .into_iter()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            messages.insert(
                0,
                AiChatMessage {
                    role: AiRole::User,
                    content: summary,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                    source: Some("compacted".into()),
                },
            );

            self.agents[agent_idx].last_input_tokens = None;
            self.status_message = Some(format!(
                "Context compacted: {removed_count} message(s) → summary ({} token window)",
                context_window
            ));
            self.status_message_deadline = Some(Instant::now() + Duration::from_secs(3));
        }
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

    /// Simulate applying a mutation on the old content and return the new content.
    /// Used to compute the diff between old and post-change state before applying.
    fn simulate_mutation(old_content: &str, mutation: &AgentMutation) -> String {
        match mutation {
            AgentMutation::InsertAfterLine {
                after_line_1based,
                text,
            } => {
                let mut lines: Vec<&str> = old_content.lines().collect();
                // If the content is empty, start with one empty line so we can insert at index 0
                if lines.is_empty() {
                    lines.push("");
                }
                // after_line_1based is 1-based — convert to 0-based insert position.
                // Insert position = after_line_1based, because inserting at that
                // index places the text BEFORE the line at that index, which is
                // equivalent to AFTER the previous line.
                let insert_at = (*after_line_1based).min(lines.len());

                // Collect text lines and insert in reverse order so they appear
                // in the correct sequence (each insert at the same position
                // pushes previous insertions rightward).
                let new_lines: Vec<&str> = text.lines().collect();
                for line in new_lines.iter().rev() {
                    lines.insert(insert_at, line);
                }

                // Preserve trailing newline semantics
                let mut result = lines.join("\n");
                if old_content.ends_with('\n') || text.ends_with('\n') {
                    result.push('\n');
                }
                result
            }
            AgentMutation::ReplaceLine {
                row_0based,
                new_text,
            } => {
                let mut lines: Vec<&str> = old_content.lines().collect();
                if *row_0based < lines.len() {
                    lines[*row_0based] = new_text;
                } else {
                    lines.push(new_text);
                }
                let mut result = lines.join("\n");
                if old_content.ends_with('\n') {
                    result.push('\n');
                }
                result
            }
            AgentMutation::CreateFile { text, .. } => {
                // The simulation is just the file content itself
                text.clone()
            }
            AgentMutation::DeleteRange {
                start_row_0based,
                end_row_0based,
            } => {
                let lines: Vec<&str> = old_content.lines().collect();
                let mut result = String::new();
                for (i, line) in lines.iter().enumerate() {
                    if i >= *start_row_0based && i < *end_row_0based {
                        continue;
                    }
                    result.push_str(line);
                    result.push('\n');
                }
                result
            }
            AgentMutation::ReplaceRange {
                start_row_0based,
                end_row_0based,
                new_text,
            } => {
                let lines: Vec<&str> = old_content.lines().collect();
                let mut result = String::new();
                for (i, line) in lines.iter().enumerate() {
                    if i < *start_row_0based {
                        result.push_str(line);
                        result.push('\n');
                    } else if i == *start_row_0based {
                        result.push_str(new_text);
                        if !new_text.is_empty() && !new_text.ends_with('\n') {
                            result.push('\n');
                        }
                    } else if i >= *end_row_0based {
                        result.push_str(line);
                        result.push('\n');
                    }
                }
                result
            }
        }
    }

    /// Whether an agent change is waiting for user confirmation.
    pub fn has_agent_change_prompt(&self) -> bool {
        !self.pending_agent_changes.is_empty()
    }

    /// Queue a pending change from the agent, show confirmation prompt.
    pub fn queue_agent_change(&mut self, change: AgentPendingChange) {
        let desc = change.description.clone();

        // Compute diff between old snapshot and simulated new content
        let diff = if self.find_feature_idx_for_file(&change.file_path).is_some() {
            let old_content = &change.old_buffer_snapshot;
            let new_content = Self::simulate_mutation(old_content, &change.mutation);
            crate::diff::diff_buffers(old_content, &new_content)
        } else {
            Vec::new()
        };
        self.pending_change_diffs.push(diff);
        self.compute_change_summary();

        self.pending_agent_changes.push(change);

        // Auto-show diff in Explore tab and Change Summary in MindMap tab
        if let Some(diff) = self.pending_change_diffs.last() {
            self.explore_diff_lines = Some(diff.clone());
        }
        if !self.pending_change_summary.is_empty() {
            self.change_summary_visible = true;
            self.change_summary_selection = 0;
        }

        self.status = format!("AI wants to {}. [Y] accept [N] reject", desc);
    }

    /// Recompute [`pending_change_summary`] from [`pending_change_diffs`].
    fn compute_change_summary(&mut self) {
        let mut summary: Vec<ChangeSummaryNode> = Vec::new();

        for (change_idx, diff_lines) in self.pending_change_diffs.iter().enumerate() {
            let Some(change) = self.pending_agent_changes.get(change_idx) else {
                continue;
            };
            let Some(feature_idx) = self.find_feature_idx_for_file(&change.file_path) else {
                continue;
            };

            for dl in diff_lines {
                if dl.kind == ChangeKind::Unchanged {
                    continue;
                }
                // Map line to MindMap node
                if let Some(nm) = crate::mindmap::find_closest_node(
                    &self.mindmap_index,
                    feature_idx,
                    dl.line_number_1based,
                ) {
                    summary.push(ChangeSummaryNode {
                        kind: dl.kind,
                        node_id: nm.node_id.clone(),
                        feature_idx,
                        scenario_name: change.scenario_name.clone(),
                        step_text: dl.text.clone(),
                        old_step_text: dl.old_text.clone(),
                        line_number_1based: dl.line_number_1based,
                    });
                }
            }
        }

        self.pending_change_summary = summary;
    }

    /// Toggle the Explore-tab diff view on/off.
    pub fn toggle_explore_diff_view(&mut self) {
        if self.explore_diff_lines.is_some() {
            self.explore_diff_lines = None;
            self.status = "Exited diff view".to_string();
        } else if self.has_agent_change_prompt() {
            if let Some(diff) = self.pending_change_diffs.first() {
                self.explore_diff_lines = Some(diff.clone());
            }
            self.active_tab = MainTab::Explore;
            self.explore_focus = ColumnFocus::Step;
            self.status = "Diff view — explore tab shows changes. [D] to close.".to_string();
        }
    }

    /// Toggle the MindMap Change Summary panel on/off.
    pub fn toggle_change_summary(&mut self) {
        if self.change_summary_visible {
            self.change_summary_visible = false;
        } else if self.has_agent_change_prompt() && !self.pending_change_summary.is_empty() {
            self.change_summary_visible = true;
            self.change_summary_selection = 0;
            self.active_tab = MainTab::MindMap;
            self.status = format!(
                "Change Summary — {} change(s). ↑↓ navigate, Enter jump, Esc close.",
                self.pending_change_summary.len()
            );
        }
    }

    /// Navigate to a MindMap node by ID (used by Change Summary jump).
    pub fn navigate_to_mindmap_node(&mut self, node_id: &str) {
        if let Some(path) = crate::mindmap::node_id_to_path(node_id, &self.mindmap_index) {
            self.tree_state.select(path);
            self.mindmap_index.apply_highlight_categories(node_id);
            self.view_stage = ViewStage::TreeAndEditor;
            self.active_tab = MainTab::MindMap;
            self.mindmap_focus = MindMapFocus::Main;
            self.rebuild_preview();
            self.change_summary_visible = false;
        }
    }

    /// Accept and apply the first pending agent change.
    ///
    /// Returns `(tool_call_id, result_text)` for feeding back to the LLM.
    pub fn accept_agent_change(&mut self) -> Result<(String, String)> {
        let change = self.pending_agent_changes.remove(0);

        // Handle CreateFile specially — it doesn't need an existing feature_idx
        if matches!(&change.mutation, AgentMutation::CreateFile { .. }) {
            let (file_name, text) = match &change.mutation {
                AgentMutation::CreateFile {
                    file_name: name,
                    text,
                } => (name.clone(), text.clone()),
                _ => unreachable!(),
            };
            let full_path = self.project.root_dir.join(&file_name);
            std::fs::write(&full_path, &text)
                .with_context(|| format!("failed to create {}", full_path.display()))?;
            let feature = gherkin::parse_feature(&text, full_path.clone());
            self.project.features.push(feature);
            let buffer = EditorBuffer::from_string(text.clone());
            self.buffers.push(buffer);
            self.buffer_dirty.push(false);
            self.disk_stamps.push(FileStamp::capture(&full_path));
            self.refresh_project_from_buffers();
            self.switch_to_buffer(self.buffers.len() - 1);
            let result = format!("Successfully created {file_name}");
            self.clear_pending_change_state();
            self.set_status_message(format!("AI created file: {file_name}"));
            return Ok((change.tool_call_id, result));
        }

        let feature_idx = self
            .find_feature_idx_for_file(&change.file_path)
            .context("feature file not found")?;

        let (cursor_row, result) = match &change.mutation {
            AgentMutation::InsertAfterLine {
                after_line_1based,
                text,
            } => {
                self.insert_text_into_buffer(&change.file_path, *after_line_1based, text)?;
                let new_row = *after_line_1based; // 0-based after insert_line
                let result = format!(
                    "Successfully inserted scenario \"{}\" into {} at line {}.",
                    change.scenario_name,
                    change.file_path,
                    after_line_1based + 1
                );
                (new_row, result)
            }
            AgentMutation::ReplaceLine {
                row_0based,
                new_text,
            } => {
                self.buffers[feature_idx].replace_line(*row_0based, new_text);
                self.set_buffer_dirty(feature_idx, true);
                if self.active_buffer_idx == Some(feature_idx) {
                    self.buffer = self.buffers[feature_idx].clone();
                }
                let result = format!(
                    "Successfully updated step \"{}\" in {} at line {}.",
                    change.scenario_name,
                    change.file_path,
                    row_0based + 1
                );
                (*row_0based, result)
            }
            AgentMutation::DeleteRange {
                start_row_0based,
                end_row_0based,
            } => {
                let new_content = {
                    let buffer = &self.buffers[feature_idx];
                    let original = buffer.as_string();
                    let lines: Vec<&str> = original.lines().collect();
                    let mut content = String::new();
                    for (i, line) in lines.iter().enumerate() {
                        if i >= *start_row_0based && i < *end_row_0based {
                            continue;
                        }
                        content.push_str(line);
                        content.push('\n');
                    }
                    content
                };
                self.buffers[feature_idx] = EditorBuffer::from_string(new_content);
                self.set_buffer_dirty(feature_idx, true);
                if self.active_buffer_idx == Some(feature_idx) {
                    self.buffer = self.buffers[feature_idx].clone();
                }
                let result = format!(
                    "Successfully deleted lines {}-{} in {} for scenario \"{}\".",
                    start_row_0based + 1,
                    end_row_0based,
                    change.file_path,
                    change.scenario_name
                );
                (*start_row_0based, result)
            }
            AgentMutation::ReplaceRange {
                start_row_0based,
                end_row_0based,
                new_text,
            } => {
                let new_content = {
                    let buffer = &self.buffers[feature_idx];
                    let original = buffer.as_string();
                    let lines: Vec<&str> = original.lines().collect();
                    let mut content = String::new();
                    for (i, line) in lines.iter().enumerate() {
                        if i < *start_row_0based {
                            content.push_str(line);
                            content.push('\n');
                        }
                    }
                    content.push_str(new_text);
                    if !new_text.is_empty() && !new_text.ends_with('\n') {
                        content.push('\n');
                    }
                    for (i, line) in lines.iter().enumerate() {
                        if i >= *end_row_0based {
                            content.push_str(line);
                            content.push('\n');
                        }
                    }
                    content
                };
                self.buffers[feature_idx] = EditorBuffer::from_string(new_content);
                self.set_buffer_dirty(feature_idx, true);
                if self.active_buffer_idx == Some(feature_idx) {
                    self.buffer = self.buffers[feature_idx].clone();
                }
                let result = format!(
                    "Successfully replaced lines {}-{} in {} for scenario \"{}\".",
                    start_row_0based + 1,
                    end_row_0based,
                    change.file_path,
                    change.scenario_name
                );
                (*start_row_0based, result)
            }
            AgentMutation::CreateFile { .. } => unreachable!(),
        };

        // Re-parse the project to update Gherkin AST and MindMap
        self.refresh_project_from_buffers();

        // Switch to the modified buffer and move cursor to the change area
        if self.active_buffer_idx != Some(feature_idx) {
            self.switch_to_buffer(feature_idx);
        }
        self.cursor_row = cursor_row.min(self.buffer.line_count().saturating_sub(1));
        self.cursor_col = 0;
        self.desired_col = 0;
        self.scroll_row = self.cursor_row.saturating_sub(4);

        self.clear_pending_change_state();

        self.set_status_message(format!(
            "AI applied change \"{}\" in {}",
            change.scenario_name, change.file_path
        ));

        Ok((change.tool_call_id, result))
    }

    /// Reject and discard the first pending agent change.
    ///
    /// Returns `(tool_call_id, result_text)` for feeding back to the LLM.
    pub fn reject_agent_change(&mut self) -> (String, String) {
        let change = self.pending_agent_changes.remove(0);
        self.clear_pending_change_state();
        let desc = change.description.clone();
        self.status = format!("Rejected AI change: {desc}");
        self.quit_pending_confirm = false;
        let result = format!("User rejected the change: {desc}");
        (change.tool_call_id, result)
    }

    fn clear_pending_change_state(&mut self) {
        self.explore_diff_lines = None;
        self.change_summary_visible = false;
        self.change_summary_selection = 0;
        if !self.pending_change_diffs.is_empty() {
            self.pending_change_diffs.remove(0);
        }
        if !self.pending_change_summary.is_empty() {
            self.pending_change_summary.remove(0);
        }
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

        // Re-apply gray-out for the current selection after rebuilding
        if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
            self.mindmap_index.apply_highlight_categories(id);
        }

        if self.active_tab == MainTab::MindMap
            && (self.view_stage == ViewStage::TreeAndEditor || self.mindmap_ai_panel_visible)
        {
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
        // Set focus to the selected scenario so the Editor dims other scenarios' steps
        if let Some(scenario) = self
            .project
            .features
            .get(self.explore_selected_feature)
            .and_then(|f| f.scenarios.get(self.explore_selected_scenario))
        {
            // scenario.line_number is 1-based, convert to 0-based row
            let scenario_row = scenario.line_number.saturating_sub(1);
            self.editor_focus_scenario_row = Some(scenario_row);
            // Position the scenario at the top of the editor
            self.scroll_row = scenario_row;
        } else {
            self.editor_focus_scenario_row = None;
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
        self.editor_focus_scenario_row = None;
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

        // Append location index for nodes appearing in multiple scenarios
        if let Some(id) = mindmap::selected_node_id(&self.tree_state)
            && let Some(locations) = self.mindmap_index.locations_for(id)
        {
            let count = locations.len();
            if count > 1 {
                let idx = self
                    .mindmap_location_selection
                    .get(id)
                    .copied()
                    .unwrap_or(0);
                // Prepend "(i/count)" before the colon separator, or append to Background
                if let Some(col) = self.preview_title.find(':') {
                    let base = &self.preview_title[..col];
                    let rest = &self.preview_title[col..];
                    self.preview_title = format!("{} ({}/{}){}", base, idx + 1, count, rest);
                } else {
                    // Background or no colon — just prepend
                    self.preview_title = format!("({}/{}) {}", idx + 1, count, self.preview_title);
                }
            }
        }
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
        if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
            self.mindmap_index.apply_highlight_categories(id);
        }
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_move_down(&mut self) {
        self.tree_state.key_down();
        if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
            self.mindmap_index.apply_highlight_categories(id);
        }
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_home(&mut self) {
        self.tree_state.select_first();
        if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
            self.mindmap_index.apply_highlight_categories(id);
        }
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_end(&mut self) {
        self.tree_state.select_last();
        if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
            self.mindmap_index.apply_highlight_categories(id);
        }
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    /// Keep editor preview in sync with tree selection.
    fn tree_follow_editor(&mut self) {
        if self.view_stage != ViewStage::TreeAndEditor && !self.mindmap_ai_panel_visible {
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
        let selected_path = self.tree_state.selected().to_vec();
        let is_opened = self.tree_state.opened().contains(&selected_path);

        if is_opened {
            // Already expanded: navigate into the first child
            if let Some(current_id) = mindmap::selected_node_id(&self.tree_state)
                && self.mindmap_index.has_children(current_id)
            {
                self.tree_state.key_down();
                if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
                    self.mindmap_index.apply_highlight_categories(id);
                }
            }
        } else {
            // Collapsed: expand the node to reveal children
            self.tree_state.key_right();
            if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
                self.mindmap_index.apply_highlight_categories(id);
            }
        }
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
        if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
            self.mindmap_index.apply_highlight_categories(id);
        }
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
            // AiBlurInput / AiFocusInput are allowed through: Esc unfocuses first,
            // Enter refocuses, and only then does a second Esc trigger AgentChangeReject.
            if matches!(action, Action::AiBlurInput | Action::AiFocusInput) {
                // fall through to normal handling below
            } else {
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
                    self.agent_mut().messages.push(AiChatMessage {
                        role: AiRole::User,
                        content: msg,
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                        source: Some("MindMap".into()),
                    });
                    self.active_tab = MainTab::Ai;
                    self.agent_mut().status = AiStatus::Waiting;
                    self.agent_mut().partial_response.clear();
                    self.agent_mut().scroll_offset = 0;
                    self.agent_mut().agent_loop_count = 0;
                    self.status = "Sending MindMap context to AI...".to_string();

                    if !crate::llm::LlmConfig::is_configured() {
                        self.agent_mut().messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content: "AI is not configured. Set TESHI_LLM_API_KEY in your environment to enable AI responses.".to_string(),
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: None,
                                source: None,
                            });
                        self.agent_mut().status = AiStatus::Idle;
                        self.agent_mut().partial_response.clear();
                        self.status = "AI not configured".to_string();
                    } else if let Some(ref handle) = self.agent().llm_handle {
                        let messages = self.build_chat_messages_for_agent(self.selected_agent);
                        let tools = Some(crate::agent::get_tools());
                        if handle
                            .send(crate::llm::LlmRequest::Chat {
                                system: Some(self.ai_system_prompt(None)),
                                messages,
                                tools,
                            })
                            .is_err()
                        {
                            self.agent_mut().status = AiStatus::Error;
                            self.agent_mut().partial_response.clear();
                            self.status = "AI error: background LLM thread has exited".to_string();
                        }
                    } else {
                        self.agent_mut().status = AiStatus::Error;
                        self.agent_mut().partial_response.clear();
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
            Action::ToggleScenarioDropdown => {
                if self.active_tab == MainTab::MindMap {
                    let id = mindmap::selected_node_id(&self.tree_state);
                    let has_multi = id
                        .and_then(|id| self.mindmap_index.locations_for(id))
                        .map(|l| l.len() > 1)
                        .unwrap_or(false);
                    if has_multi {
                        if self.scenario_dropdown_open {
                            self.scenario_dropdown_open = false;
                        } else {
                            self.scenario_dropdown_open = true;
                            if let Some(id) = id {
                                let entry = self
                                    .mindmap_location_selection
                                    .entry(id.to_string())
                                    .or_insert(0);
                                self.scenario_dropdown_selection = *entry;
                            }
                        }
                    }
                }
                self.quit_pending_confirm = false;
            }
            Action::ScenarioDropdownSelect => {
                if self.scenario_dropdown_open {
                    if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
                        self.mindmap_location_selection
                            .entry(id.to_string())
                            .and_modify(|e| *e = self.scenario_dropdown_selection);
                        self.rebuild_preview();
                    }
                    self.scenario_dropdown_open = false;
                }
                self.quit_pending_confirm = false;
            }
            Action::ScenarioDropdownUp => {
                if self.scenario_dropdown_open && self.scenario_dropdown_selection > 0 {
                    self.scenario_dropdown_selection -= 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::ScenarioDropdownDown => {
                if self.scenario_dropdown_open
                    && let Some(id) = mindmap::selected_node_id(&self.tree_state)
                    && let Some(locations) = self.mindmap_index.locations_for(id)
                    && self.scenario_dropdown_selection + 1 < locations.len()
                {
                    self.scenario_dropdown_selection += 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::ScenarioDropdownClose => {
                self.scenario_dropdown_open = false;
                self.quit_pending_confirm = false;
            }
            Action::AuthPanelClose => {
                self.auth_panel_active = false;
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelOpen => {
                self.model_panel_active = true;
                self.model_panel_selection = 0;
                self.model_panel_mode = ModelPanelMode::List;
                self.model_profiles = crate::profiles::ModelProfile::load_all();
                self.status = "Model profiles [m]. a add · ↑↓ select · Enter activate · Esc close"
                    .to_string();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelClose => {
                self.model_panel_active = false;
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelUp => {
                if self.model_panel_selection > 0 {
                    self.model_panel_selection -= 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelDown => {
                if self.model_panel_selection + 1 < self.model_profiles.len() {
                    self.model_panel_selection += 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelActivate => {
                if let Some(profile) = self.model_profiles.get(self.model_panel_selection).cloned()
                {
                    if let Err(e) = crate::profiles::ModelProfile::write_active_id(&profile.id) {
                        self.status = format!("Failed to save active profile: {e}");
                    } else {
                        self.model_active_id = Some(profile.id.clone());
                        self.activate_model_profile(&profile);
                        self.model_panel_active = false;
                    }
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelAdd => {
                self.model_panel_mode = ModelPanelMode::Adding;
                self.model_form_focus = 0;
                self.model_form_name.clear();
                self.model_form_provider.clear();
                self.model_form_model.clear();
                self.model_form_base_url.clear();
                self.model_form_api_key.clear();
                self.model_form_max_tokens = String::from("4096");
                self.model_form_temperature = String::from("0.7");
                self.status =
                    "Fill in the fields and press Enter to save. Tab to switch fields.".to_string();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelDelete => {
                if let Some(profile) = self.model_profiles.get(self.model_panel_selection).cloned()
                {
                    let name = profile.name.clone();
                    if let Err(e) = profile.delete_from_disk() {
                        self.status = format!("Failed to delete profile: {e}");
                    } else {
                        self.model_profiles = crate::profiles::ModelProfile::load_all();
                        if self.model_panel_selection >= self.model_profiles.len() {
                            self.model_panel_selection =
                                self.model_profiles.len().saturating_sub(1);
                        }
                        // If the deleted profile was active, clear the active state
                        if self.model_active_id.as_deref() == Some(&profile.id) {
                            self.model_active_id = None;
                            self.active_model_label = None;
                            // Fall back to env-var config
                            self.spawn_llm_if_configured();
                        }
                        self.status = format!("Deleted profile: {name}");
                    }
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormCancel => {
                self.model_panel_mode = ModelPanelMode::List;
                self.status = "Model profiles [m]. a add · ↑↓ select · Enter activate · Esc close"
                    .to_string();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormNext => {
                let max = 7usize;
                if self.model_form_focus < max {
                    self.model_form_focus += 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormPrev => {
                if self.model_form_focus > 0 {
                    self.model_form_focus -= 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormInsert(ch) => {
                let field = match self.model_form_focus {
                    0 => &mut self.model_form_name,
                    1 => &mut self.model_form_provider,
                    2 => &mut self.model_form_model,
                    3 => &mut self.model_form_base_url,
                    4 => &mut self.model_form_api_key,
                    5 => &mut self.model_form_max_tokens,
                    6 => &mut self.model_form_temperature,
                    _ => return Ok(()),
                };
                field.push(ch);
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormBackspace => {
                let field = match self.model_form_focus {
                    0 => &mut self.model_form_name,
                    1 => &mut self.model_form_provider,
                    2 => &mut self.model_form_model,
                    3 => &mut self.model_form_base_url,
                    4 => &mut self.model_form_api_key,
                    5 => &mut self.model_form_max_tokens,
                    6 => &mut self.model_form_temperature,
                    _ => return Ok(()),
                };
                field.pop();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormSubmit => {
                let name = self.model_form_name.trim().to_string();
                if name.is_empty() {
                    self.status = "Name is required.".to_string();
                    self.quit_pending_confirm = false;
                    return Ok(());
                }
                let provider = if self.model_form_provider.trim().is_empty() {
                    "openai".to_string()
                } else {
                    self.model_form_provider.trim().to_string()
                };
                let model = self.model_form_model.trim().to_string();
                if model.is_empty() {
                    self.status = "Model identifier is required.".to_string();
                    self.quit_pending_confirm = false;
                    return Ok(());
                }
                let base_url = if self.model_form_base_url.trim().is_empty() {
                    "https://api.openai.com/v1".to_string()
                } else {
                    self.model_form_base_url.trim().to_string()
                };
                let api_key = self.model_form_api_key.trim().to_string();
                let max_tokens: u32 = self.model_form_max_tokens.trim().parse().unwrap_or(4096);
                let temperature: f32 = self.model_form_temperature.trim().parse().unwrap_or(0.7);

                let mut profile =
                    crate::profiles::ModelProfile::new(&name, &provider, &model, &base_url);
                profile.api_key = api_key;
                profile.max_tokens = max_tokens;
                profile.temperature = temperature;

                if let Err(e) = profile.save_to_disk() {
                    self.status = format!("Failed to save profile: {e}");
                    self.quit_pending_confirm = false;
                    return Ok(());
                }

                // Auto-activate the new profile
                if let Err(e) = crate::profiles::ModelProfile::write_active_id(&profile.id) {
                    self.status = format!("Profile saved but failed to activate: {e}");
                } else {
                    self.model_active_id = Some(profile.id.clone());
                    self.activate_model_profile(&profile);
                    self.status = format!("Added and activated profile: {}", profile.name);
                }

                self.model_profiles = crate::profiles::ModelProfile::load_all();
                self.model_panel_mode = ModelPanelMode::List;
                self.model_panel_selection = 0;
                self.quit_pending_confirm = false;
            }
            // ── Session browser panel actions ─────────────────
            Action::SessionPanelOpen => {
                return self.cmd_sessions();
            }
            Action::SessionPanelClose => {
                self.session_panel_active = false;
                self.quit_pending_confirm = false;
            }
            Action::SessionPanelUp => {
                if self.session_panel_selection > 0 {
                    self.session_panel_selection -= 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::SessionPanelDown => {
                if self.session_panel_selection + 1 < self.session_list.len() {
                    self.session_panel_selection += 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::SessionPanelActivate => {
                if let Some(session) = self.session_list.get(self.session_panel_selection).cloned()
                {
                    self.agent_mut().messages = session.messages;
                    self.agent_mut().partial_response.clear();
                    self.agent_mut().input.clear();
                    self.agent_mut().input_cursor = 0;
                    self.agent_mut().status = AiStatus::Idle;
                    self.agent_mut().scroll_offset = 0;
                    self.session_panel_active = false;
                    self.status = format!(
                        "Loaded session with {} messages",
                        self.agent().messages.len()
                    );
                }
                self.quit_pending_confirm = false;
            }
            Action::SessionPanelDelete => {
                if let Some(session) = self.session_list.get(self.session_panel_selection).cloned()
                {
                    let id = session.id.clone();
                    if let Err(e) = crate::session::Session::delete(&id) {
                        self.status = format!("Failed to delete session: {e}");
                    } else {
                        self.session_list = crate::session::Session::load_all();
                        if self.session_panel_selection >= self.session_list.len() {
                            self.session_panel_selection =
                                self.session_list.len().saturating_sub(1);
                        }
                        self.status = format!("Deleted session {id}");
                    }
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
                if current_step_keyword_index(&line, self.buffer.language()).is_none() {
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
            Action::AiPaste(text) => {
                self.ai_input_focused = true;
                // Normalize Windows line endings
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                let byte_idx = char_to_byte_idx(&self.agent().input, self.agent().input_cursor);
                self.agent_mut().input.insert_str(byte_idx, &text);
                self.agent_mut().input_cursor += text.chars().count();
                self.quit_pending_confirm = false;
            }
            Action::AiSendChar(ch) => {
                self.ai_input_focused = true;
                let byte_idx = char_to_byte_idx(&self.agent().input, self.agent().input_cursor);
                self.agent_mut().input.insert(byte_idx, ch);
                self.agent_mut().input_cursor += 1;
                self.quit_pending_confirm = false;
                // Activate slash command suggestions when "/" is typed
                if self.agent().input.starts_with('/') && !self.slash_suggestion_active {
                    self.slash_suggestion_active = true;
                    self.slash_suggestion_selection = 0;
                }
                // Reset selection as user types — filtered list changes
                if self.slash_suggestion_active {
                    self.slash_suggestion_selection = 0;
                }
            }
            Action::AiSendMessage => {
                if self.agent().input.trim().is_empty() {
                    self.ai_input_focused = false;
                    self.status = "Input deactivated".to_string();
                    return Ok(());
                }
                if self.agent().status == AiStatus::Waiting {
                    return Ok(());
                }

                let user_msg = std::mem::take(&mut self.agent_mut().input);
                self.agent_mut().input_cursor = 0;

                // Intercept slash commands before sending to LLM
                if let Some(cmd) = user_msg.strip_prefix('/') {
                    self.slash_suggestion_active = false;
                    self.slash_suggestion_selection = 0;
                    let cmd = cmd.trim();
                    if cmd == "auth" || cmd.starts_with("auth ") {
                        self.open_auth_panel(cmd.strip_prefix("auth ").unwrap_or(""));
                        return Ok(());
                    }
                    if cmd == "new" {
                        return self.cmd_new();
                    }
                    if cmd == "exit" || cmd == "quit" {
                        return self.cmd_exit();
                    }
                    if cmd == "resume" {
                        return self.cmd_resume();
                    }
                    if cmd == "copy" || cmd.starts_with("copy ") {
                        let n = cmd
                            .strip_prefix("copy ")
                            .and_then(|s| s.trim().parse().ok())
                            .unwrap_or(1);
                        return self.cmd_copy(n);
                    }
                    if cmd == "models" || cmd == "model" {
                        return self.cmd_models();
                    }
                    if cmd == "sessions" || cmd == "session" {
                        return self.cmd_sessions();
                    }
                    self.status = "Unknown slash command. Try /new, /exit, /resume, /copy, /models, /sessions".to_string();
                    return Ok(());
                }

                self.agent_mut().scroll_offset = 0;
                self.agent_mut().messages.push(AiChatMessage {
                    role: AiRole::User,
                    content: user_msg.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                    source: None,
                });
                // Auto-rename from "Agent N" to the first user message
                if self.agent().title.starts_with("Agent ") {
                    let name = user_msg
                        .chars()
                        .take(30)
                        .collect::<String>()
                        .trim()
                        .to_string();
                    if !name.is_empty() {
                        self.agent_mut().title = name;
                    }
                }
                self.agent_mut().status = AiStatus::Waiting;
                self.agent_mut().partial_response.clear();
                self.agent_mut().agent_loop_count = 0;
                self.status = "Sending message to AI...".to_string();

                // If the LLM is not configured, add a mock response
                if !crate::llm::LlmConfig::is_configured()
                    && self.config.default_provider_config().is_none()
                {
                    self.agent_mut().messages.push(AiChatMessage {
                        role: AiRole::Assistant,
                        content: "AI is not configured. Run 'teshi auth login' to configure a provider, or set TESHI_LLM_API_KEY in your environment.".to_string(),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                        source: None,
                    });
                    self.agent_mut().status = AiStatus::Idle;
                    self.agent_mut().partial_response.clear();
                    self.status = "AI not configured".to_string();
                } else if self.agent().llm_handle.is_some() {
                    // Compact context before sending to avoid exceeding token limits
                    self.compact_context_if_needed(self.selected_agent);
                    use crate::llm::LlmRequest;
                    let messages = self.build_chat_messages_for_agent(self.selected_agent);
                    let tools = Some(crate::agent::get_tools());
                    let handle = self.agent().llm_handle.as_ref().unwrap();
                    if handle
                        .send(LlmRequest::Chat {
                            system: Some(self.ai_system_prompt(Some(&user_msg))),
                            messages,
                            tools,
                        })
                        .is_err()
                    {
                        self.agent_mut().status = AiStatus::Error;
                        self.agent_mut().partial_response.clear();
                        self.status = "AI error: background LLM thread has exited".to_string();
                    }
                } else {
                    // LLM is configured but the handle is None — shouldn't happen normally.
                    self.agent_mut().status = AiStatus::Error;
                    self.agent_mut().partial_response.clear();
                    self.status = "AI error: LLM handle not available".to_string();
                }
                self.quit_pending_confirm = false;
            }
            Action::AiBackspace => {
                if self.agent().input_cursor > 0 {
                    let byte_idx =
                        char_to_byte_idx(&self.agent().input, self.agent().input_cursor - 1);
                    self.agent_mut().input.remove(byte_idx);
                    self.agent_mut().input_cursor -= 1;
                }
                self.quit_pending_confirm = false;
                // Reset selection as backspace changes the filter
                if self.slash_suggestion_active {
                    self.slash_suggestion_selection = 0;
                }
                // Dismiss slash suggestions if input no longer starts with "/"
                if self.slash_suggestion_active && !self.agent().input.starts_with('/') {
                    self.slash_suggestion_active = false;
                    self.slash_suggestion_selection = 0;
                }
            }
            Action::AiDelete => {
                if self.agent().input_cursor < self.agent().input.chars().count() {
                    let byte_idx = char_to_byte_idx(&self.agent().input, self.agent().input_cursor);
                    self.agent_mut().input.remove(byte_idx);
                }
                self.quit_pending_confirm = false;
            }
            Action::AiCursorLeft => {
                self.agent_mut().input_cursor = self.agent().input_cursor.saturating_sub(1);
                self.quit_pending_confirm = false;
            }
            Action::AiCursorRight => {
                let max = self.agent().input.chars().count();
                self.agent_mut().input_cursor = (self.agent().input_cursor + 1).min(max);
                self.quit_pending_confirm = false;
            }
            Action::AiCursorHome => {
                self.agent_mut().input_cursor = 0;
                self.quit_pending_confirm = false;
            }
            Action::AiCursorEnd => {
                self.agent_mut().input_cursor = self.agent().input.chars().count();
                self.quit_pending_confirm = false;
            }
            Action::AiScrollUp => {
                self.agent_mut().scroll_offset = self.agent().scroll_offset.saturating_add(5);
            }
            Action::AiScrollDown => {
                self.agent_mut().scroll_offset = self.agent().scroll_offset.saturating_sub(5);
            }
            Action::AiScrollLeft => {
                self.agent_mut().horizontal_scroll =
                    self.agent().horizontal_scroll.saturating_sub(5);
            }
            Action::AiScrollRight => {
                self.agent_mut().horizontal_scroll =
                    self.agent().horizontal_scroll.saturating_add(5);
            }
            Action::AiScrollTop => {
                self.agent_mut().scroll_offset = usize::MAX;
            }
            Action::AiScrollBottom => {
                self.agent_mut().scroll_offset = 0;
            }
            Action::AiSlashPrev => {
                self.slash_suggestion_selection = self.slash_suggestion_selection.saturating_sub(1);
                self.quit_pending_confirm = false;
            }
            Action::AiSlashNext => {
                let max = crate::app::SLASH_COMMANDS.len().saturating_sub(1);
                if self.slash_suggestion_selection < max {
                    self.slash_suggestion_selection += 1;
                }
                self.quit_pending_confirm = false;
            }
            Action::AiSlashSelect => {
                self.quit_pending_confirm = false;
                // Filter commands the same way the UI does
                let filter = self
                    .agent()
                    .input
                    .strip_prefix('/')
                    .unwrap_or("")
                    .to_lowercase();
                let filtered: Vec<&(&str, &str)> = crate::app::SLASH_COMMANDS
                    .iter()
                    .filter(|(name, _)| filter.is_empty() || name.starts_with(&filter))
                    .collect();
                if self.slash_suggestion_selection < filtered.len() {
                    let (name, _) = filtered[self.slash_suggestion_selection];
                    self.slash_suggestion_active = false;
                    self.slash_suggestion_selection = 0;
                    // Execute the command directly
                    return match *name {
                        "new" => self.cmd_new(),
                        "exit" => self.cmd_exit(),
                        "resume" => self.cmd_resume(),
                        "copy" => {
                            self.agent_mut().input = "/copy ".into();
                            self.agent_mut().input_cursor = self.agent().input.len();
                            Ok(())
                        }
                        "models" => self.cmd_models(),
                        "sessions" => self.cmd_sessions(),
                        _ => Ok(()),
                    };
                }
                // If nothing matches, keep the popup open so the user can keep typing
            }
            Action::AiSlashDismiss => {
                self.slash_suggestion_active = false;
                self.slash_suggestion_selection = 0;
                self.quit_pending_confirm = false;
            }
            Action::AiBlurInput => {
                self.ai_input_focused = false;
                self.slash_suggestion_active = false;
                self.slash_suggestion_selection = 0;
            }
            Action::AiCancel => {
                if self.agent().status == AiStatus::Waiting
                    || self.agent().status == AiStatus::AwaitingApproval
                {
                    // Cancel the LLM request
                    if let Some(ref handle) = self.agent().llm_handle {
                        handle.cancel();
                    }
                    // Reset agent state
                    let agent = self.agent_mut();
                    agent.status = AiStatus::Idle;
                    agent.partial_response.clear();
                    agent.tool_status = None;
                    // Clear any tool call state by sending a cancellation message
                    agent.messages.push(AiChatMessage {
                        role: AiRole::Assistant,
                        content: "[cancelled]".to_string(),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                        source: None,
                    });
                    self.status = "AI request cancelled".to_string();
                }
                self.selection_anchor = None;
                self.selection_end = None;
                self.quit_pending_confirm = false;
            }
            Action::AiFocusInput => {
                self.ai_input_focused = true;
            }
            Action::AgentNew => {
                let id = self.next_agent_id;
                self.next_agent_id += 1;
                self.agents
                    .push(AgentThread::new(id, &format!("Agent {id}")));
                self.selected_agent = self.agents.len() - 1;
                self.ai_input_focused = true;
                self.spawn_llm_if_configured();
                self.status = format!("Created agent {id}");
                self.quit_pending_confirm = false;
            }
            Action::AgentClose => {
                if self.agents.len() <= 1 {
                    self.status = "Cannot close the last agent".to_string();
                } else {
                    let name = self.agents[self.selected_agent].title.clone();
                    self.agents.remove(self.selected_agent);
                    if self.selected_agent >= self.agents.len() {
                        self.selected_agent = self.agents.len().saturating_sub(1);
                    }
                    self.status = format!("Closed {name}");
                }
                self.quit_pending_confirm = false;
            }
            Action::AgentNext => {
                if self.agents.len() > 1 {
                    self.selected_agent = (self.selected_agent + 1) % self.agents.len();
                    self.status = format!("Switched to {}", self.agent().title);
                }
                self.quit_pending_confirm = false;
            }
            Action::AgentPrev => {
                if self.agents.len() > 1 {
                    self.selected_agent = self.selected_agent.saturating_sub(1);
                    self.status = format!("Switched to {}", self.agent().title);
                }
                self.quit_pending_confirm = false;
            }
            Action::ExternalChangeReload | Action::ExternalChangeKeepLocal => {}
            Action::CopySelection => self.copy_selection_to_clipboard()?,
            Action::ClearSelection => {
                self.selection_anchor = None;
                self.selection_end = None;
            }
            // Handled in early-return guard above; unreachable here.
            Action::AgentChangeAccept | Action::AgentChangeReject => {}
            Action::AgentChangeDiff => self.toggle_explore_diff_view(),
            Action::ToggleChangeSummary => self.toggle_change_summary(),
            Action::ChangeSummaryUp => {
                if self.change_summary_visible && !self.pending_change_summary.is_empty() {
                    self.change_summary_selection = self.change_summary_selection.saturating_sub(1);
                }
            }
            Action::ChangeSummaryDown => {
                if self.change_summary_visible {
                    let max = self.pending_change_summary.len().saturating_sub(1);
                    self.change_summary_selection = (self.change_summary_selection + 1).min(max);
                }
            }
            Action::ChangeSummaryJump => {
                if self.change_summary_visible
                    && let Some(node_id) = self
                        .pending_change_summary
                        .get(self.change_summary_selection)
                        .map(|n| n.node_id.clone())
                {
                    self.navigate_to_mindmap_node(&node_id);
                }
            }
            Action::ClearInputState => {
                self.selection_anchor = None;
                self.selection_end = None;
                self.scenario_dropdown_open = false;
                if self.change_summary_visible {
                    self.change_summary_visible = false;
                } else if self.active_tab == MainTab::MindMap
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
                    // If AI is streaming, cancel the LLM request
                    if self.agent().status == AiStatus::Waiting
                        || self.agent().status == AiStatus::AwaitingApproval
                    {
                        if let Some(ref handle) = self.agent().llm_handle {
                            handle.cancel();
                        }
                        let agent = self.agent_mut();
                        agent.partial_response.clear();
                        agent.tool_status = None;
                        agent.messages.push(AiChatMessage {
                            role: AiRole::Assistant,
                            content: "[cancelled]".to_string(),
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: None,
                            source: None,
                        });
                        self.status = "AI request cancelled".to_string();
                    } else {
                        self.status = "Input cleared".to_string();
                    }
                    self.agent_mut().input.clear();
                    self.agent_mut().input_cursor = 0;
                    self.agent_mut().partial_response.clear();
                    self.agent_mut().status = AiStatus::Idle;
                    self.slash_suggestion_active = false;
                    self.slash_suggestion_selection = 0;
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
            self.status = "Unsaved changes. Press q or Ctrl+C again to quit.".to_string();
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
        self.scenario_dropdown_open = false;
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
            self.ai_input_focused = false;
        }
        if self.active_tab == MainTab::Ai {
            self.ai_input_focused = false;
        }
        self.status = match tab {
            MainTab::MindMap => "Switched to MindMap tab",
            MainTab::Explore => "Switched to Explore tab",
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
                if let Some(idx) = current_step_keyword_index(&line, self.buffer.language()) {
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
        if let Some(new_line) = replace_step_keyword_line(&line, keyword, self.buffer.language()) {
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
        self.selection_anchor = None;
        self.selection_end = None;
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
                    if crate::bdd_nav::step_edit_start_col(&line, self.buffer.language()).is_some()
                        || self
                            .buffer
                            .language()
                            .match_structural_prefix(trimmed)
                            .is_some_and(|(_, st)| {
                                matches!(
                                    st,
                                    StructuralType::Scenario
                                        | StructuralType::ScenarioOutline
                                        | StructuralType::Background
                                        | StructuralType::Feature
                                )
                            })
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

    fn copy_selection_to_clipboard(&mut self) -> Result<()> {
        let text = self.selection_text();
        if text.is_empty() {
            self.status = "No selection to copy".to_string();
            return Ok(());
        }
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| anyhow::anyhow!("failed to open clipboard: {e}"))?;
        clipboard
            .set_text(&text)
            .map_err(|e| anyhow::anyhow!("failed to set clipboard: {e}"))?;
        let line_count = text.lines().count();
        self.status = format!("Copied {line_count} line(s)");
        self.selection_anchor = None;
        self.selection_end = None;
        self.clipboard = Some(text);
        Ok(())
    }

    /// Return the selected text (empty string if no active selection).
    fn selection_text(&self) -> String {
        let (anchor, end) = match (self.selection_anchor, self.selection_end) {
            (Some(a), Some(e)) => (a, e),
            _ => return String::new(),
        };
        self.buffer.text_range(anchor, end)
    }

    /// Handle a mouse event from crossterm.
    ///
    /// Dispatches to clickable UI regions first, then falls back to
    /// editor text-selection and AI-chat scroll behaviour.
    pub fn handle_mouse_event(
        &mut self,
        kind: MouseEventKind,
        col: u16,
        row: u16,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        // Update hover tracking
        let pos = ratatui::layout::Position::new(col, row);

        match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // 1. Try clickable UI regions (tabs, tree, explore columns, etc.)
                if let Some(region) = self.hit_test(&pos).cloned() {
                    return self.handle_region_click(&region, &pos);
                }

                // 2. Fall through to editor text selection
                if let Some((buf_row, buf_col)) = self.screen_to_buffer_pos(col, row) {
                    self.selection_anchor = Some((buf_row, buf_col));
                    self.selection_end = Some((buf_row, buf_col));
                    // In editor mode, clicking on a scenario header focuses that scenario
                    if self.is_editor_active() && buf_row < self.buffer.line_count() {
                        let clicked_line = self.buffer.line(buf_row).trim_start().to_string();
                        if self
                            .buffer
                            .language()
                            .match_structural_prefix(&clicked_line)
                            .is_some_and(|(_, st)| {
                                matches!(
                                    st,
                                    StructuralType::Scenario | StructuralType::ScenarioOutline
                                )
                            })
                        {
                            self.editor_focus_scenario_row = Some(buf_row);
                            self.scroll_row = buf_row;
                            self.status = "Focused scenario".to_string();
                        }
                    }
                } else {
                    self.selection_anchor = None;
                    self.selection_end = None;
                }
            }
            MouseEventKind::Drag(button) => {
                if button == MouseButton::Left
                    && let Some((buf_row, buf_col)) = self.screen_to_buffer_pos(col, row)
                {
                    self.selection_end = Some((buf_row, buf_col));
                }
            }
            MouseEventKind::Up(MouseButton::Right) => {
                // Try clickable regions first (e.g. right-click on a tab/item)
                if let Some(region) = self.hit_test(&pos).cloned() {
                    return self.handle_region_click(&region, &pos);
                }
                // Fall through: copy selection
                self.handle_action(Action::CopySelection)?;
            }
            MouseEventKind::ScrollUp => {
                // Editor panel scroll
                if let Some(rect) = self.editor_panel_rect
                    && rect.contains(pos)
                {
                    if self.scroll_row > 0 {
                        self.scroll_row = self.scroll_row.saturating_sub(1);
                        self.quit_pending_confirm = false;
                    }
                    return Ok(());
                }
                // Preview panel scroll
                if let Some(rect) = self.preview_panel_rect
                    && rect.contains(pos)
                {
                    if self.preview_cursor_row > 0 {
                        self.preview_cursor_row = self.preview_cursor_row.saturating_sub(1);
                    }
                    return Ok(());
                }
                // Fall through to AI context scroll
                let in_ai_context = self.active_tab == MainTab::Ai
                    || (self.active_tab == MainTab::MindMap
                        && self.mindmap_focus == MindMapFocus::AiPanel);
                if in_ai_context && !self.auth_panel_active {
                    if modifiers.contains(KeyModifiers::SHIFT) {
                        self.agent_mut().horizontal_scroll =
                            self.agent().horizontal_scroll.saturating_sub(3);
                    } else {
                        self.handle_action(Action::AiScrollUp)?;
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                // Editor panel scroll
                if let Some(rect) = self.editor_panel_rect
                    && rect.contains(pos)
                {
                    let max_scroll = self.buffer.line_count().saturating_sub(1);
                    self.scroll_row = self.scroll_row.saturating_add(1).min(max_scroll);
                    self.quit_pending_confirm = false;
                    return Ok(());
                }
                // Preview panel scroll
                if let Some(rect) = self.preview_panel_rect
                    && rect.contains(pos)
                {
                    self.preview_cursor_row = self.preview_cursor_row.saturating_add(1);
                    return Ok(());
                }
                // Fall through to AI context scroll
                let in_ai_context = self.active_tab == MainTab::Ai
                    || (self.active_tab == MainTab::MindMap
                        && self.mindmap_focus == MindMapFocus::AiPanel);
                if in_ai_context && !self.auth_panel_active {
                    if modifiers.contains(KeyModifiers::SHIFT) {
                        self.agent_mut().horizontal_scroll =
                            self.agent().horizontal_scroll.saturating_add(3);
                    } else {
                        self.handle_action(Action::AiScrollDown)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Find the first clickable region that contains `pos`.
    fn hit_test(&self, pos: &ratatui::layout::Position) -> Option<&ClickableRegion> {
        // Tab bar: row is always 0, check x-position against known label widths
        if pos.y == 0 {
            let tab_bar_x = 0; // tab bar starts at column 0
            let tab_labels = [
                (MainTab::Explore, " Explore [1] ", 0u16),
                (MainTab::MindMap, " MindMap [2] ", 15u16),
                (MainTab::Ai, " AI [3] ", 30u16),
            ];
            for &(ref tab, label, start_x) in &tab_labels {
                let end_x = start_x + label.chars().count() as u16;
                if pos.x >= tab_bar_x + start_x && pos.x < tab_bar_x + end_x {
                    // Return a matching Tab region from clickable_regions
                    return self
                        .clickable_regions
                        .iter()
                        .find(|r| matches!(r, ClickableRegion::Tab(t) if *t == *tab));
                }
            }
        }

        for region in &self.clickable_regions {
            match region {
                ClickableRegion::Tab(_) => {
                    // Already handled above
                }
                ClickableRegion::Tree => {
                    if let Some(rect) = self.tree_panel_rect
                        && rect.contains(*pos)
                    {
                        return Some(region);
                    }
                }
                ClickableRegion::ExploreFeature {
                    row_y,
                    col_x,
                    col_right,
                    ..
                } => {
                    if *row_y == pos.y && pos.x >= *col_x && pos.x < *col_right {
                        return Some(region);
                    }
                }
                ClickableRegion::ExploreScenario {
                    row_y,
                    col_x,
                    col_right,
                    ..
                } => {
                    if *row_y == pos.y && pos.x >= *col_x && pos.x < *col_right {
                        return Some(region);
                    }
                }
                ClickableRegion::ExploreStep {
                    row_y,
                    col_x,
                    col_right,
                    ..
                } => {
                    if *row_y == pos.y && pos.x >= *col_x && pos.x < *col_right {
                        return Some(region);
                    }
                }
                ClickableRegion::EditorPanel => {
                    if let Some(rect) = self.editor_panel_rect
                        && rect.contains(*pos)
                    {
                        return Some(region);
                    }
                }
                ClickableRegion::PreviewPanel => {
                    if let Some(rect) = self.preview_panel_rect
                        && rect.contains(*pos)
                    {
                        return Some(region);
                    }
                }
            }
        }
        None
    }

    /// Execute the action associated with clicking on a UI region.
    fn handle_region_click(
        &mut self,
        region: &ClickableRegion,
        pos: &ratatui::layout::Position,
    ) -> Result<()> {
        match region {
            ClickableRegion::Tab(tab) => {
                self.active_tab = *tab;
                self.status = format!("Switched to {tab:?} tab");
            }
            ClickableRegion::Tree => {
                // tui-tree-widget uses absolute terminal coordinates internally
                if self.tree_state.click_at(*pos)
                    && let Some(id) = crate::mindmap::selected_node_id(&self.tree_state)
                {
                    self.mindmap_index.apply_highlight_categories(id);
                    // rebuild preview
                    if self.view_stage == crate::app::ViewStage::TreeAndEditor {
                        self.rebuild_preview();
                    }
                }
            }
            ClickableRegion::ExploreFeature { feature_idx, .. } => {
                self.explore_selected_feature = *feature_idx;
                self.explore_focus = crate::app::ColumnFocus::Feature;
                // Reset scenario/step selection
                self.explore_selected_scenario = 0;
                self.explore_selected_step = 0;
            }
            ClickableRegion::ExploreScenario { scenario_idx, .. } => {
                self.explore_selected_scenario = *scenario_idx;
                self.explore_focus = crate::app::ColumnFocus::Scenario;
                self.explore_selected_step = 0;
            }
            ClickableRegion::ExploreStep { step_idx, .. } => {
                self.explore_selected_step = *step_idx;
                self.explore_focus = crate::app::ColumnFocus::Step;
            }
            _ => {}
        }
        Ok(())
    }

    /// Convert screen coordinates to buffer (row, col).
    ///
    /// Returns `None` if the click is outside the editor panel area or not on a
    /// valid buffer row.
    fn screen_to_buffer_pos(&self, screen_col: u16, screen_row: u16) -> Option<(usize, usize)> {
        let rect = self.editor_panel_rect?;
        if screen_col < rect.x || screen_row < rect.y {
            return None;
        }
        let rel_row = screen_row.saturating_sub(rect.y) as usize;
        let rel_col = screen_col.saturating_sub(rect.x) as usize;
        if rel_row >= rect.height as usize || rel_col >= rect.width as usize {
            return None;
        }
        let visible_rows = self.visible_editor_rows();
        let scroll_idx = visible_rows
            .iter()
            .position(|&row| row == self.scroll_row)
            .or_else(|| visible_rows.iter().position(|&row| row >= self.scroll_row))
            .unwrap_or(0);
        let visible_idx = scroll_idx.saturating_add(rel_row);
        let buf_row = visible_rows.get(visible_idx).copied()?;
        let buf_col = rel_col.min(self.buffer.line_len_chars(buf_row));
        Some((buf_row, buf_col))
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
                self.buffer
                    .language()
                    .match_structural_prefix(trimmed)
                    .is_some_and(|(_, st)| {
                        matches!(
                            st,
                            StructuralType::Scenario | StructuralType::ScenarioOutline
                        )
                    })
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
        let all_kw = self.buffer.language().all_step_keywords();
        let len = all_kw.iter().filter(|kw| kw.as_str() != "*").count();
        let i = p.selected as isize + delta;
        p.selected = i.clamp(0, len as isize - 1) as usize;
        self.quit_pending_confirm = false;
    }

    fn confirm_step_keyword_picker(&mut self) {
        let Some(picker) = self.step_keyword_picker else {
            return;
        };
        let line = self.buffer.line(picker.buffer_row);
        let all_kw = self.buffer.language().all_step_keywords();
        let kws: Vec<&str> = all_kw
            .iter()
            .filter(|kw| kw.as_str() != "*")
            .map(|s| s.as_str())
            .collect();
        let new_kw = kws[picker.selected];
        if let Some(new_line) = replace_step_keyword_line(&line, new_kw, self.buffer.language()) {
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

impl App {
    /// Opens the auth management panel in the TUI.
    ///
    /// Parses `subcommand` (the text after `/auth` in the slash command) to
    /// determine which view to show. Currently supports:
    /// - `""` → overview (list providers + status)
    /// - `"add"` → add mode
    /// - `"status"` → status mode
    /// - `"remove <provider>"` → remove confirmation
    fn open_auth_panel(&mut self, subcommand: &str) {
        self.auth_panel_active = true;
        self.status = match subcommand {
            "" => "Auth panel opened. Press Esc to close.".into(),
            "add" => "Auth: add provider mode.".into(),
            "status" => "Auth: showing provider status.".into(),
            cmd if cmd.starts_with("remove ") => {
                let provider = cmd.strip_prefix("remove ").unwrap_or("?");
                format!("Auth: remove provider '{}'.", provider)
            }
            _ => format!(
                "Unknown auth subcommand: '{}'. Try: auth, auth add, auth status, auth remove <provider>",
                subcommand
            ),
        };
    }

    // ── Slash command handlers ─────────────────────────────────────────

    /// Handle `/new` — start a new session.
    fn cmd_new(&mut self) -> Result<()> {
        // Save current session if there are messages
        if !self.agent().messages.is_empty() {
            let session = crate::session::Session::from_messages(
                std::mem::take(&mut self.agent_mut().messages),
                self.active_model_label.clone(),
            );
            let _ = session.save();
        }
        self.agent_mut().messages.clear();
        self.agent_mut().partial_response.clear();
        self.agent_mut().input.clear();
        self.agent_mut().input_cursor = 0;
        self.agent_mut().status = AiStatus::Idle;
        self.agent_mut().tool_status = None;
        self.agent_mut().scroll_offset = 0;
        self.agent_mut().agent_loop_count = 0;
        self.status = "New session started".to_string();
        Ok(())
    }

    /// Handle `/exit` or `/quit` — exit the application.
    fn cmd_exit(&mut self) -> Result<()> {
        // Save current session if there are messages
        if !self.agent().messages.is_empty() {
            let session = crate::session::Session::from_messages(
                std::mem::take(&mut self.agent_mut().messages),
                self.active_model_label.clone(),
            );
            let _ = session.save();
        }
        self.should_quit = true;
        Ok(())
    }

    /// Handle `/resume` — load the most recent session.
    fn cmd_resume(&mut self) -> Result<()> {
        let sessions = crate::session::Session::load_all();
        if let Some(s) = sessions.into_iter().next() {
            self.agent_mut().messages = s.messages;
            self.agent_mut().partial_response.clear();
            self.agent_mut().input.clear();
            self.agent_mut().input_cursor = 0;
            self.agent_mut().status = AiStatus::Idle;
            self.agent_mut().scroll_offset = 0;
            self.status = format!(
                "Resumed session from {} messages",
                self.agent().messages.len()
            );
        } else {
            self.status = "No saved sessions found".to_string();
        }
        Ok(())
    }

    /// Handle `/copy [N]` — copy last N assistant responses to clipboard.
    fn cmd_copy(&mut self, n: usize) -> Result<()> {
        let n = n.max(1);
        let texts: Vec<&str> = self
            .agent()
            .messages
            .iter()
            .rev()
            .filter(|m| m.role == AiRole::Assistant)
            .take(n)
            .map(|m| m.content.as_str())
            .collect();
        if texts.is_empty() {
            self.status = "No assistant responses to copy".to_string();
            return Ok(());
        }
        let combined = texts.join("\n\n---\n\n");
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| anyhow::anyhow!("failed to open clipboard: {e}"))?;
        clipboard
            .set_text(&combined)
            .map_err(|e| anyhow::anyhow!("failed to set clipboard: {e}"))?;
        self.status = format!(
            "Copied last {} assistant response(s) to clipboard",
            texts.len()
        );
        Ok(())
    }

    /// Handle `/models` — open the model profile panel.
    fn cmd_models(&mut self) -> Result<()> {
        self.model_panel_active = true;
        self.model_panel_selection = 0;
        self.model_panel_mode = ModelPanelMode::List;
        self.model_profiles = crate::profiles::ModelProfile::load_all();
        self.status =
            "Model profiles [m]. a add · ↑↓ select · Enter activate · Esc close".to_string();
        Ok(())
    }

    /// Handle `/sessions` — open the session browser panel.
    fn cmd_sessions(&mut self) -> Result<()> {
        self.session_panel_active = true;
        self.session_panel_selection = 0;
        self.session_list = crate::session::Session::load_all();
        self.status = "Sessions. ↑↓ select · Enter load · d delete · Esc close".to_string();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use super::{
        AgentThread, App, BddFocusSlot, ColumnFocus, MainTab, MindMapFocus, ModelPanelMode,
        ViewStage, current_step_keyword_index, replace_step_keyword_line,
    };
    use crate::bdd_nav::step_edit_start_col;
    use crate::editor_buffer::EditorBuffer;
    use crate::gherkin_lang::GherkinLanguages;
    use crate::keymap::Action;

    fn en() -> &'static crate::gherkin_lang::GherkinLanguage {
        GherkinLanguages::global().get("en")
    }

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
        let app = App::from_file(&path, crate::config::load_config().unwrap())
            .expect("app should open fixture file");
        (app, path)
    }

    #[test]
    fn test_step_edit_boundary_detection() {
        let en = en();
        assert_eq!(step_edit_start_col("  Given I log in", en), Some(8));
        assert_eq!(step_edit_start_col("When x", en), Some(5));
        assert_eq!(step_edit_start_col("Feature: x", en), None);
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
        let en = en();
        assert_eq!(
            replace_step_keyword_line("  Given x", "When", en).as_deref(),
            Some("  When x")
        );
        assert_eq!(
            replace_step_keyword_line("But last", "Given", en).as_deref(),
            Some("Given last")
        );
        assert_eq!(current_step_keyword_index("  Given x", en), Some(0));
        assert_eq!(current_step_keyword_index("But last", en), Some(4));
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
            selection_anchor: None,
            selection_end: None,
            editor_panel_rect: None,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: None,
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Step,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            editor_focus_scenario_row: None,
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
            scenario_dropdown_open: false,
            scenario_dropdown_selection: 0,
            agents: vec![AgentThread::new(0, "Agent 1")],
            selected_agent: 0,
            next_agent_id: 1,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
            config: crate::config::load_config().unwrap(),
            auth_panel_active: false,
            model_profiles: crate::profiles::ModelProfile::load_all(),
            model_active_id: crate::profiles::ModelProfile::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            active_model_label: None,
            model_panel_mode: ModelPanelMode::List,
            model_form_focus: 0,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            session_panel_active: false,
            session_panel_selection: 0,
            session_list: Vec::new(),
            skill_registry: crate::agent::skills::SkillRegistry::new(),
            generation_stage: crate::agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
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
            selection_anchor: None,
            selection_end: None,
            editor_panel_rect: None,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: None,
            runner_rx: None,
            last_external_check: Instant::now(),
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            editor_focus_scenario_row: None,
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
            scenario_dropdown_open: false,
            scenario_dropdown_selection: 0,
            agents: vec![AgentThread::new(0, "Agent 1")],
            selected_agent: 0,
            next_agent_id: 1,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            quit_pending_confirm: false,
            status_message: None,
            status_message_deadline: None,
            config: crate::config::load_config().unwrap(),
            auth_panel_active: false,
            model_profiles: crate::profiles::ModelProfile::load_all(),
            model_active_id: crate::profiles::ModelProfile::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            active_model_label: None,
            model_panel_mode: ModelPanelMode::List,
            model_form_focus: 0,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            session_panel_active: false,
            session_panel_selection: 0,
            session_list: Vec::new(),
            skill_registry: crate::agent::skills::SkillRegistry::new(),
            generation_stage: crate::agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
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
