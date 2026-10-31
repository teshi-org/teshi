use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use crate::bdd_nav::{
    bdd_editor_nav_rows, bdd_step_rows, current_step_keyword_index, delete_scenario_block,
    delete_step, insert_scenario_after_current, insert_step_above, insert_step_below,
    line_body_edit_min_col_in_buffer, next_node_row, prev_node_row, replace_step_keyword_line,
    scenario_content_rows, scenario_header_for_row, scenario_step_rows, swap_step_with_next,
    swap_step_with_prev,
};
use crate::config::AppConfig;
use crate::editor_buffer::EditorBuffer;

pub use crate::diff::{ChangeKind, DiffLine};
use crate::keymap::Action;
use crate::mindmap;
use crate::runner::{self, RunCase, RunEvent, RunRequest, RunnerConfig};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use teshi_core::StepIndex;
use teshi_core::gherkin::{self, BddProject, BddScenario};
use teshi_core::gherkin_lang::{StepKeywordType, StructuralType};
use teshi_core::highlight::HighlightKind;

const GIT_STATUS_REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);
const PROJECT_SWITCH_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Available slash commands: (name, description)
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("new", "Start a new session"),
    ("exit", "Exit Teshi"),
    ("resume", "Resume the most recent session"),
    ("retry", "Continue an interrupted ACP task"),
    ("copy", "Copy last N assistant responses to clipboard"),
    ("details", "Expand or collapse thinking and tool details"),
    ("models", "Open model settings"),
    ("sessions", "Browse saved sessions"),
    ("project", "Open or switch the active project"),
    ("approval", "Switch approval mode (Manual/Auto/Bypass)"),
    ("agent", "Select session Agent (Native or installed ACP)"),
    ("acp", "Manage installed ACP Agents"),
    (
        "settings",
        "Select ACP model, mode, thinking and other settings",
    ),
    ("agent-profile", "Select Native agent profile"),
    (
        "generate",
        "Start requirements → test points → scenarios generation",
    ),
    (
        "continue",
        "Continue generation after approving test points",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    Explore,
    MindMap,
    Ai,
    Requirements,
    TestPoints,
}

pub use teshi_agent_runtime::{AiChatMessage, AiRole};

/// Current state of the AI interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiStatus {
    Idle,
    Waiting,
    AwaitingApproval,
    Error,
}

/// A stable position in the plain-text projection of one chat block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatAnchor {
    /// Message/block key in the active conversation.
    pub block: usize,
    /// Character offset within the block's plain-text projection.
    pub offset: usize,
    /// Fingerprint captured when the anchor was created.
    pub fingerprint: u64,
}

/// A mouse selection in the active agent's chat history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatSelection {
    /// Agent whose conversation owns the selection.
    pub agent_id: usize,
    /// Position where the drag started.
    pub anchor: ChatAnchor,
    /// Position where the drag currently ends.
    pub focus: ChatAnchor,
}

/// A pending ACP `session/request_permission` prompt for one agent.
#[derive(Debug, Clone)]
pub struct AcpPermissionPrompt {
    pub request_id: u64,
    pub options: Vec<teshi_agent::backend::PermissionChoice>,
    pub selected: usize,
}

/// A single independent agent conversation with its own state and LLM connection.
#[derive(Debug)]
pub struct AgentThread {
    #[allow(dead_code)]
    pub id: usize,
    pub title: String,
    pub status: AiStatus,
    pub input: String,
    pub input_cursor: usize,
    pub tool_status: Option<String>,
    pub scroll_offset: usize,
    pub horizontal_scroll: usize,
    /// Whether supporting reasoning and tool titles are expanded in this conversation.
    pub activity_details_expanded: bool,
    pub(crate) backend: teshi_agent_runtime::backend::AgentBackendRuntime,
    retiring_backends: Vec<teshi_agent_runtime::backend::AgentBackendRuntime>,
    /// Conversation-local ACP picker and hit regions.
    pub(crate) acp_controls_ui: crate::acp_controls::ControlsUi,
    /// Cumulative input tokens across all requests for this agent.
    pub total_input_tokens: u64,
    /// Cumulative output tokens across all requests for this agent.
    pub total_output_tokens: u64,
    /// Last reported input token count (used for context trimming).
    pub last_input_tokens: Option<u32>,
    /// Start of the current task, including tool continuations and approval waits.
    pub task_started: Option<Instant>,
    /// Final elapsed time of the most recently observed task.
    pub task_elapsed: Duration,
    /// Selected agent profile ID (None = use default).
    pub profile_id: Option<String>,
    /// Current conversation's ACP selection, also included in session snapshots.
    pub selected_acp_id: Option<String>,
    selection_root: Option<std::path::PathBuf>,
    selection_warning: Option<String>,
    /// Permission prompt from the external ACP agent, if one is pending.
    pub pending_acp_permission: Option<AcpPermissionPrompt>,
    /// Latest on-demand browser visual observation. Older image bytes are not
    /// retained in the active model projection, while their text messages
    /// remain in the durable chat history.
    pub latest_visual_observation: Option<teshi_engine::BrowserScreenshot>,
    /// Finish reason from the most recent production model response.
    pub last_finish_reason: Option<String>,
    saved_session: Option<crate::session::Session>,
    last_session_snapshot: String,
    last_session_save: Option<std::time::Instant>,
}

impl AgentThread {
    fn replace_backend(&mut self, kind: teshi_agent::backend::AgentBackendKind) {
        let mut previous = std::mem::replace(
            &mut self.backend,
            teshi_agent_runtime::backend::AgentBackendRuntime::new(
                kind,
                teshi_agent_runtime::max_agent_iterations(),
            ),
        );
        previous.request_shutdown();
        previous.poll_shutdown();
        if previous.shutdown_pending() {
            self.retiring_backends.push(previous);
        }
    }

    fn shutdown_pending(&self) -> bool {
        self.backend.shutdown_pending()
            || self
                .retiring_backends
                .iter()
                .any(|backend| backend.shutdown_pending())
    }

    fn poll_shutdown(&mut self) {
        self.backend.poll_shutdown();
        self.retiring_backends.retain_mut(|backend| {
            backend.poll_shutdown();
            backend.shutdown_pending()
        });
    }

    /// Startup backend fallback notice, retained until an explicit selection.
    pub fn selection_warning(&self) -> Option<&str> {
        self.selection_warning.as_deref()
    }

    fn retain_partial_response(&mut self) {
        let content = std::mem::take(self.partial_response_mut());
        let reasoning = self.backend.take_partial_reasoning();
        if !content.is_empty() || !reasoning.is_empty() {
            self.messages_mut().push(AiChatMessage {
                role: AiRole::Assistant,
                content,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                source: Some("agent_progress".into()),
            });
        }
    }

    pub fn new(id: usize, title: &str) -> Self {
        // Unit tests opt in with an isolated root; never write user preferences.
        let root = if cfg!(test) {
            None
        } else {
            crate::acp_agents::store_dir().ok()
        };
        Self::new_with_selection(id, title, root)
    }

    fn new_with_selection(id: usize, title: &str, root: Option<std::path::PathBuf>) -> Self {
        let kind = match std::env::var("TESHI_AGENT_BACKEND").as_deref() {
            Ok("acp") => teshi_agent::backend::AgentBackendKind::Acp,
            _ => teshi_agent::backend::AgentBackendKind::Native,
        };
        let mut thread = Self::new_with_backend(id, title, kind);
        thread.selection_root = root;
        if std::env::var("TESHI_AGENT_BACKEND").is_ok() {
            return thread;
        }
        let Some(root) = thread.selection_root.as_ref() else {
            return thread;
        };
        let choice = crate::agent_selection::load(root).and_then(|choice| match choice {
            Some(crate::agent_selection::Selection::Acp { agent_id }) => {
                if !crate::acp_agents::load(root)?
                    .iter()
                    .any(|agent| agent.id == agent_id)
                {
                    anyhow::bail!("ACP Agent {agent_id} is unavailable; using Teshi Native");
                }
                Ok(Some(agent_id))
            }
            _ => Ok(None),
        });
        match choice {
            Ok(Some(id)) => {
                thread.backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
                    teshi_agent::backend::AgentBackendKind::Acp,
                    teshi_agent_runtime::max_agent_iterations(),
                );
                thread.selected_acp_id = Some(id);
            }
            Ok(None) => {}
            Err(error) => thread.selection_warning = Some(format!("Agent preference: {error}")),
        }
        thread
    }

    pub fn new_with_backend(
        id: usize,
        title: &str,
        kind: teshi_agent::backend::AgentBackendKind,
    ) -> Self {
        Self {
            id,
            title: title.to_string(),
            status: AiStatus::Idle,
            input: String::new(),
            input_cursor: 0,
            tool_status: None,
            scroll_offset: 0,
            horizontal_scroll: 0,
            activity_details_expanded: false,
            backend: teshi_agent_runtime::backend::AgentBackendRuntime::new(
                kind,
                teshi_agent_runtime::max_agent_iterations(),
            ),
            retiring_backends: Vec::new(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            last_input_tokens: None,
            task_started: None,
            task_elapsed: Duration::ZERO,
            profile_id: None,
            selected_acp_id: None,
            selection_root: None,
            selection_warning: None,
            acp_controls_ui: Default::default(),
            pending_acp_permission: None,
            latest_visual_observation: None,
            last_finish_reason: None,
            saved_session: None,
            last_session_snapshot: String::new(),
            last_session_save: None,
        }
    }
}

impl AgentThread {
    /// Checkpoint visible history, including an unfinished streamed reply.
    fn save_session_to(
        &mut self,
        root: &Path,
        project: &Path,
        model: Option<String>,
    ) -> Result<()> {
        if self.messages().is_empty() {
            return Ok(());
        }
        if let Some(owner) = self
            .saved_session
            .as_ref()
            .and_then(|s| s.project_root.as_ref())
        {
            anyhow::ensure!(
                teshi_engine::ProjectIdentity::new(owner)?
                    == teshi_engine::ProjectIdentity::new(project)?,
                "Cannot checkpoint a session owned by another project"
            );
        }
        let mut messages = self.messages().to_vec();
        if !self.partial_response().is_empty() || !self.backend.partial_reasoning().is_empty() {
            messages.push(AiChatMessage {
                role: AiRole::Assistant,
                content: self.partial_response().to_string(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: (!self.backend.partial_reasoning().is_empty())
                    .then(|| self.backend.partial_reasoning().to_string()),
                source: Some("agent_progress".into()),
            });
        }
        let snapshot = serde_json::to_string(&(
            &messages,
            self.backend_kind(),
            &self.selected_acp_id,
            &model,
            project,
        ))?;
        if snapshot == self.last_session_snapshot {
            return Ok(());
        }
        let mut session = self
            .saved_session
            .clone()
            .unwrap_or_else(|| crate::session::Session::from_messages(Vec::new(), model.clone()));
        session.messages = messages;
        session.message_count = session.messages.len();
        session.model_label = model;
        // Loading legacy history is not an assignment to the active project.
        // New conversations acquire an owner; existing snapshots keep theirs.
        if self.saved_session.is_none() {
            session.project_root = Some(project.to_path_buf());
        }
        session.backend_kind = Some(self.backend_kind());
        session.acp_agent_id = self.selected_acp_id.clone();
        session.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .to_string();
        teshi_engine::write_atomic(&root.join(format!("{}.json", session.id)), &session)?;
        self.saved_session = Some(session);
        self.last_session_snapshot = snapshot;
        Ok(())
    }

    pub fn backend_kind(&self) -> teshi_agent::backend::AgentBackendKind {
        self.backend.kind()
    }
    /// Whether the last ACP turn ended before normal completion.
    pub fn task_interrupted(&self) -> bool {
        matches!(
            self.last_finish_reason.as_deref(),
            Some("refusal" | "max_tokens" | "max_turn_requests")
        ) && self.backend_kind() == teshi_agent::backend::AgentBackendKind::Acp
            && self.status == AiStatus::Idle
    }

    pub fn messages(&self) -> &[AiChatMessage] {
        self.backend.messages()
    }
    pub fn messages_mut(&mut self) -> &mut Vec<AiChatMessage> {
        self.backend.messages_mut()
    }
    pub fn partial_response(&self) -> &str {
        self.backend.partial_response()
    }
    pub fn partial_response_mut(&mut self) -> &mut String {
        self.backend.partial_response_mut()
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
    /// Show the "Edit model" form.
    Editing,
}

/// Stable model/session target retained until deletion succeeds or is cancelled.
#[derive(Debug, Clone)]
pub struct DeleteConfirmation {
    pub action: Action,
    pub target_id: String,
    pub label: String,
    pub error: Option<String>,
}

/// Mode of the agent profile management overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPanelMode {
    /// Show the list of profiles.
    List,
    SelectBackend,
    InstalledAcp,
    Registry,
}

/// One recorded action in an exploration trace.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraceAction {
    pub step_index: usize,
    pub action: String,
    pub ref_id: Option<String>,
    pub text: Option<String>,
    pub url: String,
    pub timestamp_ms: u64,
    pub ok: bool,
    pub error: Option<String>,
}

/// Full exploration session state.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ExploreState {
    /// Feature file path (relative to project root).
    pub feature_path: String,
    /// Gherkin scenario steps: (line_number, keyword, text).
    pub steps: Vec<(usize, String, String)>,
    /// Index into steps currently being worked on.
    pub current_step: usize,
    /// Accumulated trace of actions taken.
    pub trace: Vec<TraceAction>,
    /// Step counter for the entire exploration (all steps).
    pub step_count: u32,
    /// Maximum steps allowed before termination.
    pub max_steps: u32,
    /// URL whitelist patterns (substring match).
    pub url_whitelist: Vec<String>,
    /// Current page URL (updated from snapshot responses).
    pub current_url: String,
    /// Session ID for trace file naming.
    pub session_id: String,
    /// Whether exploration has completed successfully.
    pub completed: bool,
    /// Whether exploration was terminated (step limit / violation).
    pub terminated: bool,
    /// Termination reason if terminated.
    pub termination_reason: Option<String>,
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

impl ColumnFocus {
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Feature => 0,
            Self::Scenario => 1,
            Self::Step => 2,
        }
    }
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
    /// HTTP exchanges for this case (redacted unless inspector_plaintext).
    pub exchanges: Vec<teshi_core::HttpExchange>,
    /// Step start/end records in scenario order.
    pub steps: Vec<StepRunRecord>,
    /// When true, Explore fetches plaintext exchanges from the API sidecar.
    pub inspector_plaintext: bool,
}

/// One Gherkin step observed during an Explore run.
#[derive(Debug, Clone)]
pub struct StepRunRecord {
    /// Correlation id from `start_step` / `end_step`.
    pub step_id: String,
    /// Step body after `[API]` stripping when provided.
    pub text: String,
    /// `passed` / `failed` when ended.
    pub status: Option<String>,
    /// Failure message from the step.
    pub message: Option<String>,
}

/// First capture recorded for `step_text`, matched on the recorded text.
fn index_of_step_capture(
    refs: &[crate::image_view::ScreenshotRef],
    step_text: &str,
) -> Option<usize> {
    let step_text = step_text.trim();
    if step_text.is_empty() {
        return None;
    }
    refs.iter()
        .position(|shot| shot.step_text.trim() == step_text)
}

fn empty_running_detail(case_id: String) -> CaseDetail {
    CaseDetail {
        case_id,
        status: RunStatus::Running,
        duration_ms: None,
        message: None,
        stack: None,
        attachments: Vec::new(),
        logs: Vec::new(),
        exchanges: Vec::new(),
        steps: Vec::new(),
        inspector_plaintext: false,
    }
}

#[derive(Debug, Clone)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// UI state for the step-keyword list opened by editing a keyword prefix.
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

struct GitStatusRefreshResult {
    revision: u64,
    project_stamp: teshi_engine::ProjectStamp,
    model: teshi_core::git::FeatureGitStatusModel,
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

/// One changed Step row in the Explore Steps pane, including removed Steps
/// that no longer exist in the current AST.
#[derive(Debug, Clone)]
pub(crate) struct ChangedStep {
    /// Step text, with attached doc-string and table lines folded in.
    pub text: String,
    /// Diff status relative to `HEAD`.
    pub kind: ChangeKind,
    /// Resolved keyword colour from the current AST, so a changed `And`/`But`
    /// inherits exactly like the browse view. `None` for removed Steps and for
    /// lines that no longer map back onto the AST.
    pub keyword_kind: Option<HighlightKind>,
}

/// Keyword highlight kind of a Step that still exists in the current AST.
///
/// The diff line must agree with the AST text so a shifted line number cannot
/// recolour a different Step.
fn ast_keyword_kind(
    scenario: Option<&BddScenario>,
    line_number: Option<usize>,
    text: &str,
) -> Option<HighlightKind> {
    let steps = &scenario?.steps;
    let index = steps
        .iter()
        .position(|step| Some(step.line_number) == line_number)?;
    let step = &steps[index];
    if format!("{} {}", step.keyword, step.text).trim() != text.trim() {
        return None;
    }
    let inherited =
        steps[..index]
            .iter()
            .rev()
            .find_map(|candidate| match candidate.keyword_type {
                StepKeywordType::Given => Some(HighlightKind::Given),
                StepKeywordType::When => Some(HighlightKind::When),
                StepKeywordType::Then => Some(HighlightKind::Then),
                StepKeywordType::And | StepKeywordType::But => None,
            });
    Some(match step.keyword_type {
        StepKeywordType::Given => HighlightKind::Given,
        StepKeywordType::When => HighlightKind::When,
        StepKeywordType::Then => HighlightKind::Then,
        StepKeywordType::And | StepKeywordType::But => inherited.unwrap_or(HighlightKind::AndBut),
    })
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
    /// Planned test-point IDs to revalidate immediately before a scenario insertion is accepted.
    pub test_point_ids: Vec<String>,
    /// The tool call ID this change responds to (for feeding back to the LLM).
    pub tool_call_id: String,
    /// Snapshot of buffer content before the change (for diff computation).
    pub old_buffer_snapshot: String,
    /// The agent that queued this change (for correct tool result routing).
    pub agent_idx: usize,
}

/// Pending confirmation of the AI generation source scope before Gathering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationScopePrompt {
    /// Iteration window suggested from the Requirements view (not implicitly bound).
    pub iteration: teshi_core::authoring::RequirementIterationFilter,
    /// User message to send after the user confirms the scope.
    pub pending_user_message: String,
}

/// Documents visible in the Requirements tab after constructing TUI state.
#[derive(Debug, Clone)]
pub struct RequirementsTabSnapshot {
    /// Stable document ids in index order.
    pub document_ids: Vec<String>,
    /// Currently selected document id, if any.
    pub selected_document_id: Option<String>,
    /// Center-pane Markdown for the selected document.
    pub selected_body: String,
}

/// Builds TUI state as `teshi` with no PATH and snapshots the Requirements tab.
///
/// Used by requirement E2E to assert that the user-level store is visible
/// without launching the interactive event loop.
///
/// # Errors
///
/// Returns an error when configuration or the requirement store cannot be loaded.
pub fn requirements_tab_without_project(
    requirements_root: &Path,
) -> Result<RequirementsTabSnapshot> {
    let cli = crate::cli::Cli {
        command: None,
        recursive: false,
        paths: Vec::new(),
        requirements_root: Some(requirements_root.to_path_buf()),
    };
    let app = App::from_cli(&cli)?;
    let document_ids = app
        .authoring_ui
        .artifacts
        .as_ref()
        .map(|artifacts| {
            artifacts
                .index
                .documents
                .iter()
                .map(|doc| doc.id.clone())
                .collect()
        })
        .unwrap_or_default();
    Ok(RequirementsTabSnapshot {
        document_ids,
        selected_document_id: app.authoring_ui.selected_document_id.clone(),
        selected_body: app.authoring_ui.buffer.as_string(),
    })
}

/// Rendered geometry and offsets belonging to one Explore column.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExploreColumnViewport {
    pub(crate) rect: ratatui::layout::Rect,
    pub(crate) offset: usize,
    pub(crate) max_vertical: usize,
    pub(crate) max_horizontal: usize,
    pub(crate) last_selected: Option<usize>,
    pub(crate) last_owner: Option<(usize, Option<usize>, Option<usize>)>,
}

/// Application state shared by the terminal views and input handlers.
pub struct App {
    // ── Multi-file project ──────────────────────────────────────────
    pub project: BddProject,
    /// Optional Git status/diff projection aligned with `project.features`.
    pub git_status: teshi_core::git::FeatureGitStatusModel,
    /// Latest background Git projection result, if a refresh is in flight.
    git_status_rx: Option<Receiver<GitStatusRefreshResult>>,
    /// Whether a newer Git projection should start after the current worker exits.
    git_status_refresh_requested: bool,
    /// Earliest time at which a refresh requested by an editor mutation may run.
    git_status_refresh_deadline: Option<Instant>,
    /// Revision of the in-memory project/buffer snapshot represented by Git.
    git_status_revision: u64,
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
    /// Diagnostics for the current raw editor buffer.  This is deliberately
    /// separate from the permissive parsed project so incomplete content can
    /// remain visible and editable while errors are reported beside it.
    pub buffer_diagnostics: Vec<teshi_core::ValidationDiagnostic>,
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
    /// Current mouse screen position for hover effects; `None` if mouse hasn't moved yet.
    pub mouse_position: Option<(u16, u16)>,
    /// Screen rectangle of the editor panel, updated each render frame.
    pub editor_panel_rect: Option<ratatui::layout::Rect>,
    /// Body rectangle of the active chat history, updated each render frame.
    /// Reusable message layouts for chat rendering and selection.
    pub(crate) chat_layout_cache: std::cell::RefCell<crate::ui::chat::ChatLayoutCache>,
    pub chat_history_rect: Option<ratatui::layout::Rect>,
    /// Current chat text selection, if any.
    pub chat_selection: Option<ChatSelection>,
    // X11 serves selection data from the owning process. Retain the handle
    // after a successful copy so another process can request that data.
    system_clipboard: Option<arboard::Clipboard>,
    /// Whether a left-button drag is extending the chat selection.
    pub chat_selection_dragging: bool,
    /// Clickable regions registered during the last render frame.
    pub clickable_regions: Vec<ClickableRegion>,
    /// Screen rectangle of the tree panel, updated each render frame.
    pub tree_panel_rect: Option<ratatui::layout::Rect>,
    /// Screen rectangle of the preview panel, updated each render frame.
    pub preview_panel_rect: Option<ratatui::layout::Rect>,
    /// Replay screenshot panel state for the MindMap preview.
    pub screenshot: crate::image_view::ScreenshotPanel,
    /// Which scenarios have saved captures, for tree markers and the fallback
    /// list.
    pub capture_index: crate::image_view::CaptureIndex,
    /// Newest capture of each parsed scenario, for Explore capture markers.
    explore_captures: HashMap<(usize, usize), crate::image_view::CaptureEntry>,
    undo_stack: Vec<(EditorBuffer, usize, usize)>,
    redo_stack: Vec<(EditorBuffer, usize, usize)>,
    pub runner_config: Option<RunnerConfig>,
    runner_rx: Option<Receiver<RunEvent>>,
    runner_cancel: Option<Sender<()>>,
    last_external_check: Instant,
    pub(crate) feature_discovery_root: Option<PathBuf>,
    feature_discovery_worker: Option<crate::feature_discovery::BackgroundDiscovery>,
    pub(crate) lazy_features: Option<crate::lazy_features::LazyFeatures>,
    external_change_prompt: Option<ExternalChangePrompt>,
    /// Pending text modifications requested by AI agent tools, awaiting user confirmation.
    pub pending_agent_changes: Vec<AgentPendingChange>,
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
    pub(crate) feature_browser: crate::feature_browser::FeatureBrowser,
    /// Selected HEAD-only deleted Feature row, when the selection is not a
    /// current project Feature.
    pub explore_selected_deleted_feature: Option<usize>,
    pub explore_selected_scenario: usize,
    pub explore_selected_step: usize,
    /// Independent horizontal offsets for the three Explore columns.
    pub explore_horizontal_scroll: [usize; 3],
    pub(crate) explore_viewports: [ExploreColumnViewport; 3],
    /// Explore scrollbar currently controlled by a left-button drag.
    pub explore_horizontal_scroll_drag: Option<ColumnFocus>,
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
    /// Active exploration session state (None = not exploring).
    pub explore_state: Option<ExploreState>,
    /// Approval mode for agent tool calls (Manual / Auto / Bypass).
    pub approval_mode: teshi_agent::approval::ApprovalMode,
    /// Whether the approval mode selection panel is active.
    pub approval_panel_active: bool,
    /// Selected index within the approval mode panel.
    pub approval_panel_selection: usize,
    pub slash_suggestion_active: bool,
    pub slash_suggestion_selection: usize,
    /// Whether the AI tab input bar has keyboard focus (Esc toggles this off).
    pub ai_input_focused: bool,
    pub quit_pending_confirm: bool,
    /// Which button is focused in quit confirm panel: 0 = Yes, 1 = No.
    pub quit_panel_selection: usize,
    /// Temporary one-shot status message (e.g. "AI applied filter: @smoke").
    pub status_message: Option<String>,
    /// When the status message should be cleared (3-second lifespan).
    status_message_deadline: Option<Instant>,
    /// Product version displayed in the TUI header.
    pub version: String,
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
    // ── Model profile form state ───────────────────────
    pub model_panel_mode: ModelPanelMode,
    /// When editing, the ID of the profile being edited.
    pub model_form_editing_id: Option<String>,
    pub model_form_focus: usize,
    /// Character offset within the focused model form field.
    pub model_form_cursor: usize,
    /// Stable target and label captured before a destructive confirmation.
    pub pending_delete: Option<DeleteConfirmation>,
    pub model_form_name: String,
    pub model_form_provider: String,
    pub model_form_model: String,
    pub model_form_base_url: String,
    pub model_form_api_key: String,
    pub model_form_max_tokens: String,
    pub model_form_temperature: String,
    // ── Session browser state ──────────────────────────
    pub session_panel_active: bool,
    /// Whether the browser explicitly displays unassigned legacy history.
    pub session_panel_legacy: bool,
    pending_session_restore: Option<(
        usize,
        teshi_engine::ProjectStamp,
        crate::session::Session,
        bool,
    )>,
    pub session_panel_selection: usize,
    pub session_list: Vec<crate::session::Session>,
    // ── Agent state ─────────────────────────────────
    pub agent_registry: teshi_agent::registry::AgentRegistry,
    pub agent_profile_panel_active: bool,
    pub agent_profile_panel_selection: usize,
    // ── Agent profile form state ────────────────────────
    pub agent_panel_mode: AgentPanelMode,
    pub installed_acp_agents: Vec<crate::acp_agents::InstalledAgent>,
    pub acp_registry_agents: Vec<teshi_acp::registry::RegistryAgent>,
    pub(crate) acp_registry_rx:
        Option<Receiver<std::result::Result<Vec<teshi_acp::registry::RegistryAgent>, String>>>,
    acp_install_rx:
        Option<Receiver<std::result::Result<crate::acp_agents::InstalledAgent, String>>>,
    // ── Generation pipeline state ───────────────────────
    pub generation_stage: teshi_agent::pipeline::GenerationStage,
    pub pipeline_requirement: Option<teshi_agent::pipeline::Requirement>,
    pub pipeline_plan: Option<teshi_agent::pipeline::GenerationPlan>,
    /// Confirmed requirement-library window for the active generation session.
    pub generation_scope: Option<teshi_agent::pipeline::RequirementSourceScope>,
    /// Overlay asking the user to confirm store/iteration before Gathering.
    pub generation_scope_prompt: Option<GenerationScopePrompt>,
    /// Why restored generation cannot continue until the user reconfirms scope.
    pub generation_paused_reason: Option<String>,
    /// Requirements / test-point authoring UI state.
    pub authoring_ui: crate::authoring_tab::AuthoringUiState,
    /// Test Points tab UI state.
    pub test_points_ui: crate::test_points_tab::TestPointsUiState,
    /// Resolved user-level requirement store root for this process.
    pub requirements_root: PathBuf,
    /// Project identity and switch lifecycle state. Project-scoped async
    /// results must carry the stamp returned by `project_stamp`.
    pub project_switch: crate::project_context::ProjectSwitchState,
}

struct PreparedProjectContext {
    lazy_features: Option<crate::lazy_features::LazyFeatures>,
    project: BddProject,
    git_status: teshi_core::git::FeatureGitStatusModel,
    step_index: StepIndex,
    mindmap_index: mindmap::MindMapIndex,
    buffers: Vec<EditorBuffer>,
    buffer_dirty: Vec<bool>,
    disk_stamps: Vec<Option<FileStamp>>,
    active_buffer_idx: Option<usize>,
    tree_state: tui_tree_widget::TreeState<String>,
    buffer: EditorBuffer,
    file_path: Option<PathBuf>,
    authoring_ui: crate::authoring_tab::AuthoringUiState,
    test_points_ui: crate::test_points_tab::TestPointsUiState,
    runner_config: Option<RunnerConfig>,
    agent_registry: teshi_agent::registry::AgentRegistry,
}

/// Convert a character index to the corresponding byte offset in a UTF-8 string.
fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn set_system_clipboard(text: &str) -> Result<arboard::Clipboard> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| anyhow::anyhow!("failed to open clipboard: {error}"))?;
    clipboard
        .set_text(text)
        .map_err(|error| anyhow::anyhow!("failed to set clipboard: {error}"))?;
    Ok(clipboard)
}

fn completed_assistant_responses(messages: &[AiChatMessage], limit: usize) -> Vec<&str> {
    let mut responses: Vec<&str> = messages
        .iter()
        .rev()
        .filter(|message| {
            message.role == AiRole::Assistant
                && message.source.as_deref() != Some("agent_progress")
                && message.source.as_deref() != Some("acp_activity")
                && message.source.as_deref() != Some("acp_reasoning")
                && !message.content.trim().is_empty()
                && message
                    .tool_calls
                    .as_ref()
                    .is_none_or(|calls| calls.is_empty())
        })
        .take(limit)
        .map(|message| message.content.as_str())
        .collect();
    responses.reverse();
    responses
}

/// A clickable region registered during rendering for mouse hit-testing.
#[derive(Debug, Clone)]
pub enum ClickableRegion {
    /// A visible control dispatching the same action as its keyboard binding.
    ActionButton {
        rect: ratatui::layout::Rect,
        action: Action,
        modal: bool,
    },
    /// A filter or priority picker row in the Requirements modal.
    RequirementFilterOption {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A rendered editable field in Test Points.
    TestPointField {
        rect: ratatui::layout::Rect,
        field: crate::test_points_tab::DetailField,
    },
    /// A rendered requirement excerpt in Test Points.
    TestPointExcerpt {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A realized-scenario row in Test Points.
    TestPointScenario {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A selectable row in the native model profile modal.
    ModelProfileRow {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// An editable field in the native model profile form.
    ModelFormField {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A selectable profile or backend row in the active Agent modal.
    AgentProfileRow {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A pending change row in the Change Summary modal.
    ChangeSummaryRow {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A visible keyword row in the step editor picker.
    StepKeywordRow {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A modal list whose wheel moves only its own selection.
    ModalList {
        rect: ratatui::layout::Rect,
        up: Action,
        down: Action,
    },
    /// A selectable row in the project session modal.
    SessionRow {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// A selectable MindMap source location.
    MindMapLocation {
        rect: ratatui::layout::Rect,
        index: usize,
    },
    /// The actual rendered chat input rectangle.
    ChatInput {
        rect: ratatui::layout::Rect,
    },
    Tab {
        tab: MainTab,
        rect: ratatui::layout::Rect,
    },
    Tree,
    /// Approval mode badge in the chat panel title bar (click to open picker).
    ApprovalBadge {
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
    /// An option row in the approval mode selection panel.
    ApprovalOption {
        option_idx: usize,
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
    ExploreFeature {
        feature_idx: usize,
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
    ExploreDeletedFeature {
        deleted_idx: usize,
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
    ExploreHorizontalScrollbar {
        column: ColumnFocus,
        rect: ratatui::layout::Rect,
        content_width: usize,
        viewport_width: usize,
    },
    /// Editor panel area — reserved for click-to-focus.
    #[allow(dead_code)]
    EditorPanel,
    /// Preview panel area — reserved for click-to-focus.
    #[allow(dead_code)]
    PreviewPanel,
    /// A rendered Test Points pane. The pane index is 0=tree, 1=details,
    /// 2=linked excerpts; narrow layouts omit the third pane.
    TestPointsPane {
        pane: usize,
        rect: ratatui::layout::Rect,
    },
    /// A row in the recent-project picker.
    ProjectRecent {
        index: usize,
        rect: ratatui::layout::Rect,
    },
    /// The picker action that opens a typed project path.
    ProjectOpenPath {
        rect: ratatui::layout::Rect,
    },
    /// The current project control in the header.
    ProjectControl {
        rect: ratatui::layout::Rect,
    },
    /// Quit confirmation "Yes" button.
    QuitConfirmYes {
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
    /// Quit confirmation "No" button.
    QuitConfirmNo {
        row_y: u16,
        col_x: u16,
        col_right: u16,
    },
}

impl App {
    pub fn current_project_stamp(&self) -> teshi_engine::ProjectStamp {
        self.project_switch.current_stamp()
    }

    /// Returns whether a result produced for `stamp` may mutate this App.
    pub fn accepts_project_event(&self, stamp: &teshi_engine::ProjectStamp) -> bool {
        self.project_switch.accepts(stamp)
    }

    fn project_dirty_objects(&self) -> Vec<crate::project_context::DirtyProjectObject> {
        let mut dirty = self
            .buffer_dirty
            .iter()
            .enumerate()
            .filter(|(_, is_dirty)| **is_dirty)
            .filter_map(|(index, _)| {
                self.project.features.get(index).map(|feature| {
                    crate::project_context::DirtyProjectObject::Feature(feature.file_path.clone())
                })
            })
            .collect::<Vec<_>>();
        if self.authoring_ui.buffer_dirty
            && let Some(id) = self.authoring_ui.selected_document_id.clone()
        {
            dirty.push(crate::project_context::DirtyProjectObject::Requirement(id));
        }
        if self.test_points_ui.field_dirty
            && let Some(id) = self.test_points_ui.selected_test_point_id.clone()
        {
            dirty.push(crate::project_context::DirtyProjectObject::TestPoint(id));
        }
        if !self.pending_agent_changes.is_empty() {
            dirty.push(crate::project_context::DirtyProjectObject::AgentProposal);
        }
        dirty
    }

    fn project_tasks_busy(&self) -> bool {
        self.runner_rx.is_some()
            || self.explore_state.is_some()
            || self.generation_stage != teshi_agent::pipeline::GenerationStage::Idle
            || self.agents.iter().any(|agent| {
                agent.shutdown_pending()
                    || matches!(agent.status, AiStatus::Waiting | AiStatus::AwaitingApproval)
                    || matches!(
                        agent.backend.status(),
                        teshi_agent::backend::AgentBackendStatus::Running
                            | teshi_agent::backend::AgentBackendStatus::WaitingForPermission
                            | teshi_agent::backend::AgentBackendStatus::WaitingForHumanReview
                    )
            })
    }

    /// Start a safe project switch. Preparation is detached from the active
    /// fields; failures leave the current project and its edits untouched.
    pub fn request_project_switch(&mut self, path: impl AsRef<Path>) -> Result<()> {
        if !self.project_switch.is_active()
            && matches!(
                self.project_switch.overlay,
                crate::project_context::SwitchOverlay::None
            )
            && self.project_switch.pending.is_some()
        {
            self.status = "A project switch is already being prepared".to_string();
            self.set_status_message(self.status.clone());
            return Ok(());
        }
        let target =
            match crate::project_context::resolve_project_target(path.as_ref().to_path_buf()) {
                Ok(identity) => identity,
                Err(error) => {
                    self.status = format!("Could not resolve project: {error}");
                    self.set_status_message(self.status.clone());
                    return Ok(());
                }
            };
        if !target.root().is_dir() {
            self.status = format!(
                "Project directory does not exist: {}",
                target.root().display()
            );
            self.set_status_message(self.status.clone());
            return Ok(());
        }
        let dirty = self.project_dirty_objects();
        let busy = self.project_tasks_busy();
        let decision = self.project_switch.begin(target.clone(), dirty, busy);
        match decision {
            crate::project_context::SwitchDecision::Noop => {
                self.status = "Already on that project; active work was preserved".to_string();
            }
            crate::project_context::SwitchDecision::NeedsConfirmation => {
                let labels = self
                    .project_switch
                    .pending
                    .as_ref()
                    .map(|request| {
                        request
                            .dirty
                            .iter()
                            .map(crate::project_context::DirtyProjectObject::label)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                self.status = if labels.is_empty() {
                    "Project switch needs running tasks stopped: S stop · Esc cancel".to_string()
                } else {
                    format!(
                        "Unsaved project state: {} · S save & switch · D discard & switch · Esc cancel",
                        labels.join(", ")
                    )
                };
            }
            crate::project_context::SwitchDecision::Ready => {
                self.commit_project_switch(target)?;
            }
        }
        self.set_status_message(self.status.clone());
        Ok(())
    }

    fn commit_project_switch(&mut self, target: teshi_engine::ProjectIdentity) -> Result<()> {
        // Dirty-state resolution can take time. Recheck ownership at the final
        // boundary even if no tasks were busy when the request first began.
        if self.project_tasks_busy() {
            self.project_switch.overlay = crate::project_context::SwitchOverlay::Busy { target };
            self.status = "Project-owned tasks must stop before switching".to_string();
            return Ok(());
        }
        // Completed ACP sessions and idle native transports are project-owned
        // too. Shut them down before preparation, without a blocking Drop/join.
        for agent in &mut self.agents {
            agent.backend.request_shutdown();
            agent.backend.poll_shutdown();
        }
        if self.project_tasks_busy() {
            self.project_switch.overlay = crate::project_context::SwitchOverlay::Busy { target };
            self.project_switch
                .start_quiesce_wait(PROJECT_SWITCH_STOP_TIMEOUT);
            self.status = "Waiting for project-owned Agent backends to exit".to_string();
            return Ok(());
        }
        if let Err(error) = self.save_sessions(true) {
            self.project_switch.cancel();
            self.status = format!("Could not checkpoint project A: {error}");
            return Ok(());
        }
        self.project_switch.mark_preparing();
        let prepared = match self.prepare_project_context(&target) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.project_switch.cancel();
                self.status = format!("Could not prepare project B: {error}");
                return Ok(());
            }
        };

        self.project = prepared.project;
        self.feature_discovery_root = None;
        self.feature_discovery_worker = None;
        self.lazy_features = prepared.lazy_features;
        self.git_status = prepared.git_status;
        self.git_status_rx = None;
        self.git_status_refresh_requested = false;
        self.git_status_deadline_clear();
        self.git_status_revision = 0;
        self.step_index = prepared.step_index;
        self.mindmap_index = prepared.mindmap_index;
        self.mindmap_location_selection.clear();
        self.buffers = prepared.buffers;
        self.buffer_dirty = prepared.buffer_dirty;
        self.disk_stamps = prepared.disk_stamps;
        self.active_buffer_idx = prepared.active_buffer_idx;
        self.view_stage = ViewStage::TreeOnly;
        self.tree_state = prepared.tree_state;
        self.buffer = prepared.buffer;
        self.file_path = prepared.file_path;
        self.buffer_diagnostics.clear();
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.desired_col = 0;
        self.scroll_row = 0;
        self.focus_slot = BddFocusSlot::Body;
        self.preview_buffer = None;
        self.preview_title.clear();
        self.preview_cursor_row = 0;
        self.preview_scroll_row = 0;
        // These snapshots and source coordinates belong to A. Retain them
        // until preparation succeeds, then prevent any edit/undo from using
        // them against B's newly installed buffer.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.step_input_active = false;
        self.step_input_row = 0;
        self.step_input_min_col = 0;
        self.step_keyword_picker = None;
        self.pending_char = None;
        self.scenario_fold.clear();
        self.selection_anchor = None;
        self.selection_end = None;
        self.external_change_prompt = None;
        self.last_external_check = Instant::now();
        self.clickable_regions.clear();
        self.editor_panel_rect = None;
        self.tree_panel_rect = None;
        self.preview_panel_rect = None;
        self.dirty = false;
        self.runner_config = prepared.runner_config;
        self.runner_rx = None;
        self.runner_cancel = None;
        self.pending_agent_changes.clear();
        self.pending_change_diffs.clear();
        self.pending_change_summary.clear();
        self.explore_diff_lines = None;
        self.change_summary_visible = false;
        self.change_summary_selection = 0;
        self.explore_focus = ColumnFocus::Feature;
        self.explore_selected_feature = 0;
        self.feature_browser = Default::default();
        self.explore_selected_deleted_feature = None;
        self.explore_selected_scenario = 0;
        self.explore_selected_step = 0;
        self.explore_horizontal_scroll = [0; 3];
        self.explore_horizontal_scroll_drag = None;
        self.explore_edit_mode = false;
        self.editor_focus_scenario_row = None;
        self.scenario_dropdown_open = false;
        self.scenario_dropdown_selection = 0;
        self.explore_viewports = Default::default();
        self.explore_feature_scenario_memory.clear();
        self.explore_scenario_step_memory.clear();
        self.explore_case_map.clear();
        self.explore_case_status.clear();
        self.explore_case_details.clear();
        self.explore_detail_open = false;
        self.explore_detail_case = None;
        self.explore_run_summary = None;
        self.explore_state = None;
        self.generation_stage = teshi_agent::pipeline::GenerationStage::Idle;
        self.pipeline_requirement = None;
        self.pipeline_plan = None;
        self.generation_scope = None;
        self.generation_scope_prompt = None;
        self.generation_paused_reason = None;
        self.authoring_ui = prepared.authoring_ui;
        self.test_points_ui = prepared.test_points_ui;
        self.agent_registry = prepared.agent_registry;
        self.project_switch.mark_committing();
        self.project_switch.mark_committed(target.clone());
        self.session_panel_legacy = false;
        self.session_panel_active = false;
        self.pending_session_restore = None;
        self.session_list = self.project_sessions();
        self.session_panel_selection = 0;
        self.clear_chat_selection();
        self.reset_agents_for_project(target.root());
        if let Err(error) = teshi_engine::add_recent_project(target.root()) {
            self.status = format!("Switched project, but recent history was not saved: {error}");
        } else {
            self.status = format!("Switched project: {}", target.root().display());
        }
        self.restore_generation_session_from_disk();
        self.show_selection_warning();
        Ok(())
    }

    fn git_status_deadline_clear(&mut self) {
        self.git_status_refresh_deadline = None;
    }

    fn prepare_project_context(
        &self,
        target: &teshi_engine::ProjectIdentity,
    ) -> Result<PreparedProjectContext> {
        let root = target.root();
        if self.dirty || self.buffer_dirty.iter().any(|dirty| *dirty) {
            crate::lazy_features::validate_replacement(root).map_err(anyhow::Error::msg)?;
        }

        let lazy_features = Some(crate::lazy_features::LazyFeatures::new(
            root.to_path_buf(),
            true,
        ));
        let project = BddProject {
            root_dir: root.to_path_buf(),
            features: Vec::new(),
        };
        let git_status = teshi_core::git::FeatureGitStatusModel::empty_for_project(
            &project,
            if lazy_features.is_some() {
                teshi_core::git::GitProjectScope::CurrentFiles
            } else {
                teshi_core::git::GitProjectScope::Recursive
            },
        );
        let step_index = StepIndex::build(&project);
        let mut mindmap_index = mindmap::build_index(&project);
        let buffers = project
            .features
            .iter()
            .map(|feature| {
                fs::read_to_string(&feature.file_path)
                    .map(EditorBuffer::from_string)
                    .with_context(|| format!("read {}", feature.file_path.display()))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let buffer_dirty = vec![false; buffers.len()];
        let tree_state = mindmap::init_tree_state(&mut mindmap_index);
        let active_buffer_idx = (!buffers.is_empty()).then_some(0);
        let buffer = buffers
            .first()
            .cloned()
            .unwrap_or_else(|| EditorBuffer::from_string(String::new()));
        let file_path = project
            .features
            .first()
            .map(|feature| feature.file_path.clone());
        let authoring_ui = crate::authoring_tab::AuthoringUiState::load_from_project(
            root,
            &self.requirements_root,
        );
        let mut test_points_ui = crate::test_points_tab::TestPointsUiState::empty();
        test_points_ui.rebuild_tree(authoring_ui.artifacts.as_ref());
        let runner_config = runner::load_optional_runner_config_for_project(root, None)
            .with_context(|| format!("load runner configuration for {}", root.display()))?;
        Ok(PreparedProjectContext {
            lazy_features,
            disk_stamps: Self::capture_disk_stamps(&project),
            project,
            git_status,
            step_index,
            mindmap_index,
            buffers,
            buffer_dirty,
            active_buffer_idx,
            tree_state,
            buffer,
            file_path,
            authoring_ui,
            test_points_ui,
            runner_config,
            agent_registry: teshi_agent::registry::AgentRegistry::load(Some(root)),
        })
    }

    fn reset_agents_for_project(&mut self, root: &Path) {
        let descriptions = self
            .agents
            .iter()
            .map(|agent| (agent.id, agent.title.clone(), agent.profile_id.clone()))
            .collect::<Vec<_>>();
        self.agents = descriptions
            .into_iter()
            .map(|(id, title, profile_id)| {
                let mut agent =
                    AgentThread::new_with_selection(id, &title, Some(root.to_path_buf()));
                agent.profile_id = profile_id;
                agent
            })
            .collect();
        if self.agents.is_empty() {
            self.agents.push(AgentThread::new_with_selection(
                0,
                "Agent 1",
                Some(root.to_path_buf()),
            ));
        }
        self.selected_agent = self.selected_agent.min(self.agents.len() - 1);
        self.active_model_label = None;
        self.ai_input_focused = true;
    }

    fn project_switch_action(&mut self, action: Action) -> Result<()> {
        use crate::project_context::SwitchOverlay;
        match self.project_switch.overlay.clone() {
            SwitchOverlay::Picker { .. } => match action {
                Action::MoveUp | Action::OverlayMoveUp | Action::AiScrollUp => {
                    self.project_switch.move_picker(-1)
                }
                Action::MoveDown | Action::OverlayMoveDown | Action::AiScrollDown => {
                    self.project_switch.move_picker(1)
                }
                Action::OverlayConfirm | Action::Enter | Action::AiSendMessage => {
                    if let Some(target) = self.project_switch.picker_target() {
                        self.request_project_switch(target.root())?;
                    } else {
                        self.status = "No recent project is available".to_string();
                        self.project_switch.cancel();
                    }
                }
                Action::ClearInputState | Action::OverlayCancel => {
                    self.project_switch.cancel();
                    self.status = "Project switch canceled".to_string();
                }
                Action::OverlayInsert('o' | 'O') => {
                    self.project_switch.begin_open_path();
                    self.status =
                        "Type a project directory and press Enter to open it; Esc cancels"
                            .to_string();
                }
                _ => {}
            },
            SwitchOverlay::OpenPath { .. } => match action {
                Action::OverlayInsert(ch) => self.project_switch.append_path_input(ch),
                Action::OverlayBackspace | Action::Backspace => {
                    self.project_switch.backspace_path_input()
                }
                Action::OverlayConfirm | Action::Enter | Action::AiSendMessage => {
                    let input = self
                        .project_switch
                        .path_input()
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if input.is_empty() {
                        self.status =
                            "Enter a project directory or press Esc to cancel".to_string();
                    } else {
                        self.request_project_switch(PathBuf::from(input))?;
                    }
                }
                Action::ClearInputState | Action::OverlayCancel => {
                    self.project_switch.cancel();
                    self.status = "Project switch canceled".to_string();
                }
                _ => {}
            },
            SwitchOverlay::Dirty { .. } => match action {
                Action::Save | Action::OverlayInsert('s' | 'S') => {
                    if let Err(error) = self.save_project_state_for_switch() {
                        self.status = format!("Cannot save before switching: {error}");
                        self.set_status_message(self.status.clone());
                        return Ok(());
                    }
                    self.switch_after_resolution()?;
                }
                Action::PendingChar('d') | Action::OverlayInsert('d' | 'D') => {
                    // Keep A's in-memory edits intact until B has been
                    // prepared. A failed B load must leave the discardable
                    // state recoverable while A remains active.
                    self.switch_after_resolution()?;
                }
                Action::ClearInputState | Action::OverlayCancel => {
                    self.project_switch.cancel();
                    self.status = "Project switch canceled".to_string();
                }
                _ => {}
            },
            SwitchOverlay::Busy { .. } => match action {
                Action::ClearInputState | Action::OverlayCancel => {
                    self.project_switch.cancel();
                    self.status = "Project switch canceled".to_string();
                }
                Action::Save | Action::OverlayInsert('s' | 'S') => {
                    self.cancel_project_tasks();
                    self.project_switch
                        .start_quiesce_wait(PROJECT_SWITCH_STOP_TIMEOUT);
                    if let Some(cancel) = self.runner_cancel.as_ref() {
                        let _ = cancel.send(());
                        self.status =
                            "Stop requested for the project-owned runner; waiting for exit"
                                .to_string();
                    } else if self.runner_rx.is_some() {
                        self.status = "Runner is still active; wait for its owned process to exit"
                            .to_string();
                    } else {
                        self.generation_stage = teshi_agent::pipeline::GenerationStage::Idle;
                        self.explore_state = None;
                        self.advance_project_switch_after_tasks()?;
                    }
                }
                _ => {}
            },
            SwitchOverlay::None => {}
        }
        // Editor footers show editing controls rather than `status`; publish
        // switch failures through the visible status message as well.
        self.set_status_message(self.status.clone());
        Ok(())
    }

    fn cancel_project_tasks(&mut self) {
        for agent in &mut self.agents {
            agent.retain_partial_response();
            agent.backend.request_shutdown();
            agent.status = AiStatus::Idle;
            agent.tool_status = None;
            agent.pending_acp_permission = None;
        }
        self.generation_stage = teshi_agent::pipeline::GenerationStage::Idle;
        self.explore_state = None;
        // Stopping work does not accept or discard an unapplied proposal. The
        // existing dirty-state guard must resolve it explicitly before commit.
    }

    /// Progress or time out the quiesce phase without touching unrelated
    /// processes. A timeout returns control to project A and leaves the owned
    /// task handles available for their normal completion path.
    pub fn poll_project_switch(&mut self) {
        for agent in &mut self.agents {
            agent.poll_shutdown();
        }
        if !self.agent().shutdown_pending()
            && let Some((agent_id, stamp, session, legacy)) = self.pending_session_restore.take()
            && self.agent().id == agent_id
            && self.project_switch.accepts(&stamp)
        {
            self.restore_session(session, legacy);
        }
        if !matches!(
            self.project_switch.overlay,
            crate::project_context::SwitchOverlay::Busy { .. }
        ) {
            return;
        }
        if !self.project_tasks_busy() {
            if let Err(error) = self.advance_project_switch_after_tasks() {
                self.project_switch.cancel();
                self.status = format!("Project switch could not continue: {error}");
            }
            self.set_status_message(self.status.clone());
        } else if self.project_switch.quiesce_timed_out() {
            self.project_switch.cancel();
            self.status = format!(
                "Project switch stopped: owned task did not exit within {}s; project A remains active",
                PROJECT_SWITCH_STOP_TIMEOUT.as_secs()
            );
            self.set_status_message(self.status.clone());
        }
    }

    fn advance_project_switch_after_tasks(&mut self) -> Result<()> {
        use crate::project_context::SwitchOverlay;
        if !matches!(self.project_switch.overlay, SwitchOverlay::Busy { .. })
            || self.project_tasks_busy()
        {
            return Ok(());
        }
        self.project_switch.phase = crate::project_context::SwitchPhase::ResolveDirty;
        if self.project_dirty_objects().is_empty() {
            self.project_switch.overlay = SwitchOverlay::None;
            self.switch_after_resolution()?;
        } else {
            let Some(target) = self
                .project_switch
                .pending
                .as_ref()
                .map(|request| request.target.clone())
            else {
                self.project_switch.cancel();
                return Ok(());
            };
            self.project_switch.overlay = SwitchOverlay::Dirty { target };
            self.status = "Tasks stopped; resolve unsaved state before switching".to_string();
        }
        Ok(())
    }

    fn switch_after_resolution(&mut self) -> Result<()> {
        let Some(request) = self.project_switch.pending.clone() else {
            self.project_switch.cancel();
            return Ok(());
        };
        self.commit_project_switch(request.target)
    }

    fn save_project_state_for_switch(&mut self) -> Result<()> {
        if !self.pending_agent_changes.is_empty() {
            anyhow::bail!("an unapplied Agent proposal must be accepted or rejected first")
        }
        if let Some(active) = self.active_buffer_idx
            && self.buffer_dirty.get(active).copied().unwrap_or(false)
        {
            self.buffers[active] = self.buffer.clone();
        }
        for (index, dirty) in self.buffer_dirty.iter_mut().enumerate() {
            if *dirty {
                let path = self
                    .project
                    .features
                    .get(index)
                    .map(|feature| feature.file_path.clone())
                    .context("dirty Feature buffer has no source path")?;
                fs::write(&path, self.buffers[index].as_string())
                    .with_context(|| format!("write {}", path.display()))?;
                *dirty = false;
            }
        }
        if self.authoring_ui.buffer_dirty {
            self.authoring_ui
                .save_current_document(&self.project.root_dir)
                .context("save requirement draft")?;
        }
        if self.test_points_ui.field_dirty {
            self.commit_test_point_field_edit()?;
            if let Some(artifacts) = self.authoring_ui.artifacts.as_ref() {
                crate::test_points_tab::TestPointsUiState::save_test_points(
                    &self.project.root_dir,
                    artifacts,
                )?;
            }
        }
        self.sync_dirty_flag_with_active_buffer();
        Ok(())
    }

    fn capture_disk_stamps(project: &BddProject) -> Vec<Option<FileStamp>> {
        project
            .features
            .iter()
            .map(|feature| FileStamp::capture(&feature.file_path))
            .collect()
    }

    /// Refresh diagnostics for the raw buffer currently shown in the editor.
    ///
    /// A newly created, file-less buffer has no source location yet, so it is
    /// left without diagnostics until it is associated with a Feature file.
    pub fn refresh_buffer_diagnostics(&mut self) {
        let Some(path) = self.file_path.clone() else {
            self.buffer_diagnostics.clear();
            return;
        };
        self.buffer_diagnostics =
            teshi_core::validate_feature_source(&self.buffer.as_string(), &path).diagnostics;
    }

    /// Creates the empty editor fixture used by rendering and keymap tests.
    ///
    /// Tests must not treat the test harness arguments, repository contents, or
    /// host locale as an application launch. Startup behavior uses `from_cli`.
    #[cfg(test)]
    pub fn from_args() -> Result<Self> {
        Ok(Self::empty(crate::config::load_config()?))
    }

    /// Builds the editor state from parsed CLI arguments.
    pub fn from_cli(cli: &crate::cli::Cli) -> Result<Self> {
        let config_root = cli
            .paths
            .first()
            .map(Path::new)
            .map(|path| {
                if path.is_file() {
                    path.parent().unwrap_or(Path::new("."))
                } else {
                    path
                }
            })
            .unwrap_or(Path::new("."));
        let config = crate::config::load_config_for_project(config_root)?;
        let requirements_root =
            teshi_engine::requirements_data_dir(cli.requirements_root.as_deref())?;
        if cli.paths.is_empty() {
            return Self::from_project_directory(
                Path::new("."),
                config,
                cli.recursive,
                true,
                requirements_root,
            );
        }

        let paths: Vec<PathBuf> = cli.paths.iter().map(PathBuf::from).collect();

        let feature_file = paths
            .iter()
            .find(|p| p.extension().is_some_and(|ext| ext == "feature"));
        if let Some(p) = feature_file {
            return Self::from_file(p, config, requirements_root);
        }

        match paths.iter().find(|p| p.is_dir()) {
            Some(p) => {
                Self::from_project_directory(p, config, cli.recursive, false, requirements_root)
            }
            None => Ok(Self::empty_with_requirements(config, requirements_root)),
        }
    }

    fn from_project_directory(
        root: &Path,
        config: AppConfig,
        recursive: bool,
        library_view: bool,
        requirements_root: PathBuf,
    ) -> Result<Self> {
        let identity = teshi_engine::ProjectIdentity::new(root)?;
        let root = identity.root();
        let locale = std::env::var("TESHI_FEATURE_LANGUAGE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(sys_locale::get_locale);
        let discovery_root = if recursive {
            root.to_path_buf()
        } else {
            crate::feature_discovery::default_feature_directory(root, locale.as_deref())?
        };
        let scan_recursively = recursive || discovery_root != root;
        let project = BddProject {
            root_dir: root.to_path_buf(),
            features: Vec::new(),
        };
        let mut app = Self::from_loaded_directory(
            root,
            &discovery_root,
            config,
            scan_recursively,
            requirements_root,
            project,
        )?;
        app.feature_discovery_root = Some(discovery_root.clone());
        // Deleted Files must still be reported for a recursive project even though
        // only the selected Feature is parsed.
        app.git_status.scope = if scan_recursively {
            teshi_core::git::GitProjectScope::Recursive
        } else {
            teshi_core::git::GitProjectScope::CurrentFiles
        };
        app.lazy_features = Some(crate::lazy_features::LazyFeatures::new(
            discovery_root,
            scan_recursively,
        ));
        // Preserve library browsing for a bare launch; directory arguments keep their project filter.
        if library_view {
            app.authoring_ui
                .set_project_filter(teshi_core::authoring::RequirementProjectFilter::All);
        }
        Ok(app)
    }

    #[cfg(test)]
    fn from_directory(
        dir: &Path,
        config: AppConfig,
        recursive: bool,
        requirements_root: PathBuf,
    ) -> Result<Self> {
        Self::from_directory_with_discovery(dir, dir, config, recursive, requirements_root)
    }

    #[cfg(test)]
    fn from_directory_with_discovery(
        dir: &Path,
        discovery_root: &Path,
        config: AppConfig,
        recursive: bool,
        requirements_root: PathBuf,
    ) -> Result<Self> {
        let project = if recursive {
            gherkin::parse_project(discovery_root)
        } else {
            gherkin::parse_project_shallow(discovery_root)
        };
        Self::from_loaded_directory(
            dir,
            discovery_root,
            config,
            recursive,
            requirements_root,
            project,
        )
    }

    fn from_loaded_directory(
        dir: &Path,
        discovery_root: &Path,
        config: AppConfig,
        recursive: bool,
        requirements_root: PathBuf,
        mut project: BddProject,
    ) -> Result<Self> {
        project.root_dir = dir.to_path_buf();
        let git_scope = if dir != discovery_root {
            teshi_core::git::GitProjectScope::CurrentFiles
        } else if recursive {
            teshi_core::git::GitProjectScope::Recursive
        } else {
            teshi_core::git::GitProjectScope::Shallow
        };
        let git_status =
            teshi_core::git::FeatureGitStatusModel::empty_for_project(&project, git_scope);
        let step_index = StepIndex::build(&project);
        let mut mindmap_index = mindmap::build_index(&project);
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
        let tree_state = mindmap::init_tree_state(&mut mindmap_index);
        let project_root = project.root_dir.clone();
        let project_identity = teshi_engine::ProjectIdentity::new(&project_root)
            .context("resolve project identity")?;
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
            git_status,
            git_status_rx: None,
            git_status_refresh_requested: false,
            git_status_refresh_deadline: None,
            git_status_revision: 0,
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
            buffer_diagnostics: Vec::new(),
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
            mouse_position: None,
            editor_panel_rect: None,
            chat_layout_cache: Default::default(),
            chat_history_rect: None,
            chat_selection: None,
            system_clipboard: None,
            chat_selection_dragging: false,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            screenshot: Default::default(),
            capture_index: Default::default(),
            explore_captures: HashMap::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config_for_project(&project_root, None).ok(),
            runner_rx: None,
            runner_cancel: None,
            last_external_check: Instant::now(),
            feature_discovery_root: None,
            feature_discovery_worker: None,
            lazy_features: None,
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            feature_browser: Default::default(),
            explore_selected_deleted_feature: None,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_horizontal_scroll: [0; 3],
            explore_viewports: Default::default(),
            explore_horizontal_scroll_drag: None,
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
            explore_state: None,
            approval_mode: teshi_agent::approval::ApprovalMode::default(),
            approval_panel_active: false,
            approval_panel_selection: 0,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            quit_pending_confirm: false,
            quit_panel_selection: 0,
            status_message: None,
            status_message_deadline: None,
            version: String::new(),
            config,
            auth_panel_active: false,
            model_profiles: crate::profiles::load_all(),
            model_active_id: crate::profiles::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            model_panel_mode: ModelPanelMode::List,
            model_form_editing_id: None,
            model_form_focus: 0,
            model_form_cursor: 0,
            pending_delete: None,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            active_model_label: None,
            session_panel_active: false,
            session_panel_legacy: false,
            pending_session_restore: None,
            session_panel_selection: 0,
            session_list: Vec::new(),
            agent_registry: teshi_agent::registry::AgentRegistry::load(Some(dir)),
            agent_profile_panel_active: false,
            agent_profile_panel_selection: 0,
            agent_panel_mode: AgentPanelMode::List,
            installed_acp_agents: Vec::new(),
            acp_registry_agents: Vec::new(),
            acp_registry_rx: None,
            acp_install_rx: None,
            generation_stage: teshi_agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
            generation_scope: None,
            generation_scope_prompt: None,
            generation_paused_reason: None,
            authoring_ui: crate::authoring_tab::AuthoringUiState::load_from_project(
                &project_root,
                &requirements_root,
            ),
            test_points_ui: crate::test_points_tab::TestPointsUiState::empty(),
            requirements_root,
            project_switch: crate::project_context::ProjectSwitchState::new(project_identity),
        };
        app.refresh_git_status();
        app.restore_generation_session_from_disk();
        app.spawn_llm_if_configured();
        app.activate_active_profile();
        app.mindmap_index.apply_highlight_categories("root");
        let n = app.buffers.len();
        app.status = format!("Opened directory with {n} feature file(s)");
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        app.show_selection_warning();
        app.refresh_capture_index();
        Ok(app)
    }

    pub(crate) fn from_file(
        path: &PathBuf,
        config: AppConfig,
        requirements_root: PathBuf,
    ) -> Result<Self> {
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
        let git_status = teshi_core::git::FeatureGitStatusModel::empty_for_project(
            &project,
            teshi_core::git::GitProjectScope::CurrentFiles,
        );
        let step_index = StepIndex::build(&project);
        let mut mindmap_index = mindmap::build_index(&project);
        let buffers = vec![EditorBuffer::from_string(content.clone())];
        let buffer_dirty = vec![false; buffers.len()];
        let disk_stamps = Self::capture_disk_stamps(&project);
        let tree_state = mindmap::init_tree_state(&mut mindmap_index);
        let project_root = project.root_dir.clone();
        let project_identity = teshi_engine::ProjectIdentity::new(&project_root)
            .context("resolve project identity")?;
        let mut app = Self {
            project,
            git_status,
            git_status_rx: None,
            git_status_refresh_requested: false,
            git_status_refresh_deadline: None,
            git_status_revision: 0,
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
            buffer_diagnostics: Vec::new(),
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
            explore_state: None,
            approval_mode: teshi_agent::approval::ApprovalMode::default(),
            approval_panel_active: false,
            approval_panel_selection: 0,
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
            mouse_position: None,
            editor_panel_rect: None,
            chat_layout_cache: Default::default(),
            chat_history_rect: None,
            chat_selection: None,
            system_clipboard: None,
            chat_selection_dragging: false,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            screenshot: Default::default(),
            capture_index: Default::default(),
            explore_captures: HashMap::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config_for_project(&project_root, None).ok(),
            runner_rx: None,
            runner_cancel: None,
            last_external_check: Instant::now(),
            feature_discovery_root: None,
            feature_discovery_worker: None,
            lazy_features: None,
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            feature_browser: Default::default(),
            explore_selected_deleted_feature: None,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_horizontal_scroll: [0; 3],
            explore_viewports: Default::default(),
            explore_horizontal_scroll_drag: None,
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
            quit_panel_selection: 0,
            status_message: None,
            status_message_deadline: None,
            version: String::new(),
            config,
            auth_panel_active: false,
            model_profiles: crate::profiles::load_all(),
            model_active_id: crate::profiles::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            model_panel_mode: ModelPanelMode::List,
            model_form_editing_id: None,
            model_form_focus: 0,
            model_form_cursor: 0,
            pending_delete: None,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            active_model_label: None,
            session_panel_active: false,
            session_panel_legacy: false,
            pending_session_restore: None,
            session_panel_selection: 0,
            session_list: Vec::new(),
            agent_registry: {
                let root_dir = path.parent().unwrap_or(Path::new("."));
                teshi_agent::registry::AgentRegistry::load(Some(root_dir))
            },
            agent_profile_panel_active: false,
            agent_profile_panel_selection: 0,
            agent_panel_mode: AgentPanelMode::List,
            installed_acp_agents: Vec::new(),
            acp_registry_agents: Vec::new(),
            acp_registry_rx: None,
            acp_install_rx: None,
            generation_stage: teshi_agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
            generation_scope: None,
            generation_scope_prompt: None,
            generation_paused_reason: None,
            authoring_ui: crate::authoring_tab::AuthoringUiState::load_from_project(
                &project_root,
                &requirements_root,
            ),
            test_points_ui: crate::test_points_tab::TestPointsUiState::empty(),
            requirements_root,
            project_switch: crate::project_context::ProjectSwitchState::new(project_identity),
        };
        app.refresh_git_status();
        app.restore_generation_session_from_disk();
        app.spawn_llm_if_configured();
        app.activate_active_profile();
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        app.mindmap_index.apply_highlight_categories("root");
        app.show_selection_warning();
        app.refresh_capture_index();
        Ok(app)
    }

    #[cfg(test)]
    fn empty(config: AppConfig) -> Self {
        Self::empty_with_requirements(
            config,
            teshi_engine::requirements_data_dir(None).unwrap_or_default(),
        )
    }

    fn empty_with_requirements(config: AppConfig, requirements_root: PathBuf) -> Self {
        let project = BddProject {
            root_dir: PathBuf::from("."),
            features: Vec::new(),
        };
        let project_root = project.root_dir.clone();
        let project_identity = teshi_engine::ProjectIdentity::new(&project_root)
            .unwrap_or_else(|_| teshi_engine::ProjectIdentity::from_root(&project_root));
        let step_index = StepIndex::build(&project);
        let mut mindmap_index = mindmap::build_index(&project);
        let tree_state = mindmap::init_tree_state(&mut mindmap_index);
        let mut app = Self {
            project,
            git_status: teshi_core::git::FeatureGitStatusModel::default(),
            git_status_rx: None,
            git_status_refresh_requested: false,
            git_status_refresh_deadline: None,
            git_status_revision: 0,
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
            buffer_diagnostics: Vec::new(),
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
            explore_state: None,
            approval_mode: teshi_agent::approval::ApprovalMode::default(),
            approval_panel_active: false,
            approval_panel_selection: 0,
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
            mouse_position: None,
            editor_panel_rect: None,
            chat_layout_cache: Default::default(),
            chat_history_rect: None,
            chat_selection: None,
            system_clipboard: None,
            chat_selection_dragging: false,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            screenshot: Default::default(),
            capture_index: Default::default(),
            explore_captures: HashMap::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: runner::load_runner_config_for_project(&project_root, None).ok(),
            runner_rx: None,
            runner_cancel: None,
            last_external_check: Instant::now(),
            feature_discovery_root: None,
            feature_discovery_worker: None,
            lazy_features: None,
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            feature_browser: Default::default(),
            explore_selected_deleted_feature: None,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_horizontal_scroll: [0; 3],
            explore_viewports: Default::default(),
            explore_horizontal_scroll_drag: None,
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
            quit_panel_selection: 0,
            status_message: None,
            status_message_deadline: None,
            version: String::new(),
            config,
            auth_panel_active: false,
            model_profiles: crate::profiles::load_all(),
            model_active_id: crate::profiles::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            model_panel_mode: ModelPanelMode::List,
            model_form_editing_id: None,
            model_form_focus: 0,
            model_form_cursor: 0,
            pending_delete: None,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            active_model_label: None,
            session_panel_active: false,
            session_panel_legacy: false,
            pending_session_restore: None,
            session_panel_selection: 0,
            session_list: Vec::new(),
            agent_registry: teshi_agent::registry::AgentRegistry::load(None),
            agent_profile_panel_active: false,
            agent_profile_panel_selection: 0,
            agent_panel_mode: AgentPanelMode::List,
            installed_acp_agents: Vec::new(),
            acp_registry_agents: Vec::new(),
            acp_registry_rx: None,
            acp_install_rx: None,
            generation_stage: teshi_agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
            generation_scope: None,
            generation_scope_prompt: None,
            generation_paused_reason: None,
            // Requirement documents live in the user-level store, not the BDD
            // project. `teshi` with no PATH still opens tab [4] against that store.
            authoring_ui: crate::authoring_tab::AuthoringUiState::load_from_project(
                &project_root,
                &requirements_root,
            ),
            test_points_ui: crate::test_points_tab::TestPointsUiState::empty(),
            requirements_root,
            project_switch: crate::project_context::ProjectSwitchState::new(project_identity),
        };
        app.authoring_ui
            .set_project_filter(teshi_core::authoring::RequirementProjectFilter::All);
        app.spawn_llm_if_configured();
        app.activate_active_profile();
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
        app.mindmap_index.apply_highlight_categories("root");
        app.show_selection_warning();
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

    fn explore_feature_item_count(&self) -> usize {
        self.feature_browser_rows().len()
    }

    pub(crate) fn explore_selected_feature_row(&self) -> Option<usize> {
        self.feature_browser_rows()
            .iter()
            .position(|row| match row {
                crate::feature_browser::Row::Directory(path, _) => {
                    self.feature_browser.directory.as_ref() == Some(path)
                }
                crate::feature_browser::Row::File(index, _, deleted) => {
                    self.feature_browser.directory.is_none()
                        && if *deleted {
                            self.explore_selected_deleted_feature == Some(*index)
                        } else {
                            self.explore_selected_deleted_feature.is_none()
                                && if let Some(catalog) = &self.lazy_features {
                                    catalog
                                        .selected
                                        .as_ref()
                                        .is_some_and(|path| catalog.paths.get(*index) == Some(path))
                                } else {
                                    self.explore_selected_feature == *index
                                }
                        }
                }
            })
    }

    fn explore_selected_scenario_count(&self) -> usize {
        if let Some(deleted_idx) = self.explore_selected_deleted_feature {
            return self
                .git_status
                .deleted
                .get(deleted_idx)
                .map(|view| view.deleted_scenarios().count())
                .unwrap_or(0);
        }
        let current_count = self
            .project
            .features
            .get(self.explore_selected_feature)
            .map(|feature| feature.scenario_count())
            .unwrap_or(0);
        let deleted_count = self
            .git_status
            .current_at(self.explore_selected_feature)
            .map(|view| view.deleted_scenarios().count())
            .unwrap_or(0);
        current_count + deleted_count
    }

    fn explore_current_scenario_count(&self) -> usize {
        self.project
            .features
            .get(self.explore_selected_feature)
            .map(|feature| feature.scenario_count())
            .unwrap_or(0)
    }

    fn explore_selected_scenario_is_deleted(&self) -> bool {
        self.explore_selected_deleted_feature.is_none()
            && self.explore_selected_scenario >= self.explore_current_scenario_count()
            && self.explore_selected_scenario < self.explore_selected_scenario_count()
    }

    fn explore_selected_step_count(&self) -> usize {
        let changes = self.explore_changed_steps();
        if !changes.is_empty() {
            return changes.len();
        }
        if self.explore_selected_deleted_feature.is_some()
            || self.explore_selected_scenario_is_deleted()
        {
            return 0;
        }
        self.project
            .features
            .get(self.explore_selected_feature)
            .and_then(|feature| feature.scenario_at(self.explore_selected_scenario))
            .map(|scenario| scenario.steps.len())
            .unwrap_or(0)
    }

    /// Changed Step entries, including removed Steps that have no current AST index.
    pub(crate) fn explore_changed_steps(&self) -> Vec<ChangedStep> {
        let feature = self.project.features.get(self.explore_selected_feature);
        let view = self
            .explore_selected_deleted_feature
            .and_then(|index| self.git_status.deleted.get(index))
            .or_else(|| self.git_status.current_at(self.explore_selected_feature));
        let language_code = if let Some(index) = self.explore_selected_deleted_feature {
            self.git_status
                .deleted
                .get(index)
                .and_then(|v| v.diff.as_ref())
                .and_then(|d| d.old_content.as_deref())
                .map(teshi_core::gherkin_lang::GherkinLanguages::detect_from_content)
                .unwrap_or("en")
        } else {
            feature.map_or("en", |f| f.language.as_str())
        };
        let language = teshi_core::gherkin_lang::GherkinLanguages::global().get(language_code);
        // Text, change kind, and the 1-based line in the current file when the
        // line still exists there.
        let source: Vec<(String, ChangeKind, Option<usize>)> =
            if let Some(lines) = &self.explore_diff_lines {
                lines
                    .iter()
                    .map(|line| {
                        (
                            line.text.clone(),
                            line.kind,
                            (line.line_number_1based > 0).then_some(line.line_number_1based),
                        )
                    })
                    .collect()
            } else if let Some(view) = view {
                let lines = if self.explore_selected_deleted_feature.is_some() {
                    view.deleted_scenarios()
                        .nth(self.explore_selected_scenario)
                        .map(|s| s.diff_lines.as_slice())
                        .unwrap_or_default()
                } else if let Some(scenario) =
                    feature.and_then(|f| f.scenario_at(self.explore_selected_scenario))
                {
                    view.scenario_diff_lines(scenario).unwrap_or_default()
                } else {
                    view.deleted_scenarios()
                        .nth(
                            self.explore_selected_scenario
                                .saturating_sub(self.explore_current_scenario_count()),
                        )
                        .map(|s| s.diff_lines.as_slice())
                        .unwrap_or_default()
                };
                lines
                    .iter()
                    .map(|line| {
                        (
                            line.text.clone(),
                            match line.status {
                                teshi_core::git::DiffStatus::Added => ChangeKind::Added,
                                teshi_core::git::DiffStatus::Deleted => ChangeKind::Deleted,
                                teshi_core::git::DiffStatus::Modified => ChangeKind::Modified,
                                teshi_core::git::DiffStatus::Unchanged => ChangeKind::Unchanged,
                            },
                            line.new_line_number,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
        // Only a live Feature can supply keyword colours; removed Features have
        // no AST counterpart in the project.
        let scenario = if self.explore_selected_deleted_feature.is_none() {
            feature.and_then(|f| f.scenario_at(self.explore_selected_scenario))
        } else {
            None
        };
        let mut steps: Vec<ChangedStep> = Vec::new();
        let mut current = None;
        let mut doc_string = false;
        for (text, kind, line_number) in source {
            let trimmed = text.trim_start();
            let fence = trimmed.starts_with("\"\"\"") || trimmed.starts_with("```");
            if !doc_string && !fence && language.is_step_line(trimmed) {
                steps.push(ChangedStep {
                    keyword_kind: ast_keyword_kind(scenario, line_number, trimmed),
                    text,
                    kind,
                });
                current = Some(steps.len() - 1);
            } else if fence || doc_string || trimmed.starts_with('|') {
                if let Some(index) = current {
                    steps[index].text.push('\n');
                    steps[index].text.push_str(&text);
                    if !matches!(kind, ChangeKind::Unchanged)
                        && matches!(steps[index].kind, ChangeKind::Unchanged)
                    {
                        steps[index].kind = ChangeKind::Modified;
                    }
                }
                if fence {
                    doc_string = !doc_string;
                }
            } else if language.is_structural(trimmed) {
                current = None;
            }
        }
        steps.retain(|step| !matches!(step.kind, ChangeKind::Unchanged));
        steps
    }

    pub(crate) fn explore_set_feature_row(&mut self, row: usize) {
        use crate::feature_browser::Row;
        match self.feature_browser_rows().get(row).cloned() {
            Some(Row::Directory(path, _)) => self.feature_browser.directory = Some(path),
            Some(Row::File(index, _, deleted)) => {
                self.feature_browser.directory = None;
                if deleted {
                    self.explore_set_deleted_feature(index);
                } else if self.lazy_features.is_some() {
                    self.select_catalog_feature(index);
                } else {
                    self.explore_set_feature(index);
                }
            }
            None => {}
        }
    }

    fn normalize_explore_selection(&mut self) {
        if let Some(deleted_idx) = self.explore_selected_deleted_feature {
            if deleted_idx >= self.git_status.deleted.len() {
                self.explore_selected_deleted_feature = None;
            } else {
                let scenario_count = self.git_status.deleted[deleted_idx]
                    .deleted_scenarios()
                    .count();
                if scenario_count == 0 {
                    self.explore_selected_scenario = 0;
                } else if self.explore_selected_scenario >= scenario_count {
                    self.explore_selected_scenario = scenario_count - 1;
                }
                self.explore_selected_step = 0;
                return;
            }
        }

        let feature_len = self.project.features.len();
        if feature_len == 0 {
            if !self.git_status.deleted.is_empty() {
                self.explore_selected_deleted_feature = Some(0);
                self.explore_selected_scenario = 0;
                self.explore_selected_step = 0;
            } else {
                self.explore_selected_feature = 0;
                self.explore_selected_scenario = 0;
                self.explore_selected_step = 0;
            }
            return;
        }
        if self.explore_selected_feature >= feature_len {
            self.explore_selected_feature = feature_len - 1;
        }
        let scenario_count = self.explore_selected_scenario_count();
        if scenario_count == 0 {
            self.explore_selected_scenario = 0;
            self.explore_selected_step = 0;
            return;
        }
        if self.explore_selected_scenario >= scenario_count {
            self.explore_selected_scenario = scenario_count - 1;
        }
        if self.explore_selected_scenario_is_deleted() {
            self.explore_selected_step = 0;
            return;
        }
        let feature = &self.project.features[self.explore_selected_feature];
        let scenarios = feature.all_scenarios();
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
        self.bump_git_status_revision();
        self.schedule_git_status_refresh();
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
            self.poll_project_switch();
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
        } else {
            self.runner_cancel = None;
            if let Err(error) = self.advance_project_switch_after_tasks() {
                self.project_switch.cancel();
                self.status = format!("Project switch could not continue: {error}");
            }
        }
        self.poll_project_switch();
    }

    /// Spawn the LLM worker when the shared profile store or `TESHI_LLM_*` is usable.
    pub fn spawn_llm_if_configured(&mut self) {
        if self.agent().backend.kind() == teshi_agent::backend::AgentBackendKind::Acp {
            return;
        }
        if self.agent_mut().backend.is_connected() {
            return;
        }
        match crate::llm::effective_config() {
            Ok(config) if !config.api_key.trim().is_empty() => {
                let label = teshi_engine::load_active_profile()
                    .ok()
                    .flatten()
                    .map(|p| format!("{} ({})", p.name, config.model))
                    .unwrap_or_else(|| format!("env: {}", config.model));
                self.active_model_label = Some(label);
                self.status = format!(
                    "LLM configured: model={}, base_url={}, provider={}",
                    config.model, config.base_url, config.provider
                );
                if let Err(error) = self.agent_mut().backend.attach_native_model(config) {
                    self.status = format!("AI backend unavailable: {error}");
                }
            }
            Ok(_) => {
                self.status =
                    "LLM not configured: set an API key on a model profile or TESHI_LLM_API_KEY"
                        .to_string();
            }
            Err(e) => {
                self.status = format!("LLM not configured: {e}");
            }
        }
    }

    fn show_selection_warning(&mut self) {
        if let Some(warning) = self.agent().selection_warning() {
            self.status = warning.to_owned();
        }
    }

    fn cancel_agent_turn(&mut self) {
        let agent = self.agent_mut();
        agent.retain_partial_response();
        agent.backend.cancel();
        agent.status = AiStatus::Idle;
        agent.tool_status = None;
        agent.pending_acp_permission = None;
    }

    /// Whether the selected agent has a pending ACP permission prompt. The
    /// keymap intercepts navigation keys while this is true.
    pub fn has_acp_permission_prompt(&self) -> bool {
        self.agent().pending_acp_permission.is_some()
    }

    /// Move the ACP permission prompt selection by `delta` (clamped).
    pub fn move_acp_permission_selection(&mut self, delta: isize) {
        let Some(prompt) = self.agent_mut().pending_acp_permission.as_mut() else {
            return;
        };
        let last = prompt.options.len().saturating_sub(1) as isize;
        let next = (prompt.selected as isize + delta).clamp(0, last);
        prompt.selected = next as usize;
    }

    /// Answer the selected agent's pending ACP permission prompt. `None`
    /// rejects the request and lets the agent decide how to continue.
    pub fn respond_acp_permission(&mut self, option_id: Option<String>) {
        let agent_idx = self.selected_agent;
        let Some(prompt) = self.agents[agent_idx].pending_acp_permission.take() else {
            return;
        };
        let resolved = self.agents[agent_idx]
            .backend
            .respond_permission(prompt.request_id, option_id);
        self.agents[agent_idx].status = if resolved {
            AiStatus::Waiting
        } else {
            AiStatus::Error
        };
        self.agents[agent_idx].tool_status = None;
        self.status = if resolved {
            "Permission sent to ACP agent...".to_string()
        } else {
            "ACP permission request is no longer pending".to_string()
        };
    }

    pub(crate) fn ensure_agent_backend(&mut self, agent_idx: usize) -> bool {
        if self.agents[agent_idx].backend.kind() == teshi_agent::backend::AgentBackendKind::Acp
            && self.agents[agent_idx].backend.status()
                == teshi_agent::backend::AgentBackendStatus::Unavailable
        {
            let cwd = self.find_project_dir().to_path_buf();
            let command = if let Some(id) = self.agents[agent_idx].selected_acp_id.as_ref() {
                let root = self.agents[agent_idx]
                    .selection_root
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(crate::acp_agents::store_dir);
                root.and_then(|root| {
                    crate::acp_agents::load(&root)?
                        .into_iter()
                        .find(|item| &item.id == id)
                        .map(|item| item.command(cwd))
                        .transpose()?
                        .ok_or_else(|| anyhow::anyhow!("ACP Agent {id} is not installed"))
                })
            } else {
                match std::env::var_os("TESHI_ACP_PROGRAM") {
                    Some(program) => {
                        let args = std::env::var("TESHI_ACP_ARGS_JSON")
                            .ok()
                            .map(|value| serde_json::from_str::<Vec<String>>(&value))
                            .transpose();
                        match args {
                            Ok(args) => Ok(teshi_acp::AcpAgentCommand {
                                program: program.into(),
                                args: args.unwrap_or_default(),
                                cwd,
                                env: std::collections::HashMap::new(),
                            }),
                            Err(error) => {
                                Err(anyhow::anyhow!("invalid TESHI_ACP_ARGS_JSON: {error}"))
                            }
                        }
                    }
                    None => Err(anyhow::anyhow!(
                        "No ACP Agent selected; use /agent or set TESHI_ACP_PROGRAM"
                    )),
                }
            };
            match command.and_then(|command| {
                let mut config = teshi_agent_runtime::backend::AcpBackendConfig::new(command);
                config.tool_bridge = Some(teshi_agent_runtime::tool_bridge::ToolBridgeConfig {
                    executable: std::env::current_exe()?,
                    tools: self.agent_tools(agent_idx),
                });
                if let Some(id) = self.agents[agent_idx].selected_acp_id.as_ref() {
                    let root = self.agents[agent_idx]
                        .selection_root
                        .clone()
                        .map(Ok)
                        .unwrap_or_else(crate::acp_agents::store_dir)?;
                    let key: String = id
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    config.model_preference_path =
                        Some(root.join("model-preferences").join(format!("{key}.json")));
                }
                config.auth_method = std::env::var("TESHI_ACP_AUTH_METHOD").ok();
                // The approval badge drives the ACP permission policy: Manual
                // prompts through a PermissionHost (Zed-style allow/reject
                // list), Auto and Bypass answer inside the ACP client.
                match self.approval_mode {
                    teshi_agent::approval::ApprovalMode::Manual => {
                        config.permission_policy = teshi_acp::PermissionPolicy::Manual;
                        config.permission_host =
                            Some(teshi_agent_runtime::ChannelPermissionHost::new());
                    }
                    teshi_agent::approval::ApprovalMode::Auto => {
                        config.permission_policy = teshi_acp::PermissionPolicy::Auto;
                    }
                    teshi_agent::approval::ApprovalMode::Bypass => {
                        config.permission_policy = teshi_acp::PermissionPolicy::Bypass;
                    }
                }
                // Explicit override wins for scripted runs and tests.
                if std::env::var("TESHI_ACP_PERMISSION_POLICY").as_deref() == Ok("auto") {
                    config.permission_policy = teshi_acp::PermissionPolicy::Auto;
                }
                config.permission_host = Some(teshi_agent_runtime::ChannelPermissionHost::new());
                self.agents[agent_idx].backend.configure_acp(config)
            }) {
                Ok(()) => {}
                Err(error) => {
                    self.agents[agent_idx].status = AiStatus::Error;
                    self.status = format!("AI backend unavailable: {error}");
                    return false;
                }
            }
        }
        true
    }

    fn start_agent_backend(&mut self, agent_idx: usize) -> bool {
        if !self.ensure_agent_backend(agent_idx) {
            return false;
        }
        match self.agents[agent_idx].backend.start() {
            Ok(()) => true,
            Err(error) => {
                self.agents[agent_idx].status = AiStatus::Error;
                self.status = format!("AI backend unavailable: {error}");
                self.set_status_message(self.status.clone());
                false
            }
        }
    }

    /// Activate the saved active profile (if any) that has a matching file on disk.
    pub fn activate_active_profile(&mut self) {
        if self.agent().backend_kind() == teshi_agent::backend::AgentBackendKind::Acp {
            return;
        }
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

    /// Respawn the LLM worker thread using the given shared model profile.
    fn activate_model_profile(&mut self, profile: &crate::profiles::ModelProfile) {
        let config = match teshi_engine::profile_to_llm_config(profile) {
            Ok(cfg) => cfg,
            Err(e) => {
                self.status = format!("Failed to activate profile: {e}");
                return;
            }
        };
        if config.api_key.trim().is_empty() {
            self.status = format!(
                "Profile '{}' has no API key; set one or use TESHI_LLM_API_KEY",
                profile.name
            );
            return;
        }
        if let Err(error) = self.agent_mut().backend.attach_native_model(config) {
            self.status = format!("AI backend unavailable: {error}");
            return;
        }
        self.agent_mut().status = AiStatus::Idle;
        self.agent_mut().tool_status = None;
        self.active_model_label = Some(format!("{} ({})", profile.name, profile.model_id));
        self.status = format!("Switched to model: {}", profile.name);
    }

    /// Get the agent definition for a given agent index.
    fn agent_profile(&self, agent_idx: usize) -> Option<&teshi_agent::definition::AgentDefinition> {
        let profile_id = self.agents.get(agent_idx)?.profile_id.as_deref();
        let id = profile_id.unwrap_or("default");
        self.agent_registry.get(id)
    }

    fn agent_tools(&self, agent_idx: usize) -> Vec<teshi_core::llm::ToolDefinition> {
        match self.agent_profile(agent_idx).map(|profile| &profile.tools) {
            Some(teshi_agent::definition::ToolPermission::None) => Vec::new(),
            Some(teshi_agent::definition::ToolPermission::Whitelist(names)) if names.is_empty() => {
                Vec::new()
            }
            Some(teshi_agent::definition::ToolPermission::Whitelist(names)) => {
                teshi_agent::get_tools(Some(names))
            }
            _ => teshi_agent::get_tools(None),
        }
    }

    /// Get the active agent definition for the selected agent.
    pub(crate) fn active_agent_profile(&self) -> &teshi_agent::definition::AgentDefinition {
        self.agent_profile(self.selected_agent)
            .unwrap_or_else(|| self.agent_registry.default())
    }

    /// Try to find a sensible project root directory.
    pub(crate) fn find_project_dir(&self) -> &Path {
        &self.project.root_dir
    }

    /// Apply backend progress to the TUI and provide Teshi's host tools.
    /// The backend owns model events, conversation records, and continuation.
    pub fn poll_llm_events(&mut self) {
        for agent in &mut self.agents {
            if matches!(agent.status, AiStatus::Waiting | AiStatus::AwaitingApproval) {
                let start = agent.task_started.get_or_insert_with(Instant::now);
                agent.task_elapsed = start.elapsed();
            } else if let Some(start) = agent.task_started.take() {
                agent.task_elapsed = start.elapsed();
            }
        }
        use teshi_agent::backend::AgentBackendEvent;
        self.discover_visible_acp_controls();
        self.sync_acp_policy();
        for i in 0..self.agents.len() {
            self.poll_acp_host_tools(i);
            loop {
                match self.agents[i].backend.try_next_event() {
                    Ok(Some(AgentBackendEvent::Ignored)) => continue,
                    Ok(Some(AgentBackendEvent::Completed {
                        model,
                        input_tokens,
                        output_tokens,
                        finish_reason,
                    })) => {
                        self.agents[i].last_finish_reason = finish_reason;
                        let interrupted = matches!(
                            self.agents[i].last_finish_reason.as_deref(),
                            Some("refusal" | "max_tokens" | "max_turn_requests")
                        ) && self.agents[i].backend_kind()
                            == teshi_agent::backend::AgentBackendKind::Acp;
                        if interrupted {
                            let reason = self.agents[i]
                                .last_finish_reason
                                .as_deref()
                                .unwrap_or("unknown");
                            let content = format!(
                                "Task interrupted: agent returned {reason}. The task may be incomplete. Use /retry or Continue task to continue from the last checkpoint."
                            );
                            self.agents[i].messages_mut().push(AiChatMessage {
                                role: AiRole::Assistant,
                                content,
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: None,
                                source: Some("agent_progress".into()),
                            });
                        }
                        self.agents[i].status = AiStatus::Idle;
                        self.agents[i].tool_status = None;
                        // The agent finished without waiting for an answer.
                        self.agents[i].pending_acp_permission = None;
                        self.agents[i].total_input_tokens += input_tokens.unwrap_or(0) as u64;
                        self.agents[i].last_input_tokens = input_tokens;
                        self.agents[i].total_output_tokens += output_tokens.unwrap_or(0) as u64;
                        if i == self.selected_agent {
                            self.status = if interrupted {
                                "Task interrupted — /retry to continue".into()
                            } else {
                                format!("AI response received ({model})")
                            };
                        }
                    }
                    Ok(Some(AgentBackendEvent::ToolActivity {
                        tool_calls,
                        assistant_message_index: assistant_msg_idx,
                        finish_reason,
                    })) => {
                        self.agents[i].last_finish_reason = finish_reason;
                        let mut runtime = self.agents[i].backend.take_for_host_call();
                        let pending_queued = runtime.execute_tool_batch(
                            assistant_msg_idx,
                            &tool_calls,
                            |tc| {
                                self.agents[i].tool_status =
                                    Some(format!("AI is calling {}...", tc.name));
                                let pending_before = self.pending_agent_changes.len();
                                let result = teshi_agent::execute_tool(
                                    self,
                                    &tc.name,
                                    &tc.arguments,
                                    &tc.id,
                                    i,
                                )
                                .map_err(|error| error.to_string());
                                let pending_approval =
                                    result.is_ok() && self.pending_agent_changes.len() > pending_before;
                                let observation_message = if result.is_ok() && tc.name == "observe_page" {
                                    self.agents[i].latest_visual_observation.as_ref().map(|observation| {
                                        let mut content = "[Browser visual observation]\nCurrent viewport after the requested observation.".to_string();
                                        if let Some(url) = &observation.url {
                                            content.push_str("\nURL: ");
                                            content.push_str(url);
                                        }
                                        if let Some(title) = &observation.title {
                                            content.push_str("\nTitle: ");
                                            content.push_str(title);
                                        }
                                        content
                                    })
                                } else {
                                    None
                                };
                                teshi_agent::backend::AgentToolOutcome {
                                    result,
                                    pending_permission: pending_approval,
                                    observation_message,
                                }
                            },
                        );
                        self.agents[i].backend = runtime;
                        if pending_queued && !self.approval_mode.requires_manual_approval() {
                            self.agents[i].partial_response_mut().clear();
                            self.agents[i].tool_status = None;
                            while self.has_agent_change_prompt() {
                                let agent_idx = self.pending_agent_changes[0].agent_idx;
                                let (tool_call_id, result) = match self.accept_agent_change() {
                                    Ok((id, res)) => (id, res),
                                    Err(e) => {
                                        self.pending_agent_changes.remove(0);
                                        self.clear_pending_change_state();
                                        (String::new(), format!("Error: {e}"))
                                    }
                                };
                                if !tool_call_id.is_empty() {
                                    self.agents[agent_idx].messages_mut().push(AiChatMessage {
                                        role: AiRole::Tool,
                                        content: result,
                                        tool_calls: None,
                                        tool_call_id: Some(tool_call_id),
                                        reasoning_content: None,
                                        source: None,
                                    });
                                }
                            }
                        }
                        use teshi_agent::backend::AgentBackendDecision;
                        let has_feature_files = self.has_feature_files();
                        let decision = self.agents[i].backend.after_tools(
                            self.generation_stage,
                            pending_queued,
                            self.approval_mode.requires_manual_approval(),
                            has_feature_files,
                        );
                        match decision {
                            AgentBackendDecision::Continue { allow_tools } => {
                                if !allow_tools && i == self.selected_agent {
                                    self.status =
                                        "Tool call limit reached, requesting final response…"
                                            .to_string();
                                }
                                self.send_backend_followup(i, allow_tools);
                            }
                            AgentBackendDecision::WaitForPermission => {
                                self.agents[i].partial_response_mut().clear();
                                self.agents[i].status = AiStatus::AwaitingApproval;
                                self.agents[i].tool_status = None;
                                if !matches!(
                                    self.generation_stage,
                                    teshi_agent::pipeline::GenerationStage::Idle
                                        | teshi_agent::pipeline::GenerationStage::Complete
                                ) {
                                    self.generation_stage =
                                        teshi_agent::pipeline::GenerationStage::Confirming;
                                }
                            }
                            AgentBackendDecision::WaitForHumanReview => {
                                self.agents[i].partial_response_mut().clear();
                                self.agents[i].status = AiStatus::Idle;
                                self.agents[i].tool_status = None;
                                if i == self.selected_agent {
                                    self.status = "Review proposed test points in Test Points tab [5], then press c to continue generation".to_string();
                                }
                            }
                            AgentBackendDecision::StopEmptyProject => {
                                self.agents[i].partial_response_mut().clear();
                                self.agents[i].status = AiStatus::Idle;
                                self.agents[i].tool_status = None;
                                if i == self.selected_agent {
                                    self.status = "The project directory has no .feature files. Add one to begin.".to_string();
                                }
                            }
                            AgentBackendDecision::StopAfterFailure => {
                                self.agents[i].status = AiStatus::Error;
                                self.status =
                                    "AI error: model kept calling tools after final-response limit"
                                        .to_string();
                            }
                            AgentBackendDecision::Cancelled => {}
                        }
                    }
                    Ok(Some(AgentBackendEvent::Failed(message))) => {
                        self.agents[i].retain_partial_response();
                        self.agents[i].status = AiStatus::Idle;
                        self.agents[i].tool_status = None;
                        self.agents[i].pending_acp_permission = None;
                        if i == self.selected_agent {
                            self.status = format!("AI: {message}");
                            self.set_status_message(self.status.clone());
                        }
                    }
                    Ok(Some(AgentBackendEvent::ExternalToolActivity { title, status, .. })) => {
                        self.agents[i].tool_status = Some(match status {
                            Some(status) => format!("{title}: {status}"),
                            None => title,
                        });
                    }
                    Ok(Some(AgentBackendEvent::PermissionRequest {
                        request_id,
                        options,
                    })) => {
                        // Park the turn until the user picks an option (or
                        // rejects). Empty option lists reject immediately.
                        if options.is_empty() {
                            self.agents[i].backend.respond_permission(request_id, None);
                            continue;
                        }
                        self.agents[i].pending_acp_permission = Some(AcpPermissionPrompt {
                            request_id,
                            options,
                            selected: 0,
                        });
                        self.agents[i].status = AiStatus::AwaitingApproval;
                        self.agents[i].tool_status = None;
                        if i == self.selected_agent {
                            self.status =
                                "ACP agent requests permission — Enter to allow, Esc to reject"
                                    .to_string();
                        }
                    }
                    Ok(Some(
                        AgentBackendEvent::MessageChunk { .. }
                        | AgentBackendEvent::ReasoningChunk { .. },
                    )) => {}
                    Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.agents[i].retain_partial_response();
                        self.agents[i].status = AiStatus::Error;
                        self.agents[i].tool_status = None;
                        self.agents[i].pending_acp_permission = None;
                        if i == self.selected_agent {
                            self.status = "AI error: background LLM thread has exited".to_string();
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Persist all conversations periodically, or immediately before leaving one.
    ///
    /// # Errors
    /// Returns an error if a conversation checkpoint cannot be written.
    pub fn save_sessions(&mut self, force: bool) -> Result<()> {
        let root = crate::session::Session::storage_dir();
        let project = self.project.root_dir.clone();
        let model = self.active_model_label.clone();
        for agent in &mut self.agents {
            if !force
                && agent
                    .last_session_save
                    .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1))
            {
                continue;
            }
            agent.save_session_to(&root, &project, model.clone())?;
            agent.last_session_save = Some(std::time::Instant::now());
        }
        Ok(())
    }

    fn poll_acp_host_tools(&mut self, agent_idx: usize) {
        while let Some(request) = self.agents[agent_idx].backend.host_tool_request() {
            if !request.is_pending() {
                continue;
            }
            if self.agents[agent_idx].backend.status()
                != teshi_agent::backend::AgentBackendStatus::Running
            {
                request.respond(Err("ACP conversation has no active turn".into()));
                continue;
            }
            let allowed =
                self.agent_profile(agent_idx)
                    .is_none_or(|profile| match &profile.tools {
                        teshi_agent::definition::ToolPermission::All => true,
                        teshi_agent::definition::ToolPermission::None => false,
                        teshi_agent::definition::ToolPermission::Whitelist(names) => {
                            names.contains(&request.call.name)
                        }
                    });
            if !allowed {
                request.respond(Err(
                    "Tool is not allowed by the current agent profile".into()
                ));
                continue;
            }
            if self.has_agent_change_prompt() {
                request.respond(Err(
                    "Wait for the pending file approval before calling another host tool".into(),
                ));
                continue;
            }
            let pending_before = self.pending_agent_changes.len();
            let result = teshi_agent::execute_tool(
                self,
                &request.call.name,
                &request.call.arguments,
                &request.call.id,
                agent_idx,
            )
            .map_err(|error| error.to_string());
            if result.is_ok() && self.pending_agent_changes.len() > pending_before {
                self.agents[agent_idx].backend.defer_host_tool(request);
                if self.approval_mode.requires_manual_approval() {
                    self.agents[agent_idx].status = AiStatus::AwaitingApproval;
                } else {
                    while self.has_agent_change_prompt() {
                        let id = self.pending_agent_changes[0].tool_call_id.clone();
                        let result = match self.accept_agent_change() {
                            Ok((_, result)) => Ok(result),
                            Err(error) => {
                                if self
                                    .pending_agent_changes
                                    .first()
                                    .is_some_and(|change| change.tool_call_id == id)
                                {
                                    self.pending_agent_changes.remove(0);
                                }
                                self.clear_pending_change_state();
                                Err(error.to_string())
                            }
                        };
                        self.agents[agent_idx]
                            .backend
                            .resolve_host_tool(&id, result);
                    }
                }
            } else {
                request.respond(result);
            }
        }
    }

    fn send_backend_followup(&mut self, agent_idx: usize, allow_tools: bool) {
        self.compact_context_if_needed(agent_idx);
        let messages = self.build_chat_messages_for_agent(agent_idx);
        let system = Some(self.ai_system_prompt(None, agent_idx));
        let allowed: Option<&[String]> =
            self.agent_profile(agent_idx).and_then(|p| match &p.tools {
                teshi_agent::definition::ToolPermission::All => None,
                teshi_agent::definition::ToolPermission::None => Some(&[] as &[String]),
                teshi_agent::definition::ToolPermission::Whitelist(list) => Some(list.as_slice()),
            });
        let tools = allow_tools.then(|| teshi_agent::get_tools(allowed));
        if let Err(error) = self.agents[agent_idx]
            .backend
            .submit_turn(system, messages, tools)
        {
            self.agents[agent_idx].status = AiStatus::Error;
            self.status = format!("AI error: {error}");
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
            let sc_count = f.scenario_count();
            let st_count: usize = f
                .all_scenarios()
                .iter()
                .map(|s| s.steps.len())
                .sum::<usize>()
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
        let latest_visual_message = self.agents[agent_idx]
            .messages()
            .iter()
            .rposition(|message| message.source.as_deref() == Some("browser_visual_observation"));

        // Inject project context as a system message at the front
        if self.agents[agent_idx]
            .messages()
            .iter()
            .any(|m| matches!(m.role, AiRole::User))
        {
            msgs.push(crate::llm::ChatMessage {
                role: "system".into(),
                content: self.build_project_context_summary().into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            });
        }

        // Original 1:1 mapping
        for (message_idx, m) in self.agents[agent_idx].messages().iter().enumerate() {
            if matches!(m.source.as_deref(), Some("acp_activity" | "acp_reasoning")) {
                continue;
            }
            let content = if Some(message_idx) == latest_visual_message {
                if let Some(observation) = &self.agents[agent_idx].latest_visual_observation {
                    teshi_core::llm::MessageContent::Blocks(vec![
                        teshi_core::llm::ContentBlock::Text {
                            text: m.content.clone(),
                        },
                        teshi_core::llm::ContentBlock::Image {
                            source: teshi_core::llm::ImageSource::Data {
                                media_type: observation.media_type.clone(),
                                data: observation.image.clone(),
                            },
                        },
                    ])
                } else {
                    m.content.clone().into()
                }
            } else {
                m.content.clone().into()
            };
            msgs.push(crate::llm::ChatMessage {
                role: match m.role {
                    AiRole::User => "user".into(),
                    AiRole::Assistant => "assistant".into(),
                    AiRole::Tool => "tool".into(),
                },
                content,
                tool_calls: m.tool_calls.clone(),
                tool_call_id: m.tool_call_id.clone(),
                reasoning_content: m.reasoning_content.clone(),
            });
        }
        msgs
    }

    /// Persists the active generation stage, requirement sources, and plan.
    ///
    /// # Errors
    ///
    /// Returns an error when the `.teshi/generation-state.json` write fails.
    pub(crate) fn persist_generation_state(&self) -> Result<()> {
        let state = teshi_agent::pipeline::GenerationSessionState {
            stage: self.generation_stage,
            requirement: self.pipeline_requirement.clone(),
            plan: self.pipeline_plan.clone(),
            source_scope: self.generation_scope.clone(),
        };
        crate::generation_state::save_generation_state(&self.project.root_dir, &state)
            .context("persist generation session state")
    }

    fn confirm_requirements_overlay(&mut self) -> Result<()> {
        if let crate::authoring_tab::RequirementsOverlay::DeleteConfirmation {
            document_id, ..
        } = self.authoring_ui.overlay.clone()
        {
            match self
                .authoring_ui
                .delete_current_document(&self.project.root_dir)
            {
                Ok(()) => self.status = format!("Deleted global requirement {document_id}"),
                Err(error) => {
                    self.status = format!("Could not delete requirement: {error}");
                }
            }
            return Ok(());
        }
        if self.generation_scope_prompt.is_some()
            && let crate::authoring_tab::RequirementsOverlay::FilterPicker { selection } =
                self.authoring_ui.overlay.clone()
        {
            let options = self.authoring_ui.filter_picker_options();
            let filter = options
                .get(selection)
                .cloned()
                .unwrap_or(teshi_core::authoring::RequirementIterationFilter::All);
            if let Some(prompt) = self.generation_scope_prompt.as_mut() {
                prompt.iteration = filter;
            }
            self.authoring_ui.cancel_overlay();
            return Ok(());
        }
        match self.authoring_ui.confirm_overlay() {
            Ok(()) => {
                self.status = "Requirements view updated".to_string();
            }
            Err(msg) => {
                self.status = msg;
            }
        }
        Ok(())
    }

    fn confirm_generation_scope(&mut self) -> Result<()> {
        let Some(prompt) = self.generation_scope_prompt.take() else {
            return Ok(());
        };
        if let teshi_core::authoring::RequirementIterationFilter::Named(name) = &prompt.iteration
            && !self
                .authoring_ui
                .discovered_iteration_names()
                .iter()
                .any(|existing| existing == name)
        {
            self.status = format!(
                "Iteration '{name}' is not in the current requirement store; choose another scope"
            );
            self.generation_scope_prompt = Some(prompt);
            return Ok(());
        }
        self.generation_scope = self
            .authoring_ui
            .artifacts
            .as_ref()
            .and_then(|artifacts| artifacts.index.store_id.clone())
            .map(|store_id| teshi_agent::pipeline::RequirementSourceScope {
                store_id,
                iteration: prompt.iteration.clone(),
                project_filter: self.authoring_ui.project_filter.clone(),
            });
        self.generation_paused_reason = None;
        self.generation_stage = teshi_agent::pipeline::GenerationStage::Gathering;
        self.persist_generation_state()?;
        self.status = "Generation source scope confirmed".to_string();
        self.dispatch_user_message(prompt.pending_user_message)
    }

    fn dispatch_user_message(&mut self, user_msg: String) -> Result<()> {
        if !self.start_agent_backend(self.selected_agent) {
            // A rejected send must not reappear in a later restored provider
            // context. Keep it as an editable draft for an explicit retry.
            self.agent_mut().input_cursor = user_msg.chars().count();
            self.agent_mut().input = user_msg;
            return Ok(());
        }
        self.agent_mut().last_finish_reason = None;
        self.agent_mut().scroll_offset = 0;
        self.agent_mut().messages_mut().push(AiChatMessage {
            role: AiRole::User,
            content: user_msg.clone(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });
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
        self.agent_mut().partial_response_mut().clear();
        self.agent_mut().pending_acp_permission = None;
        self.status = "Sending message to AI...".to_string();

        if !self.agent().backend.is_connected() {
            self.spawn_llm_if_configured();
        }
        if self.agent().backend.kind() == teshi_agent::backend::AgentBackendKind::Native
            && !crate::llm::is_configured()
        {
            self.agent_mut().messages_mut().push(AiChatMessage {
                role: AiRole::Assistant,
                content: "AI is not configured. Run 'teshi auth login', open the model panel, or set TESHI_LLM_API_KEY in your environment.".to_string(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            });
            self.agent_mut().status = AiStatus::Idle;
            self.agent_mut().partial_response_mut().clear();
            self.status = "AI not configured".to_string();
        } else if self.agent().backend.is_connected() {
            self.compact_context_if_needed(self.selected_agent);
            let messages = self.build_chat_messages_for_agent(self.selected_agent);
            let profile = self.agent_profile(self.selected_agent);
            let allowed: Option<&[String]> = profile.and_then(|p| match &p.tools {
                teshi_agent::definition::ToolPermission::All => None,
                teshi_agent::definition::ToolPermission::None => Some(&[] as &[String]),
                teshi_agent::definition::ToolPermission::Whitelist(list) => Some(list.as_slice()),
            });
            let tools = Some(teshi_agent::get_tools(allowed));
            let system = Some(self.ai_system_prompt(Some(&user_msg), self.selected_agent));
            if let Err(error) = self
                .agent_mut()
                .backend
                .submit_turn(system, messages, tools)
            {
                self.agent_mut().status = AiStatus::Error;
                self.agent_mut().partial_response_mut().clear();
                self.status = format!("AI error: {error}");
            }
        } else {
            self.agent_mut().status = AiStatus::Error;
            self.agent_mut().partial_response_mut().clear();
            self.status = "AI error: LLM handle not available".to_string();
        }
        self.quit_pending_confirm = false;
        Ok(())
    }

    /// Restores generation stage/sources/plan from disk without granting approvals.
    fn restore_generation_session_from_disk(&mut self) {
        let test_points = self
            .authoring_ui
            .artifacts
            .as_ref()
            .map(|a| a.test_points.test_points.as_slice())
            .unwrap_or(&[]);
        match crate::generation_state::restore_generation_session(
            &self.project.root_dir,
            test_points,
        ) {
            Ok((stage, Some(saved))) => {
                self.generation_stage = stage;
                self.pipeline_requirement = saved.requirement.clone();
                self.pipeline_plan = saved.plan.clone();
                self.generation_scope = saved.source_scope.clone();
                let current_store = self
                    .authoring_ui
                    .artifacts
                    .as_ref()
                    .and_then(|a| a.index.store_id.clone());
                let empty_index = teshi_core::authoring::RequirementDocumentIndex::default();
                let (index, documents) = self
                    .authoring_ui
                    .artifacts
                    .as_ref()
                    .map(|a| (&a.index, a.documents.as_slice()))
                    .unwrap_or((&empty_index, &[]));
                match teshi_agent::pipeline::evaluate_restored_scope(
                    &saved,
                    current_store.as_ref(),
                    index,
                    documents,
                ) {
                    Ok(()) => {
                        self.generation_paused_reason = None;
                    }
                    Err(err) => {
                        self.generation_paused_reason = Some(err.to_string());
                        self.status = format!("Generation paused: {err}");
                    }
                }
                self.sync_test_point_scenario_refs();
                self.test_points_ui
                    .rebuild_tree(self.authoring_ui.artifacts.as_ref());
            }
            Ok((_, None)) => {
                self.sync_test_point_scenario_refs();
                self.test_points_ui
                    .rebuild_tree(self.authoring_ui.artifacts.as_ref());
            }
            Err(e) => {
                self.status = format!("Failed to restore generation state: {e}");
                self.sync_test_point_scenario_refs();
                self.test_points_ui
                    .rebuild_tree(self.authoring_ui.artifacts.as_ref());
            }
        }
    }

    /// Human-only transition from Reviewing Test Points to Planning.
    ///
    /// Intentionally ignores `approval_mode`: Auto/Bypass cannot trigger this gate.
    fn continue_test_point_generation(&mut self) -> Result<()> {
        let test_points: Vec<teshi_core::authoring::TestPoint> = self
            .authoring_ui
            .artifacts
            .as_ref()
            .map(|a| a.test_points.test_points.clone())
            .unwrap_or_default();
        match teshi_agent::pipeline::continue_from_review(self.generation_stage, &test_points) {
            Ok(next) => {
                if let (Some(scope), Some(artifacts)) = (
                    self.generation_scope.as_ref(),
                    self.authoring_ui.artifacts.as_ref(),
                ) {
                    if let Some(requirement) = self.pipeline_requirement.as_ref()
                        && let Err(err) = teshi_agent::pipeline::revalidate_source_refs(
                            &requirement.source_refs,
                            scope,
                            &artifacts.index,
                            &artifacts.documents,
                        )
                    {
                        self.status = format!("Generation paused: {err}");
                        self.generation_paused_reason = Some(err);
                        return Ok(());
                    }
                    if let Err(err) = teshi_agent::pipeline::validate_test_point_links_in_scope(
                        &test_points,
                        scope,
                        &artifacts.index,
                    ) {
                        self.status = format!("Generation paused: {err}");
                        self.generation_paused_reason = Some(err);
                        return Ok(());
                    }
                }
                let approved =
                    teshi_agent::pipeline::approved_resolved_test_point_ids(&test_points);
                self.generation_stage = next;
                self.persist_generation_state()?;
                let agent_idx = self.selected_agent;
                let summary = format!(
                    "Human approved test points [{}] and continued generation. Call `generate_plan` using only these approved test_point_ids.",
                    approved.join(", ")
                );
                self.agents[agent_idx].messages_mut().push(AiChatMessage {
                    role: AiRole::User,
                    content: summary.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                    source: None,
                });
                self.status = format!(
                    "Advanced to Scenario Planning with {} approved test point(s)",
                    approved.len()
                );
                // Resume the agent loop when an LLM handle is available.
                if self.agents[agent_idx].backend.is_connected() {
                    self.agents[agent_idx].status = AiStatus::Waiting;
                    if !self.start_agent_backend(agent_idx) {
                        return Ok(());
                    }
                    self.compact_context_if_needed(agent_idx);
                    let messages = self.build_chat_messages_for_agent(agent_idx);
                    let profile = self.agent_profile(agent_idx);
                    let allowed: Option<&[String]> = profile.and_then(|p| match &p.tools {
                        teshi_agent::definition::ToolPermission::All => None,
                        teshi_agent::definition::ToolPermission::None => Some(&[] as &[String]),
                        teshi_agent::definition::ToolPermission::Whitelist(list) => {
                            Some(list.as_slice())
                        }
                    });
                    let tools = Some(teshi_agent::get_tools(allowed));
                    let system = Some(self.ai_system_prompt(None, agent_idx));
                    if let Err(error) = self.agents[agent_idx]
                        .backend
                        .submit_turn(system, messages, tools)
                    {
                        self.agents[agent_idx].status = AiStatus::Error;
                        self.status = format!("AI error: {error}");
                    }
                }
                Ok(())
            }
            Err(msg) => {
                self.status = msg;
                Ok(())
            }
        }
    }

    /// The system prompt used for all AI chat requests.
    /// Uses the active agent profile's instructions as the base, then appends
    /// pipeline guidance.
    fn ai_system_prompt(&self, request: Option<&str>, agent_idx: usize) -> String {
        let profile = self.agent_profile(agent_idx);
        let instructions = profile
            .map(|p| p.system_prompt.clone())
            .unwrap_or_else(|| "You are a helpful assistant.".to_string());
        let mut prompt = instructions;

        // Generation pipeline guidance
        prompt.push_str(
            "\n\n## Feature Generation Pipeline\n\
             When the user asks to create or generate a feature (including `/generate`), follow this pipeline:\n\
             1. **Requirements Gathering** — Ask questions or accept pasted requirement text. Use `list_requirement_documents` and `read_requirement_document` to inspect the current local requirement store on demand; do not assume every Markdown file is already in the prompt. Prefer `source_refs` to persisted documents/ranges in the confirmed store/iteration scope. Pasted or conversational text is not a requirement-library source. Call `submit_requirements` when done.\n\
             2. **Generating Test Points** — Call `propose_test_points` with non-Gherkin verification intents (title, objective, hierarchy). Do NOT write Given/When/Then inside test points.\n\
             3. **Reviewing Test Points** — Stop and wait. Humans approve/reject in the Test Points tab. Do NOT call `generate_plan` and do NOT treat Auto/Bypass file approval as test-point approval.\n\
             4. **Planning** — After the user continues generation, design Gherkin scenarios. Every scenario must include `test_point_ids` for approved test points. Call `generate_plan`.\n\
             5. **Writing** — Execute the plan using `create_feature_file` and `insert_scenario` (Gherkin `.feature` only).\n\
             6. **Validation** — Use `validate_feature` to check for issues.\n\
             \n\
             Test points are durable verification intents, NOT Gherkin scenarios. Scenarios realize approved test points.\n\
             Do NOT produce FreeMind `.mm` files or mock HTML.\n\
             Do NOT skip the human review gate. Start by gathering requirements.\n\
             If the user's request is already detailed, you can ask 1-2 clarifying questions then proceed.\n\
             Always check the [Project Context] to understand existing files before generating.",
        );

        // Inject pipeline stage guidance if a generation is in progress
        if !matches!(
            self.generation_stage,
            teshi_agent::pipeline::GenerationStage::Idle
                | teshi_agent::pipeline::GenerationStage::Complete
                | teshi_agent::pipeline::GenerationStage::Confirming
        ) {
            prompt.push_str("\n## Generation Pipeline Status\n");
            prompt.push_str(&format!(
                "Current stage: **{}**\n",
                self.generation_stage.label()
            ));
            prompt.push_str(self.generation_stage.prompt_guidance());
            if let Some(scope) = &self.generation_scope {
                let iteration = match &scope.iteration {
                    teshi_core::authoring::RequirementIterationFilter::All => "All".to_string(),
                    teshi_core::authoring::RequirementIterationFilter::Unassigned => {
                        teshi_core::authoring::UNASSIGNED_ITERATION_LABEL.to_string()
                    }
                    teshi_core::authoring::RequirementIterationFilter::Named(name) => name.clone(),
                };
                prompt.push_str(&format!(
                    "\nActive requirement source scope: store_id={}, iteration={iteration}. Use list/read tools; do not invent documents outside this window.\n",
                    scope.store_id
                ));
            }
            if let Some(reason) = &self.generation_paused_reason {
                prompt.push_str(&format!(
                    "\nGeneration is paused: {reason}. Do not call submit_requirements or propose_test_points until the user reconfirms the source scope.\n"
                ));
            }
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

        // Exploration mode guidance
        if self.explore_state.is_some() {
            prompt.push_str(
                "\n\n## Exploration Mode (Active)\n\
                 You are in **exploration mode**. Your task is to explore a web application\n\
                 and find the elements described in each Gherkin step below.\n\n\
                 Available browser tools:\n\
                 - `browser_snapshot` — Get a structured list of interactive elements on the current page.\n\
                 - `browser_click(ref)` — Click an element by its teshi-id ref.\n\
                 - `browser_type(ref, text)` — Type text into an input element by its teshi-id ref.\n\
                 - `browser_assert(condition_type, value)` — Assert text is visible or URL matches.\n\
                 - `browser_go_back` — Go back one page in history.\n\n\
                 For each Gherkin step, do the following:\n\
                 1. Call `browser_snapshot` to see what's on the page.\n\
                 2. Find the element that matches the step description.\n\
                 3. Interact with it (click, type) or assert the expected state.\n\
                 4. Move to the next step.\n\n\
                 IMPORTANT RULES:\n\
                 - Always call `browser_snapshot` first to understand the page.\n\
                 - Do NOT navigate to URLs outside the test application.\n\
                 - If you get stuck or can't find an element, report what you see.\n\
                 - Each step should be completed with one or two tool calls.\n\
                 - Do NOT modify Gherkin files during exploration.",
            );
        }

        prompt
    }

    /// After a pending agent change is accepted or rejected, feed the result back
    /// to the LLM as a tool result message and continue the agent loop.
    #[allow(dead_code)]
    fn feed_agent_tool_result(&mut self, tool_call_id: String, result: String) {
        self.feed_agent_tool_result_for_agent(self.selected_agent, tool_call_id, result);
    }

    /// Same as `feed_agent_tool_result` but operates on the specified agent index
    /// (used when the accepting/rejecting agent may differ from `self.selected_agent`).
    fn feed_agent_tool_result_for_agent(
        &mut self,
        agent_idx: usize,
        tool_call_id: String,
        result: String,
    ) {
        if self.agents[agent_idx]
            .backend
            .resolve_host_tool(&tool_call_id, Ok(result.clone()))
        {
            self.agents[agent_idx].status = AiStatus::Waiting;
            if self.generation_stage == teshi_agent::pipeline::GenerationStage::Confirming {
                self.generation_stage = teshi_agent::pipeline::GenerationStage::Writing;
            }
            return;
        }
        if self.agents[agent_idx].backend.status()
            != teshi_agent::backend::AgentBackendStatus::WaitingForPermission
        {
            return;
        }
        self.agents[agent_idx].messages_mut().push(AiChatMessage {
            role: AiRole::Tool,
            content: result,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            reasoning_content: None,
            source: None,
        });
        if self.has_agent_change_prompt() {
            return;
        }
        if self.generation_stage == teshi_agent::pipeline::GenerationStage::Confirming {
            self.generation_stage = teshi_agent::pipeline::GenerationStage::Writing;
        }
        use teshi_agent::backend::AgentBackendDecision;
        let has_feature_files = self.has_feature_files();
        match self.agents[agent_idx]
            .backend
            .resume_after_approval(has_feature_files)
        {
            AgentBackendDecision::Continue { allow_tools } => {
                self.agents[agent_idx].status = AiStatus::Waiting;
                self.agents[agent_idx].tool_status = Some("Teshi is thinking...".into());
                if !allow_tools {
                    self.status = "Tool call limit reached, requesting final response…".to_string();
                }
                self.send_backend_followup(agent_idx, allow_tools);
            }
            AgentBackendDecision::StopEmptyProject => {
                self.agents[agent_idx].status = AiStatus::Idle;
                self.agents[agent_idx].tool_status = None;
                self.status =
                    "The project directory has no .feature files. Add one to begin.".to_string();
            }
            _ => {}
        }
    }

    // ── Exploration loop management ──

    /// Start an exploration session for the given scenario.
    #[allow(dead_code)]
    pub fn start_exploration(
        &mut self,
        feature_path: String,
        steps: Vec<(usize, String, String)>,
        agent_idx: usize,
    ) {
        let session_id = format!(
            "explore-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        );

        self.explore_state = Some(ExploreState {
            feature_path: feature_path.clone(),
            steps: steps.clone(),
            current_step: 0,
            trace: Vec::new(),
            step_count: 0,
            max_steps: 15,
            url_whitelist: Vec::new(),
            current_url: String::new(),
            session_id,
            completed: false,
            terminated: false,
            termination_reason: None,
        });

        // Build the exploration task message
        let mut task_msg = String::new();
        task_msg.push_str("## Exploration Task\n\n");
        task_msg.push_str(&format!("Feature file: **{}**\n\n", feature_path));
        task_msg.push_str("### Scenario Steps\n\n");
        task_msg.push_str("| # | Step |\n|---|---|\n");
        for (i, (line, keyword, text)) in steps.iter().enumerate() {
            task_msg.push_str(&format!(
                "| **{}** (line {}) | {} {} |\n",
                i + 1,
                line,
                keyword,
                text
            ));
        }
        task_msg.push_str("\nExplore the web application and complete each step above.\n");
        task_msg.push_str("Start by calling `browser_snapshot` to see the current page.\n");

        // Push the task message as a user message
        self.agents[agent_idx].messages_mut().push(AiChatMessage {
            role: AiRole::User,
            content: task_msg,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });

        // Start the LLM request (same pattern as user message submit)
        if !self.agents[agent_idx].backend.is_connected() {
            self.status = "AI not configured".to_string();
            return;
        }
        if !self.start_agent_backend(agent_idx) {
            return;
        }
        self.compact_context_if_needed(agent_idx);
        let messages = self.build_chat_messages_for_agent(agent_idx);
        let profile = self.agent_profile(agent_idx);
        let allowed: Option<&[String]> = profile.and_then(|p| match &p.tools {
            teshi_agent::definition::ToolPermission::All => None,
            teshi_agent::definition::ToolPermission::None => Some(&[] as &[String]),
            teshi_agent::definition::ToolPermission::Whitelist(list) => Some(list.as_slice()),
        });
        let tools = Some(teshi_agent::get_tools(allowed));
        let system = Some(self.ai_system_prompt(None, agent_idx));
        if let Err(error) = self.agents[agent_idx]
            .backend
            .submit_turn(system, messages, tools)
        {
            self.agents[agent_idx].status = AiStatus::Error;
            self.agents[agent_idx].partial_response_mut().clear();
            self.status = format!("AI error: {error}");
            return;
        }
        self.agents[agent_idx].status = AiStatus::Waiting;
        self.agents[agent_idx].tool_status = Some("Teshi is exploring...".into());
    }

    /// Check if a URL is allowed by the whitelist patterns (substring match).
    /// An empty whitelist allows all URLs.
    #[allow(dead_code)]
    pub fn is_url_allowed(url: &str, whitelist: &[String]) -> bool {
        if whitelist.is_empty() {
            return true;
        }
        whitelist
            .iter()
            .any(|pattern| url.contains(pattern.as_str()))
    }

    /// Check exploration limits (step count, URL sandbox) and terminate if breached.
    /// Returns true if exploration was terminated.
    #[allow(dead_code)]
    pub fn check_exploration_limits(&mut self) -> bool {
        let Some(ref state) = self.explore_state.clone() else {
            return false;
        };
        if state.terminated || state.completed {
            return false;
        }

        // Check step limit
        if state.step_count >= state.max_steps {
            self.explore_state.as_mut().unwrap().terminated = true;
            self.explore_state.as_mut().unwrap().termination_reason =
                Some(format!("Step limit exceeded (max {})", state.max_steps));
            self.status = format!(
                "Exploration terminated: step limit ({}) exceeded",
                state.max_steps
            );
            self.save_exploration_trace();
            return true;
        }

        // Check URL sandbox (if a current_url is tracked)
        if !state.current_url.is_empty()
            && !Self::is_url_allowed(&state.current_url, &state.url_whitelist)
        {
            self.explore_state.as_mut().unwrap().terminated = true;
            self.explore_state.as_mut().unwrap().termination_reason = Some(format!(
                "URL not allowed by whitelist: {}",
                state.current_url
            ));
            self.status = format!(
                "Exploration terminated: URL '{}' is not in the whitelist",
                state.current_url
            );
            self.save_exploration_trace();
            return true;
        }

        false
    }

    /// Record a tool call action in the exploration trace.
    #[allow(dead_code)]
    pub fn record_exploration_action(
        &mut self,
        action: String,
        ref_id: Option<String>,
        text: Option<String>,
        ok: bool,
        error: Option<String>,
    ) {
        let Some(ref mut state) = self.explore_state else {
            return;
        };
        let url = String::new(); // Will be populated from snapshot
        state.trace.push(TraceAction {
            step_index: state.current_step,
            action,
            ref_id,
            text,
            url,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            ok,
            error,
        });
        state.step_count += 1;
    }

    /// Save the exploration trace to disk.
    #[allow(dead_code)]
    pub fn save_exploration_trace(&self) {
        let Some(ref state) = self.explore_state else {
            return;
        };
        let project_root = match std::env::current_dir() {
            Ok(p) => p,
            Err(_) => return,
        };
        let trace_dir = project_root.join(".teshi").join("traces");
        let _ = std::fs::create_dir_all(&trace_dir);
        let trace_path = trace_dir.join(format!("{}.jsonl", state.session_id));
        if let Ok(file) = std::fs::File::create(&trace_path) {
            use std::io::Write;
            let mut writer = std::io::BufWriter::new(file);
            for action in &state.trace {
                if let Ok(line) = serde_json::to_string(action) {
                    let _ = writeln!(writer, "{}", line);
                }
            }
        }
    }

    /// Visits the declared Feature scope one file at a time, including unloaded files.
    /// Loaded content takes precedence so unsaved edits are never replaced by disk data.
    pub(crate) fn feature_scope<'a>(
        &'a self,
        file_path: Option<&'a str>,
    ) -> Box<dyn Iterator<Item = anyhow::Result<std::borrow::Cow<'a, gherkin::BddFeature>>> + 'a>
    {
        // Filter catalog paths before any source IO, including errors in unrelated files.
        let matches = move |path: &Path| {
            file_path.is_none_or(|requested| {
                let requested = requested.replace('\\', "/");
                let path = path.to_string_lossy().replace('\\', "/");
                path == requested || path.ends_with(&format!("/{requested}"))
            })
        };
        if let Some(catalog) = &self.lazy_features {
            if !catalog.ready {
                return Box::new(std::iter::once(Err(anyhow::anyhow!(
                    "Feature discovery has not completed"
                ))));
            }

            Box::new(
                catalog
                    .paths
                    .iter()
                    .filter(move |path| matches(path))
                    .map(|path| {
                        if let Some(feature) = self
                            .project
                            .features
                            .iter()
                            .find(|feature| &feature.file_path == path)
                        {
                            Ok(std::borrow::Cow::Borrowed(feature))
                        } else {
                            crate::lazy_features::read_content(path)
                                .map(|(feature, _)| std::borrow::Cow::Owned(feature))
                                .map_err(anyhow::Error::msg)
                        }
                    }),
            )
        } else {
            Box::new(
                self.project
                    .features
                    .iter()
                    .filter(move |feature| matches(&feature.file_path))
                    .map(|feature| Ok(std::borrow::Cow::Borrowed(feature))),
            )
        }
    }

    fn has_feature_files(&self) -> bool {
        self.lazy_features.as_ref().map_or_else(
            || !self.project.features.is_empty(),
            |catalog| !catalog.paths.is_empty(),
        )
    }

    /// Whether the selected catalog file has been installed in the editor.
    pub(crate) fn catalog_selection_ready(&self) -> bool {
        self.lazy_features.as_ref().is_none_or(|state| {
            state.selected.as_ref().is_some_and(|path| {
                self.project
                    .features
                    .first()
                    .is_some_and(|f| &f.file_path == path)
            })
        })
    }

    /// Loads the pending launch selection for user actions that cannot wait for a
    /// frame, such as entering edit mode or starting a run right after launch.
    /// This reads one bounded Feature on the UI thread; it never replaces edits.
    pub(crate) fn ensure_selected_feature(&mut self) -> bool {
        if self.catalog_selection_ready() {
            return true;
        }
        let Some(path) = self
            .lazy_features
            .as_ref()
            .and_then(|state| state.selected.clone())
        else {
            return false;
        };
        if self.dirty
            || self.buffer_dirty.iter().any(|dirty| *dirty)
            || self.runner_rx.is_some()
            || !self.pending_agent_changes.is_empty()
        {
            return false;
        }
        let Ok((feature, source)) = crate::lazy_features::read_content(&path) else {
            return false;
        };
        if let Some(state) = self.lazy_features.as_mut() {
            state.cancel_selection(path);
        }
        self.install_lazy_feature(feature, source);
        self.catalog_selection_ready()
    }

    /// Loads an explicitly targeted catalog file for tools using editor-backed mutations.
    pub(crate) fn ensure_tool_feature(&mut self, file_path: &str) -> Result<()> {
        if self.lazy_features.is_none() || self.find_feature_idx_for_file(file_path).is_some() {
            return Ok(());
        }
        let requested = Path::new(file_path);
        let requested = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.project.root_dir.join(requested)
        };
        let identity = fs::canonicalize(&requested)
            .with_context(|| format!("read {}", requested.display()))?;
        let path = self
            .lazy_features
            .as_ref()
            .and_then(|state| {
                state
                    .paths
                    .iter()
                    .find(|path| fs::canonicalize(path).is_ok_and(|p| p == identity))
            })
            .cloned()
            .context(
                "Feature is outside the discovered project scope or discovery is still running",
            )?;
        if self.dirty
            || self.buffer_dirty.iter().any(|dirty| *dirty)
            || self.runner_rx.is_some()
            || !self.pending_agent_changes.is_empty()
        {
            anyhow::bail!("Save edits and finish pending work before loading another Feature");
        }
        let (feature, source) =
            crate::lazy_features::read_content(&path).map_err(anyhow::Error::msg)?;
        if let Some(state) = self.lazy_features.as_mut() {
            state.cancel_selection(path);
        }
        self.install_lazy_feature(feature, source);
        Ok(())
    }

    fn select_catalog_feature(&mut self, index: usize) {
        let Some(path) = self
            .lazy_features
            .as_ref()
            .and_then(|state| state.paths.get(index))
            .cloned()
        else {
            return;
        };
        if self
            .project
            .features
            .first()
            .is_some_and(|feature| feature.file_path == path)
        {
            if let Some(state) = self.lazy_features.as_mut() {
                state.selected = Some(path);
            }
            return;
        }
        if self.dirty
            || self.buffer_dirty.iter().any(|dirty| *dirty)
            || self.runner_rx.is_some()
            || !self.pending_agent_changes.is_empty()
        {
            self.status =
                "Save current edits and finish pending work before switching Feature".into();
            return;
        }
        self.explore_edit_mode = false;
        if let Some(state) = self.lazy_features.as_mut() {
            state.select(path);
        }
    }

    /// Polls the directory catalog and installs only the selected Feature's AST.
    pub(crate) fn poll_lazy_features(&mut self) {
        let busy = self.dirty
            || self.buffer_dirty.iter().any(|dirty| *dirty)
            || self.runner_rx.is_some()
            || !self.pending_agent_changes.is_empty();
        let Some(state) = self.lazy_features.as_mut() else {
            return;
        };
        state.poll_catalog();
        // A launch still shows the first Feature as eager loading did, but only
        // that file is read and parsed. Never replace edits or pending work.
        if !busy
            && state.ready
            && state.selected.is_none()
            && let Some(first) = state.paths.first().filter(|path| path.exists()).cloned()
        {
            state.select(first);
        }
        let content = state.poll_content();
        if let Some(error) = state.error.as_ref().or(state.scan_error.as_ref()) {
            self.status = format!("Feature loading failed: {error}");
        }
        let Some((feature, source, modified, len)) = content else {
            return;
        };
        // A user may start editing or running the old selection while IO is in flight.
        if self.dirty
            || self.buffer_dirty.iter().any(|dirty| *dirty)
            || self.runner_rx.is_some()
            || !self.pending_agent_changes.is_empty()
        {
            self.status =
                "Feature load deferred: save edits and finish pending work, then select again"
                    .into();
            return;
        }
        self.install_lazy_feature(feature, source);
        // Preserve the revision actually read. Recapturing it during installation
        // could acknowledge an intervening disk write without loading its content.
        self.disk_stamps = vec![Some(FileStamp {
            modified: Some(modified),
            len,
        })];
    }

    fn install_lazy_feature(&mut self, feature: gherkin::BddFeature, source: String) {
        self.git_status_rx = None;
        self.project.features = vec![feature];
        self.buffers = vec![EditorBuffer::from_string(source)];
        self.buffer_dirty = vec![false];
        self.disk_stamps = Self::capture_disk_stamps(&self.project);
        self.active_buffer_idx = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.preview_buffer = None;
        self.pending_change_diffs.clear();
        self.pending_change_summary.clear();
        self.explore_diff_lines = None;
        self.explore_feature_scenario_memory.clear();
        self.explore_scenario_step_memory.clear();
        self.explore_case_map.clear();
        self.explore_case_status.clear();
        self.explore_case_details.clear();
        self.explore_run_summary = None;
        self.explore_detail_case = None;
        self.explore_detail_open = false;
        self.external_change_prompt = None;
        self.explore_selected_feature = 0;
        self.explore_selected_deleted_feature = None;
        self.explore_selected_scenario = 0;
        self.explore_selected_step = 0;
        self.switch_to_buffer(0);
        // Keep the project scope discovered at startup; forcing current files
        // here would stop reporting Features deleted on disk.
        // The repository snapshot stays until the new projection lands, so the
        // Feature list keeps its markers instead of blanking on every selection.
        let statuses = std::mem::take(&mut self.git_status.statuses);
        let repository_root = self.git_status.repository_root.clone();
        self.git_status = teshi_core::git::FeatureGitStatusModel::empty_for_project(
            &self.project,
            self.git_status.scope,
        );
        self.git_status.statuses = statuses;
        self.git_status.repository_root = repository_root;
        self.bump_git_status_revision();
        self.rebuild_project_views(None);
        self.refresh_git_status();
        // Captures belong to this Feature now; the Explore markers would
        // otherwise wait for the next viewer open.
        self.refresh_capture_index();
        self.status = "Feature loaded; AST and MindMap scope: current Feature".into();
    }

    /// Whether the active project is waiting for a background Feature scan.
    pub(crate) fn features_loading(&self) -> bool {
        self.lazy_features
            .as_ref()
            .is_some_and(|state| state.is_loading())
            || self
                .feature_discovery_worker
                .as_ref()
                .is_some_and(|worker| worker.is_loading())
    }

    /// Polls external Feature discovery and disk changes.
    pub fn poll_external_feature_changes(&mut self) {
        if self.external_change_prompt.is_some() {
            return;
        }
        if self.last_external_check.elapsed() < Duration::from_millis(1000) {
            return;
        }
        self.last_external_check = Instant::now();

        // Index-based runner events and pending agent edits must retain their targets.
        if self.runner_rx.is_some() || !self.pending_agent_changes.is_empty() {
            return;
        }
        if self.lazy_features.is_none()
            && (self.feature_discovery_root.is_some()
                || self.git_status.scope != teshi_core::git::GitProjectScope::CurrentFiles)
        {
            let recursive = self.feature_discovery_root.is_some()
                || self.git_status.scope == teshi_core::git::GitProjectScope::Recursive;
            let discovery_root = self
                .feature_discovery_root
                .as_deref()
                .unwrap_or(&self.project.root_dir);
            let snapshot = if let Some(worker) = self.feature_discovery_worker.as_mut() {
                let Some(result) =
                    worker.poll_excluding(discovery_root, recursive, &self.project.features)
                else {
                    return;
                };
                result
            } else if self.feature_discovery_root.is_some() && !discovery_root.exists() {
                // A missing default directory never widens discovery to the project root.
                Ok(BddProject {
                    root_dir: discovery_root.to_path_buf(),
                    features: Vec::new(),
                })
            } else {
                gherkin::try_parse_project_with_depth(discovery_root, recursive)
                    .map_err(|e| e.to_string())
            };
            match snapshot {
                Ok(snapshot) => {
                    let mut added = false;
                    // Discovery snapshots contain every file. Index existing paths once
                    // instead of comparing every discovered path with the entire project.
                    let mut known_paths: std::collections::HashSet<_> = self
                        .project
                        .features
                        .iter()
                        .map(|feature| feature.file_path.clone())
                        .collect();
                    for feature in snapshot.features {
                        if !known_paths.insert(feature.file_path.clone()) {
                            continue;
                        }
                        match fs::read_to_string(&feature.file_path) {
                            Ok(body) => {
                                self.disk_stamps
                                    .push(FileStamp::capture(&feature.file_path));
                                self.buffers.push(EditorBuffer::from_string(body));
                                self.buffer_dirty.push(false);
                                self.project.features.push(feature);
                                added = true;
                            }
                            Err(error) => {
                                self.status = format!("Feature discovery failed: {error}")
                            }
                        }
                    }
                    if added {
                        // Async startup begins without an editor buffer; initialize it just as
                        // synchronous loading does, while preserving an existing unsaved buffer.
                        if self.active_buffer_idx.is_none() && !self.dirty {
                            self.switch_to_buffer(0);
                            self.sync_cursor_to_first_node();
                        }
                        self.bump_git_status_revision();
                        let selection = self.selected_tree_location();
                        self.rebuild_project_views(selection);
                        self.refresh_git_status();
                    }
                }
                Err(error) => {
                    self.status = format!("Feature refresh incomplete; retained project: {error}");
                    self.set_status_message(self.status.clone());
                    return;
                }
            }
        }

        for idx in (0..self.project.features.len()).rev() {
            let path = self.project.features[idx].file_path.clone();
            let current_stamp = FileStamp::capture(&path);
            let known_stamp = self.disk_stamps.get(idx).cloned().unwrap_or(None);
            if current_stamp == known_stamp {
                continue;
            }

            if self.buffer_dirty.get(idx).copied().unwrap_or(false) {
                // A write the app performed itself leaves the buffer equal to the
                // disk, so only the stamp is stale - that is not a conflict.
                let disk_matches_buffer = self.buffers.get(idx).is_some_and(|buffer| {
                    fs::read_to_string(&path).is_ok_and(|disk| buffer.as_string() == disk)
                });
                if disk_matches_buffer {
                    if let Some(slot) = self.disk_stamps.get_mut(idx) {
                        *slot = current_stamp;
                    }
                    self.set_buffer_dirty(idx, false);
                    continue;
                }
                self.external_change_prompt = Some(ExternalChangePrompt {
                    feature_idx: idx,
                    path: path.clone(),
                    disk_stamp: current_stamp,
                });
                self.status = format!(
                    "Feature changed on disk: {}. Reload [Enter/r] or keep local [Esc/k].",
                    path.display()
                );
                break;
            } else if current_stamp.is_none() && !path.exists() {
                self.remove_external_feature(idx);
            } else if let Err(err) = self.reload_feature_from_disk(idx, current_stamp) {
                self.status = format!("Failed to reload {}: {err}", path.display());
            }
            self.quit_pending_confirm = false;
        }
    }

    fn remove_external_feature(&mut self, idx: usize) {
        self.project.features.remove(idx);
        self.buffers.remove(idx);
        self.buffer_dirty.remove(idx);
        self.disk_stamps.remove(idx);
        self.active_buffer_idx = self.active_buffer_idx.and_then(|i| {
            if i == idx {
                None
            } else {
                Some(if i > idx { i - 1 } else { i })
            }
        });
        if self.active_buffer_idx.is_none() {
            self.buffer = EditorBuffer::from_string(String::new());
            self.file_path = None;
        }
        self.explore_selected_feature = self
            .explore_selected_feature
            .saturating_sub(usize::from(self.explore_selected_feature > idx));
        self.explore_feature_scenario_memory.clear();
        self.explore_scenario_step_memory.clear();
        self.explore_case_map.retain(|_, (i, _)| *i != idx);
        for (i, _) in self.explore_case_map.values_mut() {
            if *i > idx {
                *i -= 1;
            }
        }
        self.explore_case_status = std::mem::take(&mut self.explore_case_status)
            .into_iter()
            .filter(|((i, _), _)| *i != idx)
            .map(|((i, s), value)| ((if i > idx { i - 1 } else { i }, s), value))
            .collect();
        self.explore_case_details = std::mem::take(&mut self.explore_case_details)
            .into_iter()
            .filter(|((i, _), _)| *i != idx)
            .map(|((i, s), value)| ((if i > idx { i - 1 } else { i }, s), value))
            .collect();
        self.explore_detail_case = None;
        self.explore_detail_open = false;
        self.external_change_prompt = None;
        self.bump_git_status_revision();
        self.rebuild_project_views(None);
        self.refresh_git_status();
        self.status = "Removed externally deleted Feature; on-disk run evidence retained".into();
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
    /// Publishes a visible, expiring status message across terminal views.
    pub(crate) fn set_status_message(&mut self, msg: String) {
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

    /// Get the context window size, preferring the active model profile's setting.
    ///
    /// Resolution order:
    /// 1. `max_context_tokens` from the in-memory active model profile (when set).
    /// 2. `context_window` from config.toml provider settings.
    /// 3. Hard-coded default of 128 000.
    pub(crate) fn active_context_window(&self) -> u32 {
        // Prefer the active profile's max_context_tokens when available.
        if let Some(profile) = self
            .model_profiles
            .iter()
            .find(|p| self.model_active_id.as_deref() == Some(p.id.as_str()))
            && let Some(tokens) = profile.max_context_tokens
        {
            return tokens;
        }
        // Fall back to config.toml provider setting.
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

        let messages = self.agents[agent_idx].messages_mut();

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
        let requested = Path::new(file_path);
        let requested = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.project.root_dir.join(requested)
        };
        // Discovery canonicalizes the project root. Tools may still supply a
        // Windows short-name path, extended prefix, or a path containing `..`.
        let identity = fs::canonicalize(requested).ok();
        self.project.features.iter().position(|feature| {
            let path = feature.file_path.to_string_lossy();
            path == file_path
                || path.ends_with(file_path)
                || file_path.ends_with(path.as_ref())
                || identity.as_ref().is_some_and(|requested| {
                    fs::canonicalize(&feature.file_path)
                        .is_ok_and(|candidate| candidate == *requested)
                })
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
        self.bump_git_status_revision();
        let selected = self.selected_tree_location();
        for (idx, buffer) in self.buffers.iter().enumerate() {
            if idx < self.project.features.len() {
                let content = buffer.as_string();
                let path = self.project.features[idx].file_path.clone();
                self.project.features[idx] = gherkin::parse_feature(&content, path);
            }
        }
        self.rebuild_project_views(selected);
        self.refresh_git_status();
    }

    fn bump_git_status_revision(&mut self) {
        self.git_status_revision = self.git_status_revision.wrapping_add(1);
    }

    fn schedule_git_status_refresh(&mut self) {
        self.git_status_refresh_requested = true;
        self.git_status_refresh_deadline = Some(Instant::now() + GIT_STATUS_REFRESH_DEBOUNCE);
    }

    fn git_content_overrides(&self) -> HashMap<PathBuf, String> {
        self.project
            .features
            .iter()
            .enumerate()
            .filter_map(|(idx, feature)| {
                if !self.buffer_dirty.get(idx).copied().unwrap_or(false) {
                    return None;
                }
                let content = if self.active_buffer_idx == Some(idx) {
                    self.buffer.as_string()
                } else {
                    self.buffers.get(idx)?.as_string()
                };
                Some((feature.file_path.clone(), content))
            })
            .collect()
    }

    /// Starts a background refresh of the optional Git projection.
    ///
    /// The worker receives a project and dirty-buffer snapshot. Results are
    /// applied by [`Self::poll_git_status`] only when they still describe the
    /// current in-memory revision.
    fn refresh_git_status(&mut self) {
        self.git_status_refresh_deadline = None;
        if self.git_status_rx.is_some() {
            self.git_status_refresh_requested = true;
            return;
        }
        self.start_git_status_refresh();
    }

    fn start_git_status_refresh(&mut self) {
        self.git_status_refresh_requested = false;
        self.git_status_refresh_deadline = None;
        let revision = self.git_status_revision;
        let project_stamp = self.current_project_stamp();
        // Git rebuilds scenario mappings from diff contents; only paths are
        // needed from the editor. Avoid cloning loaded ASTs on the UI thread.
        let project = BddProject {
            root_dir: self.project.root_dir.clone(),
            features: self
                .project
                .features
                .iter()
                .map(|feature| gherkin::parse_feature("", feature.file_path.clone()))
                .collect(),
        };
        let scope = self.git_status.scope;
        let content_overrides = self.git_content_overrides();
        let (tx, rx) = mpsc::channel();
        self.git_status_rx = Some(rx);

        std::thread::spawn(move || {
            let model = teshi_core::git::load_feature_git_status_with_scope_and_content_overrides(
                &project,
                scope,
                &content_overrides,
            );
            let _ = tx.send(GitStatusRefreshResult {
                revision,
                project_stamp,
                model,
            });
        });
    }

    /// Applies the latest completed background Git projection without blocking
    /// the TUI event loop.
    pub fn poll_git_status(&mut self) {
        let Some(rx) = self.git_status_rx.take() else {
            self.maybe_start_git_status_refresh();
            return;
        };
        let mut keep_rx = true;
        let mut result = None;
        loop {
            match rx.try_recv() {
                Ok(refresh) => result = Some(refresh),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    keep_rx = false;
                    break;
                }
            }
        }
        if let Some(refresh) = result {
            if self.accepts_project_event(&refresh.project_stamp)
                && refresh.revision == self.git_status_revision
            {
                self.git_status = refresh.model;
                self.git_status_refresh_requested = false;
                self.normalize_explore_selection();
            } else {
                // A mutation happened while this worker was running. Keep the
                // stale model out of the UI, then refresh from the newest
                // buffer snapshot once this worker has exited.
                self.git_status_refresh_requested = true;
            }
        }
        if keep_rx {
            self.git_status_rx = Some(rx);
        } else {
            self.maybe_start_git_status_refresh();
        }
    }

    fn maybe_start_git_status_refresh(&mut self) {
        if self.git_status_rx.is_some() {
            return;
        }

        let now = Instant::now();
        if self.git_status_refresh_requested {
            if self
                .git_status_refresh_deadline
                .is_some_and(|deadline| now < deadline)
            {
                return;
            }
            self.start_git_status_refresh();
        }
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
        self.pending_agent_changes.push(change);
        if self.lazy_features.is_some() {
            // Lazy loading builds the MindMap index for the MindMap tab only, yet
            // the summary maps diff lines onto its nodes from any tab.
            self.rebuild_project_views(None);
        }
        self.compute_change_summary();

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

    /// Recompute `pending_change_summary` from `pending_change_diffs`.
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
        if self.active_tab == MainTab::Explore && self.explore_diff_lines.is_some() {
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
        if let Some(change) = self.pending_agent_changes.first()
            && change.tool_call_id.starts_with("teshi-mcp-")
            && !self.agents[change.agent_idx]
                .backend
                .host_tool_is_pending(&change.tool_call_id)
        {
            anyhow::bail!("ACP tool call is no longer pending; reject this stale file change");
        }
        if let Some(change) = self.pending_agent_changes.first()
            && matches!(&change.mutation, AgentMutation::InsertAfterLine { .. })
        {
            crate::agent::validate_insert_scenario_traceability(
                self,
                &change.file_path,
                &change.scenario_name,
                &change.test_point_ids,
            )
            .context("scenario traceability changed while awaiting acceptance")?;
        }
        let change = self.pending_agent_changes.remove(0);
        let updates_scenario_refs =
            matches!(&change.mutation, AgentMutation::InsertAfterLine { .. });

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
        if updates_scenario_refs && let Some(artifacts) = self.authoring_ui.artifacts.as_ref() {
            // Scenario links become durable only after the associated buffer change is accepted.
            teshi_engine::save_test_points(&self.project.root_dir, &artifacts.test_points)
                .context("persist scenario refs after accepting agent change")?;
        }

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

        self.bump_git_status_revision();
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
        self.refresh_git_status();
        self.external_change_prompt = None;
        self.status = format!("Reloaded from disk: {}", path.display());
        self.quit_pending_confirm = false;
        Ok(())
    }

    /// Bounds the current Feature's graph expansion, including duplicated path labels.
    pub(crate) fn mindmap_within_budget(&self) -> bool {
        if self.lazy_features.is_none() {
            return true;
        }
        let mut nodes = 0usize;
        let mut path_entries = 0usize;
        let mut path_bytes = 0usize;
        for feature in &self.project.features {
            let background = feature.background.as_ref().map_or(0, |b| b.steps.len());
            for scenario in feature.all_scenarios() {
                let depth = background.saturating_add(scenario.steps.len());
                if depth > 256 {
                    return false;
                }
                let steps = feature
                    .background
                    .iter()
                    .flat_map(|b| b.steps.iter())
                    .chain(scenario.steps.iter());
                for (index, step) in steps.enumerate() {
                    path_bytes = path_bytes
                        .saturating_add((step.text.len() + 64).saturating_mul(depth - index + 4));
                    if path_bytes > 16 * 1024 * 1024 {
                        return false;
                    }
                }
                nodes = nodes.saturating_add(depth);
                path_entries = path_entries.saturating_add(depth.saturating_mul(depth + 1) / 2);
                if nodes > 10_000 || path_entries > 250_000 {
                    return false;
                }
            }
        }
        true
    }

    fn rebuild_project_views(&mut self, selected_tree_location: Option<(usize, usize)>) {
        self.step_index = StepIndex::build(&self.project);
        // A pending agent change needs the node index for its Change Summary even
        // when the user reviews it from another tab.
        let build_map = (self.lazy_features.is_none()
            || self.active_tab == MainTab::MindMap
            || !self.pending_agent_changes.is_empty())
            && self.mindmap_within_budget();
        let map_project = if build_map {
            &self.project
        } else {
            &BddProject {
                root_dir: self.project.root_dir.clone(),
                features: Vec::new(),
            }
        };
        self.mindmap_index = mindmap::build_index(map_project);
        self.tree_state = mindmap::init_tree_state(&mut self.mindmap_index);
        self.mindmap_location_selection.clear();
        self.normalize_explore_selection();
        self.sync_test_point_scenario_refs();

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

    /// Rebuilds test-point → scenario refs from `@teshi-tp:*` tags without changing review state.
    fn sync_test_point_scenario_refs(&mut self) {
        let Some(artifacts) = self.authoring_ui.artifacts.as_mut() else {
            return;
        };
        if self.lazy_features.is_some() {
            let loaded: HashSet<_> = self
                .project
                .features
                .iter()
                .map(|feature| {
                    feature
                        .file_path
                        .strip_prefix(&self.project.root_dir)
                        .unwrap_or(&feature.file_path)
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect();
            let retained: Vec<_> = artifacts
                .test_points
                .test_points
                .iter()
                .map(|point| {
                    point
                        .scenario_refs
                        .iter()
                        .filter(|r| !loaded.contains(&r.feature_path))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .collect();
            teshi_core::authoring::sync_scenario_refs_from_project(
                &self.project,
                &mut artifacts.test_points.test_points,
            );
            for (point, refs) in artifacts.test_points.test_points.iter_mut().zip(retained) {
                point.scenario_refs.extend(refs);
            }
        } else {
            teshi_core::authoring::sync_scenario_refs_from_project(
                &self.project,
                &mut artifacts.test_points.test_points,
            );
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
        if !prompt.path.exists() {
            self.remove_external_feature(prompt.feature_idx);
            Ok(())
        } else {
            self.reload_feature_from_disk(prompt.feature_idx, FileStamp::capture(&prompt.path))
        }
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
                            exchanges: Vec::new(),
                            steps: Vec::new(),
                            inspector_plaintext: false,
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
                                exchanges: Vec::new(),
                                steps: Vec::new(),
                                inspector_plaintext: false,
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
                                exchanges: Vec::new(),
                                steps: Vec::new(),
                                inspector_plaintext: false,
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
                                exchanges: Vec::new(),
                                steps: Vec::new(),
                                inspector_plaintext: false,
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
                                exchanges: Vec::new(),
                                steps: Vec::new(),
                                inspector_plaintext: false,
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
                                exchanges: Vec::new(),
                                steps: Vec::new(),
                                inspector_plaintext: false,
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
                // Captures of this run belong to the Explore markers as soon
                // as it ends, without waiting for the viewer to open.
                self.refresh_capture_index();
            }
            RunEvent::StartStep {
                case_id,
                step_id,
                text,
                ..
            } => {
                if let Some(case_id) = case_id
                    && let Some(key) = self.explore_case_map.get(&case_id).copied()
                {
                    let detail = self
                        .explore_case_details
                        .entry(key)
                        .or_insert_with(|| empty_running_detail(case_id.clone()));
                    detail.steps.push(StepRunRecord {
                        step_id,
                        text: text.unwrap_or_default(),
                        status: None,
                        message: None,
                    });
                }
            }
            RunEvent::EndStep {
                case_id,
                step_id,
                status,
                message,
            } => {
                if let Some(case_id) = case_id
                    && let Some(key) = self.explore_case_map.get(&case_id).copied()
                    && let Some(detail) = self.explore_case_details.get_mut(&key)
                {
                    if let Some(step) = detail.steps.iter_mut().rev().find(|s| s.step_id == step_id)
                    {
                        step.status = status.clone();
                        step.message = message.clone();
                    }
                    if status.as_deref() == Some("failed")
                        && let Some(msg) = message
                    {
                        detail.message = Some(msg);
                    }
                }
            }
            RunEvent::HttpExchange(exchange) => {
                if let Some(case_id) = exchange.case_id.clone()
                    && let Some(key) = self.explore_case_map.get(&case_id).copied()
                {
                    let detail = self
                        .explore_case_details
                        .entry(key)
                        .or_insert_with(|| empty_running_detail(case_id));
                    detail.exchanges.push(*exchange);
                }
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
        if !self.ensure_selected_feature() {
            self.status = "Select a Feature and wait for it to load before running".into();
            return;
        }
        if self.runner_rx.is_some() {
            self.status = "Runner already active".to_string();
            return;
        }
        if self.explore_selected_deleted_feature.is_some() {
            self.status = "Deleted Feature cannot be run".to_string();
            return;
        }
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
        let needs_dispatch = self.explore_cases_need_dispatch(&cases);
        let started = if needs_dispatch {
            runner::spawn_teshi_dispatch_with_control(self.project.root_dir.clone(), cases)
        } else {
            let Some(config) = self
                .runner_config
                .clone()
                .or_else(|| runner::all_tui_cases(&cases).then(|| runner::tui_runner_config(None)))
            else {
                self.status = "Runner not configured (teshi.toml or TESHI_RUNNER_CMD)".to_string();
                return;
            };
            let request = RunRequest {
                command: "run".to_string(),
                cases,
                meta: HashMap::from([(
                    "project_root".into(),
                    self.project.root_dir.to_string_lossy().into_owned(),
                )]),
            };
            runner::spawn_runner_with_control(config, request)
        };
        match started {
            Ok((rx, cancel)) => {
                self.runner_rx = Some(rx);
                self.runner_cancel = Some(cancel);
                self.status = if needs_dispatch {
                    "API/mixed run started".to_string()
                } else {
                    "Run started".to_string()
                };
            }
            Err(err) => {
                self.status = format!("Failed to start runner: {err}");
            }
        }
    }

    fn explore_cases_need_dispatch(&self, cases: &[RunCase]) -> bool {
        cases.iter().any(|case| {
            let Some((fi, si)) = parse_case_key(&case.id) else {
                return false;
            };
            let Some(feature) = self.project.features.get(fi) else {
                return false;
            };
            let Some(scenario) = feature.scenario_at(si) else {
                return false;
            };
            teshi_engine::mode_uses_teshi_dispatch(teshi_core::scenario_engine_mode(
                feature, scenario,
            ))
        })
    }

    fn build_explore_cases(&self) -> Vec<RunCase> {
        let mut cases = Vec::new();
        if self.explore_selected_deleted_feature.is_some() {
            return cases;
        }
        let Some(feature) = self.project.features.get(self.explore_selected_feature) else {
            return cases;
        };
        match self.explore_focus {
            ColumnFocus::Feature => {
                for (si, scenario) in feature.all_scenarios().into_iter().enumerate() {
                    cases.push(build_case(
                        self.explore_selected_feature,
                        si,
                        feature,
                        scenario,
                    ));
                }
            }
            ColumnFocus::Scenario | ColumnFocus::Step => {
                if let Some(scenario) = feature.scenario_at(self.explore_selected_scenario) {
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
            && (detail.status == RunStatus::Failed
                || !detail.exchanges.is_empty()
                || detail
                    .steps
                    .iter()
                    .any(|s| s.status.as_deref() == Some("failed")))
        {
            self.explore_detail_open = true;
            self.explore_detail_case = Some(key);
        } else {
            self.status = "No run details for selection".to_string();
        }
    }

    fn toggle_inspector_plaintext(&mut self) {
        let Some(key) = self.explore_detail_case else {
            self.status = "Open run details first (Enter), then press p to expand secrets".into();
            return;
        };
        let Some(detail) = self.explore_case_details.get_mut(&key) else {
            return;
        };
        if detail.inspector_plaintext {
            detail.inspector_plaintext = false;
            self.status = "Hiding plaintext secrets".into();
            return;
        }
        let root = self.project.root_dir.clone();
        let ids: Vec<String> = detail
            .exchanges
            .iter()
            .map(|exchange| exchange.exchange_id.clone())
            .collect();
        if ids.is_empty() {
            self.status = "No HTTP exchanges to expand".into();
            return;
        }
        for (index, id) in ids.iter().enumerate() {
            let command = serde_json::json!({
                "cmd": "get_exchange",
                "request_id": "explore-expand",
                "exchange_id": id,
                "redact": false,
            });
            match teshi_engine::send_api_command(&root, command, std::time::Duration::from_secs(5))
            {
                Ok(response) => {
                    if let Some(value) = response.get("exchange")
                        && let Ok(parsed) = teshi_core::HttpExchange::from_value(value)
                        && let Some(slot) = self
                            .explore_case_details
                            .get_mut(&key)
                            .and_then(|d| d.exchanges.get_mut(index))
                    {
                        *slot = parsed;
                    }
                }
                Err(err) => {
                    self.status = format!("Plaintext expand needs a running API sidecar: {err}");
                    return;
                }
            }
        }
        if let Some(detail) = self.explore_case_details.get_mut(&key) {
            detail.inspector_plaintext = true;
        }
        self.status = "Showing plaintext secrets (p to hide)".into();
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
        self.feature_browser.directory = None;
        if idx >= self.project.features.len() {
            return;
        }
        self.persist_explore_memory();
        self.explore_selected_deleted_feature = None;
        self.explore_selected_feature = idx;
        self.restore_explore_memory();
        self.explore_detail_open = false;
        self.explore_detail_case = None;
    }

    fn explore_set_deleted_feature(&mut self, idx: usize) {
        self.feature_browser.directory = None;
        if idx >= self.git_status.deleted.len() {
            return;
        }
        self.persist_explore_memory();
        self.explore_selected_deleted_feature = Some(idx);
        self.explore_selected_scenario = 0;
        self.explore_selected_step = 0;
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
                let len = self.explore_feature_item_count();
                if len == 0 {
                    return;
                }
                let Some(current) = self.explore_selected_feature_row() else {
                    self.explore_set_feature_row(0);
                    return;
                };
                let next = clamp_idx(current as isize + delta, len);
                if next != current {
                    self.explore_set_feature_row(next);
                }
            }
            ColumnFocus::Scenario => {
                let scenarios = self.explore_selected_scenario_count();
                let next = clamp_idx(self.explore_selected_scenario as isize + delta, scenarios);
                if next != self.explore_selected_scenario {
                    self.explore_set_scenario(next);
                }
            }
            ColumnFocus::Step => {
                let steps = self.explore_selected_step_count();
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
            ColumnFocus::Feature => {
                if self.explore_feature_item_count() > 0 {
                    self.explore_set_feature_row(0);
                }
            }
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
                if let Some(last) = self.explore_feature_item_count().checked_sub(1) {
                    self.explore_set_feature_row(last);
                }
            }
            ColumnFocus::Scenario => {
                if let Some(last) = self.explore_selected_scenario_count().checked_sub(1) {
                    self.explore_set_scenario(last);
                }
            }
            ColumnFocus::Step => {
                if let Some(last) = self.explore_selected_step_count().checked_sub(1) {
                    self.explore_selected_step = last;
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
        if self.explore_selected_deleted_feature.is_some() {
            return None;
        }
        let feature = self.project.features.get(self.explore_selected_feature)?;
        let scenario = feature.scenario_at(self.explore_selected_scenario)?;
        let step = scenario.steps.get(self.explore_selected_step)?;
        Some(step.line_number)
    }

    fn explore_enter_edit(&mut self) {
        if self.explore_selected_deleted_feature.is_some() {
            self.status = "Deleted Feature cannot be edited".to_string();
            return;
        }
        let changes = self.explore_changed_steps();
        let selected_line = if let Some(change) = changes.get(self.explore_selected_step) {
            if matches!(change.kind, ChangeKind::Deleted) {
                self.status = "Deleted Step is read-only".into();
                return;
            }
            let matches: Vec<_> = self
                .project
                .features
                .get(self.explore_selected_feature)
                .and_then(|feature| feature.scenario_at(self.explore_selected_scenario))
                .into_iter()
                .flat_map(|scenario| scenario.steps.iter())
                .filter(|step| {
                    format!("{} {}", step.keyword, step.text).trim()
                        == change.text.lines().next().unwrap_or_default().trim()
                })
                .map(|step| step.line_number)
                .collect();
            if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            }
        } else {
            self.explore_selected_step_line()
        };
        let Some(line) = selected_line else {
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
            .and_then(|f| f.scenario_at(self.explore_selected_scenario))
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
                let Some(scenario) = feature.scenario_at(sci) else {
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
                if let Some(next_sc) = feature.scenario_at(sci + 1) {
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
                if let Some(first_sc) = feature.all_scenarios().into_iter().next() {
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

        let lang_code = buffer.lang_code().to_string();
        self.preview_buffer = Some(EditorBuffer::from_string_with_lang(out, lang_code));
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

    /// Records the terminal's pixel capability detected at startup.
    pub fn set_image_support(&mut self, detection: crate::image_view::Detection) {
        self.screenshot.picker = detection.picker;
        self.screenshot.support = detection.support;
        self.screenshot.support_detail = detection.detail;
    }

    /// Opens the viewer for whatever the active tab has selected.
    ///
    /// Returns false when the tab has no capture entry point, so `i` keeps its
    /// normal binding in that pane.
    fn open_screenshot_for_current_tab(&mut self) -> bool {
        match self.active_tab {
            MainTab::MindMap => {
                if self.mindmap_focus != MindMapFocus::Main || self.scenario_dropdown_open {
                    return false;
                }
                self.refresh_capture_index();
                self.open_screenshot_panel();
                true
            }
            MainTab::Explore
                if !self.explore_edit_mode
                    && (self.explore_detail_open
                        || (self.explore_focus == ColumnFocus::Step
                            && self.catalog_selection_ready()
                            && self.explore_selected_deleted_feature.is_none()
                            && self.explore_changed_steps().is_empty())) =>
            {
                // Failure Details is the primary entry: it points at the step
                // that actually broke, which is what a capture is for.
                let step = if self.explore_detail_open {
                    self.failed_step_text()
                } else {
                    self.explore_selected_step_text()
                };
                self.refresh_capture_index();
                self.open_explore_screenshot(step.as_deref());
                true
            }
            _ => false,
        }
    }

    /// Handles keys owned by the screenshot panel.
    ///
    /// Returns true when the key was consumed and must not also run its normal
    /// binding.
    pub fn handle_screenshot_key(&mut self, key: KeyEvent) -> bool {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return false;
        }
        if !self.screenshot.visible {
            return key.code == KeyCode::Char('i')
                && key.modifiers == KeyModifiers::NONE
                && self.open_screenshot_for_current_tab();
        }
        match key.code {
            KeyCode::Char('i') | KeyCode::Esc => self.close_screenshot_panel(),
            KeyCode::Char('j') => self.step_screenshot(1),
            KeyCode::Char('k') => self.step_screenshot(-1),
            KeyCode::Char('+') | KeyCode::Char('=') => self
                .screenshot
                .change_view(crate::image_view::ViewAction::ZoomIn),
            KeyCode::Char('-') => self
                .screenshot
                .change_view(crate::image_view::ViewAction::ZoomOut),
            KeyCode::Char('1') => self
                .screenshot
                .change_view(crate::image_view::ViewAction::ActualSize),
            KeyCode::Char('0') | KeyCode::Char('f') => self
                .screenshot
                .change_view(crate::image_view::ViewAction::Fit),
            KeyCode::Left | KeyCode::Char('h') => self
                .screenshot
                .change_view(crate::image_view::ViewAction::Pan(-1, 0)),
            KeyCode::Right | KeyCode::Char('l') => self
                .screenshot
                .change_view(crate::image_view::ViewAction::Pan(1, 0)),
            KeyCode::Up if self.screenshot.view.is_zoomed() => self
                .screenshot
                .change_view(crate::image_view::ViewAction::Pan(0, -1)),
            KeyCode::Down if self.screenshot.view.is_zoomed() => self
                .screenshot
                .change_view(crate::image_view::ViewAction::Pan(0, 1)),
            KeyCode::Down => self.step_screenshot(1),
            KeyCode::Up => self.step_screenshot(-1),
            KeyCode::Char('o') => self.open_screenshot_externally(),
            _ => return false,
        }
        true
    }

    /// Rescans replay history for tree markers and the fallback list.
    ///
    /// Losing the markers when the project is rebuilt would silently hide
    /// every capture, so the tree is updated on each refresh; the marker state
    /// itself decides whether a rebuild is needed.
    fn refresh_capture_index(&mut self) {
        self.capture_index = crate::image_view::CaptureIndex::scan(&self.project);
        self.explore_captures = self.captured_scenarios();
        let counts = self
            .explore_captures
            .iter()
            .map(|(key, entry)| (*key, entry.count))
            .collect();
        self.mindmap_index.set_captured_scenarios(counts);
    }

    /// Maps the newest captures onto `(feature index, scenario index)`.
    ///
    /// Entries whose scenario could not be resolved are left out; marking a
    /// whole Feature would point at nodes with no capture of their own.
    fn captured_scenarios(&self) -> HashMap<(usize, usize), crate::image_view::CaptureEntry> {
        let mut captured = HashMap::new();
        for entry in &self.capture_index.entries {
            if entry.scenario.is_empty() {
                continue;
            }
            let Some(feature_idx) = self.project.features.iter().position(|feature| {
                crate::image_view::relative_feature_path(&self.project.root_dir, &feature.file_path)
                    == entry.feature
            }) else {
                continue;
            };
            let Some(feature) = self.project.features.get(feature_idx) else {
                continue;
            };
            let Some(scenario_idx) = feature
                .all_scenarios()
                .iter()
                .position(|scenario| scenario.name == entry.scenario)
            else {
                continue;
            };
            // Entries arrive newest first, so the first one wins.
            captured
                .entry((feature_idx, scenario_idx))
                .or_insert_with(|| entry.clone());
        }
        captured
    }

    fn open_screenshot_panel(&mut self) {
        let refs = self.collect_scenario_screenshots();
        self.show_screenshot_panel(crate::image_view::ScreenshotOrigin::MindMap, refs, None);
    }

    /// Opens the viewer for the Explore selection, starting at `step_text`.
    fn open_explore_screenshot(&mut self, step_text: Option<&str>) {
        let refs = self.collect_explore_screenshots();
        self.show_screenshot_panel(
            crate::image_view::ScreenshotOrigin::Explore,
            refs,
            step_text,
        );
    }

    fn show_screenshot_panel(
        &mut self,
        origin: crate::image_view::ScreenshotOrigin,
        refs: Vec<crate::image_view::ScreenshotRef>,
        step_text: Option<&str>,
    ) {
        let start = step_text.and_then(|step| index_of_step_capture(&refs, step));
        self.screenshot.close();
        self.screenshot.origin = origin;
        self.screenshot.visible = true;
        self.screenshot.cancel();
        self.screenshot.index = start.unwrap_or(0);
        self.screenshot.refs = refs;
        if self.screenshot.refs.is_empty() {
            self.set_status_message("No captured screenshots for this scenario".to_string());
            return;
        }
        if step_text.is_some() && start.is_none() {
            self.set_status_message("No capture for this step; showing the first one".to_string());
        }
        self.load_selected_screenshot();
    }

    /// Closes the viewer and keeps the Explore Steps selection on the last
    /// capture that was shown.
    fn close_screenshot_panel(&mut self) {
        if self.screenshot.origin == crate::image_view::ScreenshotOrigin::Explore
            && let Some(step) = self.explore_step_index_for_current_capture()
        {
            self.explore_selected_step = step;
            self.persist_explore_memory();
        }
        self.screenshot.close();
    }

    fn step_screenshot(&mut self, delta: isize) {
        let total = self.screenshot.refs.len();
        if total == 0 {
            return;
        }
        let next = (self.screenshot.index as isize + delta).rem_euclid(total as isize) as usize;
        self.screenshot.index = next;
        self.load_selected_screenshot();
    }

    fn load_selected_screenshot(&mut self) {
        self.screenshot.cancel();
        let Some(shot) = self.screenshot.refs.get(self.screenshot.index).cloned() else {
            return;
        };
        self.screenshot.select(&self.project.root_dir, shot);
    }

    fn open_screenshot_externally(&mut self) {
        let Some(shot) = self.screenshot.refs.get(self.screenshot.index).cloned() else {
            return;
        };
        match crate::image_view::open_externally(&self.project.root_dir, &shot) {
            Ok(()) => self.set_status_message("Opened capture in the system viewer".to_string()),
            Err(error) => self.set_status_message(format!("Cannot open capture: {error:#}")),
        }
    }

    /// Captures recorded for the selected scenario, in step order.
    ///
    /// Only the newest run that recorded captures for this Feature is used, so
    /// two runs of the same Feature never mix in one panel.
    fn collect_scenario_screenshots(&mut self) -> Vec<crate::image_view::ScreenshotRef> {
        let Some(loc) = self.selected_node_location() else {
            return Vec::new();
        };
        let Some(feature) = self.project.features.get(loc.feature_idx) else {
            return Vec::new();
        };
        let feature_rel =
            crate::image_view::relative_feature_path(&self.project.root_dir, &feature.file_path);
        let scenario = match loc.context {
            mindmap::LocationContext::Scenario(index) => {
                feature.scenario_at(index).map(|item| item.name.clone())
            }
            mindmap::LocationContext::Background => None,
        };

        crate::image_view::screenshots_for(
            &self.project.root_dir,
            &feature_rel,
            scenario.as_deref(),
        )
    }

    /// Captures recorded for the scenario selected in Explore, in step order.
    fn collect_explore_screenshots(&mut self) -> Vec<crate::image_view::ScreenshotRef> {
        let Some(feature) = self.project.features.get(self.explore_selected_feature) else {
            return Vec::new();
        };
        let feature_rel =
            crate::image_view::relative_feature_path(&self.project.root_dir, &feature.file_path);
        let scenario = feature
            .scenario_at(self.explore_selected_scenario)
            .map(|item| item.name.clone());
        crate::image_view::screenshots_for(
            &self.project.root_dir,
            &feature_rel,
            scenario.as_deref(),
        )
    }

    /// Captures recorded for the Explore scenario, `0` when it has none.
    pub(crate) fn explore_scenario_captures(&self, scenario_idx: usize) -> usize {
        self.explore_captures
            .get(&(self.explore_selected_feature, scenario_idx))
            .map_or(0, |entry| entry.count)
    }

    /// Captures recorded for one step of the Explore scenario.
    pub(crate) fn explore_step_captures(&self, step_text: &str) -> usize {
        self.explore_captures
            .get(&(
                self.explore_selected_feature,
                self.explore_selected_scenario,
            ))
            .map_or(0, |entry| entry.step_captures(step_text.trim()))
    }

    /// Text of the step selected in the Explore Steps pane.
    fn explore_selected_step_text(&self) -> Option<String> {
        let feature = self.project.features.get(self.explore_selected_feature)?;
        let step = feature
            .scenario_at(self.explore_selected_scenario)?
            .steps
            .get(self.explore_selected_step)?;
        Some(step.text.clone())
    }

    /// Text of the step that failed in the open Failure Details popup.
    fn failed_step_text(&self) -> Option<String> {
        let detail = self.explore_case_details.get(&self.explore_detail_case?)?;
        detail
            .steps
            .iter()
            .rev()
            .find(|step| step.status.as_deref() == Some("failed"))
            .or_else(|| detail.steps.last())
            .map(|step| step.text.clone())
    }

    /// Step index in the selected scenario for the capture the viewer shows.
    fn explore_step_index_for_current_capture(&self) -> Option<usize> {
        let shot = self.screenshot.refs.get(self.screenshot.index)?;
        let feature = self.project.features.get(self.explore_selected_feature)?;
        let scenario = feature.scenario_at(self.explore_selected_scenario)?;
        let step_text = shot.step_text.trim();
        scenario.steps.iter().position(|step| {
            (!step_text.is_empty() && step.text.trim() == step_text)
                || shot.step_line == Some(step.line_number as u64)
        })
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
        self.bump_git_status_revision();
        // Re-parse
        let path = self.project.features[idx].file_path.clone();
        let content = self.buffer.as_string();
        self.project.features[idx] = gherkin::parse_feature(&content, path);
        let selected_tree_location = self.selected_tree_location();
        self.rebuild_project_views(selected_tree_location);
        self.refresh_git_status();
    }

    // ── Tree navigation ─────────────────────────────────────────────

    fn tree_move_up(&mut self) {
        // At root (path length == 1), up is a no-op: there is nothing above.
        if self.tree_state.selected().len() <= 1 {
            return;
        }
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

    fn tree_move_sibling_prev(&mut self) {
        if let Some(id) = mindmap::selected_node_id(&self.tree_state)
            && let Some(sibling_id) = self.mindmap_index.prev_sibling(id)
            && let Some(path) = self.mindmap_index.path_for(&sibling_id)
        {
            self.tree_state.select(path.clone());
            self.mindmap_index.apply_highlight_categories(&sibling_id);
            self.tree_follow_editor();
        }
        self.quit_pending_confirm = false;
    }

    fn tree_move_sibling_next(&mut self) {
        if let Some(id) = mindmap::selected_node_id(&self.tree_state)
            && let Some(sibling_id) = self.mindmap_index.next_sibling(id)
            && let Some(path) = self.mindmap_index.path_for(&sibling_id)
        {
            self.tree_state.select(path.clone());
            self.mindmap_index.apply_highlight_categories(&sibling_id);
            self.tree_follow_editor();
        }
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
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_cycle_location(&mut self, delta: isize) {
        let preview_visible =
            self.view_stage == ViewStage::TreeAndEditor || self.mindmap_ai_panel_visible;
        if !preview_visible {
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
        let selected_path = self.tree_state.selected().to_vec();
        let at_root = selected_path.len() <= 1;

        if at_root && !self.tree_state.opened().contains(&selected_path) {
            // Root is already collapsed — nothing to do.
            // Without this guard, tui_tree_widget::key_left() would pop
            // the root from the selection, leaving nothing selected.
            return;
        }

        self.tree_state.key_left();
        if let Some(id) = mindmap::selected_node_id(&self.tree_state) {
            self.mindmap_index.apply_highlight_categories(id);
        }
        self.tree_follow_editor();
        self.quit_pending_confirm = false;
    }

    fn tree_toggle(&mut self) {
        self.tree_state.toggle_selected();
        let preview_visible =
            self.view_stage == ViewStage::TreeAndEditor || self.mindmap_ai_panel_visible;
        if preview_visible {
            self.rebuild_preview();
        }
        self.quit_pending_confirm = false;
    }

    // ── Action handler ──────────────────────────────────────────────

    pub fn handle_action(&mut self, action: Action) -> Result<()> {
        // Capture a target before showing the modal. Background rows and mouse
        // actions cannot change the selection while confirmation owns input.
        let confirmed_delete = self.pending_delete.is_some() && action == Action::DeleteConfirm;
        let delete_request = self.pending_delete.clone();
        let action = if let Some(pending) = self.pending_delete.as_ref() {
            match action {
                Action::DeleteConfirm => {
                    let pending = pending.action.clone();
                    self.pending_delete = None;
                    pending
                }
                Action::DeleteCancel => {
                    self.pending_delete = None;
                    self.pending_char = None;
                    return Ok(());
                }
                _ => return Ok(()),
            }
        } else {
            action
        };
        if !confirmed_delete {
            let target = match &action {
                Action::ModelPanelDelete => self
                    .model_profiles
                    .get(self.model_panel_selection)
                    .map(|p| (p.id.clone(), format!("model profile '{}'", p.name))),
                Action::SessionPanelDelete => self
                    .session_list
                    .get(self.session_panel_selection)
                    .map(|s| (s.id.clone(), format!("session '{}'", s.id))),
                _ => None,
            };
            if let Some((target_id, label)) = target {
                self.pending_delete = Some(DeleteConfirmation {
                    action,
                    target_id,
                    label,
                    error: None,
                });
                self.pending_char = None;
                return Ok(());
            }
        }
        if confirmed_delete && let Some(request) = delete_request.as_ref() {
            let position = match request.action {
                Action::ModelPanelDelete => self
                    .model_profiles
                    .iter()
                    .position(|p| p.id == request.target_id),
                Action::SessionPanelDelete => self
                    .session_list
                    .iter()
                    .position(|s| s.id == request.target_id),
                _ => None,
            };
            let Some(position) = position else {
                let mut request = request.clone();
                request.error = Some("The selected item is no longer available.".into());
                self.pending_delete = Some(request);
                return Ok(());
            };
            match request.action {
                Action::ModelPanelDelete => self.model_panel_selection = position,
                Action::SessionPanelDelete => self.session_panel_selection = position,
                _ => {}
            }
        }
        if !matches!(action, Action::PendingChar(_)) {
            self.pending_char = None;
        }
        if !matches!(
            self.project_switch.overlay,
            crate::project_context::SwitchOverlay::None
        ) {
            return self.project_switch_action(action);
        }
        if let crate::authoring_tab::RequirementsOverlay::Unsaved { pending } =
            self.authoring_ui.overlay.clone()
        {
            return self.resolve_unsaved_requirement(action, pending);
        }
        if self.is_requirements_editor_active()
            && self.authoring_ui.editor_mode == crate::authoring_tab::RequirementsEditorMode::Browse
            && matches!(
                action,
                Action::Insert(_)
                    | Action::Enter
                    | Action::InsertNewline
                    | Action::Backspace
                    | Action::Delete
            )
        {
            return Ok(());
        }
        if self.external_change_prompt.is_some() {
            return match action {
                Action::ExternalChangeReload => self.accept_external_reload(),
                Action::ExternalChangeKeepLocal => {
                    self.keep_local_external_version();
                    Ok(())
                }
                // The prompt owns file edits, not the whole session: quitting
                // must never look like a dead keyboard.
                Action::Quit => {
                    self.quit();
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        if self.has_agent_change_prompt() {
            // AiBlurInput / AiFocusInput are allowed through: Esc unfocuses first,
            // Enter refocuses, and only then does a second Esc trigger AgentChangeReject.
            if matches!(
                action,
                Action::AiBlurInput
                    | Action::AiFocusInput
                    | Action::AgentChangeDiff
                    | Action::ToggleChangeSummary
                    | Action::ChangeSummaryUp
                    | Action::ChangeSummaryDown
                    | Action::ChangeSummaryJump
                    | Action::ClearInputState
            ) {
                // fall through to normal handling below
            } else {
                return match action {
                    Action::AgentChangeAccept => {
                        let agent_idx = self.pending_agent_changes[0].agent_idx;
                        let call_id = self.pending_agent_changes[0].tool_call_id.clone();
                        match self.accept_agent_change() {
                            Ok((tool_call_id, result)) => {
                                self.feed_agent_tool_result_for_agent(
                                    agent_idx,
                                    tool_call_id,
                                    result,
                                );
                            }
                            Err(error) if call_id.starts_with("teshi-mcp-") => {
                                // Acceptance can fail either before or after it
                                // removes the proposal. Complete the MCP call in
                                // both cases and discard only this failed item.
                                if self
                                    .pending_agent_changes
                                    .first()
                                    .is_some_and(|change| change.tool_call_id == call_id)
                                {
                                    self.pending_agent_changes.remove(0);
                                }
                                self.clear_pending_change_state();
                                self.agents[agent_idx]
                                    .backend
                                    .resolve_host_tool(&call_id, Err(error.to_string()));
                                self.agents[agent_idx].status =
                                    if self.agents[agent_idx].backend.status()
                                        == teshi_agent::backend::AgentBackendStatus::Running
                                    {
                                        AiStatus::Waiting
                                    } else {
                                        AiStatus::Idle
                                    };
                                self.status = format!("Could not apply ACP file change: {error}");
                            }
                            Err(error) => return Err(error),
                        }
                        Ok(())
                    }
                    Action::AgentChangeReject => {
                        let agent_idx = self.pending_agent_changes[0].agent_idx;
                        let (tool_call_id, result) = self.reject_agent_change();
                        if self.agents[agent_idx]
                            .backend
                            .resolve_host_tool(&tool_call_id, Err(result.clone()))
                        {
                            self.agents[agent_idx].status = AiStatus::Waiting;
                            return Ok(());
                        }
                        self.feed_agent_tool_result_for_agent(agent_idx, tool_call_id, result);
                        Ok(())
                    }
                    _ => Ok(()),
                };
            }
        }

        // Quit confirmation panel intercept (highest priority after prompts)
        if self.quit_pending_confirm {
            return match action {
                Action::DeleteConfirm => {
                    self.should_quit = true;
                    Ok(())
                }
                Action::Quit => {
                    if self.quit_panel_selection == 0 {
                        self.should_quit = true;
                    } else {
                        self.quit_pending_confirm = false;
                    }
                    Ok(())
                }
                Action::ClearInputState => {
                    self.quit_pending_confirm = false;
                    Ok(())
                }
                Action::MoveLeft => {
                    self.quit_panel_selection = 0;
                    Ok(())
                }
                Action::MoveRight => {
                    self.quit_panel_selection = 1;
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        if self.navigate_modal_list(&action) {
            return Ok(());
        }
        if self.authoring_ui.overlay_active() {
            return match action {
                Action::OverlayConfirm => self.confirm_requirements_overlay(),
                Action::OverlayCancel => {
                    self.authoring_ui.cancel_overlay();
                    Ok(())
                }
                Action::OverlayMoveUp => {
                    self.authoring_ui.overlay_move_selection(-1);
                    Ok(())
                }
                Action::OverlayMoveDown => {
                    self.authoring_ui.overlay_move_selection(1);
                    Ok(())
                }
                Action::OverlayInsert(ch) => {
                    self.authoring_ui.overlay_insert_char(ch);
                    Ok(())
                }
                Action::OverlayBackspace => {
                    self.authoring_ui.overlay_backspace();
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        if self.generation_scope_prompt.is_some() {
            return match action {
                Action::GenerationScopeConfirm => self.confirm_generation_scope(),
                Action::GenerationScopeCancel => {
                    self.generation_scope_prompt = None;
                    self.status = "Generation canceled".to_string();
                    Ok(())
                }
                Action::GenerationScopeChangeIteration => {
                    self.authoring_ui.open_filter_picker();
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        // Agent profile selection panel intercept.
        if self.agent_profile_panel_active {
            if self.agent_panel_mode != AgentPanelMode::List {
                return self.handle_acp_panel_action(action);
            }
            return match action {
                Action::AgentPanelUp => {
                    if self.agent_profile_panel_selection > 0 {
                        self.agent_profile_panel_selection -= 1;
                    }
                    self.quit_pending_confirm = false;
                    Ok(())
                }
                Action::AgentPanelDown => {
                    if self.agent_profile_panel_selection + 1 < self.agent_registry.len() {
                        self.agent_profile_panel_selection += 1;
                    }
                    self.quit_pending_confirm = false;
                    Ok(())
                }
                Action::AgentPanelSelect => {
                    let selected_idx = self.agent_profile_panel_selection;
                    let profile_clone = self.agent_registry.get_index(selected_idx).cloned();
                    if let Some(profile) = profile_clone {
                        let acp = self.agent().backend.kind()
                            == teshi_agent::backend::AgentBackendKind::Acp;
                        if acp {
                            self.cancel_agent_turn();
                        }
                        let agent = self.agent_mut();
                        agent.profile_id = Some(profile.id.clone());
                        let _ = agent;
                        if acp {
                            let tools = teshi_agent_runtime::tool_bridge::ToolBridgeConfig {
                                executable: std::env::current_exe()?,
                                tools: self.agent_tools(self.selected_agent),
                            };
                            if let Err(error) =
                                self.agent_mut().backend.reconnect_acp_host_tools(tools)
                            {
                                self.agent_mut().status = AiStatus::Error;
                                self.status = format!(
                                    "Could not reconnect ACP for profile {}: {error}",
                                    profile.name
                                );
                                self.agent_profile_panel_active = false;
                                return Ok(());
                            }
                        } else if !profile.model_ref.is_empty() {
                            let model_id = &profile.model_ref;
                            let model_profile_clone = self
                                .model_profiles
                                .iter()
                                .find(|m| m.id == *model_id)
                                .cloned();
                            if let Some(mp) = model_profile_clone {
                                self.activate_model_profile(&mp);
                            }
                        }
                        self.status = format!("Switched to agent: {}", profile.name);
                    }
                    self.agent_profile_panel_active = false;
                    self.quit_pending_confirm = false;
                    Ok(())
                }
                Action::ClearInputState | Action::AgentPanelClose => {
                    self.agent_profile_panel_active = false;
                    self.agent_panel_mode = AgentPanelMode::List;
                    self.quit_pending_confirm = false;
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        // Approval mode selection panel intercept
        if self.approval_panel_active {
            return match action {
                Action::ApprovalPanelUp => {
                    if self.approval_panel_selection > 0 {
                        self.approval_panel_selection -= 1;
                    }
                    Ok(())
                }
                Action::ApprovalPanelDown => {
                    use teshi_agent::approval::ApprovalMode;
                    if self.approval_panel_selection + 1 < ApprovalMode::ALL.len() {
                        self.approval_panel_selection += 1;
                    }
                    Ok(())
                }
                Action::ApprovalPanelSelect => {
                    use teshi_agent::approval::ApprovalMode;
                    let modes = ApprovalMode::ALL;
                    if let Some(&mode) = modes.get(self.approval_panel_selection) {
                        let old = self.approval_mode;
                        self.approval_mode = mode;
                        self.approval_panel_active = false;
                        self.status = format!(
                            "Approval: {} → {} ({})",
                            old.display_name(),
                            mode.display_name(),
                            mode.description()
                        );
                    }
                    Ok(())
                }
                Action::ClearInputState
                | Action::AuthPanelClose
                | Action::ModelPanelClose
                | Action::SessionPanelClose => {
                    self.approval_panel_active = false;
                    Ok(())
                }
                _ => Ok(()),
            };
        }

        // Modal list navigation returns above; do not let a pending dd survive
        // another command or a cancelled modal.
        if !matches!(action, Action::PendingChar(_)) {
            self.pending_char = None;
        }
        if self.active_tab == MainTab::TestPoints
            && matches!(
                action,
                Action::FocusNextColumn | Action::FocusPrevColumn | Action::SelectTab(_)
            )
        {
            self.commit_test_point_field_edit()?;
            self.test_points_ui.editing = false;
        }
        match action {
            // Explore tab navigation
            Action::FocusNextColumn => {
                self.focus_visible_pane(true);
            }
            Action::FocusPrevColumn => {
                self.focus_visible_pane(false);
            }
            Action::FocusPaneDown | Action::FocusPaneUp => {
                // The current main layouts are either a horizontal pane row
                // or a single pane.  Keep the action explicit so Ctrl-W j/k
                // is consumed as a documented no-op where no vertical
                // neighbor exists, instead of becoming editor input.
                self.quit_pending_confirm = false;
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
                    self.agent_mut().messages_mut().push(AiChatMessage {
                        role: AiRole::User,
                        content: msg,
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                        source: Some("MindMap".into()),
                    });
                    self.active_tab = MainTab::Ai;
                    self.agent_mut().status = AiStatus::Waiting;
                    self.agent_mut().partial_response_mut().clear();
                    self.agent_mut().scroll_offset = 0;
                    if !self.start_agent_backend(self.selected_agent) {
                        return Ok(());
                    }
                    self.status = "Sending MindMap context to AI...".to_string();

                    if !crate::llm::is_configured() {
                        self.agent_mut().messages_mut().push(AiChatMessage {
                                role: AiRole::Assistant,
                                content: "AI is not configured. Run 'teshi auth login', open the model panel, or set TESHI_LLM_API_KEY in your environment.".to_string(),
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: None,
                                source: None,
                            });
                        self.agent_mut().status = AiStatus::Idle;
                        self.agent_mut().partial_response_mut().clear();
                        self.status = "AI not configured".to_string();
                    } else if self.agent().backend.is_connected() {
                        let messages = self.build_chat_messages_for_agent(self.selected_agent);
                        let profile = self.agent_profile(self.selected_agent);
                        let allowed: Option<&[String]> = profile.and_then(|p| match &p.tools {
                            teshi_agent::definition::ToolPermission::All => None,
                            teshi_agent::definition::ToolPermission::None => Some(&[] as &[String]),
                            teshi_agent::definition::ToolPermission::Whitelist(list) => {
                                Some(list.as_slice())
                            }
                        });
                        let tools = Some(teshi_agent::get_tools(allowed));
                        let system = Some(self.ai_system_prompt(None, self.selected_agent));
                        if let Err(error) = self
                            .agent_mut()
                            .backend
                            .submit_turn(system, messages, tools)
                        {
                            self.agent_mut().status = AiStatus::Error;
                            self.agent_mut().partial_response_mut().clear();
                            self.status = format!("AI error: {error}");
                        }
                    } else {
                        self.agent_mut().status = AiStatus::Error;
                        self.agent_mut().partial_response_mut().clear();
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
            // Handled in the approval panel intercept above; unreachable here.
            Action::ApprovalPanelUp | Action::ApprovalPanelDown | Action::ApprovalPanelSelect => {}
            Action::AuthPanelClose => {
                self.auth_panel_active = false;
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelOpen => {
                self.cmd_models()?;
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
                    if let Err(e) = crate::profiles::write_active_id(&profile.id) {
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
                self.model_form_editing_id = None;
                self.model_panel_mode = ModelPanelMode::Adding;
                self.model_form_focus = 0;
                self.model_form_name.clear();
                self.model_form_cursor = 0;
                self.model_form_provider.clear();
                self.model_form_model.clear();
                self.model_form_base_url.clear();
                self.model_form_api_key.clear();
                self.model_form_max_tokens = String::from("4096");
                self.model_form_temperature = String::from("0.7");
                self.status = "Fill in the fields and press Enter to save.".to_string();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelEdit => {
                if let Some(profile) = self.model_profiles.get(self.model_panel_selection).cloned()
                {
                    self.model_form_editing_id = Some(profile.id.clone());
                    self.model_panel_mode = ModelPanelMode::Editing;
                    self.model_form_focus = 0;
                    self.model_form_temperature =
                        crate::profiles::profile_temperature(&profile).to_string();
                    self.model_form_name = profile.name;
                    self.model_form_cursor = self.model_form_name.chars().count();
                    self.model_form_provider = profile.provider;
                    self.model_form_model = profile.model_id;
                    self.model_form_base_url = profile.base_url;
                    self.model_form_api_key = profile.api_key;
                    self.model_form_max_tokens = profile.max_output_tokens.to_string();
                    self.status = "Edit the fields and press Enter to save.".to_string();
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelDelete => {
                if let Some(profile) = self.model_profiles.get(self.model_panel_selection).cloned()
                {
                    let name = profile.name.clone();
                    let deleted_id = profile.id.clone();
                    if let Err(e) = crate::profiles::delete_profile(&deleted_id) {
                        self.status = format!("Failed to delete profile: {e}");
                        if let Some(mut request) = delete_request {
                            request.error = Some(e.to_string());
                            self.pending_delete = Some(request);
                        }
                    } else {
                        self.model_profiles = crate::profiles::load_all();
                        self.model_active_id = crate::profiles::read_active_id();
                        if self.model_panel_selection >= self.model_profiles.len() {
                            self.model_panel_selection =
                                self.model_profiles.len().saturating_sub(1);
                        }
                        // Engine may have activated a sibling; respawn from the new active.
                        self.cancel_agent_turn();
                        if let Some(id) = self.model_active_id.clone() {
                            if let Some(p) =
                                self.model_profiles.iter().find(|p| p.id == id).cloned()
                            {
                                self.activate_model_profile(&p);
                            } else {
                                self.active_model_label = None;
                                self.spawn_llm_if_configured();
                            }
                        } else {
                            self.active_model_label = None;
                            self.spawn_llm_if_configured();
                        }
                        self.status = format!("Deleted profile: {name}");
                    }
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormCancel => {
                self.model_form_editing_id = None;
                self.model_panel_mode = ModelPanelMode::List;
                self.status =
                    "Model profiles [m]. a add · e edit · ↑↓ select · Enter activate · Esc close"
                        .to_string();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormNext => {
                let max = 6usize; // last field index (Temperature)
                if self.model_form_focus < max {
                    self.model_form_focus += 1;
                }
                self.model_form_cursor = self.model_form_field().chars().count();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormPrev => {
                if self.model_form_focus > 0 {
                    self.model_form_focus -= 1;
                }
                self.model_form_cursor = self.model_form_field().chars().count();
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormInsert(ch) => {
                let cursor = self.model_form_cursor;
                let field = self.model_form_field();
                let byte = field
                    .char_indices()
                    .nth(cursor)
                    .map_or(field.len(), |(byte, _)| byte);
                field.insert(byte, ch);
                self.model_form_cursor = cursor + 1;
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormBackspace => {
                if self.model_form_cursor > 0 {
                    self.model_form_cursor -= 1;
                    self.model_form_delete_at_cursor();
                }
                self.quit_pending_confirm = false;
            }
            Action::ModelPanelFormCursor(code) => {
                use crossterm::event::KeyCode;
                let len = self.model_form_field().chars().count();
                self.model_form_cursor = self.model_form_cursor.min(len);
                match code {
                    KeyCode::Left => {
                        self.model_form_cursor = self.model_form_cursor.saturating_sub(1)
                    }
                    KeyCode::Right => {
                        self.model_form_cursor = (self.model_form_cursor + 1).min(len)
                    }
                    KeyCode::Home => self.model_form_cursor = 0,
                    KeyCode::End => self.model_form_cursor = len,
                    KeyCode::Delete => self.model_form_delete_at_cursor(),
                    _ => {}
                }
            }
            Action::ModelPanelFormSubmit => {
                let name = self.model_form_name.trim().to_string();
                if name.is_empty() {
                    self.status = "Name is required.".to_string();
                    self.quit_pending_confirm = false;
                    return Ok(());
                }
                let provider = if self.model_form_provider.trim().is_empty() {
                    teshi_engine::PROVIDER_OPENAI.to_string()
                } else {
                    crate::profiles::normalize_provider(self.model_form_provider.trim())
                };
                let model = self.model_form_model.trim().to_string();
                if model.is_empty() {
                    self.status = "Model identifier is required.".to_string();
                    self.quit_pending_confirm = false;
                    return Ok(());
                }
                let base_url = self.model_form_base_url.trim().to_string();
                let api_key = self.model_form_api_key.trim().to_string();
                let max_tokens: u32 = self.model_form_max_tokens.trim().parse().unwrap_or(4096);
                let temperature: f32 = self.model_form_temperature.trim().parse().unwrap_or(0.7);

                let is_editing = self.model_form_editing_id.is_some();

                let mut profile = if let Some(ref edit_id) = self.model_form_editing_id {
                    // Editing existing profile — preserve the original ID.
                    match self
                        .model_profiles
                        .iter()
                        .find(|p| p.id == *edit_id)
                        .cloned()
                    {
                        Some(p) => p,
                        None => {
                            self.status = "Profile not found for editing.".to_string();
                            self.quit_pending_confirm = false;
                            return Ok(());
                        }
                    }
                } else {
                    teshi_engine::ModelProfile::new(name.clone())
                };
                profile.name = name;
                profile.provider = provider;
                profile.model_id = model;
                profile.base_url = base_url;
                profile.api_key = api_key;
                profile.max_output_tokens = max_tokens;
                crate::profiles::set_profile_temperature(&mut profile, temperature);

                if let Err(e) = crate::profiles::save(&mut profile) {
                    self.status = format!("Failed to save profile: {e}");
                    self.quit_pending_confirm = false;
                    return Ok(());
                }

                if is_editing {
                    self.status = format!("Updated profile: {}", profile.name);
                    if self.model_active_id.as_deref() == Some(&profile.id) {
                        self.activate_model_profile(&profile);
                    }
                } else if let Err(e) = crate::profiles::write_active_id(&profile.id) {
                    self.status = format!("Profile saved but failed to activate: {e}");
                } else {
                    self.model_active_id = Some(profile.id.clone());
                    self.activate_model_profile(&profile);
                    self.status = format!("Added and activated profile: {}", profile.name);
                }

                self.model_form_editing_id = None;
                self.model_profiles = crate::profiles::load_all();
                self.model_panel_mode = ModelPanelMode::List;
                self.model_panel_selection = 0;
                self.quit_pending_confirm = false;
            }
            // ── Session browser panel actions ─────────────────
            Action::SessionPanelOpen => {
                return self.cmd_sessions(false);
            }
            Action::SessionPanelClose => {
                self.session_panel_active = false;
                self.pending_session_restore = None;
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
                    self.save_sessions(true)?;
                    if !self.restore_session(session, self.session_panel_legacy) {
                        return Ok(());
                    }
                }
                self.quit_pending_confirm = false;
            }
            Action::SessionPanelDelete => {
                if let Some(session) = self.session_list.get(self.session_panel_selection).cloned()
                {
                    let id = session.id.clone();
                    if let Err(e) = crate::session::Session::delete(&id) {
                        self.status = format!("Failed to delete session: {e}");
                        if let Some(mut request) = delete_request {
                            request.error = Some(e.to_string());
                            self.pending_delete = Some(request);
                        }
                    } else {
                        self.session_list = if self.session_panel_legacy {
                            self.legacy_sessions()
                        } else {
                            self.project_sessions()
                        };
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
                if !self.ensure_selected_feature() {
                    self.status = "Select a Feature and wait for it to load before editing".into();
                    return Ok(());
                }
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
            Action::ToggleInspectorPlaintext => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.toggle_inspector_plaintext();
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
            Action::TreeSiblingPrev => self.tree_move_sibling_prev(),
            Action::TreeSiblingNext => self.tree_move_sibling_next(),

            // Editor navigation (MindMap stage 3 & legacy)
            Action::MoveUp => {
                if self.is_test_points_details_active() && self.test_points_ui.editing {
                    self.test_points_ui.field_line_vertical(false);
                    return Ok(());
                }
                if self.active_tab == MainTab::Requirements {
                    match self.authoring_ui.focus {
                        crate::authoring_tab::RequirementsFocus::Tree => {
                            self.authoring_ui.move_tree_selection(-1);
                        }
                        crate::authoring_tab::RequirementsFocus::LinkedTestPoints => {
                            self.authoring_ui.move_linked_selection(-1);
                        }
                        crate::authoring_tab::RequirementsFocus::Editor => {
                            self.authoring_ui.move_cursor_vertical(-1);
                            self.authoring_ui.extend_visual_selection();
                        }
                    }
                    self.quit_pending_confirm = false;
                } else if self.active_tab == MainTab::TestPoints {
                    match self.test_points_ui.focus {
                        crate::test_points_tab::TestPointsFocus::Tree => {
                            self.test_points_ui.move_tree_selection(-1);
                            self.reload_test_point_field_buffer();
                        }
                        crate::test_points_tab::TestPointsFocus::Details => {
                            self.commit_test_point_field_edit()?;
                            self.test_points_ui.move_detail_field(-1);
                            self.reload_test_point_field_buffer();
                        }
                        crate::test_points_tab::TestPointsFocus::Excerpts => {
                            let count = self.current_excerpt_count();
                            self.test_points_ui.move_excerpt_selection(-1, count);
                        }
                    }
                    self.quit_pending_confirm = false;
                } else if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_selection(-1);
                    self.quit_pending_confirm = false;
                } else {
                    self.move_up();
                }
            }
            Action::MoveDown => {
                if self.is_test_points_details_active() && self.test_points_ui.editing {
                    self.test_points_ui.field_line_vertical(true);
                    return Ok(());
                }
                if self.active_tab == MainTab::Requirements {
                    match self.authoring_ui.focus {
                        crate::authoring_tab::RequirementsFocus::Tree => {
                            self.authoring_ui.move_tree_selection(1);
                        }
                        crate::authoring_tab::RequirementsFocus::LinkedTestPoints => {
                            self.authoring_ui.move_linked_selection(1);
                        }
                        crate::authoring_tab::RequirementsFocus::Editor => {
                            self.authoring_ui.move_cursor_vertical(1);
                            self.authoring_ui.extend_visual_selection();
                        }
                    }
                    self.quit_pending_confirm = false;
                } else if self.active_tab == MainTab::TestPoints {
                    match self.test_points_ui.focus {
                        crate::test_points_tab::TestPointsFocus::Tree => {
                            self.test_points_ui.move_tree_selection(1);
                            self.reload_test_point_field_buffer();
                        }
                        crate::test_points_tab::TestPointsFocus::Details => {
                            self.commit_test_point_field_edit()?;
                            self.test_points_ui.move_detail_field(1);
                            self.reload_test_point_field_buffer();
                        }
                        crate::test_points_tab::TestPointsFocus::Excerpts => {
                            let count = self.current_excerpt_count();
                            self.test_points_ui.move_excerpt_selection(1, count);
                        }
                    }
                    self.quit_pending_confirm = false;
                } else if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_selection(1);
                    self.quit_pending_confirm = false;
                } else {
                    self.move_down();
                }
            }
            Action::MoveSiblingUp => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.quit_pending_confirm = false;
                } else {
                    self.move_sibling_up();
                }
            }
            Action::MoveSiblingDown => {
                if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.quit_pending_confirm = false;
                } else {
                    self.move_sibling_down();
                }
            }
            Action::MoveLeft => {
                if self.active_tab == MainTab::Requirements {
                    if self.authoring_ui.focus == crate::authoring_tab::RequirementsFocus::Editor {
                        self.authoring_ui.move_cursor_left();
                        self.authoring_ui.extend_visual_selection();
                    }
                } else if self.active_tab == MainTab::TestPoints {
                    if self.test_points_ui.focus == crate::test_points_tab::TestPointsFocus::Details
                    {
                        self.test_points_ui.move_field_cursor(-1);
                    }
                } else if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    // No-op in Explore browse mode
                } else {
                    self.move_left();
                }
            }
            Action::MoveRight => {
                if self.active_tab == MainTab::Requirements {
                    if self.authoring_ui.focus == crate::authoring_tab::RequirementsFocus::Editor {
                        self.authoring_ui.move_cursor_right();
                        self.authoring_ui.extend_visual_selection();
                    }
                } else if self.active_tab == MainTab::TestPoints {
                    if self.test_points_ui.focus == crate::test_points_tab::TestPointsFocus::Details
                    {
                        self.test_points_ui.move_field_cursor(1);
                    }
                } else if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    // No-op in Explore browse mode
                } else {
                    self.move_right();
                }
            }
            Action::MoveHome => {
                if self.is_test_points_details_active() {
                    self.test_points_ui.field_line_boundary(false);
                } else if self.is_requirements_editor_active() {
                    self.authoring_ui.cursor_col = 0;
                    self.authoring_ui.desired_col = 0;
                    self.authoring_ui.extend_visual_selection();
                } else if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_home();
                    self.quit_pending_confirm = false;
                } else {
                    self.move_home();
                }
            }
            Action::MoveEnd => {
                if self.is_test_points_details_active() {
                    self.test_points_ui.field_line_boundary(true);
                } else if self.is_requirements_editor_active() {
                    let col = self
                        .authoring_ui
                        .buffer
                        .line_len_chars(self.authoring_ui.cursor_row);
                    self.authoring_ui.cursor_col = col;
                    self.authoring_ui.desired_col = col;
                    self.authoring_ui.extend_visual_selection();
                } else if self.active_tab == MainTab::Explore && !self.explore_edit_mode {
                    self.explore_move_end();
                    self.quit_pending_confirm = false;
                } else {
                    self.move_end();
                }
            }
            Action::PageUp => self.navigate_page(false)?,
            Action::PageDown => self.navigate_page(true)?,
            Action::NavigateFirst => self.navigate_boundary(false)?,
            Action::NavigateLast => self.navigate_boundary(true)?,
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
            Action::Undo => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.undo_edit();
                } else {
                    self.undo();
                }
            }
            Action::Redo => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.redo_edit();
                } else {
                    self.redo();
                }
            }
            Action::PendingChar(ch) => {
                self.pending_char = Some(ch);
                self.status = if self.is_requirements_editor_active()
                    && matches!(
                        self.authoring_ui.editor_mode,
                        crate::authoring_tab::RequirementsEditorMode::Browse
                            | crate::authoring_tab::RequirementsEditorMode::VisualChar
                            | crate::authoring_tab::RequirementsEditorMode::VisualLine
                    ) {
                    match ch {
                        'd' => "Vim pending: dd delete".to_string(),
                        'y' => "Vim pending: yy yank".to_string(),
                        'g' => "Vim pending: gg top".to_string(),
                        _ => "Vim pending command".to_string(),
                    }
                } else {
                    match ch {
                        'd' => "`dd` to delete".to_string(),
                        'y' => "`yy` to copy".to_string(),
                        _ => "Pending command".to_string(),
                    }
                };
                self.quit_pending_confirm = false;
            }
            Action::Insert(ch) => {
                if self.is_requirements_editor_active() {
                    if ch == '\t' {
                        for _ in 0..4 {
                            self.authoring_ui.insert_char(' ');
                        }
                    } else if !ch.is_control() {
                        self.authoring_ui.insert_char(ch);
                    }
                    self.quit_pending_confirm = false;
                } else if self.is_test_points_details_active() && self.test_points_ui.editing {
                    self.test_points_ui.insert_char(ch);
                    self.quit_pending_confirm = false;
                } else if !self.step_input_active {
                    return Ok(());
                } else {
                    self.push_undo();
                    self.buffer
                        .insert_char(self.cursor_row, self.cursor_col, ch);
                    self.cursor_col += 1;
                    self.desired_col = self.cursor_col;
                    self.mark_current_buffer_dirty();
                    self.quit_pending_confirm = false;
                }
            }
            Action::Enter => {
                if self.active_tab == MainTab::TestPoints
                    && self.test_points_ui.focus
                        == crate::test_points_tab::TestPointsFocus::Excerpts
                {
                    self.navigate_to_requirement_excerpt(
                        self.test_points_ui.selected_excerpt_index,
                    )?;
                    self.quit_pending_confirm = false;
                } else if self.is_requirements_editor_active() {
                    self.authoring_ui.insert_newline();
                    self.quit_pending_confirm = false;
                } else if self.step_input_active {
                    self.step_input_active = false;
                    self.focus_slot = BddFocusSlot::Body;
                    self.status = "Edit committed".to_string();
                    self.set_status_message("Edit committed".to_string());
                }
            }
            Action::Backspace => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.backspace();
                    self.quit_pending_confirm = false;
                } else if self.is_test_points_details_active() {
                    self.test_points_ui.backspace();
                    self.quit_pending_confirm = false;
                } else if !self.step_input_active {
                    return Ok(());
                } else {
                    if self.cursor_col <= self.step_input_min_col {
                        return Ok(());
                    }
                    self.push_undo();
                    let (row, col, changed) =
                        self.buffer.backspace(self.cursor_row, self.cursor_col);
                    self.cursor_row = row;
                    self.cursor_col = col;
                    self.desired_col = col;
                    if changed {
                        self.mark_current_buffer_dirty();
                        self.quit_pending_confirm = false;
                    }
                }
            }
            Action::Delete => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.delete_forward();
                    self.quit_pending_confirm = false;
                } else if self.is_test_points_details_active() {
                    self.test_points_ui.delete_forward();
                    self.quit_pending_confirm = false;
                } else if !self.step_input_active {
                    return Ok(());
                } else {
                    self.push_undo();
                    if self.buffer.delete(self.cursor_row, self.cursor_col) {
                        self.mark_current_buffer_dirty();
                        self.quit_pending_confirm = false;
                    }
                }
            }
            Action::ReqNewTestPoint => {
                if self.active_tab == MainTab::Requirements {
                    if let Some(id) = self.authoring_ui.create_test_point_from_selection() {
                        if let Some(artifacts) = self.authoring_ui.artifacts.as_ref() {
                            teshi_engine::save_test_points(
                                &self.project.root_dir,
                                &artifacts.test_points,
                            )
                            .with_context(|| "save test points after create")?;
                        }
                        self.status = format!("Created proposed test point {id}");
                    } else {
                        self.status =
                            "Select non-empty requirement text to create a test point".to_string();
                    }
                    self.quit_pending_confirm = false;
                }
            }
            Action::ReqNewDocument => {
                if self.active_tab == MainTab::Requirements {
                    if self.authoring_ui.guard_unsaved(
                        crate::authoring_tab::RequirementsPendingAction::Action(
                            Action::ReqNewDocument,
                        ),
                    ) {
                        return Ok(());
                    }
                    match self.authoring_ui.create_default_document() {
                        Ok(id) => {
                            self.status = format!("Created requirement document {id}.md");
                        }
                        Err(error) => {
                            self.status = format!("Could not create requirement: {error}");
                        }
                    }
                    self.quit_pending_confirm = false;
                }
            }
            Action::ReqDeleteDocument => {
                if self.active_tab == MainTab::Requirements {
                    if self.authoring_ui.guard_unsaved(
                        crate::authoring_tab::RequirementsPendingAction::Action(
                            Action::ReqDeleteDocument,
                        ),
                    ) {
                        return Ok(());
                    }
                    if let Err(error) = self.authoring_ui.open_delete_confirmation() {
                        self.status = error;
                    }
                    self.quit_pending_confirm = false;
                }
            }
            Action::ReqEnterInsert => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.enter_insert_mode();
                }
            }
            Action::ReqExitInsert => {
                if matches!(
                    self.authoring_ui.editor_mode,
                    crate::authoring_tab::RequirementsEditorMode::VisualChar
                        | crate::authoring_tab::RequirementsEditorMode::VisualLine
                ) {
                    self.authoring_ui.exit_visual_mode();
                } else {
                    self.authoring_ui.exit_insert_mode();
                }
                self.pending_char = None;
            }
            Action::ReqVimAppend => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.move_cursor_right();
                    self.authoring_ui.enter_insert_mode();
                }
            }
            Action::ReqVimOpenBelow => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.vim_open_line(false);
                }
            }
            Action::ReqVimOpenAbove => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.vim_open_line(true);
                }
            }
            Action::ReqVimDeleteChar => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.vim_delete_char();
                }
            }
            Action::ReqVimDeleteLine => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.vim_delete_line();
                    self.pending_char = None;
                }
            }
            Action::ReqVimYankLine => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.vim_yank_line();
                    self.pending_char = None;
                    self.status = "Yanked requirement line".to_string();
                }
            }
            Action::ReqVimPasteAfter => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.paste_register(false);
                }
            }
            Action::ReqVimPasteBefore => {
                if self.is_requirements_editor_active() {
                    self.authoring_ui.paste_register(true);
                }
            }
            Action::ReqVimWordForward => self.authoring_ui.vim_word_forward(),
            Action::ReqVimWordBackward => self.authoring_ui.vim_word_backward(),
            Action::ReqVimWordEnd => self.authoring_ui.vim_word_end(),
            Action::ReqVimLineStart => self.authoring_ui.vim_line_start(),
            Action::ReqVimFirstNonBlank => self.authoring_ui.vim_first_nonblank(),
            Action::ReqVimLineEnd => self.authoring_ui.vim_line_end(),
            Action::ReqVimGoTop => {
                self.authoring_ui.vim_go_top();
                self.pending_char = None;
            }
            Action::ReqVimGoBottom => self.authoring_ui.vim_go_bottom(),
            Action::ReqVimVisualChar => self.authoring_ui.enter_visual_char(),
            Action::ReqVimVisualLine => self.authoring_ui.enter_visual_line(),
            Action::ReqVimVisualYank => {
                if self.authoring_ui.yank_visual_selection() {
                    self.status = "Yanked requirement selection".to_string();
                }
                self.authoring_ui.exit_visual_mode();
                self.pending_char = None;
            }
            Action::ReqVimVisualDelete => {
                self.authoring_ui.delete_visual_selection();
                self.pending_char = None;
            }
            Action::ReqProjectScope => {
                self.status = match self.authoring_ui.cycle_project_filter() {
                    Ok(()) => format!(
                        "Requirement scope: {}",
                        self.authoring_ui.project_scope_label()
                    ),
                    Err(error) => error,
                };
            }
            Action::ReqProjectLink => {
                self.status = match self.authoring_ui.toggle_current_project_link() {
                    Ok(linked) => if linked {
                        "Linked requirement to current project"
                    } else {
                        "Unlinked requirement from current project"
                    }
                    .into(),
                    Err(error) => error.to_string(),
                };
            }
            Action::ReqFilterOverlay => {
                if self.active_tab == MainTab::Requirements {
                    self.authoring_ui.open_filter_picker();
                    self.quit_pending_confirm = false;
                }
            }
            Action::ReqGroupToggle => {
                if self.active_tab == MainTab::Requirements {
                    self.authoring_ui.toggle_group_mode();
                    self.status = match self.authoring_ui.group_mode {
                        teshi_core::authoring::RequirementGroupMode::Path => {
                            "Grouped by path".to_string()
                        }
                        teshi_core::authoring::RequirementGroupMode::Iteration => {
                            "Grouped by iteration".to_string()
                        }
                    };
                    self.quit_pending_confirm = false;
                }
            }
            Action::ReqEditIteration => {
                if self.active_tab == MainTab::Requirements {
                    if self.authoring_ui.selected_document_id.is_some() {
                        self.authoring_ui.open_iteration_editor();
                    } else {
                        self.status = "Select a requirement document to set its iteration".into();
                    }
                    self.quit_pending_confirm = false;
                }
            }
            Action::ReqEditPriority => {
                if self.active_tab == MainTab::Requirements {
                    if self.authoring_ui.selected_document_id.is_some() {
                        self.authoring_ui.open_priority_editor();
                    } else {
                        self.status = "Select a requirement document to set its priority".into();
                    }
                    self.quit_pending_confirm = false;
                }
            }
            Action::OverlayConfirm
            | Action::OverlayCancel
            | Action::OverlayMoveUp
            | Action::OverlayMoveDown
            | Action::OverlayInsert(_)
            | Action::OverlayBackspace
            | Action::GenerationScopeConfirm
            | Action::GenerationScopeCancel
            | Action::GenerationScopeChangeIteration => {}
            Action::TpEnterInsert => {
                if self.is_test_points_details_active()
                    && self.test_points_ui.selected_test_point_id.is_some()
                {
                    self.test_points_ui.editing = true;
                }
            }
            Action::TpExitInsert => {
                self.commit_test_point_field_edit()?;
                self.test_points_ui.editing = false;
            }
            Action::DeleteConfirm | Action::DeleteCancel => {}
            Action::TpApprove => {
                if self.active_tab == MainTab::TestPoints {
                    self.commit_test_point_field_edit()?;
                    let Some(tp_id) = self.test_points_ui.selected_test_point_id.clone() else {
                        self.status = "Select a test point to approve".to_string();
                        self.quit_pending_confirm = false;
                        return Ok(());
                    };
                    let mut approved = false;
                    if let Some(artifacts) = self.authoring_ui.artifacts.as_mut()
                        && let Some(tp) = artifacts
                            .test_points
                            .test_points
                            .iter_mut()
                            .find(|tp| tp.id == tp_id)
                    {
                        approved = crate::test_points_tab::TestPointsUiState::approve(tp);
                        if approved {
                            crate::test_points_tab::TestPointsUiState::save_test_points(
                                &self.project.root_dir,
                                artifacts,
                            )?;
                            self.test_points_ui.rebuild_tree(Some(artifacts));
                        }
                    }
                    self.status = if approved {
                        format!("Approved test point {tp_id}")
                    } else {
                        "Cannot approve: resolve stale links or review state".to_string()
                    };
                    self.quit_pending_confirm = false;
                }
            }
            Action::TpReject => {
                if self.active_tab == MainTab::TestPoints {
                    self.commit_test_point_field_edit()?;
                    let Some(tp_id) = self.test_points_ui.selected_test_point_id.clone() else {
                        self.status = "Select a test point to reject".to_string();
                        self.quit_pending_confirm = false;
                        return Ok(());
                    };
                    let mut rejected = false;
                    if let Some(artifacts) = self.authoring_ui.artifacts.as_mut()
                        && let Some(tp) = artifacts
                            .test_points
                            .test_points
                            .iter_mut()
                            .find(|tp| tp.id == tp_id)
                    {
                        rejected = crate::test_points_tab::TestPointsUiState::reject(tp);
                        if rejected {
                            crate::test_points_tab::TestPointsUiState::save_test_points(
                                &self.project.root_dir,
                                artifacts,
                            )?;
                            self.test_points_ui.rebuild_tree(Some(artifacts));
                        }
                    }
                    self.status = if rejected {
                        format!("Rejected test point {tp_id}")
                    } else {
                        "Cannot reject this test point".to_string()
                    };
                    self.quit_pending_confirm = false;
                }
            }
            Action::TpBatchApprove => {
                if self.active_tab == MainTab::TestPoints {
                    self.commit_test_point_field_edit()?;
                    let ids = self.test_points_ui.visible_leaf_ids.clone();
                    let count = if let Some(artifacts) = self.authoring_ui.artifacts.as_mut() {
                        let approved = crate::test_points_tab::TestPointsUiState::batch_approve(
                            &mut artifacts.test_points.test_points,
                            &ids,
                        );
                        if approved > 0 {
                            crate::test_points_tab::TestPointsUiState::save_test_points(
                                &self.project.root_dir,
                                artifacts,
                            )?;
                            self.test_points_ui.rebuild_tree(Some(artifacts));
                        }
                        approved
                    } else {
                        0
                    };
                    self.status = format!("Batch approved {count} test point(s)");
                    self.quit_pending_confirm = false;
                }
            }
            Action::TpReviewFilter => {
                if self.active_tab == MainTab::TestPoints {
                    self.test_points_ui.cycle_review_filter();
                    let artifacts = self.authoring_ui.artifacts.as_ref();
                    self.test_points_ui.rebuild_tree(artifacts);
                    self.reload_test_point_field_buffer();
                    self.status = "Cycled test-point review filter".to_string();
                    self.quit_pending_confirm = false;
                }
            }
            Action::TpFollowExcerpt => {
                if self.active_tab == MainTab::TestPoints {
                    self.navigate_to_requirement_excerpt(
                        self.test_points_ui.selected_excerpt_index,
                    )?;
                    self.quit_pending_confirm = false;
                }
            }
            Action::TpContinueGeneration => {
                self.continue_test_point_generation()?;
                self.quit_pending_confirm = false;
            }
            Action::TpFollowScenario => {
                if self.active_tab == MainTab::TestPoints {
                    self.navigate_to_test_point_scenario(
                        self.test_points_ui.selected_scenario_ref_index,
                    )?;
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
            Action::RefreshFeatures => {
                if let Some(catalog) = self.lazy_features.as_mut() {
                    catalog.request_refresh();
                }
                self.poll_lazy_features();
                self.last_external_check = Instant::now() - Duration::from_secs(2);
                self.status.clear();
                self.poll_external_feature_changes();
                let message = if self.status.starts_with("Feature refresh incomplete;") {
                    self.status.clone()
                } else {
                    "Feature refresh requested; active runs and pending edits defer synchronization"
                        .into()
                };
                self.set_status_message(message);
            }
            Action::ExplorePan(delta) => {
                let index = self.explore_focus.index();
                self.explore_horizontal_scroll[index] = self.explore_horizontal_scroll[index]
                    .saturating_add_signed(delta)
                    .min(self.explore_viewports[index].max_horizontal);
            }
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
            Action::AiNewline => {
                self.ai_input_focused = true;
                let byte_idx = char_to_byte_idx(&self.agent().input, self.agent().input_cursor);
                self.agent_mut().input.insert(byte_idx, '\n');
                self.agent_mut().input_cursor += 1;
                self.quit_pending_confirm = false;
            }
            Action::AiClipboardPaste => {
                self.ai_input_focused = true;
                if let Ok(mut cb) = arboard::Clipboard::new()
                    && let Ok(text) = cb.get_text()
                {
                    let text = text.replace("\r\n", "\n").replace('\r', "\n");
                    let byte_idx = char_to_byte_idx(&self.agent().input, self.agent().input_cursor);
                    self.agent_mut().input.insert_str(byte_idx, &text);
                    self.agent_mut().input_cursor += text.chars().count();
                }
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
            Action::AiContinueTask => {
                self.continue_interrupted_task()?;
            }
            Action::AiSendMessage => {
                if self.agent().input.trim().is_empty() {
                    self.ai_input_focused = false;
                    self.status = "Input deactivated".to_string();
                    return Ok(());
                }
                if self.agent().status == AiStatus::Waiting
                    && !self.agent().input.trim().starts_with('/')
                {
                    return Ok(());
                }

                let user_msg = std::mem::take(&mut self.agent_mut().input);
                self.agent_mut().input_cursor = 0;

                // Intercept slash commands before sending to LLM
                if let Some(cmd) = user_msg.strip_prefix('/') {
                    self.slash_suggestion_active = false;
                    self.slash_suggestion_selection = 0;
                    return self.execute_slash_command(cmd);
                }

                self.dispatch_user_message(user_msg)?;
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
            Action::AiCursorUp | Action::AiCursorDown => {
                let chars: Vec<char> = self.agent().input.chars().collect();
                let cursor = self.agent().input_cursor.min(chars.len());
                let start = chars[..cursor]
                    .iter()
                    .rposition(|c| *c == '\n')
                    .map_or(0, |i| i + 1);
                let column = cursor - start;
                let target = if matches!(action, Action::AiCursorUp) {
                    if start == 0 {
                        cursor
                    } else {
                        let end = start - 1;
                        let previous = chars[..end]
                            .iter()
                            .rposition(|c| *c == '\n')
                            .map_or(0, |i| i + 1);
                        previous + column.min(end - previous)
                    }
                } else if let Some(end) = chars[cursor..].iter().position(|c| *c == '\n') {
                    let next = cursor + end + 1;
                    let length = chars[next..]
                        .iter()
                        .position(|c| *c == '\n')
                        .unwrap_or(chars.len() - next);
                    next + column.min(length)
                } else {
                    cursor
                };
                self.agent_mut().input_cursor = target;
                self.quit_pending_confirm = false;
            }
            Action::AiCursorHome => {
                let cursor = self.agent().input_cursor;
                self.agent_mut().input_cursor = self
                    .agent()
                    .input
                    .chars()
                    .take(cursor)
                    .enumerate()
                    .filter(|(_, c)| *c == '\n')
                    .map(|(i, _)| i + 1)
                    .last()
                    .unwrap_or(0);
                self.quit_pending_confirm = false;
            }
            Action::AiCursorEnd => {
                let cursor = self.agent().input_cursor;
                self.agent_mut().input_cursor = cursor
                    + self
                        .agent()
                        .input
                        .chars()
                        .skip(cursor)
                        .take_while(|c| *c != '\n')
                        .count();
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
            Action::AiSlashChoose(index) => {
                let filter = self
                    .agent()
                    .input
                    .strip_prefix('/')
                    .unwrap_or("")
                    .to_lowercase();
                let count = SLASH_COMMANDS
                    .iter()
                    .filter(|(name, _)| filter.is_empty() || name.starts_with(&filter))
                    .count();
                if index < count {
                    self.slash_suggestion_selection = index;
                }
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
                    // Clear the input (cmd is about to execute); "copy" will override below
                    self.agent_mut().input.clear();
                    self.agent_mut().input_cursor = 0;
                    // Execute the command through the same dispatcher as submitted commands.
                    return if *name == "copy" {
                        self.agent_mut().input = "/copy ".into();
                        self.agent_mut().input_cursor = self.agent().input.len();
                        Ok(())
                    } else {
                        self.execute_slash_command(name)
                    };
                }
                // No suggestion matched the typed text (an argument-bearing or
                // unknown command such as `/copy 3`); submit it to the same
                // dispatcher as a submitted command.
                if let Some(command) = self.agent().input.strip_prefix('/').map(str::to_owned) {
                    self.agent_mut().input.clear();
                    self.agent_mut().input_cursor = 0;
                    self.slash_suggestion_active = false;
                    return self.execute_slash_command(&command);
                }
            }
            Action::AiSlashDismiss => {
                self.slash_suggestion_active = false;
                self.slash_suggestion_selection = 0;
                self.quit_pending_confirm = false;
            }
            Action::AcpPermissionUp => {
                self.move_acp_permission_selection(-1);
            }
            Action::AcpPermissionChoose(index) => {
                if let Some(prompt) = self.agent_mut().pending_acp_permission.as_mut()
                    && index < prompt.options.len()
                {
                    prompt.selected = index;
                }
            }
            Action::AcpPermissionDown => {
                self.move_acp_permission_selection(1);
            }
            Action::AcpPermissionConfirm => {
                let option_id = self
                    .agent()
                    .pending_acp_permission
                    .as_ref()
                    .and_then(|prompt| prompt.options.get(prompt.selected))
                    .map(|option| option.id.clone());
                if let Some(option_id) = option_id {
                    self.respond_acp_permission(Some(option_id));
                }
            }
            Action::AcpPermissionReject => {
                self.respond_acp_permission(None);
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
                    self.cancel_agent_turn();
                    // Reset agent state
                    let agent = self.agent_mut();
                    agent.status = AiStatus::Idle;
                    agent.partial_response_mut().clear();
                    agent.tool_status = None;
                    // Clear any tool call state by sending a cancellation message
                    agent.messages_mut().push(AiChatMessage {
                        role: AiRole::Assistant,
                        content: "[cancelled]".to_string(),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                        source: Some("agent_progress".into()),
                    });
                    self.status = "AI request cancelled".to_string();
                }
                self.selection_anchor = None;
                self.selection_end = None;
                self.clear_chat_selection();
                self.quit_pending_confirm = false;
            }
            Action::AiFocusInput => {
                self.ai_input_focused = true;
            }
            Action::AgentNew => {
                self.clear_chat_selection();
                let id = self.next_agent_id;
                self.next_agent_id += 1;
                let root = self.agent().selection_root.clone();
                self.agents.push(AgentThread::new_with_selection(
                    id,
                    &format!("Agent {id}"),
                    root,
                ));
                self.selected_agent = self.agents.len() - 1;
                self.ai_input_focused = true;
                self.spawn_llm_if_configured();
                self.status = format!("Created agent {id}");
                self.show_selection_warning();
                self.quit_pending_confirm = false;
            }
            Action::AgentClose => {
                self.clear_chat_selection();
                if self.agents.len() <= 1 {
                    self.status = "Cannot close the last agent".to_string();
                } else {
                    self.save_sessions(true)?;
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
                self.clear_chat_selection();
                if self.agents.len() > 1 {
                    self.selected_agent = (self.selected_agent + 1) % self.agents.len();
                    self.status = format!("Switched to {}", self.agent().title);
                }
                self.quit_pending_confirm = false;
            }
            Action::AgentPrev => {
                self.clear_chat_selection();
                if self.agents.len() > 1 {
                    self.selected_agent = self.selected_agent.saturating_sub(1);
                    self.status = format!("Switched to {}", self.agent().title);
                }
                self.quit_pending_confirm = false;
            }
            Action::AgentSelect(index) => {
                if index < self.agents.len() {
                    self.clear_chat_selection();
                    self.selected_agent = index;
                    self.status = format!("Switched to {}", self.agent().title);
                }
                self.quit_pending_confirm = false;
            }
            Action::ExternalChangeReload | Action::ExternalChangeKeepLocal => {}
            Action::CopySelection => self.copy_selection_to_clipboard()?,
            Action::ClearSelection => {
                self.selection_anchor = None;
                self.selection_end = None;
                self.clear_chat_selection();
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
            Action::CancelPaneNavigation => {
                self.pending_char = None;
                self.quit_pending_confirm = false;
            }
            Action::ClearInputState => {
                self.selection_anchor = None;
                self.selection_end = None;
                self.clear_chat_selection();
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
                        self.cancel_agent_turn();
                        let agent = self.agent_mut();
                        agent.partial_response_mut().clear();
                        agent.tool_status = None;
                        agent.messages_mut().push(AiChatMessage {
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
                    self.agent_mut().partial_response_mut().clear();
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
            // Agent panel actions are handled in the intercept above — unreachable here.
            Action::AgentPanelUp
            | Action::AgentPanelDown
            | Action::AgentPanelSelect
            | Action::AgentPanelClose
            | Action::AgentPanelAdd
            | Action::AgentPanelEdit
            | Action::AgentPanelDelete
            | Action::AgentPanelFormNext
            | Action::AgentPanelFormPrev
            | Action::AgentPanelFormTabPrev
            | Action::AgentPanelFormTabNext
            | Action::AgentPanelFormInsert(_)
            | Action::AgentPanelFormBackspace
            | Action::AgentPanelFormSubmit
            | Action::AgentPanelFormCancel => {}
        }
        self.clamp_cursor();
        Ok(())
    }

    fn resolve_unsaved_requirement(
        &mut self,
        action: Action,
        pending: crate::authoring_tab::RequirementsPendingAction,
    ) -> Result<()> {
        use crate::authoring_tab::RequirementsPendingAction;
        match action {
            Action::OverlayCancel => {
                self.authoring_ui.cancel_overlay();
                return Ok(());
            }
            Action::OverlayInsert('s' | 'S') | Action::Save => {
                if let Err(error) = self
                    .authoring_ui
                    .save_current_document(&self.project.root_dir)
                {
                    self.status = format!("Could not save requirement: {error}");
                    self.set_status_message(self.status.clone());
                    return Ok(());
                }
            }
            Action::OverlayInsert('d' | 'D') => {
                self.authoring_ui.discard_current_document();
            }
            _ => return Ok(()),
        }
        self.authoring_ui.cancel_overlay();
        match pending {
            RequirementsPendingAction::Document(id) => {
                self.authoring_ui.select_tree_node(&format!("req-doc:{id}"));
            }
            RequirementsPendingAction::ProjectFilter(filter) => {
                self.authoring_ui.set_project_filter(filter);
            }
            RequirementsPendingAction::Filter(filter) => {
                if let Err(error) = self.authoring_ui.try_set_iteration_filter(filter) {
                    self.status = error;
                }
            }
            RequirementsPendingAction::Iteration { document_id, value } => {
                if self.authoring_ui.selected_document_id.as_deref() == Some(&document_id)
                    && let Err(error) = self.authoring_ui.save_current_iteration(&value)
                {
                    self.status = error;
                }
            }
            RequirementsPendingAction::Action(action) => return self.handle_action(action),
        }
        Ok(())
    }

    fn save(&mut self) -> Result<()> {
        if self.active_tab == MainTab::Requirements {
            if self.authoring_ui.buffer_dirty {
                match self
                    .authoring_ui
                    .save_current_document(&self.project.root_dir)
                {
                    Ok(()) => self.status = "Saved requirement document".to_string(),
                    Err(error) => self.status = format!("Could not save requirement: {error}"),
                }
                self.set_status_message(self.status.clone());
            }
            return Ok(());
        }
        if self.active_tab == MainTab::TestPoints {
            self.commit_test_point_field_edit()?;
            if let Some(artifacts) = self.authoring_ui.artifacts.as_ref() {
                crate::test_points_tab::TestPointsUiState::save_test_points(
                    &self.project.root_dir,
                    artifacts,
                )?;
                self.status = "Saved test points".to_string();
            }
            return Ok(());
        }
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
        if self
            .authoring_ui
            .guard_unsaved(crate::authoring_tab::RequirementsPendingAction::Action(
                Action::Quit,
            ))
        {
            return;
        }
        if self.dirty {
            if !self.quit_pending_confirm {
                self.quit_pending_confirm = true;
                return;
            }
            self.should_quit = true;
        } else {
            self.should_quit = true;
        }
    }

    fn clamp_cursor(&mut self) {
        if self.is_requirements_editor_active() {
            self.authoring_ui.clamp_cursor();
            return;
        }
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

    /// Move focus through the panes visible in the active tab.
    fn focus_visible_pane(&mut self, next: bool) {
        match self.active_tab {
            MainTab::Requirements => {
                if next {
                    self.authoring_ui.focus_next_column();
                } else {
                    self.authoring_ui.focus_prev_column();
                }
            }
            MainTab::TestPoints => {
                if next {
                    self.test_points_ui.focus_next_column();
                } else {
                    self.test_points_ui.focus_prev_column();
                }
                self.reload_test_point_field_buffer();
            }
            MainTab::Explore if !self.explore_edit_mode => {
                if next {
                    self.explore_focus_next();
                } else {
                    self.explore_focus_prev();
                }
            }
            MainTab::MindMap if self.mindmap_ai_panel_visible => {
                if next && self.mindmap_focus == MindMapFocus::Main {
                    self.mindmap_focus = MindMapFocus::AiPanel;
                    self.ai_input_focused = false;
                } else if !next && self.mindmap_focus == MindMapFocus::AiPanel {
                    self.mindmap_focus = MindMapFocus::Main;
                    self.ai_input_focused = false;
                }
            }
            MainTab::Ai | MainTab::MindMap | MainTab::Explore => {}
        }
        self.quit_pending_confirm = false;
    }

    fn select_tab(&mut self, tab: MainTab) {
        if self.active_tab == tab {
            return;
        }
        if self.active_tab == MainTab::TestPoints
            && let Err(error) = self.commit_test_point_field_edit()
        {
            self.status = format!("Could not save test point: {error}");
            return;
        }
        self.test_points_ui.editing = false;
        self.clear_chat_selection();
        if self.active_tab == MainTab::Requirements
            && self.authoring_ui.guard_unsaved(
                crate::authoring_tab::RequirementsPendingAction::Action(Action::SelectTab(tab)),
            )
        {
            return;
        }
        self.authoring_ui.exit_insert_mode();
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
            if self.lazy_features.is_some() {
                self.rebuild_project_views(None);
            }
            self.refresh_capture_index();
            self.view_stage = ViewStage::TreeOnly;
            self.mindmap_focus = MindMapFocus::Main;
            self.ai_input_focused = false;
        }
        if self.active_tab == MainTab::Ai {
            self.ai_input_focused = false;
        }
        if self.active_tab == MainTab::Explore {
            // Captures may have been recorded by another process since the last
            // look; the Steps and Scenarios markers refresh with the tab.
            self.refresh_capture_index();
        }
        if self.active_tab == MainTab::TestPoints {
            let artifacts = self.authoring_ui.artifacts.as_ref();
            self.test_points_ui.rebuild_tree(artifacts);
            if let (Some(id), Some(artifacts)) = (
                self.test_points_ui.selected_test_point_id.as_ref(),
                artifacts,
            ) && let Some(tp) = artifacts
                .test_points
                .test_points
                .iter()
                .find(|tp| &tp.id == id)
            {
                self.test_points_ui.load_field_buffer_from(tp);
            }
        }
        self.status = match tab {
            MainTab::MindMap => "Switched to MindMap tab",
            MainTab::Explore => "Switched to Explore tab",
            MainTab::Ai => "Switched to AI tab",
            MainTab::Requirements => "Switched to Requirements tab",
            MainTab::TestPoints => "Switched to Test Points tab",
        }
        .to_string();
    }

    fn is_requirements_editor_active(&self) -> bool {
        self.active_tab == MainTab::Requirements
            && self.authoring_ui.focus == crate::authoring_tab::RequirementsFocus::Editor
    }

    fn is_test_points_details_active(&self) -> bool {
        self.active_tab == MainTab::TestPoints
            && self.test_points_ui.focus == crate::test_points_tab::TestPointsFocus::Details
    }

    fn model_form_field(&mut self) -> &mut String {
        match self.model_form_focus {
            0 => &mut self.model_form_name,
            1 => &mut self.model_form_provider,
            2 => &mut self.model_form_model,
            3 => &mut self.model_form_base_url,
            4 => &mut self.model_form_api_key,
            5 => &mut self.model_form_max_tokens,
            _ => &mut self.model_form_temperature,
        }
    }

    fn navigate_modal_list(&mut self, action: &Action) -> bool {
        if !matches!(
            action,
            Action::NavigateFirst
                | Action::NavigateLast
                | Action::PageUp
                | Action::PageDown
                | Action::PendingChar('g')
        ) {
            return false;
        }
        let filter_count = self.authoring_ui.filter_picker_options().len();
        let keyword_count = self.buffer.language().all_step_keywords().len();
        let list = if let Some(prompt) = self.agent_mut().pending_acp_permission.as_mut() {
            Some((&mut prompt.selected, prompt.options.len()))
        } else {
            None
        };
        // End the agent borrow before inspecting other independent overlays.
        if let Some((selection, count)) = list {
            apply_list_navigation(selection, count, action);
        } else if let crate::authoring_tab::RequirementsOverlay::FilterPicker { selection } =
            &mut self.authoring_ui.overlay
        {
            apply_list_navigation(selection, filter_count, action);
        } else if let crate::authoring_tab::RequirementsOverlay::PriorityEdit {
            selection, ..
        } = &mut self.authoring_ui.overlay
        {
            apply_list_navigation(selection, 5, action);
        } else if self.agent_profile_panel_active {
            let count = match self.agent_panel_mode {
                AgentPanelMode::List => self.agent_registry.len(),
                AgentPanelMode::SelectBackend => self.installed_acp_agents.len() + 1,
                AgentPanelMode::InstalledAcp => self.installed_acp_agents.len(),
                AgentPanelMode::Registry => self.acp_registry_agents.len(),
            };
            apply_list_navigation(&mut self.agent_profile_panel_selection, count, action);
        } else if self.model_panel_active && self.model_panel_mode == ModelPanelMode::List {
            apply_list_navigation(
                &mut self.model_panel_selection,
                self.model_profiles.len(),
                action,
            );
        } else if self.session_panel_active {
            apply_list_navigation(
                &mut self.session_panel_selection,
                self.session_list.len(),
                action,
            );
        } else if self.approval_panel_active {
            apply_list_navigation(
                &mut self.approval_panel_selection,
                teshi_agent::approval::ApprovalMode::ALL.len(),
                action,
            );
        } else if self.change_summary_visible {
            apply_list_navigation(
                &mut self.change_summary_selection,
                self.pending_change_summary.len(),
                action,
            );
        } else if let Some(picker) = self.step_keyword_picker.as_mut() {
            apply_list_navigation(&mut picker.selected, keyword_count, action);
        } else if self.scenario_dropdown_open {
            let count = mindmap::selected_node_id(&self.tree_state)
                .and_then(|id| self.mindmap_index.locations_for(id))
                .map_or(0, |locations| locations.len());
            apply_list_navigation(&mut self.scenario_dropdown_selection, count, action);
        } else {
            return false;
        }
        if let Action::PendingChar(ch) = action {
            self.pending_char = Some(*ch);
        }
        true
    }

    fn model_form_delete_at_cursor(&mut self) {
        let cursor = self.model_form_cursor;
        let field = self.model_form_field();
        if let Some((byte, _)) = field.char_indices().nth(cursor) {
            field.remove(byte);
        }
    }

    fn navigate_page(&mut self, down: bool) -> Result<()> {
        if self.chat_browse_active() {
            self.handle_action(if down {
                Action::AiScrollDown
            } else {
                Action::AiScrollUp
            })?;
        } else if self.active_tab == MainTab::MindMap && self.mindmap_focus == MindMapFocus::Main {
            for _ in 0..10 {
                if down {
                    self.tree_move_down();
                } else {
                    self.tree_move_up();
                }
            }
        } else if self.active_tab == MainTab::Requirements
            || self.active_tab == MainTab::TestPoints
            || (self.active_tab == MainTab::Explore && !self.explore_edit_mode)
        {
            for _ in 0..10 {
                self.handle_action(if down {
                    Action::MoveDown
                } else {
                    Action::MoveUp
                })?;
            }
        } else if down {
            self.page_down();
        } else {
            self.page_up();
        }
        Ok(())
    }

    fn chat_browse_active(&self) -> bool {
        !self.ai_input_focused
            && (self.active_tab == MainTab::Ai
                || (self.active_tab == MainTab::MindMap
                    && self.mindmap_focus == MindMapFocus::AiPanel))
    }

    fn navigate_boundary(&mut self, end: bool) -> Result<()> {
        let delta = if end { isize::MAX } else { -isize::MAX };
        if self.chat_browse_active() {
            self.handle_action(if end {
                Action::AiScrollBottom
            } else {
                Action::AiScrollTop
            })?;
        } else if self.active_tab == MainTab::Requirements {
            match self.authoring_ui.focus {
                crate::authoring_tab::RequirementsFocus::Tree => {
                    self.authoring_ui.move_tree_selection(delta)
                }
                crate::authoring_tab::RequirementsFocus::LinkedTestPoints => {
                    self.authoring_ui.move_linked_selection(delta)
                }
                crate::authoring_tab::RequirementsFocus::Editor => {
                    if end {
                        self.authoring_ui.vim_go_bottom();
                    } else {
                        self.authoring_ui.vim_go_top();
                    }
                }
            }
        } else if self.active_tab == MainTab::TestPoints {
            self.commit_test_point_field_edit()?;
            match self.test_points_ui.focus {
                crate::test_points_tab::TestPointsFocus::Tree => {
                    self.test_points_ui.move_tree_selection(delta)
                }
                crate::test_points_tab::TestPointsFocus::Details => {
                    self.test_points_ui.move_detail_field(delta)
                }
                crate::test_points_tab::TestPointsFocus::Excerpts => {
                    let count = self.current_excerpt_count();
                    self.test_points_ui.move_excerpt_selection(delta, count);
                }
            }
            self.reload_test_point_field_buffer();
        } else if self.active_tab == MainTab::MindMap && self.mindmap_focus == MindMapFocus::Main {
            if end {
                self.tree_end();
            } else {
                self.tree_home();
            }
        } else {
            self.handle_action(if end {
                Action::MoveEnd
            } else {
                Action::MoveHome
            })?;
        }
        Ok(())
    }

    fn commit_test_point_field_edit(&mut self) -> Result<()> {
        if !self.test_points_ui.field_dirty {
            return Ok(());
        }
        let Some(tp_id) = self.test_points_ui.selected_test_point_id.clone() else {
            self.test_points_ui.field_dirty = false;
            return Ok(());
        };
        let value = self.test_points_ui.field_buffer.clone();
        let field = self.test_points_ui.detail_field;
        if let Some(artifacts) = self.authoring_ui.artifacts.as_mut()
            && let Some(tp) = artifacts
                .test_points
                .test_points
                .iter_mut()
                .find(|tp| tp.id == tp_id)
        {
            crate::test_points_tab::TestPointsUiState::apply_field_buffer(tp, field, &value);
            crate::test_points_tab::TestPointsUiState::save_test_points(
                &self.project.root_dir,
                artifacts,
            )?;
            self.test_points_ui.rebuild_tree(Some(artifacts));
        }
        self.test_points_ui.field_dirty = false;
        Ok(())
    }

    fn navigate_to_requirement_excerpt(&mut self, excerpt_index: usize) -> Result<()> {
        let (tp_id, excerpt) = {
            let artifacts = self
                .authoring_ui
                .artifacts
                .as_ref()
                .context("authoring artifacts not loaded")?;
            let tp_id = self
                .test_points_ui
                .selected_test_point_id
                .clone()
                .context("no test point selected")?;
            let tp = artifacts
                .test_points
                .test_points
                .iter()
                .find(|tp| tp.id == tp_id)
                .context("selected test point missing")?;
            let excerpts = crate::test_points_tab::excerpts_for_test_point(tp, artifacts);
            let excerpt = excerpts
                .get(excerpt_index)
                .context("requirement excerpt not found")?
                .clone();
            if excerpt.resolution != teshi_core::authoring::ResolutionState::Resolved {
                self.status = "Requirement link is stale — resolve in Requirements tab".to_string();
                return Ok(());
            }
            (tp_id, excerpt)
        };

        self.active_tab = MainTab::Requirements;
        self.authoring_ui
            .select_document_by_id(&excerpt.document_id);
        self.authoring_ui.highlight_test_point_id = Some(tp_id);
        self.authoring_ui.focus = crate::authoring_tab::RequirementsFocus::Editor;

        if let Some(artifacts) = self.authoring_ui.artifacts.as_ref()
            && let Some(doc) = artifacts
                .documents
                .iter()
                .find(|d| d.meta.id == excerpt.document_id)
            && let (Some((start_row, start_col)), Some((end_row, end_col))) = (
                teshi_core::authoring::char_position_to_line_col(
                    doc.body.as_str(),
                    excerpt.position.start,
                ),
                teshi_core::authoring::char_position_to_line_col(
                    doc.body.as_str(),
                    excerpt.position.end,
                ),
            )
        {
            self.authoring_ui.selection_anchor = Some((start_row, start_col));
            self.authoring_ui.selection_end = Some((end_row, end_col));
            self.authoring_ui.scroll_row = start_row.saturating_sub(2);
            self.authoring_ui.cursor_row = start_row;
            self.authoring_ui.cursor_col = start_col;
        }
        self.status = format!("Opened requirement excerpt in {}", excerpt.document_title);
        Ok(())
    }

    /// Opens Explore at a scenario referenced by the selected test point.
    fn navigate_to_test_point_scenario(&mut self, scenario_ref_index: usize) -> Result<()> {
        let sc_ref = {
            let artifacts = self
                .authoring_ui
                .artifacts
                .as_ref()
                .context("authoring artifacts not loaded")?;
            let tp_id = self
                .test_points_ui
                .selected_test_point_id
                .clone()
                .context("no test point selected")?;
            let tp = artifacts
                .test_points
                .test_points
                .iter()
                .find(|tp| tp.id == tp_id)
                .context("selected test point missing")?;
            tp.scenario_refs
                .get(scenario_ref_index)
                .cloned()
                .context("no realized scenario for this test point")?
        };

        let feature_idx = self
            .find_feature_idx_for_file(&sc_ref.feature_path)
            .with_context(|| format!("feature '{}' not found", sc_ref.feature_path))?;
        self.active_tab = MainTab::Explore;
        self.explore_set_feature(feature_idx);

        if let Some(name) = sc_ref.scenario_name.as_deref()
            && let Some(scenario_idx) = self
                .project
                .features
                .get(feature_idx)
                .and_then(|f| f.all_scenarios().into_iter().position(|s| s.name == name))
        {
            self.explore_set_scenario(scenario_idx);
        } else if let Some(line) = sc_ref.scenario_line
            && let Some(scenario_idx) = self.project.features.get(feature_idx).and_then(|f| {
                f.all_scenarios()
                    .into_iter()
                    .position(|s| s.line_number == line)
            })
        {
            self.explore_set_scenario(scenario_idx);
        }

        self.status = format!(
            "Opened scenario {} in {}",
            sc_ref.scenario_name.as_deref().unwrap_or("(unnamed)"),
            sc_ref.feature_path
        );
        Ok(())
    }

    fn reload_test_point_field_buffer(&mut self) {
        let Some(tp_id) = self.test_points_ui.selected_test_point_id.clone() else {
            self.test_points_ui.load_field_buffer();
            return;
        };
        if let Some(tp) = self
            .authoring_ui
            .artifacts
            .as_ref()
            .and_then(|a| a.test_points.test_points.iter().find(|tp| tp.id == tp_id))
        {
            self.test_points_ui.load_field_buffer_from(tp);
        } else {
            self.test_points_ui.load_field_buffer();
        }
    }

    fn current_excerpt_count(&self) -> usize {
        let Some(tp_id) = self.test_points_ui.selected_test_point_id.as_ref() else {
            return 0;
        };
        let Some(artifacts) = self.authoring_ui.artifacts.as_ref() else {
            return 0;
        };
        let Some(tp) = artifacts
            .test_points
            .test_points
            .iter()
            .find(|tp| &tp.id == tp_id)
        else {
            return 0;
        };
        crate::test_points_tab::excerpts_for_test_point(tp, artifacts).len()
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
        for (scenario_idx, scenario) in feature.all_scenarios().into_iter().enumerate() {
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
                self.set_status_message("Editing active".to_string());
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

    fn clear_chat_selection(&mut self) {
        self.chat_selection = None;
        self.chat_selection_dragging = false;
    }

    fn try_copy_to_clipboard(&mut self, text: &str) -> bool {
        match set_system_clipboard(text) {
            Ok(clipboard) => {
                // Acquire the new owner before dropping the previous handle.
                // A failed write must preserve the existing owner and selection.
                self.system_clipboard = Some(clipboard);
                true
            }
            Err(error) => {
                self.status = format!("Could not copy to clipboard: {error}");
                self.set_status_message(self.status.clone());
                false
            }
        }
    }

    fn copy_selection_to_clipboard(&mut self) -> Result<()> {
        if self.chat_selection.is_some()
            && (self.active_tab == MainTab::Ai
                || (self.active_tab == MainTab::MindMap
                    && self.mindmap_focus == MindMapFocus::AiPanel))
        {
            let Some(text) = crate::ui::chat::selected_chat_text(self) else {
                self.status = "Chat selection is no longer available".to_string();
                self.set_status_message(self.status.clone());
                self.clear_chat_selection();
                return Ok(());
            };
            if text.is_empty() {
                self.status = "No chat selection to copy".to_string();
                self.set_status_message(self.status.clone());
                return Ok(());
            }
            if !self.try_copy_to_clipboard(&text) {
                return Ok(());
            }
            self.status = format!("Copied {} character(s)", text.chars().count());
            self.set_status_message(self.status.clone());
            self.clear_chat_selection();
            return Ok(());
        }
        let text = self.selection_text();
        if text.is_empty() {
            self.status = "No selection to copy".to_string();
            self.set_status_message(self.status.clone());
            return Ok(());
        }
        if !self.try_copy_to_clipboard(&text) {
            return Ok(());
        }
        let line_count = text.lines().count();
        self.status = format!("Copied {line_count} line(s)");
        self.set_status_message(self.status.clone());
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
        // Pane and modal handlers may consume motion; update hover coordinates
        // first so moving away from a control always clears its feedback.
        if matches!(kind, MouseEventKind::Moved) {
            self.mouse_position = Some((col, row));
        }
        if !matches!(
            self.project_switch.overlay,
            crate::project_context::SwitchOverlay::None
        ) {
            let pos = ratatui::layout::Position::new(col, row);
            if matches!(kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(region) = self.hit_test(&pos).cloned()
                && matches!(
                    self.project_switch.overlay,
                    crate::project_context::SwitchOverlay::Picker { .. }
                )
                && matches!(
                    region,
                    ClickableRegion::ProjectRecent { .. } | ClickableRegion::ProjectOpenPath { .. }
                )
            {
                self.handle_region_click(&region, &pos)?;
            }
            return Ok(());
        }
        if self.authoring_ui.overlay_active() {
            let pos = ratatui::layout::Position::new(col, row);
            if matches!(kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(region) = self.hit_test(&pos).cloned()
                && matches!(
                    region,
                    ClickableRegion::ActionButton { modal: true, .. }
                        | ClickableRegion::RequirementFilterOption { .. }
                )
            {
                self.handle_region_click(&region, &pos)?;
            }
            return Ok(());
        }
        if self.pending_delete.is_some()
            || self.model_panel_active
            || self.session_panel_active
            || self.auth_panel_active
            || self.approval_panel_active
            || self.agent_profile_panel_active
            || self.quit_pending_confirm
            || self.change_summary_visible
            || self.generation_scope_prompt.is_some()
            || self.scenario_dropdown_open
            || self.has_external_change_prompt()
            || self.step_keyword_picker.is_some()
            || self.has_agent_change_prompt()
            || self.has_acp_permission_prompt()
        {
            let pos = ratatui::layout::Position::new(col, row);
            if matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
                && let Some(action) =
                    self.clickable_regions
                        .iter()
                        .rev()
                        .find_map(|region| match region {
                            ClickableRegion::ModalList { rect, up, down } if rect.contains(pos) => {
                                Some(if kind == MouseEventKind::ScrollUp {
                                    up.clone()
                                } else {
                                    down.clone()
                                })
                            }
                            _ => None,
                        })
            {
                self.handle_action(action)?;
            }
            if matches!(kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(region) = self.hit_test(&pos).cloned()
                && match &region {
                    ClickableRegion::ActionButton { modal, .. } => *modal,
                    ClickableRegion::ModelProfileRow { .. } => self.model_panel_active,
                    ClickableRegion::ModelFormField { .. } => self.model_panel_active,
                    ClickableRegion::AgentProfileRow { .. } => self.agent_profile_panel_active,
                    ClickableRegion::ChangeSummaryRow { .. } => self.change_summary_visible,
                    ClickableRegion::StepKeywordRow { .. } => self.step_keyword_picker.is_some(),
                    ClickableRegion::SessionRow { .. } => self.session_panel_active,
                    ClickableRegion::MindMapLocation { .. } => self.scenario_dropdown_open,
                    ClickableRegion::ApprovalOption { .. } => self.approval_panel_active,
                    ClickableRegion::QuitConfirmYes { .. }
                    | ClickableRegion::QuitConfirmNo { .. } => self.quit_pending_confirm,
                    _ => false,
                }
            {
                self.handle_region_click(&region, &pos)?;
            }
            return Ok(());
        }
        if self.handle_acp_controls_mouse(kind, col, row) {
            return Ok(());
        }
        // Update hover tracking
        let pos = ratatui::layout::Position::new(col, row);

        if self.active_tab == MainTab::Explore
            && !self.explore_edit_mode
            && !self.explore_detail_open
            && matches!(
                kind,
                MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollLeft
                    | MouseEventKind::ScrollRight
            )
            && let Some(column) = self
                .explore_viewports
                .iter()
                .position(|view| view.rect.contains(pos))
        {
            let backwards = matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft);
            let horizontal = modifiers.contains(KeyModifiers::SHIFT)
                || matches!(
                    kind,
                    MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
                );
            let view = &mut self.explore_viewports[column];
            let (offset, maximum) = if horizontal {
                (
                    &mut self.explore_horizontal_scroll[column],
                    view.max_horizontal,
                )
            } else {
                (&mut view.offset, view.max_vertical)
            };
            *offset = if backwards {
                offset.saturating_sub(3)
            } else {
                offset.saturating_add(3).min(maximum)
            };
            self.quit_pending_confirm = false;
            return Ok(());
        }

        if self.active_tab == MainTab::Requirements {
            // Header metadata controls share a pane's border; dispatch them
            // before pane focus/scroll handling consumes the pointer event.
            if matches!(kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(region @ ClickableRegion::ActionButton { .. }) =
                    self.hit_test(&pos).cloned()
            {
                return self.handle_region_click(&region, &pos);
            }
            use crate::authoring_tab::RequirementsFocus;
            let ui = &mut self.authoring_ui;
            let pane = ui.pane_areas.iter().position(|area| area.contains(pos));
            if matches!(kind, MouseEventKind::Up(MouseButton::Left)) {
                ui.selection_dragging = false;
                ui.scrollbar_drag = None;
            }
            let dragging = matches!(kind, MouseEventKind::Drag(MouseButton::Left));
            if dragging && let Some((pane, axis)) = ui.scrollbar_drag {
                let area = ui.pane_areas[pane];
                let (position, start, length) = if axis == 0 {
                    (col, area.x + 1, area.width.saturating_sub(2))
                } else {
                    (row, area.y + 1, area.height.saturating_sub(2))
                };
                let value = position.saturating_sub(start).min(length.saturating_sub(1)) as usize
                    * ui.scroll_limits[pane][axis]
                    / length.saturating_sub(1).max(1) as usize;
                ui.set_pane_scroll(pane, axis, value);
                return Ok(());
            }
            if let Some(pane) = pane {
                let area = ui.pane_areas[pane];
                if matches!(kind, MouseEventKind::Down(MouseButton::Left)) {
                    let axis = if row == area.bottom().saturating_sub(1)
                        && ui.scroll_limits[pane][0] > 0
                    {
                        Some(0)
                    } else if col == area.right().saturating_sub(1) && ui.scroll_limits[pane][1] > 0
                    {
                        Some(1)
                    } else {
                        None
                    };
                    if let Some(axis) = axis {
                        let (position, start, length) = if axis == 0 {
                            (col, area.x + 1, area.width.saturating_sub(2))
                        } else {
                            (row, area.y + 1, area.height.saturating_sub(2))
                        };
                        let value = position.saturating_sub(start).min(length.saturating_sub(1))
                            as usize
                            * ui.scroll_limits[pane][axis]
                            / length.saturating_sub(1).max(1) as usize;
                        ui.set_pane_scroll(pane, axis, value);
                        ui.scrollbar_drag = Some((pane, axis));
                        return Ok(());
                    }
                }
                if matches!(
                    kind,
                    MouseEventKind::ScrollUp
                        | MouseEventKind::ScrollDown
                        | MouseEventKind::ScrollLeft
                        | MouseEventKind::ScrollRight
                ) {
                    let axis = usize::from(
                        !modifiers.contains(KeyModifiers::SHIFT)
                            && !matches!(
                                kind,
                                MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
                            ),
                    );
                    let old = if axis == 0 {
                        ui.horizontal_scroll[pane]
                    } else {
                        match pane {
                            0 => ui.tree_state.get_offset(),
                            1 => ui.scroll_row,
                            _ => ui.linked_scroll,
                        }
                    };
                    let value =
                        if matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft) {
                            old.saturating_sub(3)
                        } else {
                            old.saturating_add(3)
                        };
                    ui.set_pane_scroll(pane, axis, value);
                    return Ok(());
                }
                if matches!(kind, MouseEventKind::Down(MouseButton::Left)) {
                    let focus = [
                        RequirementsFocus::Tree,
                        RequirementsFocus::Editor,
                        RequirementsFocus::LinkedTestPoints,
                    ][pane];
                    if ui.focus != focus {
                        ui.exit_insert_mode();
                    }
                    ui.focus = focus;
                    if pane == 0 {
                        let previous = ui.tree_state.selected().to_vec();
                        ui.tree_state.click_at(ratatui::layout::Position::new(
                            pos.x.saturating_add(
                                ui.horizontal_scroll[0].min(u16::MAX as usize) as u16
                            ),
                            pos.y,
                        ));
                        if let Some(id) = ui.tree_state.selected().last().cloned() {
                            ui.select_tree_node(&id);
                            if ui.overlay_active() {
                                ui.tree_state.select(previous);
                            }
                        }
                    }
                    if pane == 2 {
                        let inner = ui.pane_areas[2].inner(ratatui::layout::Margin {
                            horizontal: 1,
                            vertical: 1,
                        });
                        let index = ui.linked_scroll + pos.y.saturating_sub(inner.y) as usize;
                        if inner.contains(pos) && index < ui.filtered_linked_test_points().len() {
                            ui.selected_linked_index = index;
                        }
                    }
                }
                if pane == 1
                    && (matches!(kind, MouseEventKind::Down(MouseButton::Left))
                        || (ui.selection_dragging
                            && matches!(kind, MouseEventKind::Drag(MouseButton::Left))))
                {
                    let inner = ui.pane_areas[1].inner(ratatui::layout::Margin {
                        horizontal: 1,
                        vertical: 1,
                    });
                    if inner.contains(pos) {
                        let Some((row, col)) = ui.editor_position_at(
                            (pos.x - inner.x) as usize,
                            (pos.y - inner.y) as usize,
                        ) else {
                            return Ok(());
                        };
                        if matches!(kind, MouseEventKind::Down(MouseButton::Left)) {
                            ui.selection_anchor = Some((row, col));
                            ui.selection_dragging = true;
                        }
                        ui.selection_end = Some((row, col));
                        ui.cursor_row = row;
                        ui.cursor_col = col;
                        ui.desired_col = col;
                    }
                }
                return Ok(());
            }
        }

        let chat_context = (self.active_tab == MainTab::Ai
            || (self.active_tab == MainTab::MindMap && self.mindmap_ai_panel_visible))
            && !self.model_panel_active
            && !self.session_panel_active
            && !self.auth_panel_active
            && !self.approval_panel_active
            && !self.agent_profile_panel_active
            && !self.quit_pending_confirm
            && !self.change_summary_visible
            && self.generation_scope_prompt.is_none()
            && !self.scenario_dropdown_open
            && !self.has_external_change_prompt()
            && self.step_keyword_picker.is_none()
            && !self.has_agent_change_prompt()
            && !self.has_acp_permission_prompt();
        let in_chat_history = chat_context
            && self
                .chat_history_rect
                .is_some_and(|rect| rect.contains(pos));
        if chat_context && (in_chat_history || self.chat_selection_dragging) {
            match kind {
                MouseEventKind::Down(MouseButton::Left) if in_chat_history => {
                    self.ai_input_focused = false;
                    if self.active_tab == MainTab::MindMap {
                        self.mindmap_focus = MindMapFocus::AiPanel;
                    }
                    let anchor = crate::ui::chat::chat_anchor_at(self, col, row);
                    self.chat_selection = anchor.map(|anchor| ChatSelection {
                        agent_id: self.selected_agent,
                        anchor,
                        focus: anchor,
                    });
                    self.chat_selection_dragging = self.chat_selection.is_some();
                    return Ok(());
                }
                MouseEventKind::Drag(MouseButton::Left) if self.chat_selection_dragging => {
                    let anchor = crate::ui::chat::chat_anchor_at(self, col, row);
                    if let Some(anchor) = anchor
                        && let Some(selection) = self.chat_selection.as_mut()
                    {
                        selection.focus = anchor;
                    }
                    return Ok(());
                }
                MouseEventKind::Up(MouseButton::Left) if self.chat_selection_dragging => {
                    self.chat_selection_dragging = false;
                    return Ok(());
                }
                MouseEventKind::Up(MouseButton::Right) if in_chat_history => {
                    self.handle_action(Action::CopySelection)?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Test Points has no global scroll focus: a wheel event belongs to
        // the pane beneath the pointer. TreeState owns the tree offset while
        // the two text panes keep independent bounded offsets.
        if self.active_tab == MainTab::TestPoints
            && matches!(
                kind,
                MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollLeft
                    | MouseEventKind::ScrollRight
            )
            && let Some(pane) = self
                .clickable_regions
                .iter()
                .find_map(|region| match region {
                    ClickableRegion::TestPointsPane { pane, rect } if rect.contains(pos) => {
                        Some(*pane)
                    }
                    _ => None,
                })
        {
            match (pane, kind) {
                (0, MouseEventKind::ScrollUp) => {
                    self.test_points_ui.tree_state.scroll_up(3);
                }
                (0, MouseEventKind::ScrollDown) => {
                    self.test_points_ui.tree_state.scroll_down(3);
                }
                (_, MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft) => {
                    self.test_points_ui.scroll_pane(pane, -3)
                }
                (_, MouseEventKind::ScrollDown | MouseEventKind::ScrollRight) => {
                    self.test_points_ui.scroll_pane(pane, 3)
                }
                _ => {}
            }
            return Ok(());
        }

        match kind {
            MouseEventKind::Moved => {
                self.mouse_position = Some((col, row));
                return Ok(());
            }
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
                    && let Some(column) = self.explore_horizontal_scroll_drag
                {
                    if let Some(region) = self
                        .clickable_regions
                        .iter()
                        .find(|region| matches!(region, ClickableRegion::ExploreHorizontalScrollbar { column: candidate, .. } if *candidate == column))
                        .cloned()
                    {
                        self.handle_region_click(&region, &pos)?;
                    }
                } else if button == MouseButton::Left
                    && let Some((buf_row, buf_col)) = self.screen_to_buffer_pos(col, row)
                {
                    self.selection_end = Some((buf_row, buf_col));
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.explore_horizontal_scroll_drag = None;
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
                // MindMap tree scroll. Route by pointer position so the tree remains
                // scrollable even when the preview/AI panel has keyboard focus.
                if self.active_tab == MainTab::MindMap
                    && let Some(rect) = self.tree_panel_rect
                    && rect.contains(pos)
                {
                    self.tree_state.scroll_up(3);
                    return Ok(());
                }
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
                // MindMap tree scroll. `TreeState` clamps at the last visible item.
                if self.active_tab == MainTab::MindMap
                    && let Some(rect) = self.tree_panel_rect
                    && rect.contains(pos)
                {
                    self.tree_state.scroll_down(3);
                    return Ok(());
                }
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
        let _ = &self.dirty;
        // Specific controls overlay broad pane regions. Reverse render order
        // gives modal controls priority over the content beneath them.
        if let Some(region) = self
            .clickable_regions
            .iter()
            .rev()
            .find(|region| match region {
                ClickableRegion::ActionButton { rect, .. }
                | ClickableRegion::RequirementFilterOption { rect, .. }
                | ClickableRegion::TestPointField { rect, .. }
                | ClickableRegion::TestPointExcerpt { rect, .. }
                | ClickableRegion::TestPointScenario { rect, .. } => rect.contains(*pos),
                ClickableRegion::ModelProfileRow { rect, .. }
                | ClickableRegion::ModelFormField { rect, .. }
                | ClickableRegion::AgentProfileRow { rect, .. }
                | ClickableRegion::ChangeSummaryRow { rect, .. }
                | ClickableRegion::StepKeywordRow { rect, .. }
                | ClickableRegion::SessionRow { rect, .. }
                | ClickableRegion::MindMapLocation { rect, .. }
                | ClickableRegion::ChatInput { rect } => rect.contains(*pos),
                _ => false,
            })
        {
            return Some(region);
        }
        if let Some(scrollbar) = self.clickable_regions.iter().find(|region| {
            matches!(region, ClickableRegion::ExploreHorizontalScrollbar { rect, .. } if rect.contains(*pos))
        }) {
            return Some(scrollbar);
        }
        for region in &self.clickable_regions {
            match region {
                ClickableRegion::ActionButton { .. }
                | ClickableRegion::RequirementFilterOption { .. }
                | ClickableRegion::TestPointField { .. }
                | ClickableRegion::TestPointExcerpt { .. }
                | ClickableRegion::TestPointScenario { .. } => {}
                ClickableRegion::ModelProfileRow { .. }
                | ClickableRegion::ModelFormField { .. }
                | ClickableRegion::AgentProfileRow { .. }
                | ClickableRegion::ChangeSummaryRow { .. }
                | ClickableRegion::StepKeywordRow { .. }
                | ClickableRegion::ModalList { .. }
                | ClickableRegion::SessionRow { .. }
                | ClickableRegion::MindMapLocation { .. }
                | ClickableRegion::ChatInput { .. } => {}
                ClickableRegion::Tab { rect, .. } => {
                    if rect.contains(*pos) {
                        return Some(region);
                    }
                }
                ClickableRegion::Tree => {
                    if let Some(rect) = self.tree_panel_rect
                        && rect.contains(*pos)
                    {
                        return Some(region);
                    }
                }
                ClickableRegion::ApprovalBadge {
                    row_y,
                    col_x,
                    col_right,
                } => {
                    if *row_y == pos.y && pos.x >= *col_x && pos.x < *col_right {
                        return Some(region);
                    }
                }
                ClickableRegion::ApprovalOption {
                    row_y,
                    col_x,
                    col_right,
                    ..
                } => {
                    if *row_y == pos.y && pos.x >= *col_x && pos.x < *col_right {
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
                ClickableRegion::ExploreDeletedFeature {
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
                ClickableRegion::ExploreHorizontalScrollbar { rect, .. } => {
                    if rect.contains(*pos) {
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
                ClickableRegion::TestPointsPane { rect, .. } => {
                    if rect.contains(*pos) {
                        return Some(region);
                    }
                }
                ClickableRegion::ProjectRecent { rect, .. }
                | ClickableRegion::ProjectOpenPath { rect }
                | ClickableRegion::ProjectControl { rect } => {
                    if rect.contains(*pos) {
                        return Some(region);
                    }
                }
                ClickableRegion::QuitConfirmYes {
                    row_y,
                    col_x,
                    col_right,
                } => {
                    if *row_y == pos.y && pos.x >= *col_x && pos.x < *col_right {
                        return Some(region);
                    }
                }
                ClickableRegion::QuitConfirmNo {
                    row_y,
                    col_x,
                    col_right,
                } => {
                    if *row_y == pos.y && pos.x >= *col_x && pos.x < *col_right {
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
            ClickableRegion::ActionButton { action, .. } => {
                self.handle_action(action.clone())?;
            }
            ClickableRegion::RequirementFilterOption { index, .. } => {
                if let crate::authoring_tab::RequirementsOverlay::FilterPicker { selection }
                | crate::authoring_tab::RequirementsOverlay::PriorityEdit {
                    selection, ..
                } = &mut self.authoring_ui.overlay
                {
                    *selection = *index;
                    self.handle_action(Action::OverlayConfirm)?;
                }
            }
            ClickableRegion::TestPointField { field, .. } => {
                self.commit_test_point_field_edit()?;
                self.test_points_ui.editing = false;
                self.test_points_ui.focus = crate::test_points_tab::TestPointsFocus::Details;
                self.test_points_ui.detail_field = *field;
                self.reload_test_point_field_buffer();
            }
            ClickableRegion::TestPointExcerpt { index, .. } => {
                self.commit_test_point_field_edit()?;
                self.test_points_ui.editing = false;
                self.test_points_ui.focus = crate::test_points_tab::TestPointsFocus::Excerpts;
                self.test_points_ui.selected_excerpt_index = *index;
            }
            ClickableRegion::TestPointScenario { index, .. } => {
                self.commit_test_point_field_edit()?;
                self.test_points_ui.editing = false;
                self.test_points_ui.focus = crate::test_points_tab::TestPointsFocus::Details;
                self.test_points_ui.selected_scenario_ref_index = *index;
            }
            ClickableRegion::ModelProfileRow { index, .. } => {
                self.model_panel_selection = *index;
            }
            ClickableRegion::ModelFormField { index, .. } => {
                self.model_form_focus = (*index).min(6);
                self.model_form_cursor = self.model_form_field().chars().count();
            }
            ClickableRegion::AgentProfileRow { index, .. } => {
                self.agent_profile_panel_selection = *index;
            }
            ClickableRegion::ChangeSummaryRow { index, .. } => {
                self.change_summary_selection = *index;
            }
            ClickableRegion::StepKeywordRow { index, .. } => {
                if let Some(picker) = self.step_keyword_picker.as_mut() {
                    picker.selected = *index;
                }
            }
            ClickableRegion::ModalList { .. } => {}
            ClickableRegion::SessionRow { index, .. } => {
                self.session_panel_selection = *index;
            }
            ClickableRegion::MindMapLocation { index, .. } => {
                self.scenario_dropdown_selection = *index;
            }
            ClickableRegion::ChatInput { .. } => {
                self.ai_input_focused = true;
                if self.active_tab == MainTab::MindMap {
                    self.mindmap_focus = MindMapFocus::AiPanel;
                }
            }
            ClickableRegion::Tab { tab, .. } => {
                self.select_tab(*tab);
            }
            ClickableRegion::Tree => {
                self.mindmap_focus = MindMapFocus::Main;
                self.ai_input_focused = false;
                // tui-tree-widget uses absolute terminal coordinates internally
                if self.tree_state.click_at(*pos)
                    && let Some(id) = crate::mindmap::selected_node_id(&self.tree_state)
                {
                    self.mindmap_index.apply_highlight_categories(id);
                    // rebuild preview when the preview panel is visible
                    let preview_visible = self.view_stage == ViewStage::TreeAndEditor
                        || self.mindmap_ai_panel_visible;
                    if preview_visible {
                        self.rebuild_preview();
                    }
                }
            }
            ClickableRegion::ExploreFeature { feature_idx, .. } => {
                self.explore_set_feature_row(*feature_idx);
                self.explore_focus = crate::app::ColumnFocus::Feature;
                // Reset scenario/step selection
                self.explore_selected_scenario = 0;
                self.explore_selected_step = 0;
            }
            ClickableRegion::ExploreDeletedFeature { deleted_idx, .. } => {
                self.explore_set_deleted_feature(*deleted_idx);
                self.explore_focus = crate::app::ColumnFocus::Feature;
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
            ClickableRegion::ExploreHorizontalScrollbar {
                column,
                rect,
                content_width,
                viewport_width,
            } => {
                let max_scroll = content_width.saturating_sub(*viewport_width);
                let track_width = rect.width as usize;
                if track_width > 0 && max_scroll > 0 {
                    let thumb_width = ((*viewport_width * track_width) / *content_width)
                        .clamp(4.min(track_width), track_width);
                    let travel = track_width.saturating_sub(thumb_width);
                    let pointer = pos.x.saturating_sub(rect.x) as usize;
                    let thumb_start = pointer.saturating_sub(thumb_width / 2).min(travel);
                    self.explore_horizontal_scroll[column.index()] = thumb_start
                        .saturating_mul(max_scroll)
                        .checked_div(travel)
                        .unwrap_or(0);
                    self.explore_horizontal_scroll_drag = Some(*column);
                    self.explore_focus = *column;
                }
            }
            ClickableRegion::ApprovalBadge { .. } => {
                // Click opens the approval mode picker panel
                self.approval_panel_active = true;
                self.approval_panel_selection = teshi_agent::approval::ApprovalMode::ALL
                    .iter()
                    .position(|m| *m == self.approval_mode)
                    .unwrap_or(0);
            }
            ClickableRegion::ApprovalOption { option_idx, .. } => {
                // Click selects and confirms the option
                use teshi_agent::approval::ApprovalMode;
                let modes = ApprovalMode::ALL;
                if let Some(&mode) = modes.get(*option_idx) {
                    let old = self.approval_mode;
                    self.approval_mode = mode;
                    self.approval_panel_active = false;
                    self.status = format!(
                        "Approval: {} → {} ({})",
                        old.display_name(),
                        mode.display_name(),
                        mode.description()
                    );
                }
            }
            ClickableRegion::EditorPanel | ClickableRegion::PreviewPanel => {}
            ClickableRegion::TestPointsPane { pane, .. } => {
                if self.active_tab != MainTab::TestPoints {
                    return Ok(());
                }
                self.commit_test_point_field_edit()?;
                let focus = match pane {
                    0 => crate::test_points_tab::TestPointsFocus::Tree,
                    1 => crate::test_points_tab::TestPointsFocus::Details,
                    _ => crate::test_points_tab::TestPointsFocus::Excerpts,
                };
                if self.test_points_ui.focus != focus {
                    self.test_points_ui.commit_field_if_dirty();
                    self.test_points_ui.editing = false;
                    self.test_points_ui.focus = focus;
                    self.reload_test_point_field_buffer();
                }
                if *pane == 0 {
                    self.test_points_ui.tree_state.click_at(*pos);
                    if let Some(id) = self
                        .test_points_ui
                        .tree_state
                        .selected()
                        .last()
                        .and_then(|node| node.strip_prefix(crate::test_points_tab::TREE_TP_PREFIX))
                        .map(str::to_owned)
                    {
                        self.test_points_ui.select_test_point(&id);
                    }
                }
            }
            ClickableRegion::ProjectRecent { index, .. } => {
                if let crate::project_context::SwitchOverlay::Picker { selection } =
                    &mut self.project_switch.overlay
                {
                    *selection = (*index).min(self.project_switch.recent.len().saturating_sub(1));
                    if let Some(target) = self.project_switch.picker_target() {
                        self.request_project_switch(target.root())?;
                    }
                }
            }
            ClickableRegion::ProjectOpenPath { .. } => {
                self.project_switch.begin_open_path();
                self.status =
                    "Type a project directory and press Enter to open it; Esc cancels".to_string();
            }
            ClickableRegion::ProjectControl { .. } => {
                self.cmd_project(None)?;
            }
            ClickableRegion::QuitConfirmYes { .. } => {
                self.should_quit = true;
            }
            ClickableRegion::QuitConfirmNo { .. } => {
                self.quit_pending_confirm = false;
            }
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
            || self.is_requirements_editor_active()
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
        let rows = bdd_editor_nav_rows(&self.buffer)
            .into_iter()
            .filter(|row| !hidden.contains(row))
            .collect();
        (rows, body_chain_nav)
    }

    fn apply_vertical_nav_jump(&mut self, new_row: usize, body_chain_nav: bool) {
        self.cursor_row = new_row;
        self.cursor_col = 0;
        self.desired_col = 0;
        // When the cursor moves into a scenario's steps, focus that scenario
        // so its steps stay visible while other scenarios' steps are dimmed.
        self.editor_focus_scenario_row = scenario_header_for_row(&self.buffer, new_row);
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

    fn move_sibling_up(&mut self) {
        if self.step_input_active || self.step_keyword_picker.is_some() {
            return;
        }
        if self.is_editor_nav_mode() {
            let Some(scenario_row) = scenario_header_for_row(&self.buffer, self.cursor_row) else {
                return;
            };
            let steps = scenario_step_rows(&self.buffer, scenario_row);
            let hidden = self.hidden_editor_rows();
            let visible_steps: Vec<usize> =
                steps.into_iter().filter(|r| !hidden.contains(r)).collect();
            if let Some(pos) = visible_steps.iter().position(|&r| r == self.cursor_row)
                && pos > 0
            {
                let new_row = visible_steps[pos - 1];
                self.apply_vertical_nav_jump(new_row, self.focus_slot == BddFocusSlot::Body);
            }
        }
        self.quit_pending_confirm = false;
    }

    fn move_sibling_down(&mut self) {
        if self.step_input_active || self.step_keyword_picker.is_some() {
            return;
        }
        if self.is_editor_nav_mode() {
            let Some(scenario_row) = scenario_header_for_row(&self.buffer, self.cursor_row) else {
                return;
            };
            let steps = scenario_step_rows(&self.buffer, scenario_row);
            let hidden = self.hidden_editor_rows();
            let visible_steps: Vec<usize> =
                steps.into_iter().filter(|r| !hidden.contains(r)).collect();
            if let Some(pos) = visible_steps.iter().position(|&r| r == self.cursor_row)
                && pos + 1 < visible_steps.len()
            {
                let new_row = visible_steps[pos + 1];
                self.apply_vertical_nav_jump(new_row, self.focus_slot == BddFocusSlot::Body);
            }
        }
        self.quit_pending_confirm = false;
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

fn apply_list_navigation(selection: &mut usize, count: usize, action: &Action) {
    let last = count.saturating_sub(1);
    *selection = match action {
        Action::NavigateFirst => 0,
        Action::NavigateLast => last,
        Action::PageUp => selection.saturating_sub(10),
        Action::PageDown => selection.saturating_add(10).min(last),
        _ => (*selection).min(last),
    };
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
        until_line: scenario.steps.last().map(|s| s.line_number),
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

    fn open_agent_backend_panel(&mut self, mode: AgentPanelMode) {
        let root = match crate::acp_agents::store_dir() {
            Ok(root) => root,
            Err(error) => {
                self.status = format!("ACP storage unavailable: {error}");
                return;
            }
        };
        self.installed_acp_agents = match crate::acp_agents::load(&root) {
            Ok(agents) => agents,
            Err(error) => {
                self.status = format!("ACP installations unavailable: {error}");
                return;
            }
        };
        self.agent_panel_mode = mode;
        self.agent_profile_panel_selection = 0;
        self.agent_profile_panel_active = true;
        self.status = match mode {
            AgentPanelMode::SelectBackend => {
                "Select Agent: ↑↓ move · Enter select · Esc close".into()
            }
            AgentPanelMode::InstalledAcp => {
                "ACP Agents: ↑↓ move · Enter select · i Registry · Esc close".into()
            }
            _ => String::new(),
        };
    }

    fn open_acp_registry(&mut self) {
        self.open_agent_backend_panel(AgentPanelMode::InstalledAcp);
        if !self.agent_profile_panel_active {
            return;
        }
        self.acp_registry_agents.clear();
        self.agent_panel_mode = AgentPanelMode::Registry;
        self.status = "Loading official ACP Registry...".into();
        let (tx, rx) = std::sync::mpsc::channel();
        self.acp_registry_rx = Some(rx);
        let registry_url = std::env::var("TESHI_ACP_REGISTRY_URL")
            .unwrap_or_else(|_| teshi_acp::registry::OFFICIAL_REGISTRY_URL.to_owned());
        std::thread::spawn(move || {
            let result = (|| -> Result<_> {
                let runtime = tokio::runtime::Runtime::new()?;
                Ok(runtime
                    .block_on(teshi_acp::registry::fetch_registry(
                        &registry_url,
                        Duration::from_secs(10),
                    ))?
                    .agents)
            })()
            .map_err(|error| error.to_string());
            let _ = tx.send(result);
        });
    }

    pub fn poll_acp_registry(&mut self) {
        if let Some(rx) = &self.acp_install_rx {
            match rx.try_recv() {
                Ok(Ok(installed)) => {
                    self.acp_install_rx = None;
                    self.status = format!("Installed {}. Use /agent to select it.", installed.name);
                    if let Ok(root) = crate::acp_agents::store_dir()
                        && let Ok(agents) = crate::acp_agents::load(&root)
                    {
                        self.installed_acp_agents = agents;
                    }
                }
                Ok(Err(error)) => {
                    self.acp_install_rx = None;
                    self.status = format!("ACP install failed: {error}");
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.acp_install_rx = None;
                    self.status = "ACP install failed: worker stopped".into();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(rx) = &self.acp_registry_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(agents)) => {
                self.acp_registry_rx = None;
                if self.agent_profile_panel_active
                    && self.agent_panel_mode == AgentPanelMode::Registry
                {
                    self.acp_registry_agents = agents;
                    self.status = "ACP Registry: ↑↓ move · Enter install · Esc close".into();
                }
            }
            Ok(Err(error)) => {
                self.acp_registry_rx = None;
                if self.agent_profile_panel_active
                    && self.agent_panel_mode == AgentPanelMode::Registry
                {
                    self.agent_profile_panel_active = false;
                    self.status = format!("ACP Registry unavailable: {error}");
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.acp_registry_rx = None;
                if self.agent_profile_panel_active
                    && self.agent_panel_mode == AgentPanelMode::Registry
                {
                    self.agent_profile_panel_active = false;
                    self.status = "ACP Registry unavailable: fetch worker stopped".into();
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    fn select_backend(&mut self, installed: Option<crate::acp_agents::InstalledAgent>) {
        self.cancel_agent_turn();
        let thread = self.agent_mut();
        let messages = std::mem::take(thread.messages_mut());
        let selected_id = installed.as_ref().map(|agent| agent.id.clone());
        let kind = if installed.is_some() {
            teshi_agent::backend::AgentBackendKind::Acp
        } else {
            teshi_agent::backend::AgentBackendKind::Native
        };
        thread.replace_backend(kind);
        *thread.messages_mut() = messages;
        thread.selected_acp_id = selected_id;
        thread.selection_warning = None;
        thread.acp_controls_ui = Default::default();
        thread.status = AiStatus::Idle;
        self.active_model_label = installed.as_ref().map(|agent| agent.name.clone());
        self.status = match installed {
            Some(agent) => format!(
                "Selected ACP Agent {}; remembered for new sessions",
                agent.name
            ),
            None => "Selected Teshi Native; remembered for new sessions".into(),
        };
        if let Some(root) = self.agent().selection_root.as_ref() {
            let selection = match self.agent().selected_acp_id.clone() {
                Some(agent_id) => crate::agent_selection::Selection::Acp { agent_id },
                None => crate::agent_selection::Selection::Native,
            };
            if let Err(error) = crate::agent_selection::save(root, &selection) {
                self.status = format!("Agent selected, but preference could not be saved: {error}");
            }
        }
        self.agent_profile_panel_active = false;
        if kind == teshi_agent::backend::AgentBackendKind::Acp {
            self.connect_acp_controls();
        }
    }

    fn handle_acp_panel_action(&mut self, action: Action) -> Result<()> {
        let count = match self.agent_panel_mode {
            AgentPanelMode::SelectBackend => self.installed_acp_agents.len() + 1,
            AgentPanelMode::InstalledAcp => self.installed_acp_agents.len(),
            AgentPanelMode::Registry => self.acp_registry_agents.len(),
            AgentPanelMode::List => 0,
        };
        match action {
            Action::AgentPanelUp => {
                self.agent_profile_panel_selection =
                    self.agent_profile_panel_selection.saturating_sub(1)
            }
            Action::AgentPanelDown => {
                if self.agent_profile_panel_selection + 1 < count {
                    self.agent_profile_panel_selection += 1;
                }
            }
            Action::AgentPanelSelect => match self.agent_panel_mode {
                AgentPanelMode::SelectBackend => {
                    let item = if self.agent_profile_panel_selection == 0 {
                        None
                    } else {
                        self.installed_acp_agents
                            .get(self.agent_profile_panel_selection - 1)
                            .cloned()
                    };
                    self.select_backend(item);
                }
                AgentPanelMode::InstalledAcp => {
                    // Rows start at the first installed agent (no Native row).
                    if let Some(agent) = self
                        .installed_acp_agents
                        .get(self.agent_profile_panel_selection)
                        .cloned()
                    {
                        self.select_backend(Some(agent));
                    }
                }
                AgentPanelMode::Registry if self.acp_install_rx.is_none() => {
                    if let Some(agent) = self
                        .acp_registry_agents
                        .get(self.agent_profile_panel_selection)
                        .cloned()
                    {
                        if self
                            .installed_acp_agents
                            .iter()
                            .any(|item| item.id == agent.id)
                        {
                            self.status = format!(
                                "{} is already installed. Use /agent to select it.",
                                agent.name.as_deref().unwrap_or(&agent.id)
                            );
                            return Ok(());
                        }
                        let root = crate::acp_agents::store_dir()?;
                        let (tx, rx) = std::sync::mpsc::channel();
                        self.acp_install_rx = Some(rx);
                        self.status = format!(
                            "Installing {}...",
                            agent.name.as_deref().unwrap_or(&agent.id)
                        );
                        std::thread::spawn(move || {
                            let result = crate::acp_agents::install(&root, &agent)
                                .map_err(|error| error.to_string());
                            let _ = tx.send(result);
                        });
                    }
                }
                _ => {}
            },
            Action::AgentPanelAdd if self.agent_panel_mode == AgentPanelMode::InstalledAcp => {
                self.open_acp_registry()
            }
            Action::AgentPanelClose | Action::ClearInputState => {
                self.agent_profile_panel_active = false
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute a slash command from either submitted input or the suggestion menu.
    ///
    /// The suggestion menu clears the input before calling this method, so all
    /// commands that have executable behavior must be handled here rather than
    /// in a second dispatcher.
    pub(crate) fn execute_slash_command(&mut self, command: &str) -> Result<()> {
        let command = command.trim();
        if command == "details" {
            let agent = self.agent_mut();
            agent.activity_details_expanded = !agent.activity_details_expanded;
            agent.scroll_offset = 0;
            self.status = "Thinking and tool details toggled".into();
            return Ok(());
        }
        if command == "settings" {
            self.open_acp_controls(None);
            return Ok(());
        }
        if command == "auth" || command.starts_with("auth ") {
            self.open_auth_panel(command.strip_prefix("auth ").unwrap_or(""));
            return Ok(());
        }
        if command == "new" {
            return self.cmd_new();
        }
        if command == "exit" || command == "quit" {
            return self.cmd_exit();
        }
        if command == "retry" {
            return self.continue_interrupted_task();
        }
        if command == "resume" {
            return self.cmd_resume();
        }
        if command == "copy" {
            return self.cmd_copy(None);
        }
        if let Some(argument) = command.strip_prefix("copy ") {
            return self.cmd_copy(Some(argument));
        }
        if command == "models" || command == "model" {
            return self.cmd_models();
        }
        if command == "sessions" || command == "session" {
            return self.cmd_sessions(false);
        }
        if command == "sessions legacy" || command == "session legacy" {
            return self.cmd_sessions(true);
        }
        if command == "project" || command == "projects" {
            return self.cmd_project(None);
        }
        if let Some(argument) = command.strip_prefix("project ") {
            return self.cmd_project(Some(argument));
        }
        if command == "approval" || command == "approve" || command == "app" {
            self.approval_panel_active = true;
            self.approval_panel_selection = teshi_agent::approval::ApprovalMode::ALL
                .iter()
                .position(|m| *m == self.approval_mode)
                .unwrap_or(0);
            self.status = "Approval mode".to_string();
            return Ok(());
        }
        if command == "agent" || command == "agents" {
            self.open_agent_backend_panel(AgentPanelMode::SelectBackend);
            return Ok(());
        }
        if command == "acp" {
            self.open_agent_backend_panel(AgentPanelMode::InstalledAcp);
            return Ok(());
        }
        if let Some(spec) = command.strip_prefix("acp add ") {
            let Some((id, command_json)) = spec.split_once(' ') else {
                self.status = "Usage: /acp add <id> [\"program\",\"arg\",...]".into();
                return Ok(());
            };
            let result = (|| -> Result<String> {
                let argv: Vec<String> = serde_json::from_str(command_json)?;
                let root = crate::acp_agents::store_dir()?;
                let installed = crate::acp_agents::add_custom(&root, id, &argv)?;
                Ok(installed.name)
            })();
            self.status = match result {
                Ok(name) => format!("Added custom ACP Agent {name}. Use /agent to select it."),
                Err(error) => format!("Custom ACP Agent failed: {error}"),
            };
            return Ok(());
        }
        if command == "agent-profile" {
            self.agent_registry =
                teshi_agent::registry::AgentRegistry::load(Some(self.find_project_dir()));
            self.agent_profile_panel_active = true;
            self.agent_panel_mode = AgentPanelMode::List;
            self.agent_profile_panel_selection = self
                .agent_registry
                .iter()
                .position(|p| {
                    Some(p.id.as_str()) == self.agent().profile_id.as_deref()
                        || (self.agent().profile_id.is_none() && p.id == "default")
                })
                .unwrap_or(0);
            self.status = "Agent profiles: ↑↓ select · Enter apply · Esc close".to_string();
            return Ok(());
        }
        if command == "generate" || command.starts_with("generate ") {
            let rest = command
                .strip_prefix("generate")
                .unwrap_or("")
                .trim()
                .to_string();
            let pending_user_message = if rest.is_empty() {
                "I want to generate a feature from requirements. Start the Feature Generation Pipeline: gather requirements (I can paste detailed text next), propose non-Gherkin test points for human review, then plan and write Gherkin .feature files. Do not use FreeMind or mock HTML.".to_string()
            } else {
                format!(
                    "Please generate a feature from these requirements using the Feature Generation Pipeline. Propose non-Gherkin test points for human review before planning scenarios. Write Gherkin .feature files only (no FreeMind or mock HTML).\n\nRequirements:\n{rest}"
                )
            };
            self.generation_scope_prompt = Some(GenerationScopePrompt {
                iteration: self.authoring_ui.iteration_filter.clone(),
                pending_user_message,
            });
            self.status =
                "Confirm generation source scope (Enter confirm · Esc cancel · i change iteration)"
                    .to_string();
            return Ok(());
        }
        if command == "continue" || command == "continue-generation" {
            return self.continue_test_point_generation();
        }
        self.status = "Unknown slash command. Try /new, /exit, /resume, /retry, /copy, /models, /sessions, /project, /approval, /agent, /generate, /continue".to_string();
        Ok(())
    }

    fn continue_interrupted_task(&mut self) -> Result<()> {
        if !self.agent().task_interrupted() {
            self.status = "No interrupted ACP task to continue".into();
            return Ok(());
        }
        // Dispatch through the existing backend without replacing its ACP session.
        self.dispatch_user_message("Continue the interrupted task from the last checkpoint. First verify which operations have already completed; do not repeat completed tool actions. If you cannot safely continue, explain what is blocking progress.".into())
    }

    fn cmd_project(&mut self, argument: Option<&str>) -> Result<()> {
        if let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) {
            return self.request_project_switch(PathBuf::from(argument));
        }
        let recent = teshi_engine::load_recent_project_entries().unwrap_or_default();
        self.project_switch.begin_picker(recent);
        self.status = "Project picker: ↑/↓ select · Enter open · Esc cancel".to_string();
        Ok(())
    }

    /// Handle `/new` — start a new session.
    fn cmd_new(&mut self) -> Result<()> {
        self.clear_chat_selection();
        self.save_sessions(true)?;
        self.cancel_agent_turn();
        self.agent_mut().saved_session = None;
        self.agent_mut().last_session_snapshot.clear();
        self.agent_mut().messages_mut().clear();
        self.agent_mut().partial_response_mut().clear();
        self.agent_mut().input.clear();
        self.agent_mut().input_cursor = 0;
        self.agent_mut().status = AiStatus::Idle;
        self.agent_mut().tool_status = None;
        self.agent_mut().scroll_offset = 0;
        let id = self.agent().id;
        let title = self.agent().title.clone();
        let profile_id = self.agent().profile_id.clone();
        let root = self.agent().selection_root.clone();
        *self.agent_mut() = AgentThread::new_with_selection(id, &title, root);
        self.agent_mut().profile_id = profile_id;
        self.spawn_llm_if_configured();
        self.status = "New session started".to_string();
        self.show_selection_warning();
        Ok(())
    }

    /// Handle `/exit` or `/quit` — exit the application.
    fn cmd_exit(&mut self) -> Result<()> {
        self.save_sessions(true)?;
        self.should_quit = true;
        Ok(())
    }

    /// Handle `/resume` — load the most recent session.
    fn cmd_resume(&mut self) -> Result<()> {
        self.save_sessions(true)?;
        let sessions = self.project_sessions();
        if let Some(s) = sessions.into_iter().next() {
            if !self.restore_session(s, false) {
                return Ok(());
            }
            self.agent_mut().partial_response_mut().clear();
            self.agent_mut().input.clear();
            self.agent_mut().input_cursor = 0;
            self.agent_mut().status = AiStatus::Idle;
            self.agent_mut().scroll_offset = 0;
            self.status = format!(
                "Resumed session from {} messages",
                self.agent().messages().len()
            );
        } else {
            self.status = "No saved sessions found".to_string();
        }
        Ok(())
    }

    /// Handle `/copy [N]` — copy the last N completed assistant responses.
    fn cmd_copy(&mut self, argument: Option<&str>) -> Result<()> {
        let n = match argument.map(str::trim) {
            None | Some("") => 1,
            Some(value) => match value.parse::<usize>() {
                Ok(0) | Err(_) => {
                    self.status = "Usage: /copy [N] where N is a positive integer".to_string();
                    self.set_status_message(self.status.clone());
                    return Ok(());
                }
                Ok(n) => n,
            },
        };
        let texts = completed_assistant_responses(self.agent().messages(), n);
        if texts.is_empty() {
            self.status = "No completed assistant responses to copy".to_string();
            self.set_status_message(self.status.clone());
            return Ok(());
        }
        let combined = texts.join("\n\n---\n\n");
        let count = texts.len();
        if !self.try_copy_to_clipboard(&combined) {
            return Ok(());
        }
        self.status = format!("Copied last {} assistant response(s) to clipboard", count);
        self.set_status_message(self.status.clone());
        Ok(())
    }

    /// Handle `/models` — open the model profile panel.
    fn cmd_models(&mut self) -> Result<()> {
        if let Some(state) = self.agent().backend.acp_controls() {
            let id = state
                .config
                .options
                .iter()
                .find(|o| o.category.as_deref() == Some("model"))
                .map(|o| o.id.clone());
            self.open_acp_controls(id);
            return Ok(());
        }
        self.model_panel_active = true;
        self.model_panel_selection = 0;
        self.model_panel_mode = ModelPanelMode::List;
        self.model_profiles = crate::profiles::load_all();
        self.status = "Model profiles [m]. a add · e edit · ↑↓ select · Enter activate · Esc close"
            .to_string();
        Ok(())
    }

    /// Handle `/sessions` — open the session browser panel.
    fn cmd_sessions(&mut self, legacy: bool) -> Result<()> {
        self.pending_session_restore = None;
        self.session_panel_active = true;
        self.session_panel_legacy = legacy;
        self.session_panel_selection = 0;
        self.session_list = if legacy {
            self.legacy_sessions()
        } else {
            self.project_sessions()
        };
        self.status = if legacy {
            "Legacy / Unassigned. Loading keeps history unassigned. Enter load · Esc close"
                .to_string()
        } else {
            "Sessions for the active project. /sessions legacy shows unassigned history".to_string()
        };
        Ok(())
    }

    fn project_sessions(&self) -> Vec<crate::session::Session> {
        let active = self.project_switch.active.identity.clone();
        crate::session::Session::load_all()
            .into_iter()
            .filter(|s| {
                s.project_root.as_ref().is_some_and(|root| {
                    teshi_engine::ProjectIdentity::new(root)
                        .ok()
                        .is_some_and(|identity| identity == active)
                })
            })
            .collect()
    }

    fn legacy_sessions(&self) -> Vec<crate::session::Session> {
        crate::session::Session::load_all()
            .into_iter()
            .filter(|session| session.project_root.is_none())
            .collect()
    }

    fn restore_session(&mut self, session: crate::session::Session, legacy: bool) -> bool {
        self.pending_session_restore = None;
        if session.project_root.is_none() && !legacy {
            self.status = "Unassigned session: use /sessions legacy to load it explicitly".into();
            self.set_status_message(self.status.clone());
            return false;
        }
        if session.project_root.is_some() && legacy {
            self.status = "Project-owned sessions cannot be loaded from the Legacy view".into();
            self.set_status_message(self.status.clone());
            return false;
        }
        if let Some(root) = session.project_root.as_ref() {
            let identity = match teshi_engine::ProjectIdentity::new(root) {
                Ok(identity) => identity,
                Err(error) => {
                    self.status = format!("Cannot resolve session project: {error}");
                    self.set_status_message(self.status.clone());
                    return false;
                }
            };
            if identity != self.project_switch.active.identity {
                self.status = format!(
                    "Session belongs to another project: {}",
                    identity.root().display()
                );
                self.set_status_message(self.status.clone());
                return false;
            }
        }
        // Keep owned handles until shutdown is acknowledged. Replacing a live
        // ACP backend would synchronously join its worker on the UI thread.
        let agent = self.agent_mut();
        agent.retain_partial_response();
        agent.backend.request_shutdown();
        agent.poll_shutdown();
        agent.status = AiStatus::Idle;
        agent.tool_status = None;
        agent.pending_acp_permission = None;
        if self.agent().shutdown_pending() {
            self.pending_session_restore = Some((
                self.agent().id,
                self.current_project_stamp(),
                session,
                legacy,
            ));
            self.session_panel_active = true;
            self.session_panel_legacy = legacy;
            self.status =
                "Waiting for the previous Agent to exit before loading history; Esc cancels".into();
            self.set_status_message(self.status.clone());
            return false;
        }
        // A restored session replaces the source blocks behind any rendered
        // chat anchors.  Do not let a selection from the previous session be
        // copied against the newly loaded message list.
        self.clear_chat_selection();
        let kind = session
            .backend_kind
            .unwrap_or(teshi_agent::backend::AgentBackendKind::Native);
        self.active_model_label = session.model_label.clone();
        let agent = self.agent_mut();
        agent.backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
            kind,
            teshi_agent_runtime::max_agent_iterations(),
        );
        *agent.messages_mut() = session.messages.clone();
        agent.selected_acp_id = session.acp_agent_id.clone();
        agent.acp_controls_ui = Default::default();
        agent.saved_session = Some(session);
        agent.last_session_snapshot.clear();
        agent.tool_status = None;
        agent.pending_acp_permission = None;
        agent.latest_visual_observation = None;
        agent.last_finish_reason = None;
        agent.input.clear();
        agent.input_cursor = 0;
        agent.status = AiStatus::Idle;
        agent.scroll_offset = 0;
        self.session_panel_active = false;
        self.status = if legacy {
            "Loaded legacy session; history remains unassigned".into()
        } else {
            format!(
                "Loaded session with {} messages",
                self.agent().messages().len()
            )
        };
        self.set_status_message(self.status.clone());
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use anyhow::Context;

    use super::{
        AgentPanelMode, AgentThread, AiChatMessage, AiRole, AiStatus, App, BddFocusSlot,
        ChatAnchor, ChatSelection, ClickableRegion, ColumnFocus, GitStatusRefreshResult, MainTab,
        MindMapFocus, ModelPanelMode, RunStatus, ViewStage, completed_assistant_responses,
        current_step_keyword_index, replace_step_keyword_line,
    };
    use crate::bdd_nav::step_edit_start_col;
    use crate::editor_buffer::EditorBuffer;
    use crate::keymap::Action;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    use tempfile::tempdir;
    use teshi_core::StepIndex;
    use teshi_core::gherkin_lang::GherkinLanguages;

    fn requirements_test_app() -> (App, tempfile::TempDir, tempfile::TempDir) {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        fs::write(&feature, "Feature: Sample\n").unwrap();
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        app.active_tab = MainTab::Requirements;
        app.authoring_ui.create_document("req.md", "Req").unwrap();
        app.authoring_ui
            .save_current_document(project.path())
            .unwrap();
        app.authoring_ui.focus = crate::authoring_tab::RequirementsFocus::Editor;
        (app, project, store)
    }

    #[test]
    fn deletion_confirmation_keeps_a_missing_target_and_does_not_delete_another_row() {
        let (mut app, _project, _store) = requirements_test_app();
        app.pending_delete = Some(super::DeleteConfirmation {
            action: Action::ModelPanelDelete,
            target_id: "missing-fixture-profile".into(),
            label: "fixture profile".into(),
            error: None,
        });
        app.model_profiles.clear();
        app.handle_action(Action::DeleteConfirm).unwrap();
        let pending = app.pending_delete.as_ref().unwrap();
        assert_eq!(pending.target_id, "missing-fixture-profile");
        assert!(
            pending
                .error
                .as_ref()
                .unwrap()
                .contains("no longer available")
        );
        app.handle_action(Action::DeleteCancel).unwrap();
        assert!(app.pending_delete.is_none());
    }

    #[test]
    fn quit_yes_exits_even_when_the_cancel_button_was_selected() {
        let (mut app, _project, _store) = requirements_test_app();
        app.quit_pending_confirm = true;
        app.quit_panel_selection = 1;
        app.handle_action(Action::DeleteConfirm).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn copy_candidates_keep_completed_assistant_order_and_exclude_progress() {
        let messages = vec![
            AiChatMessage {
                role: AiRole::User,
                content: "question".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "old answer".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "streaming progress".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: Some("agent_progress".into()),
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "   ".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: Some("thinking only".into()),
                source: None,
            },
            AiChatMessage {
                role: AiRole::Assistant,
                content: "new answer".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            },
        ];

        assert_eq!(
            completed_assistant_responses(&messages, 2),
            vec!["old answer", "new answer"]
        );
        assert_eq!(
            completed_assistant_responses(&messages, 1),
            vec!["new answer"]
        );
    }

    #[test]
    fn copy_command_rejects_invalid_counts_and_empty_history() {
        let (mut app, _project, _store) = slash_test_app();

        app.execute_slash_command("copy 0").unwrap();
        assert_eq!(app.status, "Usage: /copy [N] where N is a positive integer");
        app.execute_slash_command("copy not-a-number").unwrap();
        assert_eq!(app.status, "Usage: /copy [N] where N is a positive integer");
        app.execute_slash_command("copy").unwrap();
        assert_eq!(app.status, "No completed assistant responses to copy");
    }

    #[test]
    fn requirements_save_refreshes_rendered_title_and_rejects_invalid_edits() {
        let (mut app, _project, store) = requirements_test_app();
        let original = app.authoring_ui.artifacts.as_ref().unwrap().index.documents[0].clone();
        let renamed = "用户认证需求";
        app.authoring_ui.buffer =
            EditorBuffer::from_string(format!("#    {renamed}    \n\nBody\n"));
        app.authoring_ui.buffer_dirty = true;
        app.handle_action(Action::Save).unwrap();
        assert_eq!(app.status, "Saved requirement document");
        assert!(!app.authoring_ui.buffer_dirty);
        let saved = teshi_engine::read_requirement_document(store.path(), &original.id).unwrap();
        assert_eq!(saved.meta.title, renamed);
        assert_eq!(saved.meta.id, original.id);
        assert_eq!(saved.meta.path, original.path);
        assert_eq!(
            app.authoring_ui.selected_document_id.as_deref(),
            Some(original.id.as_str())
        );

        let mut terminal = Terminal::new(TestBackend::new(150, 24)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let tree = app.authoring_ui.pane_areas[0];
        let mut tree_text = String::new();
        for y in tree.y..tree.bottom() {
            let mut x = tree.x;
            while x < tree.right() {
                let symbol = terminal.backend().buffer()[(x, y)].symbol();
                tree_text.push_str(symbol);
                x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
            }
        }
        assert!(tree_text.contains(renamed), "tree was: {tree_text}");

        for (body, message) in [
            ("#   \n", "Requirement title cannot be empty"),
            (
                "No H1\n# Later",
                "Requirement must start with an H1 title: # Title",
            ),
        ] {
            app.authoring_ui.buffer = EditorBuffer::from_string(body.into());
            app.authoring_ui.buffer_dirty = true;
            app.handle_action(Action::Save).unwrap();
            assert!(app.status.contains(message), "{}", app.status);
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app))
                .unwrap();
            let footer = (0..150)
                .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
                .collect::<String>();
            assert!(footer.contains(message), "footer was: {footer}");
            assert!(app.authoring_ui.buffer_dirty);
            assert_eq!(app.authoring_ui.buffer.as_string(), body);
            assert_eq!(
                teshi_engine::read_requirement_document(store.path(), &original.id).unwrap(),
                saved
            );
            assert_eq!(
                app.authoring_ui.artifacts.as_ref().unwrap().documents[0].meta,
                saved.meta
            );
        }
    }

    #[test]
    fn requirements_new_document_action_uses_matching_id_path_and_title() {
        let (mut app, _project, _store) = requirements_test_app();
        app.handle_action(Action::ReqNewDocument).unwrap();
        assert_eq!(
            app.authoring_ui.selected_document_id.as_deref(),
            Some("doc-2")
        );
        assert_eq!(app.authoring_ui.buffer.as_string(), "# Document 2\n\n");
        assert!(app.authoring_ui.buffer_dirty);
        let artifacts = app.authoring_ui.artifacts.as_ref().unwrap();
        let new = artifacts.index.documents.last().unwrap();
        assert_eq!(new.id, "doc-2");
        assert_eq!(new.path.len(), 26);
        assert!(new.path.ends_with("Z.md"));
        assert_eq!(&new.path[8..9], "T");
        assert_eq!(new.title, "Document 2");
    }

    #[test]
    fn requirements_new_document_dirty_guard_preserves_cancel_and_resumes_after_discard() {
        let (mut app, _project, _store) = requirements_test_app();
        let original = app.authoring_ui.buffer.as_string();
        app.authoring_ui.enter_insert_mode();
        app.authoring_ui.insert_char('x');
        let edited = app.authoring_ui.buffer.as_string();

        app.handle_action(Action::ReqNewDocument).unwrap();
        assert!(matches!(
            app.authoring_ui.overlay,
            crate::authoring_tab::RequirementsOverlay::Unsaved { .. }
        ));
        assert_eq!(
            app.authoring_ui.selected_document_id.as_deref(),
            Some("doc-1")
        );
        assert_eq!(app.authoring_ui.buffer.as_string(), edited);
        assert_eq!(
            app.authoring_ui
                .artifacts
                .as_ref()
                .unwrap()
                .index
                .documents
                .len(),
            1
        );

        app.handle_action(Action::OverlayCancel).unwrap();
        assert_eq!(app.authoring_ui.buffer.as_string(), edited);
        assert!(app.authoring_ui.buffer_dirty);
        assert_eq!(
            app.authoring_ui.selected_document_id.as_deref(),
            Some("doc-1")
        );

        app.handle_action(Action::ReqNewDocument).unwrap();
        app.handle_action(Action::OverlayInsert('d')).unwrap();
        assert_eq!(
            app.authoring_ui.selected_document_id.as_deref(),
            Some("doc-2")
        );
        assert_eq!(app.authoring_ui.buffer.as_string(), "# Document 2\n\n");
        assert_eq!(
            app.authoring_ui
                .artifacts
                .as_ref()
                .unwrap()
                .index
                .documents
                .len(),
            2
        );
        assert_ne!(app.authoring_ui.buffer.as_string(), original);
    }

    fn slash_test_app() -> (App, tempfile::TempDir, tempfile::TempDir) {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        fs::write(&feature, "Feature: Sample\n").unwrap();
        let app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        (app, project, store)
    }

    fn stop_fixture_agents(app: &mut App) {
        // Constructors may attach an idle model from inherited configuration.
        // These state tests drive their own event loop; quiesce those owned
        // workers so the tested precondition is a clean, idle project A.
        for agent in &mut app.agents {
            agent.backend.request_shutdown();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while app
            .agents
            .iter()
            .any(|agent| agent.backend.shutdown_pending())
        {
            assert!(
                std::time::Instant::now() < deadline,
                "idle fixture backend did not stop"
            );
            for agent in &mut app.agents {
                agent.backend.poll_shutdown();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn project_switch_app() -> (App, tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
        let project_a = tempdir().unwrap();
        let project_b = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        fs::write(
            project_a.path().join("a.feature"),
            "Feature: Project A\n\n  Scenario: A\n    Given project A\n",
        )
        .unwrap();
        fs::write(
            project_b.path().join("b.feature"),
            "Feature: Project B\n\n  Scenario: B\n    Given project B\n",
        )
        .unwrap();
        fs::write(
            project_a.path().join("teshi.toml"),
            "[runner]\ncmd = \"project-a-runner\"\n",
        )
        .unwrap();
        fs::write(
            project_b.path().join("teshi.toml"),
            "[runner]\ncmd = \"project-b-runner\"\n",
        )
        .unwrap();
        let mut app = App::from_file(
            &project_a.path().join("a.feature"),
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        stop_fixture_agents(&mut app);
        (app, project_a, project_b, store)
    }

    #[test]
    fn session_restore_enforces_ownership_and_explicit_legacy_access() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        let mut session = crate::session::Session::from_messages(
            vec![AiChatMessage {
                role: AiRole::User,
                content: "owned history".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: None,
            }],
            None,
        );
        session.project_root = Some(project_b.path().to_path_buf());
        assert!(!app.restore_session(session.clone(), false));
        assert!(app.status.contains("another project"));
        assert!(app.agent().messages().is_empty());
        assert!(app.agent().saved_session.is_none());

        session.project_root = None;
        assert!(!app.restore_session(session.clone(), false));
        assert!(app.status.contains("/sessions legacy"));
        assert!(app.agent().messages().is_empty());
        assert!(app.restore_session(session.clone(), true));
        let checkpoints = tempdir().unwrap();
        app.agent_mut()
            .save_session_to(checkpoints.path(), project_a.path(), None)
            .unwrap();
        let saved = crate::session::Session::load_from_file(
            &checkpoints.path().join(format!("{}.json", session.id)),
        )
        .unwrap();
        assert!(
            saved.project_root.is_none(),
            "legacy history must stay unassigned"
        );

        session.project_root = Some(project_a.path().join("."));
        assert!(!app.restore_session(session.clone(), true));
        assert!(app.restore_session(session, false));
        assert_eq!(app.agent().messages()[0].content, "owned history");
        assert!(
            app.agent_mut()
                .save_session_to(checkpoints.path(), project_b.path(), None)
                .is_err(),
            "checkpoint must enforce ownership independently of restore"
        );
    }

    #[test]
    fn deferred_history_restore_is_bound_to_agent_project_and_cancel() {
        let (mut app, project_a, _project_b, _store) = project_switch_app();
        let mut session = crate::session::Session::from_messages(Vec::new(), None);
        session.project_root = Some(project_a.path().to_path_buf());
        let stamp = app.current_project_stamp();
        app.pending_session_restore =
            Some((app.agent().id + 1, stamp.clone(), session.clone(), false));
        app.poll_project_switch();
        assert!(app.agent().saved_session.is_none());

        let mut stale = stamp.clone();
        stale.generation = stale.generation.next();
        app.pending_session_restore = Some((app.agent().id, stale, session.clone(), false));
        app.poll_project_switch();
        assert!(app.agent().saved_session.is_none());

        app.pending_session_restore = Some((app.agent().id, stamp.clone(), session.clone(), false));
        app.handle_action(Action::SessionPanelClose).unwrap();
        app.poll_project_switch();
        assert!(app.agent().saved_session.is_none());

        app.pending_session_restore = Some((app.agent().id, stamp, session, false));
        app.poll_project_switch();
        assert!(app.agent().saved_session.is_some());
        assert!(app.pending_session_restore.is_none());
    }

    #[test]
    fn project_switch_reloads_links_to_the_same_global_requirement() {
        let (mut app, project_a, project_b, store) = project_switch_app();
        let body = "# Shared requirement\n\nGlobal verification intent.\n";
        let mut index: teshi_core::authoring::RequirementDocumentIndex =
            serde_json::from_slice(&fs::read(store.path().join("_teshi.json")).unwrap()).unwrap();
        index
            .documents
            .push(teshi_core::authoring::RequirementDocumentMeta::new(
                "shared",
                "shared.md",
                "Shared requirement",
                teshi_engine::compute_document_revision(body),
            ));
        fs::write(store.path().join("shared.md"), body).unwrap();
        let project_a_identity =
            teshi_engine::ensure_requirement_project(project_a.path()).unwrap();
        let project_b_identity =
            teshi_engine::ensure_requirement_project(project_b.path()).unwrap();
        index.documents[0]
            .projects
            .insert(project_a_identity.id, Default::default());
        index.documents[0]
            .projects
            .insert(project_b_identity.id, Default::default());
        teshi_engine::save_requirement_document_index(store.path(), &index).unwrap();
        let mut originals = Vec::new();
        for (project, point) in [(project_a.path(), "A-only"), (project_b.path(), "B-only")] {
            let path = project.join("testpoints/testpoints.json");
            let points = serde_json::json!({"version": 1, "test_points": [{
                "id": point, "title": point, "objective": "Verify intent",
                "hierarchy_path": ["Shared"], "review_state": "proposed",
                "requirement_links": [{
                    "store_id": index.store_id, "document_id": "shared",
                    "document_revision": index.documents[0].revision,
                    "position": {"start": 2, "end": 20},
                    "quote": {"quote": "Shared requirement"}, "resolution": "resolved"
                }]
            }]});
            teshi_engine::write_atomic(&path, &points).unwrap();
            originals.push((path.clone(), fs::read(&path).unwrap()));
        }
        let index_bytes = fs::read(store.path().join("_teshi.json")).unwrap();
        app.authoring_ui = crate::authoring_tab::AuthoringUiState::load_from_project(
            project_a.path(),
            store.path(),
        );
        app.authoring_ui.select_document_by_id("shared");
        app.authoring_ui.highlight_test_point_id = Some("A-only".into());
        app.authoring_ui.selected_linked_index = 9;
        assert_eq!(
            app.authoring_ui.linked_test_points_for_document()[0].id,
            "A-only"
        );

        app.request_project_switch(project_b.path()).unwrap();
        assert_eq!(app.requirements_root, store.path());
        assert_eq!(
            app.authoring_ui.selected_document_id.as_deref(),
            Some("shared")
        );
        assert_eq!(
            app.authoring_ui.linked_test_points_for_document()[0].id,
            "B-only"
        );
        assert!(app.authoring_ui.highlight_test_point_id.is_none());
        assert_eq!(app.authoring_ui.selected_linked_index, 0);
        stop_fixture_agents(&mut app);
        app.request_project_switch(project_a.path()).unwrap();
        assert_eq!(
            app.authoring_ui.linked_test_points_for_document()[0].id,
            "A-only"
        );
        for (path, bytes) in originals {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert_eq!(
            fs::read(store.path().join("_teshi.json")).unwrap(),
            index_bytes
        );
        assert_eq!(
            fs::read_to_string(store.path().join("shared.md")).unwrap(),
            body
        );
    }

    #[test]
    fn project_switch_commits_new_project_and_preserves_global_requirement_root() {
        let (mut app, _project_a, project_b, store) = project_switch_app();
        app.active_model_label = Some("Project A session model".into());
        let before = app.current_project_stamp();

        app.request_project_switch(project_b.path()).unwrap();
        assert!(app.project.features.is_empty());
        await_catalog(&mut app, 1);
        load_catalog_file(&mut app, 0);

        assert_eq!(app.project.root_dir, project_b.path());
        assert_eq!(
            app.project.features[0].file_path,
            project_b.path().join("b.feature")
        );
        assert_eq!(app.requirements_root, store.path());
        assert_eq!(
            app.runner_config.as_ref().map(|config| config.cmd.as_str()),
            Some("project-b-runner")
        );
        assert_eq!(
            app.project_switch.active.identity,
            teshi_engine::ProjectIdentity::new(project_b.path()).unwrap()
        );
        assert_eq!(
            app.project_switch.active.generation,
            before.generation.next()
        );
        assert!(app.project_switch.is_active());
        assert!(app.active_model_label.is_none());
        assert!(!app.project_switch.accepts(&before));
    }

    #[test]
    fn project_switch_clears_editor_history_and_source_bound_view_state() {
        let (mut app, _project_a, project_b, _store) = project_switch_app();
        let old = EditorBuffer::from_string("Feature: old project A history\n".to_string());
        app.undo_stack.push((old.clone(), 0, 0));
        app.redo_stack.push((old, 0, 0));
        app.scenario_fold.insert(99);
        app.selection_anchor = Some((99, 4));
        app.selection_end = Some((100, 8));
        app.step_input_active = true;
        app.step_keyword_picker = Some(super::StepKeywordPicker {
            buffer_row: 99,
            selected: 1,
        });
        app.pending_char = Some('d');
        app.explore_horizontal_scroll = [80, 90, 100];
        app.explore_horizontal_scroll_drag = Some(ColumnFocus::Step);
        app.explore_edit_mode = true;
        app.editor_focus_scenario_row = Some(99);
        app.scenario_dropdown_open = true;
        app.scenario_dropdown_selection = 99;
        let b_bytes = fs::read(project_b.path().join("b.feature")).unwrap();

        app.request_project_switch(project_b.path()).unwrap();

        assert!(app.undo_stack.is_empty(), "A's undo history survived in B");
        assert!(app.redo_stack.is_empty(), "A's redo history survived in B");
        assert!(app.scenario_fold.is_empty());
        assert!(app.selection_anchor.is_none());
        assert!(app.selection_end.is_none());
        assert!(!app.step_input_active);
        assert!(app.step_keyword_picker.is_none());
        assert!(app.pending_char.is_none());
        assert_eq!(app.explore_horizontal_scroll, [0; 3]);
        assert!(app.explore_horizontal_scroll_drag.is_none());
        assert!(!app.explore_edit_mode);
        assert!(app.editor_focus_scenario_row.is_none());
        assert!(!app.scenario_dropdown_open);
        assert_eq!(app.scenario_dropdown_selection, 0);
        let b_buffer = app.buffer.as_string();
        app.undo();
        app.redo();
        assert_eq!(app.buffer.as_string(), b_buffer);
        assert!(!app.dirty);
        assert_eq!(
            fs::read(project_b.path().join("b.feature")).unwrap(),
            b_bytes
        );
    }

    #[test]
    fn failed_project_preparation_keeps_editor_history_and_selection() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        fs::write(project_b.path().join("teshi.toml"), "[runner\n").unwrap();
        let old = "Feature: prior A edit\n";
        app.undo_stack
            .push((EditorBuffer::from_string(old.to_string()), 0, 0));
        app.redo_stack.push((app.buffer.clone(), 0, 0));
        app.selection_anchor = Some((0, 1));
        app.selection_end = Some((0, 3));
        app.scenario_fold.insert(2);
        app.request_project_switch(project_b.path()).unwrap();
        assert_eq!(app.project.root_dir, project_a.path());
        assert!(app.status.contains("Could not prepare project B"));
        assert_eq!(app.undo_stack.len(), 1);
        assert_eq!(app.redo_stack.len(), 1);
        assert_eq!(app.selection_anchor, Some((0, 1)));
        assert_eq!(app.selection_end, Some((0, 3)));
        assert!(app.scenario_fold.contains(&2));
        app.undo();
        assert_eq!(app.buffer.as_string(), old);
    }

    #[test]
    fn unreadable_target_source_preserves_a_edits_history_and_generation() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        fs::write(project_b.path().join("bad.feature"), [0xff]).unwrap();
        let draft = "Feature: Unsaved A\n";
        app.buffer = EditorBuffer::from_string(draft.to_string());
        app.buffer_dirty[0] = true;
        app.dirty = true;
        app.undo_stack.push((app.buffer.clone(), 0, 0));
        let stamp = app.current_project_stamp();
        app.request_project_switch(project_b.path()).unwrap();
        app.project_switch_action(Action::OverlayInsert('D'))
            .unwrap();
        assert_eq!(app.project.root_dir, project_a.path());
        assert_eq!(app.current_project_stamp(), stamp);
        assert_eq!(app.buffer.as_string(), draft);
        assert!(app.buffer_dirty[0]);
        assert_eq!(app.undo_stack.len(), 1);
        assert!(app.status.contains("bad.feature"), "{}", app.status);
        assert!(app.status.contains("Could not prepare project B"));
    }

    #[test]
    fn project_switch_cancel_keeps_dirty_project_a_and_missing_target_is_non_destructive() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        let before = app.current_project_stamp();
        app.buffer_dirty[0] = true;
        app.buffer = EditorBuffer::from_string("Feature: unsaved A\n".to_string());

        app.request_project_switch(project_b.path()).unwrap();
        assert!(matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::Dirty { .. }
        ));
        app.project_switch_action(Action::OverlayCancel).unwrap();
        assert_eq!(app.current_project_stamp(), before);
        assert_eq!(app.project.root_dir, project_a.path());
        assert!(app.buffer_dirty[0]);
        assert!(app.project_switch.is_active());

        let missing = project_b.path().join("does-not-exist");
        app.request_project_switch(missing).unwrap();
        assert_eq!(app.current_project_stamp(), before);
        assert_eq!(app.project.root_dir, project_a.path());
        assert!(app.status.contains("does not exist"));
    }

    #[test]
    fn project_switch_discard_does_not_destroy_a_before_b_preparation_succeeds() {
        let (mut app, project_a, _project_b, _store) = project_switch_app();
        let broken_project = tempdir().unwrap();
        fs::write(broken_project.path().join("teshi.toml"), "[runner\n").unwrap();
        let draft = "Feature: unsaved A\n  Scenario: Draft\n    Given local edits\n";
        app.buffer = EditorBuffer::from_string(draft.to_string());
        app.buffer_dirty[0] = true;
        app.dirty = true;

        app.request_project_switch(broken_project.path()).unwrap();
        assert!(matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::Dirty { .. }
        ));
        app.project_switch_action(Action::OverlayInsert('D'))
            .unwrap();

        assert_eq!(app.project.root_dir, project_a.path());
        assert_eq!(app.buffer.as_string(), draft);
        assert!(app.buffer_dirty[0]);
        assert!(app.dirty);
        assert!(app.project_switch.is_active());
        assert!(app.status.contains("Could not prepare project B"));
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| { message.contains("Could not prepare project B") })
        );
    }

    #[test]
    fn project_switch_busy_cancel_preserves_runner_and_project() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        let before = app.current_project_stamp();
        let (_events, rx) = std::sync::mpsc::channel();
        let (cancel, cancellation) = std::sync::mpsc::channel();
        app.runner_rx = Some(rx);
        app.runner_cancel = Some(cancel);
        app.request_project_switch(project_b.path()).unwrap();
        app.project_switch_action(Action::OverlayCancel).unwrap();

        assert_eq!(app.current_project_stamp(), before);
        assert_eq!(app.project.root_dir, project_a.path());
        assert!(app.runner_rx.is_some());
        assert!(cancellation.try_recv().is_err());
    }

    #[test]
    fn project_switch_during_human_review_never_approves_or_loses_test_points() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        start_mcp_fixture(&mut app, "mcp_review");
        wait_mcp_fixture(&mut app, |app| {
            app.agents[0].backend.status() == teshi_agent::backend::AgentBackendStatus::Completed
        });
        assert_eq!(
            app.generation_stage,
            teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
        );
        let points_path = project_a.path().join("testpoints/testpoints.json");
        let proposed = fs::read(&points_path).unwrap();
        let before = app.current_project_stamp();
        app.request_project_switch(project_b.path()).unwrap();
        assert!(matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::Busy { .. }
        ));
        app.project_switch_action(Action::OverlayCancel).unwrap();
        assert_eq!(app.current_project_stamp(), before);
        assert_eq!(
            app.generation_stage,
            teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
        );
        assert_eq!(fs::read(&points_path).unwrap(), proposed);

        app.request_project_switch(project_b.path()).unwrap();
        app.project_switch_action(Action::OverlayInsert('s'))
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.project.root_dir != project_b.path() {
            assert!(
                std::time::Instant::now() < deadline,
                "review switch did not finish"
            );
            app.poll_llm_events();
            app.poll_project_switch();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(fs::read(&points_path).unwrap(), proposed);
        assert!(!project_b.path().join("testpoints/testpoints.json").exists());
        assert!(!project_a.path().join("blocked.feature").exists());
        app.request_project_switch(project_a.path()).unwrap();
        let points = &app
            .authoring_ui
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points;
        assert_eq!(points.len(), 1);
        assert_eq!(
            points[0].review_state,
            teshi_core::authoring::ReviewState::Proposed
        );
        assert_eq!(fs::read(&points_path).unwrap(), proposed);
        stop_fixture_agents(&mut app);
    }

    #[test]
    fn project_switch_with_pending_permission_cancels_without_approval() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        app.approval_mode = teshi_agent::approval::ApprovalMode::Manual;
        start_mcp_fixture(&mut app, "permission_then_exit");
        wait_mcp_fixture(&mut app, App::has_acp_permission_prompt);
        let request_id = app
            .agent()
            .pending_acp_permission
            .as_ref()
            .unwrap()
            .request_id;
        let before = app.current_project_stamp();
        app.request_project_switch(project_b.path()).unwrap();
        assert!(matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::Busy { .. }
        ));
        app.project_switch_action(Action::OverlayCancel).unwrap();
        assert_eq!(app.current_project_stamp(), before);
        assert_eq!(app.project.root_dir, project_a.path());
        assert_eq!(
            app.agent()
                .pending_acp_permission
                .as_ref()
                .unwrap()
                .request_id,
            request_id
        );

        app.request_project_switch(project_b.path()).unwrap();
        app.project_switch_action(Action::OverlayInsert('s'))
            .unwrap();
        assert!(!app.has_acp_permission_prompt());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.project.root_dir != project_b.path() {
            assert!(
                std::time::Instant::now() < deadline,
                "permission switch did not finish"
            );
            app.poll_llm_events();
            app.poll_project_switch();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.has_acp_permission_prompt());
        assert!(app.agent().messages().is_empty());
        assert!(!app.current_project_stamp().eq(&before));
        stop_fixture_agents(&mut app);
    }

    #[test]
    fn project_switch_timeout_keeps_runner_owned_and_dirty_edits_visible() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        let before = app.current_project_stamp();
        let (_events, rx) = std::sync::mpsc::channel();
        let (cancel, cancellation) = std::sync::mpsc::channel();
        app.runner_rx = Some(rx);
        app.runner_cancel = Some(cancel);
        app.buffer_dirty[0] = true;
        app.buffer = EditorBuffer::from_string("Feature: recoverable draft\n".to_string());
        app.request_project_switch(project_b.path()).unwrap();
        app.project_switch_action(Action::OverlayInsert('s'))
            .unwrap();
        assert_eq!(cancellation.try_recv(), Ok(()));
        assert_eq!(app.current_project_stamp(), before);
        app.project_switch.start_quiesce_wait(Duration::ZERO);
        app.poll_project_switch();

        assert_eq!(app.project.root_dir, project_a.path());
        assert!(app.project_switch.is_active());
        assert!(app.runner_rx.is_some());
        assert!(app.runner_cancel.is_some());
        assert!(app.buffer_dirty[0]);
        assert!(app.buffer.as_string().contains("recoverable draft"));
        assert!(
            app.status_message
                .as_deref()
                .unwrap()
                .contains("did not exit")
        );
    }

    #[test]
    fn project_switch_waits_for_runner_exit_acknowledgement() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        let before = app.current_project_stamp();
        let (events, rx) = std::sync::mpsc::channel();
        let (cancel, cancellation) = std::sync::mpsc::channel();
        app.runner_rx = Some(rx);
        app.runner_cancel = Some(cancel);
        app.request_project_switch(project_b.path()).unwrap();
        app.project_switch_action(Action::OverlayInsert('s'))
            .unwrap();
        assert_eq!(cancellation.try_recv(), Ok(()));
        app.poll_runner_events();
        assert_eq!(app.current_project_stamp(), before);
        assert_eq!(app.project.root_dir, project_a.path());

        events
            .send(crate::runner::RunEvent::RunnerExit {
                code: Some(1),
                success: false,
            })
            .unwrap();
        app.poll_runner_events();
        assert_eq!(app.project.root_dir, project_b.path());
        assert_eq!(
            app.current_project_stamp().generation,
            before.generation.next()
        );
        assert!(app.runner_rx.is_none());
        assert!(app.runner_cancel.is_none());
        assert_eq!(app.runner_config.as_ref().unwrap().cmd, "project-b-runner");
    }

    #[test]
    fn project_switch_rechecks_busy_tasks_after_dirty_resolution() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        app.buffer_dirty[0] = true;
        app.request_project_switch(project_b.path()).unwrap();
        let (_events, rx) = std::sync::mpsc::channel();
        app.runner_rx = Some(rx);
        app.project_switch_action(Action::OverlayInsert('d'))
            .unwrap();

        assert_eq!(app.project.root_dir, project_a.path());
        assert!(app.buffer_dirty[0]);
        assert!(matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::Busy { .. }
        ));
    }

    #[test]
    fn stopping_project_tasks_preserves_unapplied_agent_proposal() {
        let (mut app, project_a, project_b, _store) = project_switch_app();
        app.queue_agent_change(super::AgentPendingChange {
            description: "retain proposed edit".into(),
            file_path: project_a.path().join("a.feature").display().to_string(),
            mutation: super::AgentMutation::ReplaceLine {
                row_0based: 0,
                new_text: "Feature: proposed A".into(),
            },
            scenario_name: String::new(),
            test_point_ids: Vec::new(),
            tool_call_id: "pending".into(),
            old_buffer_snapshot: app.buffer.as_string(),
            agent_idx: 0,
        });
        let (_events, rx) = std::sync::mpsc::channel();
        app.runner_rx = Some(rx);
        app.request_project_switch(project_b.path()).unwrap();
        app.project_switch_action(Action::OverlayInsert('s'))
            .unwrap();
        assert_eq!(app.pending_agent_changes.len(), 1);
        assert_eq!(app.pending_agent_changes[0].tool_call_id, "pending");
        assert_eq!(app.project.root_dir, project_a.path());
    }

    #[test]
    fn project_picker_mouse_row_switches_and_open_path_enters_path_mode() {
        let (mut app, _project_a, project_b, _store) = project_switch_app();
        app.project_switch
            .begin_picker(vec![teshi_engine::RecentProjectEntry {
                root: project_b.path().to_string_lossy().into_owned(),
                display_name: "Project B".to_string(),
                last_opened_ms: 1,
            }]);
        render_test_app(&mut app, 120);
        let recent = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::ProjectRecent { rect, .. } => Some(rect.to_owned()),
                _ => None,
            })
            .expect("recent project row should be clickable");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            recent.x + 1,
            recent.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(app.project.root_dir, project_b.path());

        app.project_switch.begin_picker(Vec::new());
        render_test_app(&mut app, 120);
        let open_path = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::ProjectOpenPath { rect } => Some(rect.to_owned()),
                _ => None,
            })
            .expect("open path row should be clickable");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            open_path.x + 1,
            open_path.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::OpenPath { .. }
        ));
    }

    #[test]
    fn project_picker_keeps_controls_and_pointer_rows_aligned_with_wrapped_paths() {
        for width in [120, 64] {
            let (mut app, _project_a, _project_b, _store) = project_switch_app();
            app.project_switch.begin_picker(
                (0..10)
                    .map(|index| teshi_engine::RecentProjectEntry {
                        root: format!("/{}-{index}/shared", "long-directory/".repeat(10)),
                        display_name: format!("Project {index}"),
                        last_opened_ms: index,
                    })
                    .collect(),
            );
            app.project_switch.move_picker(9);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let row_text = |row: u16| -> String {
                (0..width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect()
            };
            let open = app
                .clickable_regions
                .iter()
                .find_map(|region| match region {
                    ClickableRegion::ProjectOpenPath { rect } => Some(*rect),
                    _ => None,
                })
                .expect("Open path must remain visible after wrapping recent project paths");
            assert!(row_text(open.y).contains("[O] Open path"));
            let selected_rows = app
                .clickable_regions
                .iter()
                .filter_map(|region| match region {
                    ClickableRegion::ProjectRecent { index: 9, rect } => Some(*rect),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                selected_rows.len() > 1,
                "the selected path should wrap at width {width}"
            );
            assert!(
                selected_rows
                    .iter()
                    .any(|rect| row_text(rect.y).contains("Project 9"))
            );
            assert!(selected_rows.iter().all(|rect| rect.y < open.y));
            app.handle_mouse_event(
                MouseEventKind::Down(MouseButton::Left),
                open.x + 1,
                open.y,
                KeyModifiers::NONE,
            )
            .unwrap();
            assert!(matches!(
                app.project_switch.overlay,
                crate::project_context::SwitchOverlay::OpenPath { .. }
            ));
        }
    }

    #[test]
    fn session_checkpoints_keep_identity_partial_reply_and_backend() {
        let root = tempdir().unwrap();
        let project = tempdir().unwrap();
        let mut agent =
            AgentThread::new_with_backend(1, "ACP", teshi_agent::backend::AgentBackendKind::Acp);
        agent.selected_acp_id = Some("example".into());
        agent.messages_mut().push(super::AiChatMessage {
            role: AiRole::User,
            content: "question".into(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });
        agent.partial_response_mut().push_str("unfinished");
        agent
            .save_session_to(root.path(), project.path(), None)
            .unwrap();
        let id = agent.saved_session.as_ref().unwrap().id.clone();
        let path = root.path().join(format!("{id}.json"));
        let first = crate::session::Session::load_from_file(&path).unwrap();
        assert_eq!(first.messages.last().unwrap().content, "unfinished");
        assert_eq!(first.acp_agent_id.as_deref(), Some("example"));
        assert_eq!(first.project_root.as_deref(), Some(project.path()));
        agent.partial_response_mut().push_str(" reply");
        agent
            .save_session_to(root.path(), project.path(), None)
            .unwrap();
        let second = crate::session::Session::load_from_file(&path).unwrap();
        assert_eq!(second.id, id);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.messages.len(), 2);
        assert_eq!(second.messages.last().unwrap().content, "unfinished reply");
        assert_eq!(
            agent.messages().len(),
            1,
            "checkpoint must not mutate model history"
        );

        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        fs::write(&feature, "Feature: Sample\n").unwrap();
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        app.chat_selection = Some(ChatSelection {
            agent_id: 0,
            anchor: ChatAnchor {
                block: 0,
                offset: 0,
                fingerprint: 1,
            },
            focus: ChatAnchor {
                block: 0,
                offset: 1,
                fingerprint: 1,
            },
        });
        stop_fixture_agents(&mut app);
        assert!(app.restore_session(second, false));
        assert!(app.chat_selection.is_none());
        assert_eq!(
            app.agent().backend_kind(),
            teshi_agent::backend::AgentBackendKind::Acp
        );
        assert_eq!(app.agent().selected_acp_id.as_deref(), Some("example"));
        assert_eq!(app.agent().saved_session.as_ref().unwrap().id, id);
    }

    #[test]
    fn failed_session_checkpoint_retains_previous_identity() {
        let root = tempdir().unwrap();
        let project = tempdir().unwrap();
        let mut agent = AgentThread::new_with_backend(
            1,
            "Native",
            teshi_agent::backend::AgentBackendKind::Native,
        );
        agent.messages_mut().push(super::AiChatMessage {
            role: AiRole::User,
            content: "question".into(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });
        let blocked = root.path().join("file");
        fs::write(&blocked, "not a directory").unwrap();
        assert!(
            agent
                .save_session_to(&blocked, project.path(), None)
                .is_err()
        );
        assert!(agent.saved_session.is_none());
        assert!(agent.last_session_snapshot.is_empty());
        agent
            .save_session_to(root.path(), project.path(), None)
            .unwrap();
        assert!(agent.saved_session.is_some());
    }

    #[test]
    fn unfinished_checkpoint_keeps_progress_style_after_restore_for_both_backends() {
        use teshi_agent::backend::AgentBackendKind;
        for kind in [AgentBackendKind::Native, AgentBackendKind::Acp] {
            let (mut app, project, _store) = slash_test_app();
            let storage = tempdir().unwrap();
            let mut agent = AgentThread::new_with_backend(0, "Controlled", kind);
            agent.messages_mut().push(super::AiChatMessage {
                role: AiRole::User,
                content: "Inspect".into(),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                source: None,
            });
            agent
                .partial_response_mut()
                .push_str("Partial result completed");
            agent
                .save_session_to(storage.path(), project.path(), None)
                .unwrap();
            let id = agent.saved_session.as_ref().unwrap().id.clone();
            // Read the actual checkpoint, without relying on a graceful-exit save.
            let saved =
                crate::session::Session::load_from_file(&storage.path().join(format!("{id}.json")))
                    .unwrap();
            stop_fixture_agents(&mut app);
            assert!(app.restore_session(saved, false));
            assert_eq!(app.agent().backend_kind(), kind);
            let reply = app.agent().messages().last().unwrap();
            assert_eq!(reply.content, "Partial result completed");
            assert_eq!(reply.source.as_deref(), Some("agent_progress"));
            assert!(app.agent().partial_response().is_empty());
        }
    }

    #[test]
    fn tui_runner_failure_events_preserve_step_and_screen_attachment_in_explore() {
        let (mut app, _project, _store) = slash_test_app();
        app.explore_case_map.insert("f0:s0".into(), (0, 0));
        for line in [
            r#"{"type":"start_run","total":1}"#,
            r#"{"type":"start_case","case_id":"f0:s0","name":"Sample"}"#,
            r#"{"type":"start_step","case_id":"f0:s0","step_id":"3","text":"expected content is visible"}"#,
            r#"{"type":"end_step","case_id":"f0:s0","step_id":"3","status":"failed","message":"timed out waiting for content"}"#,
            r#"{"type":"case_failed","case_id":"f0:s0","duration_ms":50,"error":{"message":"TUI step at line 3: timed out","attachments":[{"type":"text","path":"owned-case/screen.txt"}]}}"#,
            r#"{"type":"end_run","passed":0,"failed":1,"skipped":0}"#,
        ] {
            app.apply_run_event(
                crate::runner::parse_event_line(line).expect("valid native TUI event"),
            );
        }
        assert_eq!(app.explore_run_summary.as_ref().unwrap().failed, 1);
        assert_eq!(
            app.explore_case_status.get(&(0, 0)),
            Some(&super::RunStatus::Failed)
        );
        let detail = app.explore_case_details.get(&(0, 0)).unwrap();
        assert_eq!(detail.steps[0].status.as_deref(), Some("failed"));
        assert_eq!(
            detail.steps[0].message.as_deref(),
            Some("timed out waiting for content")
        );
        assert_eq!(detail.attachments[0].kind, "text");
        assert_eq!(detail.attachments[0].path, "owned-case/screen.txt");
        assert_eq!(detail.duration_ms, Some(50));
    }

    #[test]
    fn selected_acp_backend_does_not_fall_back_to_native() {
        let (mut app, _project, _store) = slash_test_app();
        app.agents[0] = super::AgentThread::new_with_backend(
            0,
            "ACP",
            teshi_agent::backend::AgentBackendKind::Acp,
        );
        app.dispatch_user_message("hello".into()).unwrap();
        assert_eq!(
            app.agents[0].backend.kind(),
            teshi_agent::backend::AgentBackendKind::Acp
        );
        assert_ne!(
            app.agents[0].backend.kind(),
            teshi_agent::backend::AgentBackendKind::Native
        );
        if std::env::var_os("TESHI_ACP_PROGRAM").is_none() {
            assert!(
                app.status.contains("No ACP Agent selected"),
                "{}",
                app.status
            );
            assert_eq!(app.agents[0].status, AiStatus::Error);
        }
    }

    #[test]
    fn newline_action_inserts_a_literal_line_break() {
        let (mut app, _project, _store) = slash_test_app();
        app.agent_mut().input = "first line".into();
        app.agent_mut().input_cursor = app.agent().input.chars().count();
        app.handle_action(Action::AiNewline).unwrap();
        app.agent_mut().input.push_str("second line");
        assert_eq!(app.agent().input, "first line\nsecond line");
    }

    #[test]
    fn agent_selection_survives_new_agent_new_session_and_restart() {
        let (mut app, _project, _store) = slash_test_app();
        let preferences = tempdir().unwrap();
        app.agent_mut().selection_root = Some(preferences.path().to_path_buf());
        assert_eq!(
            app.agent().backend.kind(),
            teshi_agent::backend::AgentBackendKind::Native
        );
        app.installed_acp_agents = vec![crate::acp_agents::InstalledAgent {
            id: "fixture".into(),
            name: "Fixture".into(),
            program: "fixture".into(),
            args: vec![],
            env: Default::default(),
            env_from: Default::default(),
        }];
        teshi_acp::profile::upsert(preferences.path(), app.installed_acp_agents[0].clone())
            .unwrap();
        app.agent_panel_mode = AgentPanelMode::SelectBackend;
        app.agent_profile_panel_active = true;
        app.handle_action(Action::AgentPanelDown).unwrap();
        app.handle_action(Action::AgentPanelSelect).unwrap();
        assert_eq!(app.agent().selected_acp_id.as_deref(), Some("fixture"));
        assert_eq!(
            app.agent().backend.kind(),
            teshi_agent::backend::AgentBackendKind::Acp
        );
        app.execute_slash_command("new").unwrap();
        assert_eq!(app.agent().selected_acp_id.as_deref(), Some("fixture"));
        assert_eq!(
            app.agent().backend.kind(),
            teshi_agent::backend::AgentBackendKind::Acp
        );
        app.handle_action(Action::AgentNew).unwrap();
        assert_eq!(app.agent().selected_acp_id.as_deref(), Some("fixture"));
        let restarted =
            AgentThread::new_with_selection(9, "Restarted", Some(preferences.path().into()));
        assert_eq!(restarted.selected_acp_id.as_deref(), Some("fixture"));
        assert!(restarted.messages().is_empty());
        app.select_backend(None);
        let restarted =
            AgentThread::new_with_selection(10, "Native", Some(preferences.path().into()));
        assert_eq!(
            restarted.backend_kind(),
            teshi_agent::backend::AgentBackendKind::Native
        );
        assert_eq!(restarted.selected_acp_id, None);
        crate::agent_selection::save(
            preferences.path(),
            &crate::agent_selection::Selection::Acp {
                agent_id: "removed".into(),
            },
        )
        .unwrap();
        let restarted =
            AgentThread::new_with_selection(11, "Removed", Some(preferences.path().into()));
        assert_eq!(
            restarted.backend_kind(),
            teshi_agent::backend::AgentBackendKind::Native
        );
        assert!(restarted.selection_warning().unwrap().contains("removed"));
        *app.agent_mut() = restarted;
        app.show_selection_warning();
        assert!(app.agent().selection_warning().is_some());
        assert!(app.status.contains("unavailable"));
        app.select_backend(None);
        assert!(app.agent().selection_warning().is_none());
    }

    #[test]
    fn restoring_history_does_not_replace_explicit_agent_preference() {
        let (mut app, _project, _store) = slash_test_app();
        let root = tempdir().unwrap();
        app.agent_mut().selection_root = Some(root.path().into());
        crate::agent_selection::save(
            root.path(),
            &crate::agent_selection::Selection::Acp {
                agent_id: "beta".into(),
            },
        )
        .unwrap();
        let mut session = crate::session::Session::from_messages(Vec::new(), None);
        session.backend_kind = Some(teshi_agent::backend::AgentBackendKind::Acp);
        session.acp_agent_id = Some("alpha".into());
        session.project_root = Some(app.project.root_dir.clone());
        stop_fixture_agents(&mut app);
        assert!(app.restore_session(session, false));
        assert_eq!(app.agent().selected_acp_id.as_deref(), Some("alpha"));
        assert_eq!(
            crate::agent_selection::load(root.path()).unwrap(),
            Some(crate::agent_selection::Selection::Acp {
                agent_id: "beta".into()
            })
        );
        app.select_backend(None);
        assert_eq!(
            crate::agent_selection::load(root.path()).unwrap(),
            Some(crate::agent_selection::Selection::Native)
        );
    }

    #[test]
    fn registry_panel_renders_description_and_install_state() {
        let (mut app, _project, _store) = slash_test_app();
        app.agent_panel_mode = AgentPanelMode::Registry;
        app.agent_profile_panel_active = true;
        app.acp_registry_agents = teshi_acp::registry::Registry::parse(
            r#"{"agents":[{"id":"fixture","name":"Fixture Agent","description":"Reads test files","distribution":{"npx":{"package":"fixture"}}}]}"#
        ).unwrap().agents;
        app.installed_acp_agents = vec![crate::acp_agents::InstalledAgent {
            id: "fixture".into(),
            name: "Fixture Agent".into(),
            program: "fixture".into(),
            args: vec![],
            env: Default::default(),
            env_from: Default::default(),
        }];
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let display = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(display.contains("Fixture Agent (fixture) [installed]"));
        assert!(display.contains("Reads test files"));
    }

    #[test]
    fn installed_acp_panel_enter_selects_agent_backend() {
        let (mut app, _project, _store) = slash_test_app();
        let installed = |id: &str| crate::acp_agents::InstalledAgent {
            id: id.into(),
            name: id.into(),
            program: id.into(),
            args: vec![],
            env: Default::default(),
            env_from: Default::default(),
        };
        app.installed_acp_agents = vec![installed("first"), installed("second")];
        app.agent_panel_mode = AgentPanelMode::InstalledAcp;
        app.agent_profile_panel_active = true;
        // Rows have no "Teshi Native" entry, so one Down lands on the second agent.
        app.handle_action(Action::AgentPanelDown).unwrap();
        app.handle_action(Action::AgentPanelSelect).unwrap();
        assert_eq!(app.agent().selected_acp_id.as_deref(), Some("second"));
        assert_eq!(
            app.agent().backend.kind(),
            teshi_agent::backend::AgentBackendKind::Acp
        );
        assert!(!app.agent_profile_panel_active);
    }

    #[test]
    fn native_stream_chunks_complete_in_order() {
        let (mut app, _project, _store) = slash_test_app();
        let (tx, rx) = mpsc::channel();
        app.agents[0]
            .backend
            .attach_model_event_receiver_for_tests(rx);
        app.agents[0].backend.start().unwrap();
        app.agents[0].status = AiStatus::Waiting;
        tx.send(crate::llm::LlmEvent::Chunk {
            content: "first ".into(),
        })
        .unwrap();
        tx.send(crate::llm::LlmEvent::Chunk {
            content: "second".into(),
        })
        .unwrap();
        tx.send(crate::llm::LlmEvent::Done {
            full_text: "first second".into(),
            reasoning_content: None,
            model: "mock".into(),
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        })
        .unwrap();
        app.poll_llm_events();
        assert_eq!(
            app.agents[0].messages().last().unwrap().content,
            "first second"
        );
        assert_eq!(app.agents[0].status, AiStatus::Idle);
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::Completed
        );
    }

    #[test]
    fn cancelled_native_turn_ignores_late_tool_calls() {
        let (mut app, _project, _store) = slash_test_app();
        let (tx, rx) = mpsc::channel();
        app.agents[0]
            .backend
            .attach_model_event_receiver_for_tests(rx);
        app.agents[0].backend.start().unwrap();
        tx.send(crate::llm::LlmEvent::ToolCallRequest {
            tool_calls: vec![crate::llm::ToolCall {
                id: "late".into(),
                name: "unknown_tool".into(),
                arguments: "{}".into(),
                execution_duration_ms: None,
            }],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        })
        .unwrap();
        app.agents[0].backend.cancel();
        app.poll_llm_events();
        assert!(app.agents[0].messages().is_empty());
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::Cancelled
        );
    }

    #[test]
    fn native_tool_error_stays_in_history_before_followup() {
        let (mut app, _project, _store) = slash_test_app();
        let (tx, rx) = mpsc::channel();
        app.agents[0]
            .backend
            .attach_model_event_receiver_for_tests(rx);
        app.agents[0].backend.start().unwrap();
        tx.send(crate::llm::LlmEvent::ToolCallRequest {
            tool_calls: vec![crate::llm::ToolCall {
                id: "bad".into(),
                name: "unknown_tool".into(),
                arguments: "{}".into(),
                execution_duration_ms: None,
            }],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        })
        .unwrap();
        app.poll_llm_events();
        assert_eq!(app.agents[0].messages().len(), 2);
        assert_eq!(app.agents[0].messages()[0].role, AiRole::Assistant);
        assert_eq!(app.agents[0].messages()[1].role, AiRole::Tool);
        assert_eq!(
            app.agents[0].messages()[1].tool_call_id.as_deref(),
            Some("bad")
        );
        assert!(app.agents[0].messages()[1].content.starts_with("Error:"));
    }

    #[test]
    fn native_review_gate_stops_followup_after_tool_event() {
        let (mut app, _project, _store) = slash_test_app();
        let (tx, rx) = mpsc::channel();
        app.agents[0]
            .backend
            .attach_model_event_receiver_for_tests(rx);
        app.agents[0].backend.start().unwrap();
        app.generation_stage = teshi_agent::pipeline::GenerationStage::ReviewingTestPoints;
        tx.send(crate::llm::LlmEvent::ToolCallRequest {
            tool_calls: vec![crate::llm::ToolCall {
                id: "proposal".into(),
                name: "unknown_tool".into(),
                arguments: "{}".into(),
                execution_duration_ms: None,
            }],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        })
        .unwrap();
        app.poll_llm_events();
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::WaitingForHumanReview
        );
        assert_eq!(app.agents[0].status, AiStatus::Idle);
        assert_eq!(app.agents[0].messages().len(), 2);
    }

    fn start_mcp_fixture(app: &mut App, mode: &str) {
        use teshi_agent_runtime::{backend::AcpBackendConfig, tool_bridge::ToolBridgeConfig};
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py");
        let command = teshi_acp::AcpAgentCommand {
            program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
            args: vec![script.display().to_string(), mode.into()],
            cwd: app.find_project_dir().to_path_buf(),
            env: std::collections::HashMap::new(),
        };
        let mut config = AcpBackendConfig::new(command);
        config.client.shutdown_timeout = Duration::from_millis(100);
        config.permission_host = Some(teshi_agent_runtime::ChannelPermissionHost::new());
        config.tool_bridge = Some(ToolBridgeConfig {
            executable: std::env::current_exe().unwrap(),
            tools: app.agent_tools(0),
        });
        app.agents[0].backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
            teshi_agent::backend::AgentBackendKind::Acp,
            1,
        );
        app.agents[0].backend.configure_acp(config).unwrap();
        app.agents[0].backend.start().unwrap();
        let tools = app.agent_tools(0);
        app.agents[0]
            .backend
            .submit_turn(
                None,
                vec![crate::llm::ChatMessage::text("user", "Generate a feature")],
                Some(tools),
            )
            .unwrap();
    }

    fn wait_mcp_fixture(app: &mut App, condition: impl Fn(&App) -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.poll_llm_events();
            if condition(app) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "ACP fixture timed out: {}",
                app.status
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn interrupted_acp_task_continues_in_same_session() {
        let (mut app, _project, _store) = slash_test_app();
        start_mcp_fixture(&mut app, "refusal_once");
        wait_mcp_fixture(&mut app, |app| app.agent().task_interrupted());
        assert!(app.status.contains("Task interrupted"));
        assert!(
            app.agent()
                .messages()
                .iter()
                .any(|m| m.content.contains("agent returned refusal"))
        );
        let session = app.agent().backend.acp_controls().unwrap().session_id;
        app.active_tab = MainTab::Ai;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        assert!(app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ActionButton {
                action: Action::AiContinueTask,
                ..
            }
        )));
        for reason in ["end_turn", "cancelled"] {
            app.agent_mut().last_finish_reason = Some(reason.into());
            assert!(!app.agent().task_interrupted());
        }
        for reason in ["max_tokens", "max_turn_requests", "refusal"] {
            app.agent_mut().last_finish_reason = Some(reason.into());
            assert!(app.agent().task_interrupted());
        }
        app.agent_mut().input = "keep this draft".into();
        app.handle_action(Action::AiContinueTask).unwrap();
        // A second click cannot enqueue another turn while one is running.
        app.handle_action(Action::AiContinueTask).unwrap();
        wait_mcp_fixture(&mut app, |app| {
            app.agent().last_finish_reason.as_deref() == Some("end_turn")
        });
        assert!(!app.agent().task_interrupted());
        assert_eq!(
            app.agent().backend.acp_controls().unwrap().session_id,
            session
        );
        assert_eq!(app.agent().input, "keep this draft");
        assert_eq!(
            app.agent()
                .messages()
                .iter()
                .filter(|m| m.role == AiRole::User)
                .count(),
            1
        );
        assert!(
            app.agent()
                .messages()
                .iter()
                .any(|m| m.content == "Continued in original session")
        );
        app.execute_slash_command("retry").unwrap();
        assert_eq!(app.status, "No interrupted ACP task to continue");
        stop_fixture_agents(&mut app);
    }

    #[test]
    fn acp_mcp_tools_persist_test_points_and_enforce_human_review() {
        for mode in [
            teshi_agent::approval::ApprovalMode::Manual,
            teshi_agent::approval::ApprovalMode::Auto,
            teshi_agent::approval::ApprovalMode::Bypass,
        ] {
            let (mut app, project, _store) = requirements_test_app();
            app.approval_mode = mode;
            start_mcp_fixture(&mut app, "mcp_review");
            wait_mcp_fixture(&mut app, |app| {
                app.agents[0].backend.status()
                    == teshi_agent::backend::AgentBackendStatus::Completed
            });
            assert_eq!(
                app.generation_stage,
                teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
            );
            let points = &app
                .authoring_ui
                .artifacts
                .as_ref()
                .unwrap()
                .test_points
                .test_points;
            assert_eq!(points.len(), 1);
            assert_eq!(
                points[0].review_state,
                teshi_core::authoring::ReviewState::Proposed
            );
            assert!(project.path().join("testpoints/testpoints.json").exists());
            assert!(!project.path().join("blocked.feature").exists());
            let result: serde_json::Value =
                serde_json::from_str(&app.agents[0].messages().last().unwrap().content).unwrap();
            assert_eq!(result[0]["isError"], false);
            assert_eq!(result[1]["isError"], false);
            assert_eq!(result[2]["isError"], true);
            assert_eq!(result[3]["isError"], true);
            // Use the same review state and continue action that the TUI uses;
            // an external agent must not be able to grant this approval itself.
            assert!(crate::test_points_tab::TestPointsUiState::approve(
                &mut app
                    .authoring_ui
                    .artifacts
                    .as_mut()
                    .unwrap()
                    .test_points
                    .test_points[0]
            ));
            app.approval_mode = teshi_agent::approval::ApprovalMode::Auto;
            app.active_tab = MainTab::TestPoints;
            app.handle_action(Action::TpContinueGeneration).unwrap();
            wait_mcp_fixture(&mut app, |app| {
                app.agents[0].backend.status()
                    == teshi_agent::backend::AgentBackendStatus::Completed
            });
            let result: serde_json::Value =
                serde_json::from_str(&app.agents[0].messages().last().unwrap().content).unwrap();
            for (index, result) in result.as_array().unwrap().iter().enumerate() {
                assert_eq!(result["isError"], false, "tool {index}: {result}");
            }
            let index = app.find_feature_idx_for_file("approved.feature").unwrap();
            let feature = app.buffers[index].as_string();
            assert!(
                feature.contains("Scenario: Valid login"),
                "generated feature: {feature}"
            );
            assert!(feature.contains("Then access is granted"));
            app.active_tab = MainTab::Explore;
            app.handle_action(Action::Save).unwrap();
            assert_eq!(
                fs::read_to_string(project.path().join("approved.feature")).unwrap(),
                feature
            );
        }
    }

    #[test]
    fn acp_mcp_file_changes_wait_for_acceptance_and_report_rejection() {
        for accept in [true, false] {
            let (mut app, project, _store) = requirements_test_app();
            app.approval_mode = teshi_agent::approval::ApprovalMode::Manual;
            start_mcp_fixture(&mut app, "mcp_file");
            wait_mcp_fixture(&mut app, App::has_agent_change_prompt);
            assert!(!project.path().join("mcp-created.feature").exists());
            assert_eq!(app.agents[0].status, AiStatus::AwaitingApproval);
            app.handle_action(if accept {
                Action::AgentChangeAccept
            } else {
                Action::AgentChangeReject
            })
            .unwrap();
            wait_mcp_fixture(&mut app, |app| {
                app.agents[0].backend.status()
                    == teshi_agent::backend::AgentBackendStatus::Completed
            });
            assert_eq!(project.path().join("mcp-created.feature").exists(), accept);
            let result: serde_json::Value =
                serde_json::from_str(&app.agents[0].messages().last().unwrap().content).unwrap();
            assert_eq!(result[0]["isError"], !accept);
        }
    }

    #[test]
    fn cancelled_acp_mcp_file_change_cannot_be_accepted() {
        let (mut app, project, _store) = requirements_test_app();
        app.approval_mode = teshi_agent::approval::ApprovalMode::Manual;
        start_mcp_fixture(&mut app, "mcp_file");
        wait_mcp_fixture(&mut app, App::has_agent_change_prompt);
        app.cancel_agent_turn();
        assert!(app.accept_agent_change().is_err());
        assert!(!project.path().join("mcp-created.feature").exists());
        app.reject_agent_change();
    }

    #[test]
    fn acp_mcp_manual_write_failure_returns_error_and_clears_approval() {
        let (mut app, project, _store) = requirements_test_app();
        app.approval_mode = teshi_agent::approval::ApprovalMode::Manual;
        start_mcp_fixture(&mut app, "mcp_file");
        wait_mcp_fixture(&mut app, App::has_agent_change_prompt);
        // Make the queued destination unwritable as a file after proposal.
        fs::create_dir(project.path().join("mcp-created.feature")).unwrap();
        app.handle_action(Action::AgentChangeAccept).unwrap();
        assert!(!app.has_agent_change_prompt());
        assert!(app.pending_change_diffs.is_empty());
        assert!(app.pending_change_summary.is_empty());
        assert!(app.status.contains("Could not apply ACP file change"));
        wait_mcp_fixture(&mut app, |app| {
            app.agents[0].backend.status() == teshi_agent::backend::AgentBackendStatus::Completed
        });
        let result: serde_json::Value =
            serde_json::from_str(&app.agents[0].messages().last().unwrap().content).unwrap();
        assert_eq!(result[0]["isError"], true);
        assert!(
            result[0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("failed to create")
        );
        assert!(project.path().join("mcp-created.feature").is_dir());
    }

    #[test]
    fn acp_mcp_profile_switch_rediscovers_added_and_removed_tools() {
        let (mut app, project, _store) = requirements_test_app();
        for (id, tools) in [("mcp-chat", "[]"), ("mcp-author", "[submit_requirements]")] {
            let root = project.path().join(".teshi/agents").join(id);
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("agent.yaml"), format!("id: {id}\nname: {id}\ndescription: MCP profile test\nmodel_ref: ''\ntools: {tools}\n")).unwrap();
        }
        app.agent_registry = teshi_agent::registry::AgentRegistry::load(Some(project.path()));
        assert!(app.agent_registry.get("mcp-chat").is_some());
        app.agents[0].profile_id = Some("mcp-chat".into());
        start_mcp_fixture(&mut app, "mcp_catalog");
        wait_mcp_fixture(&mut app, |app| {
            app.agents[0].backend.status() == teshi_agent::backend::AgentBackendStatus::Completed
        });
        let initial: serde_json::Value =
            serde_json::from_str(&app.agents[0].messages().last().unwrap().content).unwrap();
        assert_eq!(initial["tools"], serde_json::json!([]));
        for (id, expected) in [
            ("mcp-author", serde_json::json!(["submit_requirements"])),
            ("mcp-chat", serde_json::json!([])),
        ] {
            let previous_revision = app.agents[0].backend.acp_controls().unwrap().revision;
            app.agent_panel_mode = AgentPanelMode::List;
            app.agent_profile_panel_active = true;
            app.agent_profile_panel_selection = app
                .agent_registry
                .iter()
                .position(|profile| profile.id == id)
                .unwrap();
            app.handle_action(Action::AgentPanelSelect).unwrap();
            assert_eq!(app.agents[0].profile_id.as_deref(), Some(id));
            assert!(app.agents[0].backend.acp_controls().unwrap().revision > previous_revision);
            let tools = app.agent_tools(0);
            app.agents[0].backend.start().unwrap();
            app.agents[0]
                .backend
                .submit_turn(
                    None,
                    vec![
                        crate::llm::ChatMessage::text("assistant", "Previous conversation"),
                        crate::llm::ChatMessage::text("user", "List permitted tools"),
                    ],
                    Some(tools),
                )
                .unwrap();
            wait_mcp_fixture(&mut app, |app| {
                app.agents[0].backend.status()
                    == teshi_agent::backend::AgentBackendStatus::Completed
            });
            let result: serde_json::Value =
                serde_json::from_str(&app.agents[0].messages().last().unwrap().content).unwrap();
            assert_eq!(result["tools"], expected);
            assert_eq!(result["restored"], true);
        }
    }

    #[test]
    fn native_propose_test_points_enters_human_review_without_followup() {
        let (mut app, _project, _store) = requirements_test_app();
        teshi_agent::execute_tool(
            &mut app,
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "requirements",
            0,
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        app.agents[0]
            .backend
            .attach_model_event_receiver_for_tests(rx);
        app.agents[0].backend.start().unwrap();
        tx.send(crate::llm::LlmEvent::ToolCallRequest {
            tool_calls: vec![crate::llm::ToolCall {
                id: "proposal".into(),
                name: "propose_test_points".into(),
                arguments: r#"{"test_points":[{"title":"Valid login","objective":"User authenticates","hierarchy_path":["Auth","Login"]}]}"#.into(),
                execution_duration_ms: None,
            }],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        }).unwrap();
        app.poll_llm_events();
        assert_eq!(
            app.generation_stage,
            teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
        );
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::WaitingForHumanReview
        );
        assert_eq!(app.agents[0].status, AiStatus::Idle);
        assert_eq!(
            app.authoring_ui
                .artifacts
                .as_ref()
                .unwrap()
                .test_points
                .test_points
                .len(),
            1
        );
    }

    #[test]
    fn native_model_proposal_pauses_before_another_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0u8; 8192];
            assert!(socket.read(&mut request).unwrap() > 0);
            let args = serde_json::json!({"test_points":[{"title":"Valid login","objective":"User authenticates","hierarchy_path":["Auth","Login"]}]}).to_string();
            let body = serde_json::json!({"model":"mock","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"proposal","type":"function","function":{"name":"propose_test_points","arguments":args}}]}}]}).to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            listener.set_nonblocking(true).unwrap();
            thread::sleep(Duration::from_millis(200));
            listener.accept().is_ok()
        });
        let (mut app, _project, _store) = requirements_test_app();
        teshi_agent::execute_tool(
            &mut app,
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "requirements",
            0,
        )
        .unwrap();
        app.agents[0]
            .backend
            .attach_native_model(crate::llm::LlmConfig {
                api_key: "test-only".into(),
                base_url: format!("http://{addr}/v1"),
                model: "mock".into(),
                max_tokens: 256,
                temperature: 0.0,
                context_window: None,
                provider: "openai".into(),
                thinking: teshi_engine::DeepSeekThinking::High,
                api_style: teshi_engine::ApiStyle::ChatCompletions,
                stream: false,
                http_headers: Default::default(),
                chat_options: Default::default(),
            })
            .unwrap();
        app.agents[0].backend.start().unwrap();
        app.agents[0]
            .backend
            .submit_turn(
                Some("test".into()),
                vec![],
                Some(teshi_agent::get_tools(None)),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && app.agents[0].backend.status() == teshi_agent::backend::AgentBackendStatus::Running
        {
            app.poll_llm_events();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.generation_stage,
            teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
        );
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::WaitingForHumanReview
        );
        app.active_tab = MainTab::TestPoints;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        assert!(
            !server.join().unwrap(),
            "human review must prevent another model request"
        );
    }

    #[test]
    fn native_model_multiple_tool_round_trips() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for round in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut received = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    received.extend_from_slice(&buffer[..n]);
                    if let Some(header_end) =
                        received.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&received[..header_end]);
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if received.len() >= header_end + 4 + length {
                            requests.push(
                                serde_json::from_slice::<serde_json::Value>(
                                    &received[header_end + 4..header_end + 4 + length],
                                )
                                .unwrap(),
                            );
                            break;
                        }
                    }
                }
                let body = if round < 2 {
                    let call_id = format!("call-{}", round + 1);
                    serde_json::json!({"model":"mock","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":call_id,"type":"function","function":{"name":"get_project_info","arguments":"{}"}}]}}]})
                } else {
                    serde_json::json!({"model":"mock","choices":[{"message":{"role":"assistant","content":"finished"}}]})
                }.to_string();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
            requests
        });

        let (mut app, _project, _store) = slash_test_app();
        app.agents[0].backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
            teshi_agent::backend::AgentBackendKind::Native,
            1,
        );
        app.agents[0]
            .backend
            .attach_native_model(crate::llm::LlmConfig {
                api_key: "test-only".into(),
                base_url: format!("http://{addr}/v1"),
                model: "mock".into(),
                max_tokens: 256,
                temperature: 0.0,
                context_window: None,
                provider: "openai".into(),
                thinking: teshi_engine::DeepSeekThinking::High,
                api_style: teshi_engine::ApiStyle::ChatCompletions,
                stream: false,
                http_headers: Default::default(),
                chat_options: Default::default(),
            })
            .unwrap();
        app.agents[0].messages_mut().push(super::AiChatMessage {
            role: AiRole::User,
            content: "inspect project".into(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });
        app.agents[0].backend.start().unwrap();
        let messages = app.build_chat_messages_for_agent(0);
        app.agents[0]
            .backend
            .submit_turn(
                Some("test".into()),
                messages,
                Some(teshi_agent::get_tools(None)),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline
            && app.agents[0].backend.status() != teshi_agent::backend::AgentBackendStatus::Completed
        {
            app.poll_llm_events();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::Completed
        );
        assert_eq!(
            app.agents[0]
                .messages()
                .iter()
                .map(|m| m.role)
                .collect::<Vec<_>>(),
            vec![
                AiRole::User,
                AiRole::Assistant,
                AiRole::Tool,
                AiRole::Assistant,
                AiRole::Tool,
                AiRole::Assistant
            ]
        );
        assert_eq!(app.agents[0].messages()[5].content, "finished");
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        let tool_result = requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "tool")
            .unwrap();
        assert_eq!(tool_result["tool_call_id"], "call-1");
        assert!(
            requests[2]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["tool_call_id"] == "call-2")
        );
        assert!(
            requests[2]
                .get("tools")
                .is_none_or(serde_json::Value::is_null)
        );
    }

    #[test]
    #[ignore = "requires the durable-paired real Chrome Rust broker"]
    fn native_model_browser_tool_round_trip_uses_the_shared_broker() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for round in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut received = Vec::new();
                let mut buffer = [0u8; 8192];
                let body_start = loop {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    received.extend_from_slice(&buffer[..n]);
                    let Some(header_end) = received.windows(4).position(|part| part == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&received[..header_end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    let body_start = header_end + 4;
                    if received.len() >= body_start + length {
                        break (body_start, length);
                    }
                };
                requests.push(
                    serde_json::from_slice::<serde_json::Value>(
                        &received[body_start.0..body_start.0 + body_start.1],
                    )
                    .unwrap(),
                );
                let body = if round == 0 {
                    serde_json::json!({
                        "model": "mock",
                        "choices": [{
                            "message": {
                                "role": "assistant",
                                "content": null,
                                "tool_calls": [{
                                    "id": "browser-call-1",
                                    "type": "function",
                                    "function": {
                                        "name": "browser_snapshot",
                                        "arguments": "{}"
                                    }
                                }]
                            }
                        }]
                    })
                } else {
                    serde_json::json!({
                        "model": "mock",
                        "choices": [{
                            "message": {
                                "role": "assistant",
                                "content": "finished"
                            }
                        }]
                    })
                }
                .to_string();
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
            requests
        });

        let (mut app, project, _store) = slash_test_app();
        let source_root = std::env::var_os("TESHI_ACCEPTANCE_PROJECT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                crate::cli::browser_endpoint::resolve_browser_project_root(Path::new(env!(
                    "CARGO_MANIFEST_DIR"
                )))
                .unwrap()
            });
        let endpoint = fs::read(source_root.join(".teshi").join("cdp-endpoint.json")).unwrap();
        fs::create_dir_all(project.path().join(".teshi")).unwrap();
        fs::write(
            project.path().join(".teshi").join("cdp-endpoint.json"),
            endpoint,
        )
        .unwrap();
        app.agents[0].backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
            teshi_agent::backend::AgentBackendKind::Native,
            1,
        );
        app.agents[0]
            .backend
            .attach_native_model(crate::llm::LlmConfig {
                api_key: "test-only".into(),
                base_url: format!("http://{addr}/v1"),
                model: "mock".into(),
                max_tokens: 256,
                temperature: 0.0,
                context_window: None,
                provider: "openai".into(),
                thinking: teshi_engine::DeepSeekThinking::High,
                api_style: teshi_engine::ApiStyle::ChatCompletions,
                stream: false,
                http_headers: Default::default(),
                chat_options: Default::default(),
            })
            .unwrap();
        app.agents[0].messages_mut().push(super::AiChatMessage {
            role: AiRole::User,
            content: "inspect the current page".into(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
        });
        app.agents[0].backend.start().unwrap();
        let messages = app.build_chat_messages_for_agent(0);
        app.agents[0]
            .backend
            .submit_turn(
                Some("test".into()),
                messages,
                Some(teshi_agent::get_tools(None)),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline
            && app.agents[0].backend.status() != teshi_agent::backend::AgentBackendStatus::Completed
        {
            app.poll_llm_events();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::Completed
        );
        let requests = server.join().unwrap();
        let tool_result = requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "tool")
            .unwrap();
        assert!(
            tool_result["content"]
                .as_str()
                .unwrap()
                .contains("broker_start_id")
        );
        assert!(
            tool_result["content"]
                .as_str()
                .unwrap()
                .contains("\"bridge\": \"rust\"")
        );
    }

    #[test]
    fn visual_observation_projection_keeps_order_and_reasoning() {
        let (mut app, _project, _store) = slash_test_app();
        *app.agents[0].messages_mut() = vec![
            super::AiChatMessage {
                role: super::AiRole::Assistant,
                content: String::new(),
                tool_calls: Some(vec![crate::llm::ToolCall {
                    id: "observe-1".into(),
                    name: "observe_page".into(),
                    arguments: "{}".into(),
                    execution_duration_ms: None,
                }]),
                tool_call_id: None,
                reasoning_content: Some("inspect the page".into()),
                source: None,
            },
            super::AiChatMessage {
                role: super::AiRole::Tool,
                content: "Visual observation captured successfully.".into(),
                tool_calls: None,
                tool_call_id: Some("observe-1".into()),
                reasoning_content: None,
                source: None,
            },
            super::AiChatMessage {
                role: super::AiRole::User,
                content: "[Browser visual observation]".into(),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: Some("browser_visual_observation".into()),
            },
        ];
        app.agents[0].latest_visual_observation = Some(teshi_engine::BrowserScreenshot {
            observation_id: "visual-1".into(),
            image: std::sync::Arc::from([0x89, 0x50, 0x4e, 0x47]),
            media_type: "image/png".into(),
            url: Some("http://127.0.0.1/fixture".into()),
            title: Some("fixture".into()),
            page_context_revision: None,
        });

        let messages = app.build_chat_messages_for_agent(0);
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(
            messages[1].reasoning_content.as_deref(),
            Some("inspect the page")
        );
        assert_eq!(messages[2].role, "tool");
        assert_eq!(messages[3].role, "user");
        assert!(matches!(
            messages[3].content,
            teshi_core::llm::MessageContent::Blocks(ref blocks)
                if matches!(blocks[1], teshi_core::llm::ContentBlock::Image { .. })
        ));
    }

    #[test]
    fn visual_context_retains_only_latest_image() {
        let (mut app, _project, _store) = slash_test_app();
        for round in 0..10_u8 {
            app.agents[0].messages_mut().push(super::AiChatMessage {
                role: super::AiRole::User,
                content: format!("[Browser visual observation] {round}"),
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                source: Some("browser_visual_observation".into()),
            });
            app.agents[0].latest_visual_observation = Some(teshi_engine::BrowserScreenshot {
                observation_id: format!("visual-{round}"),
                image: std::sync::Arc::from([round]),
                media_type: "image/png".into(),
                url: None,
                title: None,
                page_context_revision: None,
            });
        }
        let messages = app.build_chat_messages_for_agent(0);
        let image_count = messages
            .iter()
            .filter(|message| {
                matches!(
                    message.content,
                    teshi_core::llm::MessageContent::Blocks(ref blocks)
                        if blocks.iter().any(|block| matches!(
                            block,
                            teshi_core::llm::ContentBlock::Image { .. }
                        ))
                )
            })
            .count();
        assert_eq!(image_count, 1);
        assert_eq!(messages.len(), 11);
    }

    /// Live opt-in acceptance for the complete Browser Vision agent loop.
    ///
    /// This intentionally lives beside the production App loop: the test sends
    /// the prompt through `dispatch_user_message`, then lets `poll_llm_events`
    /// execute every model-requested browser tool and continuation.
    #[test]
    fn live_browser_vision_closed_loop() {
        if std::env::var("TESHI_RUN_LIVE_BROWSER_VISION_E2E").as_deref() != Ok("1") {
            println!(
                "SKIP live_browser_vision_closed_loop: set TESHI_RUN_LIVE_BROWSER_VISION_E2E=1"
            );
            return;
        }
        let api_key = match std::env::var("DEEPSEEK_API_KEY") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                panic!("FAIL live_browser_vision_closed_loop: DEEPSEEK_API_KEY is not set");
            }
        };

        let project = tempdir().expect("create live project");
        fs::write(
            project.path().join("live.feature"),
            "Feature: Browser vision live loop\n\n  Scenario: run the visible test\n    Given the browser fixture is open\n",
        )
        .expect("write live feature");
        let (fixture_url, stop_server) = start_visual_fixture_server();

        // The browser sidecar is Teshi's existing embedded runtime. The test
        // never launches Playwright, Chromium, or another browser itself.
        let runtime = teshi_engine::TeshiEngine::new(
            teshi_engine::RuntimeConfig {
                browser_service_script: teshi_engine::default_browser_service_script(),
                winapp_service_script: teshi_engine::default_winapp_service_script(),
                embedded_no_preview_stream: true,
                requirements_root: None,
            },
            None,
        );
        let original_cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("resolve Teshi workspace root")
            .to_path_buf();
        let tokio_runtime = tokio::runtime::Runtime::new().expect("create live runtime");
        tokio_runtime
            .block_on(teshi_engine::open_project(
                runtime.clone(),
                original_cwd.to_string_lossy().into_owned(),
            ))
            .expect("open live project");
        let browser = match tokio_runtime.block_on(teshi_engine::start_browser_sidecar(
            runtime.clone(),
            teshi_engine::BrowserMode::Embedded,
        )) {
            Ok(result) => result,
            Err(error) => {
                stop_server();
                panic!(
                    "FAIL live_browser_vision_closed_loop: browser runtime unavailable: {error:?}"
                );
            }
        };

        std::env::set_current_dir(project.path()).expect("enter live project");
        fs::create_dir_all(project.path().join(".teshi")).expect("create endpoint directory");
        fs::write(
            project.path().join(".teshi/cdp-endpoint.json"),
            serde_json::json!({"ws_url": browser.ws_url, "mode": "embedded"}).to_string(),
        )
        .expect("write browser endpoint");

        let result = (|| {
            let client = teshi_engine::BrowserOperations::new(
                browser.ws_url.clone(),
                Duration::from_secs(30),
            )
            .with_caller_label("teshi-live-browser-vision-test");
            navigate_live_fixture(&client, &fixture_url).expect("navigate fixture");
            let initial = {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    let snapshot = snapshot_live_page(&client).expect("read initial snapshot");
                    let snapshot_text =
                        serde_json::to_string(&snapshot).expect("serialize initial snapshot");
                    if snapshot_text.contains("RUN TEST") && snapshot_text.contains("ERROR 42") {
                        break snapshot;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "initial snapshot never exposed fixture controls: {snapshot_text}"
                    );
                    thread::sleep(Duration::from_millis(100));
                }
            };
            let initial_text = serde_json::to_string(&initial).expect("serialize initial snapshot");
            assert!(
                initial_text.contains("RUN TEST"),
                "initial snapshot lacks RUN TEST: {initial_text}"
            );
            assert!(
                initial_text.contains("ERROR 42"),
                "initial snapshot lacks ERROR 42: {initial_text}"
            );

            let store = tempdir().expect("create requirement store");
            teshi_engine::initialize_requirement_store(store.path())
                .expect("init requirement store");
            let mut app = App::from_file(
                &project.path().join("live.feature"),
                crate::config::load_config().expect("load config"),
                store.path().to_path_buf(),
            )
            .expect("create production App");
            let config = crate::llm::LlmConfig {
                api_key,
                base_url: "https://api.deepseek.com".into(),
                model: "deepseek-flash".into(),
                max_tokens: 2048,
                temperature: 0.2,
                context_window: None,
                provider: "deepseek".into(),
                thinking: teshi_engine::DeepSeekThinking::High,
                api_style: teshi_engine::ApiStyle::ChatCompletions,
                stream: true,
                http_headers: Default::default(),
                chat_options: Default::default(),
            };
            app.agents[0].backend.attach_native_model(config).unwrap();
            app.dispatch_user_message(
                "Inspect the current page and bring this test interface into a successful state.\n\nConfirm the rendered visual state before taking the main action. Use the available browser tools as needed. Prefer structured browser tools for interaction, then verify the changed state and report the final result.".into(),
            )?;

            let deadline = Instant::now() + Duration::from_secs(180);
            while Instant::now() < deadline {
                app.poll_llm_events();
                if app.agents[0].status == AiStatus::Error {
                    anyhow::bail!(
                        "production Agent entered error state: {:?}",
                        app.agents[0].messages()
                    );
                }
                let done = app.agents[0].status == AiStatus::Idle
                    && app.agents[0].messages().iter().any(|message| {
                        message.role == AiRole::Assistant
                            && message.tool_calls.is_none()
                            && !message.content.trim().is_empty()
                    });
                if done {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(
                app.agents[0].status,
                AiStatus::Idle,
                "Agent did not finish within 180 seconds"
            );

            let tool_names: Vec<String> = app.agents[0]
                .messages()
                .iter()
                .filter_map(|message| message.tool_calls.as_ref())
                .flat_map(|calls| calls.iter().map(|call| call.name.clone()))
                .collect();
            assert!(
                tool_names.len() <= 12,
                "live Agent exceeded the browser tool-call budget"
            );
            assert!(
                tool_names
                    .iter()
                    .filter(|name| *name == "observe_page")
                    .count()
                    <= 3,
                "live Agent exceeded the visual observation budget"
            );
            assert!(
                app.agents[0]
                    .messages()
                    .iter()
                    .filter(|message| message.role == AiRole::Assistant)
                    .count()
                    <= 8,
                "live Agent exceeded the model-turn budget"
            );
            assert!(
                tool_names.iter().any(|name| name == "observe_page"),
                "Agent never autonomously observed the page: {tool_names:?}"
            );
            assert!(
                tool_names.iter().any(|name| name == "browser_click"),
                "Agent never executed a structured browser click: {tool_names:?}"
            );
            assert!(app.agents[0].latest_visual_observation.is_some());
            let observation = app.agents[0].latest_visual_observation.as_ref().unwrap();
            assert_eq!(observation.media_type, "image/png");
            assert!(observation.image.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(!observation.image.is_empty());
            assert_eq!(observation.url.as_deref(), Some(fixture_url.as_str()));
            let visual_user_index = app.agents[0]
                .messages()
                .iter()
                .position(|message| message.source.as_deref() == Some("browser_visual_observation"))
                .expect("synthetic visual user message missing");
            assert!(visual_user_index >= 2);
            assert_eq!(
                app.agents[0].messages()[visual_user_index - 1].role,
                AiRole::Tool
            );
            assert!(
                app.agents[0].messages()[visual_user_index - 2]
                    .tool_calls
                    .as_ref()
                    .is_some_and(|calls| calls.iter().any(|call| call.name == "observe_page"))
            );

            let projected = app.build_chat_messages_for_agent(0);
            let image_count = projected
                .iter()
                .filter(|message| matches!(message.content, teshi_core::llm::MessageContent::Blocks(ref blocks) if blocks.iter().any(|block| matches!(block, teshi_core::llm::ContentBlock::Image { .. }))))
                .count();
            assert_eq!(
                image_count, 1,
                "active model projection retained more than one image"
            );
            for (original, projected) in app.agents[0]
                .messages()
                .iter()
                .zip(projected.iter().skip(1))
            {
                if original.role == AiRole::Assistant && original.reasoning_content.is_some() {
                    assert_eq!(projected.reasoning_content, original.reasoning_content);
                }
            }

            let final_snapshot = snapshot_live_page(&client).expect("read final snapshot");
            let final_text =
                serde_json::to_string(&final_snapshot).expect("serialize final snapshot");
            assert!(
                final_text.contains("TEST PASSED"),
                "browser page did not reach TEST PASSED: {final_text}"
            );
            let final_answer = app.agents[0]
                .messages()
                .iter()
                .rev()
                .find(|message| message.role == AiRole::Assistant && message.tool_calls.is_none())
                .map(|message| message.content.clone())
                .unwrap_or_default();
            assert!(
                !final_answer.trim().is_empty(),
                "Agent returned no final answer"
            );
            write_live_browser_vision_report(
                &original_cwd,
                &app,
                observation,
                &tool_names,
                &final_answer,
            );
            Ok::<(), anyhow::Error>(())
        })();

        std::env::set_current_dir(original_cwd).expect("restore current directory");
        tokio_runtime
            .block_on(teshi_engine::stop_browser_sidecar(&runtime))
            .ok();
        stop_server();
        result.expect("live browser vision closed loop");
    }

    /// Live opt-in acceptance that the Agent browser tools observe the same
    /// broker generation and Chrome session as every other Teshi entry point.
    ///
    /// The test drives the production Agent loop (`dispatch_user_message` plus
    /// `poll_llm_events`), so every browser observation is produced by a
    /// model-requested Agent tool call instead of a direct `BrowserOperations`
    /// invocation. It prints only non-sensitive identity fields; the broker
    /// credential and lease tokens never leave the engine client.
    #[test]
    fn live_chrome_agent_broker_identity() {
        if std::env::var("TESHI_RUN_LIVE_CHROME_AGENT_E2E").as_deref() != Ok("1") {
            println!(
                "SKIP live_chrome_agent_broker_identity: set TESHI_RUN_LIVE_CHROME_AGENT_E2E=1"
            );
            return;
        }
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .expect("DEEPSEEK_API_KEY is required for the live Chrome Agent acceptance");
        assert!(
            crate::llm::is_configured(),
            "live Agent dispatch requires TESHI_LLM_API_KEY or a configured model profile"
        );
        let project_root =
            PathBuf::from(std::env::var("TESHI_ACCEPTANCE_PROJECT").expect(
                "TESHI_ACCEPTANCE_PROJECT must name a project with .teshi/cdp-endpoint.json",
            ));
        assert!(
            project_root
                .join(".teshi")
                .join("cdp-endpoint.json")
                .is_file(),
            "acceptance project {} has no .teshi/cdp-endpoint.json",
            project_root.display()
        );

        let feature = project_root.join("acceptance.feature");
        fs::write(
            &feature,
            "Feature: Chrome Agent acceptance\n\n  Scenario: observe the acceptance page\n    Given the acceptance page is open\n",
        )
        .expect("write acceptance feature");
        let store = tempdir().expect("create requirement store");
        teshi_engine::initialize_requirement_store(store.path()).expect("init requirement store");
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().expect("load config"),
            store.path().to_path_buf(),
        )
        .expect("create production App");
        app.agents[0].backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
            teshi_agent::backend::AgentBackendKind::Native,
            1,
        );
        app.agents[0]
            .backend
            .attach_native_model(crate::llm::LlmConfig {
                api_key,
                base_url: "https://api.deepseek.com".into(),
                model: "deepseek-flash".into(),
                max_tokens: 2048,
                temperature: 0.0,
                context_window: None,
                provider: "deepseek".into(),
                thinking: teshi_engine::DeepSeekThinking::High,
                api_style: teshi_engine::ApiStyle::ChatCompletions,
                stream: true,
                http_headers: Default::default(),
                chat_options: Default::default(),
            })
            .expect("attach native model");
        let expected_url_pattern = std::env::var("TESHI_EXPECT_BROWSER_URL_PATTERN")
            .unwrap_or_else(|_| "127\\.0\\.0\\.1:8799".into());
        app.dispatch_user_message(format!(
            "Inspect the browser page that is already open.\n\n\
             1. Call `browser_snapshot` once.\n\
             2. Call `browser_assert` with condition_type `url_match` and value `{expected_url_pattern}`.\n\
             3. Report the observed page title and URL. Do not call any other tool."
        ))
        .expect("dispatch user message");
        assert_ne!(
            app.agents[0].status,
            AiStatus::Idle,
            "live Agent turn was not submitted: {}",
            app.status
        );

        let deadline = Instant::now() + Duration::from_secs(240);
        while Instant::now() < deadline {
            app.poll_llm_events();
            if app.agents[0].status == AiStatus::Error
                || app.agents[0].backend.status()
                    == teshi_agent::backend::AgentBackendStatus::Failed
            {
                panic!("Agent entered error state: {:?}", app.agents[0].messages());
            }
            let done = app.agents[0].backend.status()
                == teshi_agent::backend::AgentBackendStatus::Completed
                && app.agents[0].messages().iter().any(|message| {
                    message.role == AiRole::Assistant
                        && message.tool_calls.is_none()
                        && !message.content.trim().is_empty()
                });
            if done {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            app.agents[0].backend.status(),
            teshi_agent::backend::AgentBackendStatus::Completed,
            "Agent did not finish within 240 seconds"
        );

        let tool_names: Vec<String> = app.agents[0]
            .messages()
            .iter()
            .filter_map(|message| message.tool_calls.as_ref())
            .flat_map(|calls| calls.iter().map(|call| call.name.clone()))
            .collect();
        assert!(
            tool_names.iter().any(|name| name == "browser_snapshot"),
            "Agent never called browser_snapshot: {tool_names:?}"
        );

        let snapshots: Vec<serde_json::Value> = app.agents[0]
            .messages()
            .iter()
            .filter(|message| message.role == AiRole::Tool)
            .filter_map(|message| serde_json::from_str::<serde_json::Value>(&message.content).ok())
            .filter(|value| {
                value.get("operation").and_then(|v| v.as_str()) == Some("get_page_snapshot")
            })
            .collect();
        assert!(
            !snapshots.is_empty(),
            "Agent produced no get_page_snapshot tool result: {tool_names:?}"
        );
        let snapshot = snapshots.last().expect("snapshot tool result");
        let identity = snapshot
            .get("broker_identity")
            .expect("agent snapshot carried no broker identity");
        println!(
            "AGENT_ACCEPTANCE {}",
            serde_json::json!({
                "broker_pid": identity.get("broker_pid"),
                "broker_start_id": identity.get("broker_start_id"),
                "protocol_version": identity.get("protocol_version"),
                "extension_instance_id": snapshot.get("extension_instance_id"),
                "window_id": snapshot.get("window_id"),
                "tab_id": snapshot.get("tab_id"),
                "url": snapshot.get("url"),
                "title": snapshot.get("title"),
                "page_context_revision": snapshot.get("page_context_revision"),
            })
        );

        for (env_name, observed) in [
            (
                "TESHI_EXPECT_BROKER_START_ID",
                identity.get("broker_start_id").and_then(|v| v.as_str()),
            ),
            (
                "TESHI_EXPECT_EXTENSION_INSTANCE_ID",
                snapshot
                    .get("extension_instance_id")
                    .and_then(|v| v.as_str()),
            ),
        ] {
            if let Ok(expected) = std::env::var(env_name) {
                assert_eq!(
                    observed,
                    Some(expected.as_str()),
                    "{env_name} mismatch between the Agent tool path and the baseline"
                );
            }
        }
    }

    fn start_visual_fixture_server() -> (String, impl FnOnce()) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let port = listener
            .local_addr()
            .expect("fixture server address")
            .port();
        listener
            .set_nonblocking(true)
            .expect("set fixture server nonblocking");
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let body =
            include_str!("../../../resources/tests/fixtures/browser-visual-observation.html")
                .as_bytes()
                .to_vec();
        let thread = thread::spawn(move || {
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut request = [0_u8; 8192];
                        let _ = stream.read(&mut request);
                        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                        response.extend_from_slice(&body);
                        let _ = stream.write_all(&response);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        let url = format!("http://127.0.0.1:{port}/browser-visual-observation.html");
        (url, move || {
            let _ = stop_tx.send(());
            let _ = thread.join();
        })
    }

    fn live_target(
        client: &teshi_engine::BrowserOperations,
    ) -> anyhow::Result<(teshi_engine::BrowserTarget, String, String)> {
        let payload = client
            .execute(&teshi_engine::BrowserOperation::ListBrowserSessions)?
            .payload;
        let record = payload["sessions"]
            .as_array()
            .and_then(|sessions| sessions.iter().find(|session| session["health"] == "ready"))
            .context("no ready browser session")?;
        let instance = record["identity"]["extension_instance_id"]
            .as_str()
            .context("missing browser instance")?
            .to_string();
        let (window, tab) = record["windows"]
            .as_array()
            .and_then(|windows| {
                windows.iter().find_map(|window| {
                    let id = window["id"].as_i64()?;
                    let tab = window["tabs"]
                        .as_array()?
                        .iter()
                        .find(|tab| tab["active"] == true)?["id"]
                        .as_i64()?;
                    Some((id, tab))
                })
            })
            .context("no active browser tab")?;
        let lease = client
            .execute(&teshi_engine::BrowserOperation::AcquireBrowserLease {
                extension_instance_id: instance.clone(),
                owner_label: "teshi-live-browser-vision-test".into(),
                ttl_secs: 30,
            })?
            .payload;
        let token = lease["lease"]["lease_token"]
            .as_str()
            .context("missing browser lease")?
            .to_string();
        Ok((
            teshi_engine::BrowserTarget {
                extension_instance_id: instance,
                window_id: window,
                tab_id: tab,
            },
            token,
            record["identity"]["extension_instance_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
        ))
    }

    fn navigate_live_fixture(
        client: &teshi_engine::BrowserOperations,
        url: &str,
    ) -> anyhow::Result<()> {
        let (target, token, instance) = live_target(client)?;
        let result = client.execute(&teshi_engine::BrowserOperation::NavigateBrowser {
            target,
            lease_token: token.clone(),
            url: url.into(),
            timeout_ms: 30_000,
            wait: Some(teshi_engine::BrowserWaitCondition::LoadComplete),
            monitor: false,
        });
        let _ = client.execute(&teshi_engine::BrowserOperation::ReleaseBrowserLease {
            extension_instance_id: instance,
            lease_token: token,
        });
        result.map(|_| ()).map_err(|error| anyhow::anyhow!(error))
    }

    fn snapshot_live_page(
        client: &teshi_engine::BrowserOperations,
    ) -> anyhow::Result<teshi_engine::BrowserPageSnapshot> {
        let (target, token, instance) = live_target(client)?;
        let result = client.execute(&teshi_engine::BrowserOperation::GetPageSnapshot {
            target,
            lease_token: token.clone(),
        });
        let _ = client.execute(&teshi_engine::BrowserOperation::ReleaseBrowserLease {
            extension_instance_id: instance,
            lease_token: token,
        });
        let response = result.map_err(|error| anyhow::anyhow!(error))?;
        Ok(serde_json::from_value(response.payload)?)
    }

    fn write_live_browser_vision_report(
        report_root: &Path,
        app: &App,
        observation: &teshi_engine::BrowserScreenshot,
        tools: &[String],
        final_answer: &str,
    ) {
        let sha = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .unwrap_or_else(|| "unknown".into())
            .trim()
            .to_string();
        let report = format!(
            "# Teshi Browser Vision Live E2E\n\n- Date: 2026-09-12\n- Commit SHA: `{sha}`\n- Provider: `deepseek`\n- Model: `deepseek-flash`\n- Thinking mode: `high`\n- Fixture: `resources/tests/fixtures/browser-visual-observation.html`\n- Browser runtime: existing Teshi embedded browser sidecar\n- Live E2E result: PASS\n\n## Trace summary\n\n- Agent turns: {}\n- Tools: `{}`\n- Observation id: `{}`\n- Screenshot: `{}`, {} bytes, `full_page=false`\n- Browser URL: `{}`\n- Browser title: `{}`\n- Last model finish reason: `{}`\n- Peak active model-visible image count: `1`\n- Final answer present: `true`\n\nReasoning content was retained only as presence/length metadata in the harness; its text and screenshot bytes were not written. Final answer length: {}.\n",
            app.agents[0]
                .messages()
                .iter()
                .filter(|message| message.role == AiRole::Assistant)
                .count(),
            tools.join(", "),
            observation.observation_id,
            observation.media_type,
            observation.image.len(),
            observation.url.as_deref().unwrap_or(""),
            observation.title.as_deref().unwrap_or(""),
            app.agents[0]
                .last_finish_reason
                .as_deref()
                .unwrap_or("unknown"),
            final_answer.len()
        );
        fs::write(report_root.join("docs/browser-vision-live-e2e.md"), report)
            .expect("write sanitized live report");
    }

    #[test]
    fn slash_menu_generate_uses_command_dispatcher() {
        let (mut app, _project, _store) = slash_test_app();
        app.agent_mut().input = "/generate".into();
        app.slash_suggestion_active = true;

        app.handle_action(Action::AiSlashSelect).unwrap();

        assert!(app.generation_scope_prompt.is_some());
        assert!(app.status.contains("Confirm generation source scope"));
    }

    #[test]
    fn slash_menu_subcommand_enters_dispatcher() {
        let (mut app, _project, _store) = slash_test_app();
        app.agent_mut().input = "/acp add sample".into();
        app.slash_suggestion_active = true;
        app.handle_action(Action::AiSlashSelect).unwrap();
        assert!(app.status.contains("Usage: /acp add"));
        assert!(!app.slash_suggestion_active);
    }

    #[test]
    fn slash_menu_continue_uses_command_dispatcher() {
        let (mut app, _project, _store) = slash_test_app();
        app.generation_stage = teshi_agent::pipeline::GenerationStage::ReviewingTestPoints;
        app.agent_mut().input = "/continue".into();
        app.slash_suggestion_active = true;

        app.handle_action(Action::AiSlashSelect).unwrap();

        assert_eq!(
            app.generation_stage,
            teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
        );
        assert!(app.status.contains("approve at least one test point"));
    }

    /// Waits for the launch selection to install the first discovered Feature.
    fn await_launch_selection(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.project.features.is_empty() {
            app.poll_lazy_features();
            assert!(
                Instant::now() < deadline,
                "launch selection did not load: {}",
                app.status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn await_catalog(app: &mut App, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            app.poll_lazy_features();
            if app
                .lazy_features
                .as_ref()
                .is_some_and(|state| !state.is_loading() && state.paths.len() == count)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "catalog did not complete: {}",
                app.status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn load_catalog_file(app: &mut App, index: usize) {
        app.select_catalog_feature(index);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.catalog_selection_ready() {
            app.poll_lazy_features();
            assert!(
                Instant::now() < deadline,
                "content did not load: {}",
                app.status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn first_navigation_selects_a_single_collapsed_directory() {
        let mut app = App::from_args().unwrap();
        app.project.features = vec![teshi_core::gherkin::parse_feature(
            "Feature: Nested",
            "only/one.feature".into(),
        )];
        app.git_status = Default::default();
        app.explore_focus = ColumnFocus::Feature;
        assert_eq!(app.feature_browser_rows().len(), 1);
        assert!(app.explore_selected_feature_row().is_none());
        app.explore_move_selection(1);
        assert_eq!(app.explore_selected_feature_row(), Some(0));
        assert_eq!(app.feature_browser.directory.as_deref(), Some("only"));
    }

    #[test]
    fn targeted_feature_scope_skips_unreadable_unrelated_files() {
        let root = tempdir().unwrap();
        let store = tempdir().unwrap();
        fs::write(root.path().join("a-broken.feature"), [0xff]).unwrap();
        fs::write(
            root.path().join("good.feature"),
            "Feature: Good\n  Scenario: Example\n    Given a fixture\n",
        )
        .unwrap();
        let mut app = App::from_project_directory(
            root.path(),
            crate::config::load_config().unwrap(),
            true,
            false,
            store.path().into(),
        )
        .unwrap();
        await_catalog(&mut app, 2);
        for path in [
            "good.feature".to_owned(),
            root.path()
                .join("good.feature")
                .to_string_lossy()
                .into_owned(),
        ] {
            let selected = app
                .feature_scope(Some(&path))
                .collect::<anyhow::Result<Vec<_>>>();
            let selected = selected.unwrap();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].name, "Good");
        }
        assert!(app.feature_scope(Some("ood.feature")).next().is_none());
        assert!(
            app.feature_scope(None)
                .collect::<anyhow::Result<Vec<_>>>()
                .is_err()
        );
        assert!(app.project.features.is_empty());
    }

    #[test]
    fn lazy_selection_replaces_clean_content_and_preserves_dirty_edits() {
        let root = tempdir().unwrap();
        let store = tempdir().unwrap();
        for (name, title) in [("a", "First"), ("b", "Second")] {
            fs::write(
                root.path().join(format!("{name}.feature")),
                format!("Feature: {title}\n  Scenario: Example\n    Given a fixture\n"),
            )
            .unwrap();
        }
        let mut app = App::from_project_directory(
            root.path(),
            crate::config::load_config().unwrap(),
            true,
            false,
            store.path().into(),
        )
        .unwrap();
        await_catalog(&mut app, 2);
        await_launch_selection(&mut app);
        assert_eq!(app.project.features.len(), 1);
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(
            app.feature_scope(None)
                .map(|f| f.unwrap().name.clone())
                .collect::<Vec<_>>(),
            ["First", "Second"]
        );
        load_catalog_file(&mut app, 0);
        assert_eq!(app.project.features[0].name, "First");
        app.buffer = EditorBuffer::from_string("Feature: Unsaved".into());
        app.mark_current_buffer_dirty();
        app.select_catalog_feature(1);
        assert_eq!(app.project.features[0].name, "First");
        assert!(app.buffer.as_string().contains("Unsaved"));
        assert!(app.status.contains("Save current edits"));
        app.set_buffer_dirty(0, false);
        load_catalog_file(&mut app, 1);
        assert_eq!(app.project.features.len(), 1);
        assert_eq!(app.buffers.len(), 1);
        assert_eq!(app.project.features[0].name, "Second");
        assert_eq!(app.build_explore_cases().len(), 1);
        assert!(app.undo_stack.is_empty());
    }

    #[test]
    fn pending_change_queued_from_ai_tab_summarizes_lazy_feature() {
        let root = tempdir().unwrap();
        let store = tempdir().unwrap();
        fs::write(
            root.path().join("sample.feature"),
            "Feature: Sample\n  Scenario: Ready\n    Given a fixture\n",
        )
        .unwrap();
        let mut app = App::from_project_directory(
            root.path(),
            crate::config::load_config().unwrap(),
            true,
            false,
            store.path().into(),
        )
        .unwrap();
        await_catalog(&mut app, 1);
        load_catalog_file(&mut app, 0);
        // The MindMap index is built on demand for the MindMap tab only, but the
        // Change Summary maps diff lines onto its nodes from any tab.
        app.active_tab = MainTab::Ai;
        app.queue_agent_change(super::AgentPendingChange {
            description: "update step 0".into(),
            file_path: app.project.features[0].file_path.display().to_string(),
            mutation: super::AgentMutation::ReplaceLine {
                row_0based: 2,
                new_text: "    Given a replacement".into(),
            },
            scenario_name: "Ready".into(),
            test_point_ids: Vec::new(),
            tool_call_id: "summary".into(),
            old_buffer_snapshot: app.buffer.as_string(),
            agent_idx: 0,
        });

        assert!(app.change_summary_visible);
        assert_eq!(app.pending_change_summary.len(), 1);
    }

    #[test]
    fn feature_tool_lookup_accepts_canonical_project_path_aliases() {
        let root = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        fs::create_dir_all(root.path().join("features/nested")).unwrap();
        fs::write(
            root.path().join("features/sample.feature"),
            "Feature: Sample\n  Scenario: Ready\n    Given a fixture\n",
        )
        .unwrap();
        let config = crate::config::AppConfig {
            default_provider: None,
            providers: Default::default(),
        };
        let mut app = App::from_project_directory(
            root.path(),
            config,
            false,
            false,
            store.path().to_path_buf(),
        )
        .unwrap();
        await_catalog(&mut app, 1);
        await_launch_selection(&mut app);
        assert_eq!(app.project.features.len(), 1);
        let alias = root.path().join("features/nested/../sample.feature");
        assert_eq!(
            app.find_feature_idx_for_file(alias.to_str().unwrap()),
            Some(0)
        );
        assert_eq!(
            app.find_feature_idx_for_file("features/nested/../sample.feature"),
            Some(0)
        );
        assert_eq!(app.find_feature_idx_for_file("missing.feature"), None);
    }

    #[test]
    fn explicit_project_directory_loads_only_features_and_preserves_runner_root() {
        let root = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        fs::create_dir_all(root.path().join("features/deep")).unwrap();
        let body = "Feature: Sample\n  Scenario: Ready\n    Given a fixture\n";
        fs::write(root.path().join("features/deep/in.feature"), body).unwrap();
        fs::write(root.path().join("outside.feature"), body).unwrap();
        fs::write(
            root.path().join("teshi.toml"),
            "[runner]\ncmd = 'fixture'\ncwd = 'work'\n",
        )
        .unwrap();
        let config = crate::config::AppConfig {
            default_provider: None,
            providers: Default::default(),
        };
        let mut app = App::from_project_directory(
            root.path(),
            config,
            false,
            false,
            store.path().to_path_buf(),
        )
        .unwrap();
        await_catalog(&mut app, 1);
        await_launch_selection(&mut app);
        // Only the launched Feature is resident; the sibling outside the
        // discovery root stays unparsed.
        assert_eq!(app.project.features.len(), 1);
        assert_eq!(
            app.project.features[0].file_path,
            root.path().join("features/deep/in.feature")
        );
        load_catalog_file(&mut app, 0);
        assert_eq!(app.project.root_dir, root.path());
        assert_eq!(app.project.features.len(), 1);
        assert_eq!(
            app.project.features[0].file_path,
            root.path().join("features/deep/in.feature")
        );
        assert_eq!(
            app.runner_config.as_ref().unwrap().cwd,
            Some(root.path().join("work"))
        );
    }

    #[test]
    fn missing_features_loads_and_refreshes_only_root_files() {
        for library_view in [false, true] {
            let root = tempdir().unwrap();
            let store = tempdir().unwrap();
            teshi_engine::initialize_requirement_store(store.path()).unwrap();
            fs::create_dir_all(root.path().join("nested/deep")).unwrap();
            let body = "Feature: Sample\n  Scenario: Ready\n    Given a fixture\n";
            fs::write(root.path().join("nested/deep/hidden.feature"), body).unwrap();
            let mut app = App::from_project_directory(
                root.path(),
                crate::config::AppConfig {
                    default_provider: None,
                    providers: Default::default(),
                },
                false,
                library_view,
                store.path().to_path_buf(),
            )
            .unwrap();
            assert!(app.project.features.is_empty());
            assert_eq!(app.project.root_dir, root.path());
            assert_eq!(
                app.git_status.scope,
                teshi_core::git::GitProjectScope::CurrentFiles
            );
            assert_eq!(app.feature_discovery_root.as_deref(), Some(root.path()));
            fs::write(root.path().join("root.feature"), body).unwrap();
            await_catalog(&mut app, 1);
            load_catalog_file(&mut app, 0);
            assert_eq!(app.project.features.len(), 1);
            assert_eq!(
                app.project.features[0].file_path,
                root.path().join("root.feature")
            );
            assert!(!root.path().join("features").exists());
            let mut reloaded = App::from_project_directory(
                root.path(),
                crate::config::AppConfig {
                    default_provider: None,
                    providers: Default::default(),
                },
                false,
                library_view,
                store.path().to_path_buf(),
            )
            .unwrap();
            await_catalog(&mut reloaded, 1);
            load_catalog_file(&mut reloaded, 0);
            assert_eq!(reloaded.project.features.len(), 1);
            assert_eq!(
                reloaded.project.features[0].file_path,
                root.path().join("root.feature")
            );
        }
    }

    #[test]
    fn explicit_recursive_project_loads_outside_features_asynchronously() {
        let root = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        fs::write(
            root.path().join("outside.feature"),
            "Feature: Outside\n  Scenario: Ready\n    Given a fixture\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("nested/deep")).unwrap();
        fs::write(
            root.path().join("nested/deep/inside.feature"),
            "Feature: Inside\n  Scenario: Ready\n    Given a fixture\n",
        )
        .unwrap();
        let config = crate::config::AppConfig {
            default_provider: None,
            providers: Default::default(),
        };
        let mut app = App::from_project_directory(
            root.path(),
            config,
            true,
            false,
            store.path().to_path_buf(),
        )
        .unwrap();
        assert!(
            app.project.features.is_empty(),
            "recursive startup must render before walking"
        );
        await_catalog(&mut app, 2);
        await_launch_selection(&mut app);
        // Two paths were discovered, but a launch keeps a single AST resident.
        assert_eq!(app.project.features.len(), 1);
        load_catalog_file(&mut app, 0);
        assert_eq!(app.project.features.len(), 1);
        load_catalog_file(&mut app, 1);
        assert_eq!(app.project.features.len(), 1);
        assert_eq!(app.project.root_dir, root.path());
        assert_eq!(app.active_buffer_idx, Some(0));
        assert!(app.buffer.as_string().contains("Feature:"));
        app.handle_action(Action::SelectTab(MainTab::Ai)).unwrap();
        app.handle_action(Action::SelectTab(MainTab::Explore))
            .unwrap();
        app.handle_action(Action::Quit).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn default_discovery_and_refresh_stay_in_the_selected_language() {
        let root = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let selected = root.path().join("features/zh-CN");
        let other = root.path().join("features/en-US");
        fs::create_dir_all(&selected).unwrap();
        fs::create_dir_all(&other).unwrap();
        let feature = "Feature: Selected\n  Scenario: Example\n    Given something\n";
        fs::write(selected.join("initial.feature"), feature).unwrap();
        fs::write(other.join("other.feature"), feature).unwrap();
        fs::write(root.path().join("outside.feature"), feature).unwrap();
        let mut app = App::from_directory_with_discovery(
            root.path(),
            &selected,
            crate::config::load_config().unwrap(),
            true,
            store.path().to_path_buf(),
        )
        .unwrap();
        app.feature_discovery_root = Some(selected.clone());
        assert_eq!(app.project.root_dir, root.path());
        assert_eq!(app.project.features.len(), 1);
        fs::create_dir_all(selected.join("nested")).unwrap();
        fs::write(selected.join("nested/added.feature"), feature).unwrap();
        app.last_external_check = Instant::now() - Duration::from_secs(2);
        app.poll_external_feature_changes();
        assert_eq!(app.project.features.len(), 2);
        assert!(
            app.project
                .features
                .iter()
                .all(|f| f.file_path.starts_with(&selected))
        );
    }

    #[test]
    fn test_bare_teshi_loads_requirement_documents_into_tab() {
        use crate::authoring_tab::AuthoringUiState;

        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let mut writer = AuthoringUiState::load_from_project(project.path(), store.path());
        writer.create_document("login.md", "Login").unwrap();
        writer.buffer = EditorBuffer::from_string("# Login\n\nMust authenticate.\n".into());
        writer.save_current_document(project.path()).unwrap();

        let config = crate::config::AppConfig {
            default_provider: None,
            providers: Default::default(),
        };
        let app = App::from_project_directory(
            project.path(),
            config,
            false,
            true,
            store.path().to_path_buf(),
        )
        .unwrap();
        assert_eq!(
            app.authoring_ui.project_filter,
            teshi_core::authoring::RequirementProjectFilter::All
        );
        assert!(app.authoring_ui.discovered);
        let artifacts = app.authoring_ui.artifacts.as_ref().expect("loaded store");
        assert!(
            artifacts
                .index
                .documents
                .iter()
                .any(|doc| doc.title == "Login"),
            "expected Login document in Requirements tab"
        );
        assert!(
            app.authoring_ui
                .buffer
                .as_string()
                .contains("Must authenticate")
        );
    }

    #[test]
    fn test_requirements_insert_save_and_browse_reject_mutations() {
        use crate::authoring_tab::RequirementsEditorMode;
        let (mut app, _project, store) = requirements_test_app();
        let original = app.authoring_ui.buffer.as_string();
        for action in [
            Action::Insert('x'),
            Action::Enter,
            Action::Backspace,
            Action::Delete,
        ] {
            app.handle_action(action).unwrap();
        }
        assert_eq!(app.authoring_ui.buffer.as_string(), original);
        app.authoring_ui.cursor_row = 2;
        app.handle_action(Action::ReqEnterInsert).unwrap();
        for ch in "hjkl nsmq12345 中文🙂".chars() {
            app.handle_action(Action::Insert(ch)).unwrap();
        }
        app.handle_action(Action::Insert('\t')).unwrap();
        app.handle_action(Action::Save).unwrap();
        assert_eq!(app.authoring_ui.editor_mode, RequirementsEditorMode::Insert);
        assert_eq!(
            fs::read_to_string(store.path().join("req.md")).unwrap(),
            app.authoring_ui.buffer.as_string()
        );
        app.handle_action(Action::ReqExitInsert).unwrap();
        assert_eq!(app.authoring_ui.editor_mode, RequirementsEditorMode::Browse);
    }

    #[test]
    fn requirements_delete_cancel_preserves_document_and_dirty_guard() {
        let (mut app, _project, store) = requirements_test_app();
        let original = app.authoring_ui.buffer.as_string();
        app.authoring_ui.focus = crate::authoring_tab::RequirementsFocus::Tree;
        app.handle_action(Action::ReqDeleteDocument).unwrap();
        assert!(matches!(
            app.authoring_ui.overlay,
            crate::authoring_tab::RequirementsOverlay::DeleteConfirmation { .. }
        ));
        app.handle_action(Action::OverlayCancel).unwrap();
        assert_eq!(app.authoring_ui.buffer.as_string(), original);
        assert!(teshi_engine::read_requirement_document(store.path(), "doc-1").is_ok());

        app.authoring_ui.focus = crate::authoring_tab::RequirementsFocus::Editor;
        app.authoring_ui.enter_insert_mode();
        app.authoring_ui.insert_char('x');
        app.handle_action(Action::ReqDeleteDocument).unwrap();
        assert!(matches!(
            app.authoring_ui.overlay,
            crate::authoring_tab::RequirementsOverlay::Unsaved { .. }
        ));
        app.handle_action(Action::OverlayCancel).unwrap();
        assert!(app.authoring_ui.buffer_dirty);
        assert_ne!(app.authoring_ui.buffer.as_string(), original);
        assert!(app.authoring_ui.buffer.as_string().contains('x'));
        assert!(teshi_engine::read_requirement_document(store.path(), "doc-1").is_ok());
    }

    #[test]
    fn requirements_delete_confirmation_removes_document_and_click_control_is_registered() {
        let (mut app, project, store) = requirements_test_app();
        app.authoring_ui.focus = crate::authoring_tab::RequirementsFocus::Tree;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let delete_rect = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::ActionButton {
                    rect,
                    action: Action::ReqDeleteDocument,
                    ..
                } => Some(*rect),
                _ => None,
            })
            .expect("requirements delete control");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            delete_rect.x,
            delete_rect.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(matches!(
            app.authoring_ui.overlay,
            crate::authoring_tab::RequirementsOverlay::DeleteConfirmation { .. }
        ));
        app.handle_action(Action::OverlayConfirm).unwrap();
        assert!(app.status.contains("Deleted global requirement"));
        assert!(matches!(
            teshi_engine::read_requirement_document(store.path(), "doc-1"),
            Err(teshi_engine::RequirementStoreError::NotFound { .. })
        ));
        assert!(app.authoring_ui.selected_document_id.is_none());
        assert!(project.path().join("testpoints/testpoints.json").exists());
    }

    #[test]
    fn test_requirements_unsaved_cancel_discard_and_save_resume_navigation() {
        let (mut app, _project, store) = requirements_test_app();
        let original = app.authoring_ui.buffer.as_string();
        app.authoring_ui.cursor_row = 2;
        app.authoring_ui.enter_insert_mode();
        app.authoring_ui.insert_char('中');
        let edited = app.authoring_ui.buffer.as_string();
        app.handle_action(Action::SelectTab(MainTab::Explore))
            .unwrap();
        assert_eq!(app.active_tab, MainTab::Requirements);
        app.handle_action(Action::OverlayCancel).unwrap();
        assert_eq!(app.authoring_ui.buffer.as_string(), edited);
        app.handle_action(Action::SelectTab(MainTab::Explore))
            .unwrap();
        app.handle_action(Action::OverlayInsert('d')).unwrap();
        assert_eq!(app.active_tab, MainTab::Explore);
        assert_eq!(app.authoring_ui.buffer.as_string(), original);
        assert_eq!(
            fs::read_to_string(store.path().join("req.md")).unwrap(),
            original
        );
        app.select_tab(MainTab::Requirements);
        app.authoring_ui.cursor_row = 2;
        app.authoring_ui.insert_char('新');
        let saved = app.authoring_ui.buffer.as_string();
        app.handle_action(Action::ReqNewDocument).unwrap();
        assert_eq!(
            app.authoring_ui
                .artifacts
                .as_ref()
                .unwrap()
                .index
                .documents
                .len(),
            1
        );
        app.handle_action(Action::OverlayInsert('S')).unwrap();
        assert_eq!(
            fs::read_to_string(store.path().join("req.md")).unwrap(),
            saved
        );
        assert_eq!(
            app.authoring_ui
                .artifacts
                .as_ref()
                .unwrap()
                .index
                .documents
                .len(),
            2
        );
        app.handle_action(Action::Quit).unwrap();
        assert!(!app.should_quit);
        app.handle_action(Action::OverlayCancel).unwrap();
        assert!(!app.should_quit);
        app.handle_action(Action::Quit).unwrap();
        app.handle_action(Action::OverlayInsert('d')).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn test_requirements_failed_save_keeps_overlay_buffer_and_pending_action() {
        let (mut app, _project, store) = requirements_test_app();
        let blocked = store.path().join("blocked");
        fs::write(&blocked, "file prevents directory creation").unwrap();
        app.authoring_ui.requirements_root = blocked;
        app.authoring_ui.cursor_row = 2;
        app.authoring_ui.insert_char('中');
        let body = app.authoring_ui.buffer.as_string();
        app.authoring_ui.enter_insert_mode();
        app.handle_action(Action::Save).unwrap();
        assert!(app.authoring_ui.buffer_dirty);
        assert_eq!(
            app.authoring_ui.editor_mode,
            crate::authoring_tab::RequirementsEditorMode::Insert
        );
        assert_eq!(app.authoring_ui.buffer.as_string(), body);
        app.handle_action(Action::Quit).unwrap();
        let overlay = app.authoring_ui.overlay.clone();
        app.handle_action(Action::OverlayInsert('s')).unwrap();
        assert!(!app.should_quit);
        assert!(app.authoring_ui.buffer_dirty);
        assert_eq!(app.authoring_ui.buffer.as_string(), body);
        assert_eq!(app.authoring_ui.overlay, overlay);
        assert!(app.status.contains("Could not save requirement"));
        app.authoring_ui.requirements_root = store.path().to_path_buf();
        app.handle_action(Action::OverlayInsert('s')).unwrap();
        assert!(app.should_quit);
        assert_eq!(
            fs::read_to_string(store.path().join("req.md")).unwrap(),
            body
        );
    }

    #[test]
    fn test_requirements_scrollbar_drag_and_wheel_preserve_document() {
        let (mut app, _project, _store) = requirements_test_app();
        app.authoring_ui.buffer = crate::editor_buffer::EditorBuffer::from_string(
            (0..100)
                .map(|_| "abcdefghijklmnopqrstuvwxyz".repeat(8))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        render_test_app(&mut app, 120);
        let pane = app.authoring_ui.pane_areas[1];
        let original = app.authoring_ui.buffer.as_string();
        assert_eq!(app.authoring_ui.scroll_limits[1][0], 0);
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            pane.right() - 1,
            pane.y + 3,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(app.authoring_ui.scroll_row > 0);
        assert!(!app.authoring_ui.selection_dragging);
        app.handle_mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            pane.right() - 1,
            pane.y + 1,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(app.authoring_ui.scroll_row, 0);
        app.handle_mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            pane.right() - 1,
            pane.y + 1,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(app.authoring_ui.scrollbar_drag.is_none());
        app.handle_mouse_event(
            MouseEventKind::ScrollDown,
            pane.x + 2,
            pane.y + 2,
            KeyModifiers::SHIFT,
        )
        .unwrap();
        assert_eq!(app.authoring_ui.horizontal_scroll[1], 0);
        app.handle_mouse_event(
            MouseEventKind::ScrollDown,
            pane.x + 2,
            pane.y + 2,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(app.authoring_ui.scroll_row, 3);
        assert_eq!(app.authoring_ui.buffer.as_string(), original);
    }

    #[test]
    fn test_requirements_mouse_selection_across_soft_wrap_preserves_source_range() {
        let (mut app, _project, _store) = requirements_test_app();
        let original = "中文".repeat(100);
        app.authoring_ui.buffer = crate::editor_buffer::EditorBuffer::from_string(original.clone());
        render_test_app(&mut app, 120);
        let pane = app.authoring_ui.pane_areas[1];
        let start = app.authoring_ui.editor_position_at(2, 0).unwrap();
        let end = app.authoring_ui.editor_position_at(4, 1).unwrap();
        assert_eq!(start, (0, 1));
        assert!(end.1 > start.1);
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            pane.x + 3,
            pane.y + 1,
            KeyModifiers::NONE,
        )
        .unwrap();
        app.handle_mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            pane.x + 5,
            pane.y + 2,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(
            app.authoring_ui.selection_char_range(),
            Some(teshi_core::authoring::TextRange::new(
                start.1 as u32,
                end.1 as u32
            ))
        );
        assert_eq!(app.authoring_ui.buffer.as_string(), original);
    }

    #[test]
    fn test_requirements_mouse_selection_and_tab_guard() {
        let (mut app, _project, _store) = requirements_test_app();
        render_test_app(&mut app, 120);
        let pane = app.authoring_ui.pane_areas[1];
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            pane.x + 1,
            pane.y + 1,
            KeyModifiers::NONE,
        )
        .unwrap();
        app.handle_mouse_event(
            MouseEventKind::Drag(MouseButton::Left),
            pane.x + 4,
            pane.y + 1,
            KeyModifiers::NONE,
        )
        .unwrap();
        app.handle_mouse_event(
            MouseEventKind::Up(MouseButton::Left),
            pane.x + 4,
            pane.y + 1,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(
            app.authoring_ui.selection_char_range(),
            Some(teshi_core::authoring::TextRange::new(0, 3))
        );
        assert!(!app.authoring_ui.selection_dragging);
        app.authoring_ui.insert_char('中');
        let region = app
            .clickable_regions
            .iter()
            .find(|region| {
                matches!(
                    region,
                    ClickableRegion::Tab {
                        tab: MainTab::Explore,
                        ..
                    }
                )
            })
            .cloned()
            .unwrap();
        app.handle_region_click(&region, &ratatui::layout::Position::new(0, 0))
            .unwrap();
        assert_eq!(app.active_tab, MainTab::Requirements);
        let selection = app.authoring_ui.selection_char_range();
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            pane.x + 1,
            pane.y + 2,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(app.authoring_ui.selection_char_range(), selection);
        app.handle_action(Action::OverlayCancel).unwrap();
        assert_eq!(app.active_tab, MainTab::Requirements);
    }

    #[test]
    fn explore_horizontal_scrollbar_click_moves_and_starts_drag() {
        let mut app = App::from_args().expect("app init");
        let region = ClickableRegion::ExploreHorizontalScrollbar {
            column: ColumnFocus::Scenario,
            rect: ratatui::layout::Rect::new(10, 5, 20, 2),
            content_width: 100,
            viewport_width: 20,
        };

        app.handle_region_click(&region, &ratatui::layout::Position::new(29, 5))
            .expect("scrollbar click");

        assert_eq!(
            app.explore_horizontal_scroll[ColumnFocus::Scenario.index()],
            80
        );
        assert_eq!(
            app.explore_horizontal_scroll_drag,
            Some(ColumnFocus::Scenario)
        );
        assert_eq!(app.explore_focus, ColumnFocus::Scenario);
    }

    fn en() -> &'static teshi_core::gherkin_lang::GherkinLanguage {
        GherkinLanguages::global().get("en")
    }

    /// Helper: create an app pre-set to editor-active mode (stage 3) for existing editor tests.
    fn editor_test_app() -> App {
        let mut app = App::from_args().expect("app init should work");
        app.active_tab = MainTab::MindMap;
        app.view_stage = ViewStage::EditorAndPanel;
        app
    }

    #[test]
    fn agent_diff_opens_cached_changes_from_other_tabs_and_closes_in_explore() {
        let mut app = editor_test_app();
        app.queue_agent_change(super::AgentPendingChange {
            description: "inspect proposed edit".into(),
            file_path: "diff-fixture.feature".into(),
            mutation: super::AgentMutation::ReplaceLine {
                row_0based: 0,
                new_text: "Feature: proposed".into(),
            },
            scenario_name: String::new(),
            test_point_ids: Vec::new(),
            tool_call_id: "diff-review".into(),
            old_buffer_snapshot: "Feature: original".into(),
            agent_idx: 0,
        });
        for tab in [MainTab::MindMap, MainTab::Ai] {
            app.active_tab = tab;
            assert!(app.explore_diff_lines.is_some());
            app.toggle_explore_diff_view();
            assert_eq!(app.active_tab, MainTab::Explore);
            assert_eq!(app.explore_focus, ColumnFocus::Step);
            assert!(app.explore_diff_lines.is_some());
        }
        app.toggle_explore_diff_view();
        assert!(app.explore_diff_lines.is_none());
        assert_eq!(app.pending_agent_changes.len(), 1);
        app.toggle_explore_diff_view();
        assert!(app.explore_diff_lines.is_some());
        assert_eq!(app.active_tab, MainTab::Explore);
    }

    #[test]
    fn agent_diff_without_pending_changes_preserves_active_tab() {
        let mut app = editor_test_app();
        app.toggle_explore_diff_view();
        assert_eq!(app.active_tab, MainTab::MindMap);
        assert!(app.explore_diff_lines.is_none());
    }

    fn render_test_app(app: &mut App, width: u16) {
        let backend = TestBackend::new(width, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("test UI should render");
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
        let app = App::from_file(
            &path,
            crate::config::load_config().unwrap(),
            teshi_engine::requirements_data_dir(None).unwrap_or_default(),
        )
        .expect("app should open fixture file");
        (app, path)
    }

    #[test]
    fn test_buffer_diagnostics_update_without_replacing_partial_source() {
        let malformed = "# language: zh-CN\n功能: 登录\n  场景: 用户登录\n    当用户登录\n";
        let corrected = "# language: zh-CN\n功能: 登录\n  场景: 用户登录\n    当 用户登录\n";
        let (mut app, path) = feature_file_app("buffer-diagnostics", malformed);

        app.refresh_buffer_diagnostics();
        assert_eq!(app.buffer.as_string(), malformed);
        assert!(app.buffer_diagnostics.iter().any(|diagnostic| {
            diagnostic.code == teshi_core::diagnostic_codes::MISSING_STEP_SEPARATOR
                && diagnostic.line == 4
                && diagnostic.column == 6
        }));

        app.buffer = EditorBuffer::from_string(corrected.to_string());
        app.refresh_buffer_diagnostics();
        assert_eq!(app.buffer.as_string(), corrected);
        assert!(!app.buffer_diagnostics.iter().any(|diagnostic| {
            diagnostic.code == teshi_core::diagnostic_codes::MISSING_STEP_SEPARATOR
        }));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_editor_renders_buffer_diagnostics_without_hiding_source() {
        let mut app = editor_test_app();
        app.active_tab = MainTab::Explore;
        app.explore_edit_mode = true;
        app.file_path = Some(PathBuf::from("login.feature"));
        app.buffer = EditorBuffer::from_string(
            "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n".to_string(),
        );

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .expect("editor should render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        let compact_screen = screen.replace(' ', "");
        assert!(
            compact_screen.contains("当用户登录"),
            "source disappeared from editor"
        );
        assert!(
            screen.contains("missing_step_separator"),
            "diagnostic disappeared from editor"
        );
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
        app.cursor_row = 0;
        app.focus_slot = BddFocusSlot::Body;
        app.handle_action(Action::ActivateStepInput)
            .expect("edit should work");
        assert!(app.step_input_active);
        assert_eq!(app.step_input_min_col, 9);
    }

    #[test]
    fn test_editor_nav_moves_to_background_header() {
        let mut app = editor_test_app();
        app.buffer =
            EditorBuffer::from_string("Feature: T\n  Desc line\nBackground:\n".to_string());
        app.cursor_row = 0;
        app.handle_action(Action::MoveDown)
            .expect("move to background header should work");
        assert_eq!(app.cursor_row, 2);
        assert_eq!(app.buffer.line(2).trim_start(), "Background:");
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
    fn test_requirements_tab_select_and_create_document() {
        let mut app = App::from_args().expect("app init should work");
        app.handle_action(Action::SelectTab(MainTab::Requirements))
            .expect("select requirements");
        assert_eq!(app.active_tab, MainTab::Requirements);
        app.authoring_ui
            .create_document("new-req.md", "New Req")
            .unwrap();
        assert!(app.authoring_ui.buffer.as_string().contains("New Req"));
        assert!(app.authoring_ui.buffer_dirty);
    }

    #[test]
    fn test_test_points_review_state_transitions() {
        use crate::test_points_tab::{DetailField, TestPointsUiState};
        use teshi_core::authoring::{HierarchyPath, QuoteSelector, ReviewState, TestPoint};

        let mut tp = TestPoint {
            id: "tp-1".into(),
            title: "Login".into(),
            objective: "Verify login".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
            review_state: ReviewState::Proposed,
            requirement_links: vec![teshi_core::authoring::RequirementLink {
                store_id: None,
                document_id: "doc-1".into(),
                document_revision: "rev".into(),
                position: teshi_core::authoring::TextRange::new(0, 5),
                quote: QuoteSelector {
                    quote: "login".into(),
                    prefix: String::new(),
                    suffix: String::new(),
                },
                resolution: teshi_core::authoring::ResolutionState::Resolved,
            }],
            scenario_refs: Vec::new(),
        };

        assert!(TestPointsUiState::approve(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Approved);

        TestPointsUiState::apply_field_buffer(&mut tp, DetailField::Objective, "Changed objective");
        assert_eq!(tp.review_state, ReviewState::Proposed);

        assert!(TestPointsUiState::reject(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Rejected);

        tp.review_state = ReviewState::NeedsReview;
        assert!(TestPointsUiState::approve(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Approved);

        TestPointsUiState::apply_field_buffer(&mut tp, DetailField::Hierarchy, "Billing");
        assert_eq!(tp.review_state, ReviewState::Approved);
    }

    #[test]
    fn test_test_points_tab_select() {
        let mut app = App::from_args().expect("app init should work");
        app.handle_action(Action::SelectTab(MainTab::TestPoints))
            .expect("select test points");
        assert_eq!(app.active_tab, MainTab::TestPoints);
    }

    #[test]
    fn test_rendered_tab_titles_are_clickable_across_their_full_width() {
        let mut app = App::from_args().expect("app init should work");
        render_test_app(&mut app, 100);

        let expected = [
            (MainTab::Explore, Rect::new(1, 0, 13, 1)),
            (MainTab::MindMap, Rect::new(17, 0, 13, 1)),
            (MainTab::Ai, Rect::new(33, 0, 8, 1)),
            (MainTab::Requirements, Rect::new(44, 0, 18, 1)),
            (MainTab::TestPoints, Rect::new(65, 0, 17, 1)),
        ];
        let actual = app
            .clickable_regions
            .iter()
            .filter_map(|region| match region {
                ClickableRegion::Tab { tab, rect } => Some((*tab, *rect)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);

        for (tab, rect) in expected {
            for x in [rect.left(), rect.left() + rect.width / 2, rect.right() - 1] {
                app.active_tab = if tab == MainTab::Explore {
                    MainTab::Ai
                } else {
                    MainTab::Explore
                };
                app.handle_mouse_event(
                    MouseEventKind::Down(MouseButton::Left),
                    x,
                    rect.top(),
                    KeyModifiers::NONE,
                )
                .expect("tab click should succeed");
                assert_eq!(app.active_tab, tab, "click at column {x}");
            }
        }
    }

    #[test]
    fn test_tab_click_moves_underline_to_registered_title() {
        let mut app = App::from_args().expect("app init should work");
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let regions = app
            .clickable_regions
            .iter()
            .filter_map(|region| match region {
                ClickableRegion::Tab { tab, rect } => Some((*tab, *rect)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(regions.len(), 5);
        let project_control = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::ProjectControl { rect } => Some(*rect),
                _ => None,
            });
        for (tab, rect) in regions {
            app.handle_mouse_event(
                MouseEventKind::Down(MouseButton::Left),
                rect.x + rect.width / 2,
                rect.y,
                KeyModifiers::NONE,
            )
            .unwrap();
            assert_eq!(app.active_tab, tab);
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            for x in 0..83 {
                if project_control
                    .is_some_and(|control| control.contains(ratatui::layout::Position::new(x, 1)))
                {
                    continue;
                }
                assert_eq!(
                    buffer[(x, 1)].symbol(),
                    if (rect.x..rect.right()).contains(&x) {
                        "━"
                    } else {
                        "─"
                    },
                    "tab {tab:?}, column {x}"
                );
                if (rect.x..rect.right()).contains(&x) {
                    assert_eq!(buffer[(x, 1)].fg, crate::theme::PRIMARY);
                    assert_eq!(buffer[(x, 0)].bg, crate::theme::TAB_ACTIVE_BG);
                }
            }
        }
    }

    #[test]
    fn test_compact_tabs_remain_fully_clickable_at_narrow_width() {
        let mut app = App::from_args().expect("app init should work");
        render_test_app(&mut app, 55);

        let test_points_rect = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::Tab {
                    tab: MainTab::TestPoints,
                    rect,
                } => Some(*rect),
                _ => None,
            })
            .expect("compact Test Points tab should be registered");
        assert_eq!(test_points_rect.width, "Points [5]".len() as u16);
        assert!(test_points_rect.right() <= 55);
        assert_eq!(
            app.clickable_regions
                .iter()
                .filter(|region| matches!(region, ClickableRegion::Tab { .. }))
                .count(),
            5
        );

        for x in test_points_rect.left()..test_points_rect.right() {
            app.active_tab = MainTab::Explore;
            app.handle_mouse_event(
                MouseEventKind::Down(MouseButton::Left),
                x,
                0,
                KeyModifiers::NONE,
            )
            .expect("visible clipped tab click should succeed");
            assert_eq!(app.active_tab, MainTab::TestPoints);
        }

        app.active_tab = MainTab::Explore;
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            test_points_rect.x - 1,
            0,
            KeyModifiers::NONE,
        )
        .expect("padding click should succeed");
        assert_eq!(app.active_tab, MainTab::Explore);
    }

    #[test]
    fn test_mouse_tab_selection_uses_standard_state_transition() {
        let mut app = App::from_args().expect("app init should work");
        app.active_tab = MainTab::Explore;
        app.view_stage = ViewStage::EditorAndPanel;
        app.mindmap_focus = MindMapFocus::AiPanel;
        app.ai_input_focused = true;
        app.scenario_dropdown_open = false;
        app.explore_edit_mode = true;
        app.explore_detail_open = true;
        app.explore_detail_case = Some((0, 0));
        app.pending_char = Some('g');
        app.quit_pending_confirm = false;
        render_test_app(&mut app, 100);

        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            17,
            0,
            KeyModifiers::NONE,
        )
        .expect("MindMap tab click should succeed");

        assert_eq!(app.active_tab, MainTab::MindMap);
        assert_eq!(app.view_stage, ViewStage::TreeOnly);
        assert_eq!(app.mindmap_focus, MindMapFocus::Main);
        assert!(!app.ai_input_focused);
        assert!(!app.scenario_dropdown_open);
        assert!(!app.explore_edit_mode);
        assert!(!app.explore_detail_open);
        assert!(app.explore_detail_case.is_none());
        assert!(app.pending_char.is_none());
        assert!(!app.quit_pending_confirm);
        assert_eq!(app.status, "Switched to MindMap tab");
    }

    #[test]
    fn test_mouse_wheel_scrolls_mindmap_tree_under_pointer() {
        let steps = (0..30)
            .map(|index| format!("    Given unique step {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!("Feature: Scroll\n  Scenario: Many steps\n{steps}\n");
        let (mut app, path) = feature_file_app("mindmap-mouse-scroll", &content);
        app.active_tab = MainTab::MindMap;
        app.mindmap_ai_panel_visible = true;
        app.mindmap_focus = MindMapFocus::AiPanel;

        for path in app
            .mindmap_index
            .node_paths
            .values()
            .cloned()
            .collect::<Vec<_>>()
        {
            app.tree_state.open(path);
        }
        render_test_app(&mut app, 100);

        let tree_rect = app.tree_panel_rect.expect("tree panel should be rendered");
        assert_eq!(app.tree_state.get_offset(), 0);
        app.handle_mouse_event(
            MouseEventKind::ScrollDown,
            tree_rect.x,
            tree_rect.y,
            KeyModifiers::NONE,
        )
        .expect("tree scroll down should succeed");
        assert_eq!(app.tree_state.get_offset(), 3);

        app.handle_mouse_event(
            MouseEventKind::ScrollUp,
            tree_rect.x,
            tree_rect.y,
            KeyModifiers::NONE,
        )
        .expect("tree scroll up should succeed");
        assert_eq!(app.tree_state.get_offset(), 0);

        fs::remove_file(path).expect("feature fixture should be removed");
    }

    #[test]
    fn test_stale_mindmap_tree_rect_does_not_capture_explore_editor_wheel() {
        let steps = (0..30)
            .map(|index| format!("    Given unique step {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!("Feature: Scroll\n  Scenario: Many steps\n{steps}\n");
        let (mut app, path) = feature_file_app("stale-mindmap-tree-rect", &content);
        app.active_tab = MainTab::MindMap;

        for path in app
            .mindmap_index
            .node_paths
            .values()
            .cloned()
            .collect::<Vec<_>>()
        {
            app.tree_state.open(path);
        }
        render_test_app(&mut app, 100);
        assert!(app.tree_panel_rect.is_some());

        app.active_tab = MainTab::Explore;
        app.explore_edit_mode = true;
        render_test_app(&mut app, 100);

        let editor_rect = app
            .editor_panel_rect
            .expect("editor panel should be rendered");
        assert_eq!(app.scroll_row, 0);
        assert_eq!(app.tree_state.get_offset(), 0);
        app.handle_mouse_event(
            MouseEventKind::ScrollDown,
            editor_rect.x,
            editor_rect.y,
            KeyModifiers::NONE,
        )
        .expect("editor scroll down should succeed");

        assert_eq!(app.scroll_row, 1);
        assert_eq!(app.tree_state.get_offset(), 0);

        fs::remove_file(path).expect("feature fixture should be removed");
    }

    #[test]
    fn test_requirements_create_test_point_from_selection() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        fs::write(
            &feature,
            "Feature: Sample\n  Scenario: placeholder\n    Given noop\n",
        )
        .unwrap();
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .expect("app init should work");
        app.authoring_ui.create_document("req.md", "Req").unwrap();
        app.authoring_ui.focus = crate::authoring_tab::RequirementsFocus::Editor;
        app.authoring_ui.selection_anchor = Some((0, 0));
        app.authoring_ui.selection_end = Some((0, 4));
        app.active_tab = MainTab::Requirements;
        app.handle_action(Action::ReqNewTestPoint)
            .expect("create test point");
        let count = app
            .authoring_ui
            .artifacts
            .as_ref()
            .map(|a| a.test_points.test_points.len())
            .unwrap_or(0);
        assert_eq!(count, 1);
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
    fn deleted_feature_rows_participate_in_explore_keyboard_navigation() {
        let mut app = App::from_args().expect("app init should work");
        let current_path = PathBuf::from("current.feature");
        let current_content = "Feature: Current\n  Scenario: S\n    Given current\n";
        app.project.features = vec![teshi_core::gherkin::parse_feature(
            current_content,
            current_path.clone(),
        )];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            current: vec![teshi_core::git::map_feature_git_diff(
                current_path,
                teshi_core::git::FileGitStatus::Unmodified,
                teshi_core::git::GitFileDiff::from_contents(
                    Some(current_content),
                    Some(current_content),
                ),
            )],
            deleted: vec![teshi_core::git::map_feature_git_diff(
                PathBuf::from("deleted.feature"),
                teshi_core::git::FileGitStatus::Deleted,
                teshi_core::git::GitFileDiff::from_contents(
                    Some("Feature: Deleted\n  Scenario: S\n    Given old\n"),
                    None,
                ),
            )],
            ..Default::default()
        };
        app.explore_focus = ColumnFocus::Feature;
        app.explore_selected_feature = 0;
        app.explore_selected_deleted_feature = None;

        app.handle_action(Action::MoveDown)
            .expect("move to deleted feature");
        assert_eq!(app.explore_selected_deleted_feature, Some(0));
        assert!(app.build_explore_cases().is_empty());

        app.handle_action(Action::RunScenario)
            .expect("deleted feature run should be rejected");
        assert_eq!(app.status, "Deleted Feature cannot be run");

        app.handle_action(Action::EnterEdit)
            .expect("deleted feature edit should be rejected");
        assert_eq!(app.status, "Deleted Feature cannot be edited");

        app.handle_action(Action::MoveUp)
            .expect("move back to current feature");
        assert_eq!(app.explore_selected_deleted_feature, None);
        assert_eq!(app.explore_selected_feature, 0);
    }

    #[test]
    fn deleted_feature_scenario_navigation_uses_head_scenarios() {
        let mut app = App::from_args().expect("app init should work");
        let old = "Feature: Deleted\n  Scenario: First\n    Given old one\n  Scenario: Second\n    Given old two\n";
        app.project.features.clear();
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            deleted: vec![teshi_core::git::map_feature_git_diff(
                PathBuf::from("deleted.feature"),
                teshi_core::git::FileGitStatus::Deleted,
                teshi_core::git::GitFileDiff::from_contents(Some(old), None),
            )],
            ..Default::default()
        };
        app.explore_selected_deleted_feature = Some(0);
        app.explore_focus = ColumnFocus::Scenario;

        app.handle_action(Action::MoveDown)
            .expect("move to the second deleted scenario");
        assert_eq!(app.explore_selected_scenario, 1);

        app.handle_action(Action::MoveUp)
            .expect("move back to the first deleted scenario");
        assert_eq!(app.explore_selected_scenario, 0);
    }

    #[test]
    fn current_feature_head_deleted_scenarios_participate_in_navigation() {
        let mut app = App::from_args().expect("app init should work");
        let old = "Feature: Current\n  Scenario: Keep\n    Given keep\n  Scenario: Removed\n    Given old\n";
        let new = "Feature: Current\n  Scenario: Keep\n    Given keep\n";
        let path = PathBuf::from("current.feature");
        app.project.features = vec![teshi_core::gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = ColumnFocus::Scenario;
        app.explore_selected_scenario = 0;

        app.handle_action(Action::MoveDown)
            .expect("move to the deleted scenario");
        assert_eq!(app.explore_selected_scenario, 1);
        assert_eq!(app.explore_selected_step_count(), 1);
        assert_eq!(
            app.explore_changed_steps()[0].kind,
            crate::app::ChangeKind::Deleted
        );
        assert!(app.explore_changed_steps()[0].text.contains("Given old"));

        app.handle_action(Action::MoveUp)
            .expect("move back to the current scenario");
        assert_eq!(app.explore_selected_scenario, 0);
    }

    #[test]
    fn starting_git_refresh_keeps_deleted_feature_selection() {
        let mut app = editor_test_app();
        let old = "Feature: Deleted\n  Scenario: Removed\n    Given old\n";
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            deleted: vec![teshi_core::git::map_feature_git_diff(
                PathBuf::from("deleted.feature"),
                teshi_core::git::FileGitStatus::Deleted,
                teshi_core::git::GitFileDiff::from_contents(Some(old), None),
            )],
            ..Default::default()
        };
        app.explore_selected_deleted_feature = Some(0);

        app.start_git_status_refresh();

        assert_eq!(app.explore_selected_deleted_feature, Some(0));
        assert_eq!(app.git_status.deleted.len(), 1);
    }

    #[test]
    fn editing_a_buffer_queues_a_git_refresh_for_the_new_revision() {
        let mut app = editor_test_app();
        app.git_status_rx = None;
        app.git_status_refresh_requested = false;
        let revision = app.git_status_revision;

        app.mark_current_buffer_dirty();

        assert_eq!(app.git_status_revision, revision + 1);
        assert!(app.git_status_rx.is_none());
        assert!(app.git_status_refresh_requested);
        assert!(app.git_status_refresh_deadline.is_some());
    }

    #[test]
    fn debounced_git_refresh_starts_after_the_deadline() {
        let mut app = editor_test_app();
        app.git_status_rx = None;
        app.git_status_refresh_requested = false;
        app.git_status_refresh_deadline = None;

        app.mark_current_buffer_dirty();
        app.git_status_refresh_deadline = Some(Instant::now() - Duration::from_millis(1));
        app.poll_git_status();

        assert!(app.git_status_rx.is_some());
    }

    #[test]
    fn idle_git_poll_does_not_start_a_periodic_refresh() {
        let mut app = editor_test_app();
        app.git_status_rx = None;
        app.git_status_refresh_requested = false;
        app.git_status_refresh_deadline = None;

        app.poll_git_status();

        assert!(app.git_status_rx.is_none());
    }

    #[test]
    fn stale_git_refresh_is_followed_by_a_refresh_for_the_latest_revision() {
        let mut app = editor_test_app();
        app.git_status_rx = None;
        app.git_status_refresh_requested = false;
        app.git_status_revision = 7;
        let (sender, receiver) = std::sync::mpsc::channel();
        app.git_status_rx = Some(receiver);
        sender
            .send(GitStatusRefreshResult {
                revision: 6,
                project_stamp: app.current_project_stamp(),
                model: teshi_core::git::FeatureGitStatusModel::default(),
            })
            .expect("stale refresh should be delivered");

        app.poll_git_status();
        assert!(app.git_status_refresh_requested);

        drop(sender);
        app.poll_git_status();
        assert!(app.git_status_rx.is_some());
        assert!(!app.git_status_refresh_requested);
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
        use std::path::PathBuf;
        use teshi_core::gherkin;

        let content = "Feature: Test\n  Scenario: S1\n    Given original step\n";
        let feature = gherkin::parse_feature(content, PathBuf::from("test.feature"));
        let project = teshi_core::gherkin::BddProject {
            root_dir: PathBuf::from("."),
            features: vec![feature],
        };
        let step_index = StepIndex::build(&project);
        let buffers = vec![EditorBuffer::from_string(content.to_string())];
        let mut mindmap_index = crate::mindmap::build_index(&project);
        let tree_state = crate::mindmap::init_tree_state(&mut mindmap_index);

        let mut app = App {
            project,
            git_status: teshi_core::git::FeatureGitStatusModel::default(),
            git_status_rx: None,
            git_status_refresh_requested: false,
            git_status_refresh_deadline: None,
            git_status_revision: 0,
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
            buffer_diagnostics: Vec::new(),
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
            mouse_position: None,
            editor_panel_rect: None,
            chat_layout_cache: Default::default(),
            chat_history_rect: None,
            chat_selection: None,
            system_clipboard: None,
            chat_selection_dragging: false,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            screenshot: Default::default(),
            capture_index: Default::default(),
            explore_captures: HashMap::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: None,
            runner_rx: None,
            runner_cancel: None,
            last_external_check: Instant::now(),
            feature_discovery_root: None,
            feature_discovery_worker: None,
            lazy_features: None,
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Step,
            explore_selected_feature: 0,
            feature_browser: Default::default(),
            explore_selected_deleted_feature: None,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_horizontal_scroll: [0; 3],
            explore_viewports: Default::default(),
            explore_horizontal_scroll_drag: None,
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
            explore_state: None,
            approval_mode: teshi_agent::approval::ApprovalMode::default(),
            approval_panel_active: false,
            approval_panel_selection: 0,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            quit_pending_confirm: false,
            quit_panel_selection: 0,
            status_message: None,
            status_message_deadline: None,
            version: String::new(),
            config: crate::config::load_config().unwrap(),
            auth_panel_active: false,
            model_profiles: crate::profiles::load_all(),
            model_active_id: crate::profiles::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            active_model_label: None,
            model_panel_mode: ModelPanelMode::List,
            model_form_editing_id: None,
            model_form_focus: 0,
            model_form_cursor: 0,
            pending_delete: None,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            session_panel_active: false,
            session_panel_legacy: false,
            pending_session_restore: None,
            session_panel_selection: 0,
            session_list: Vec::new(),
            agent_registry: teshi_agent::registry::AgentRegistry::load(None),
            agent_profile_panel_active: false,
            agent_profile_panel_selection: 0,
            agent_panel_mode: AgentPanelMode::List,
            installed_acp_agents: Vec::new(),
            acp_registry_agents: Vec::new(),
            acp_registry_rx: None,
            acp_install_rx: None,
            generation_stage: teshi_agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
            generation_scope: None,
            generation_scope_prompt: None,
            generation_paused_reason: None,
            authoring_ui: crate::authoring_tab::AuthoringUiState::empty(),
            test_points_ui: crate::test_points_tab::TestPointsUiState::empty(),
            requirements_root: teshi_engine::requirements_data_dir(None).unwrap_or_default(),
            project_switch: crate::project_context::ProjectSwitchState::new(
                teshi_engine::ProjectIdentity::from_root("."),
            ),
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
        use std::path::PathBuf;
        use teshi_core::gherkin;

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
        let project = teshi_core::gherkin::BddProject {
            root_dir: PathBuf::from("."),
            features: vec![fa, fb],
        };
        let step_index = StepIndex::build(&project);
        let mut mindmap_index = crate::mindmap::build_index(&project);
        let tree_state = crate::mindmap::init_tree_state(&mut mindmap_index);

        let mut app = App {
            project,
            git_status: teshi_core::git::FeatureGitStatusModel::default(),
            git_status_rx: None,
            git_status_refresh_requested: false,
            git_status_refresh_deadline: None,
            git_status_revision: 0,
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
            buffer_diagnostics: Vec::new(),
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
            mouse_position: None,
            editor_panel_rect: None,
            chat_layout_cache: Default::default(),
            chat_history_rect: None,
            chat_selection: None,
            system_clipboard: None,
            chat_selection_dragging: false,
            clickable_regions: Vec::new(),
            tree_panel_rect: None,
            preview_panel_rect: None,
            screenshot: Default::default(),
            capture_index: Default::default(),
            explore_captures: HashMap::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            runner_config: None,
            runner_rx: None,
            runner_cancel: None,
            last_external_check: Instant::now(),
            feature_discovery_root: None,
            feature_discovery_worker: None,
            lazy_features: None,
            external_change_prompt: None,
            pending_agent_changes: Vec::new(),
            pending_change_diffs: Vec::new(),
            pending_change_summary: Vec::new(),
            explore_diff_lines: None,
            change_summary_visible: false,
            change_summary_selection: 0,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            feature_browser: Default::default(),
            explore_selected_deleted_feature: None,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_horizontal_scroll: [0; 3],
            explore_viewports: Default::default(),
            explore_horizontal_scroll_drag: None,
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
            explore_state: None,
            approval_mode: teshi_agent::approval::ApprovalMode::default(),
            approval_panel_active: false,
            approval_panel_selection: 0,
            slash_suggestion_active: false,
            slash_suggestion_selection: 0,
            ai_input_focused: true,
            quit_pending_confirm: false,
            quit_panel_selection: 0,
            status_message: None,
            status_message_deadline: None,
            version: String::new(),
            config: crate::config::load_config().unwrap(),
            auth_panel_active: false,
            model_profiles: crate::profiles::load_all(),
            model_active_id: crate::profiles::read_active_id(),
            model_panel_active: false,
            model_panel_selection: 0,
            active_model_label: None,
            model_panel_mode: ModelPanelMode::List,
            model_form_editing_id: None,
            model_form_focus: 0,
            model_form_cursor: 0,
            pending_delete: None,
            model_form_name: String::new(),
            model_form_provider: String::new(),
            model_form_model: String::new(),
            model_form_base_url: String::new(),
            model_form_api_key: String::new(),
            model_form_max_tokens: String::from("4096"),
            model_form_temperature: String::from("0.7"),
            session_panel_active: false,
            session_panel_legacy: false,
            pending_session_restore: None,
            session_panel_selection: 0,
            session_list: Vec::new(),
            agent_registry: teshi_agent::registry::AgentRegistry::load(None),
            agent_profile_panel_active: false,
            agent_profile_panel_selection: 0,
            agent_panel_mode: AgentPanelMode::List,
            installed_acp_agents: Vec::new(),
            acp_registry_agents: Vec::new(),
            acp_registry_rx: None,
            acp_install_rx: None,
            generation_stage: teshi_agent::pipeline::GenerationStage::Idle,
            pipeline_requirement: None,
            pipeline_plan: None,
            generation_scope: None,
            generation_scope_prompt: None,
            generation_paused_reason: None,
            authoring_ui: crate::authoring_tab::AuthoringUiState::empty(),
            test_points_ui: crate::test_points_tab::TestPointsUiState::empty(),
            requirements_root: teshi_engine::requirements_data_dir(None).unwrap_or_default(),
            project_switch: crate::project_context::ProjectSwitchState::new(
                teshi_engine::ProjectIdentity::from_root("."),
            ),
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
    fn test_manual_external_refresh_waits_for_active_runner() {
        let original = "Feature: T\n  Scenario: S\n    Given one\n";
        let updated = "Feature: T\n  Scenario: S\n    Given changed during run\n";
        let (mut app, path) = feature_file_app("external-running", original);
        let (_sender, receiver) = std::sync::mpsc::channel();
        app.runner_rx = Some(receiver);
        fs::write(&path, updated).expect("updated feature should be written");

        app.handle_action(Action::RefreshFeatures).unwrap();
        assert!(app.runner_rx.is_some());
        assert_eq!(app.buffer.as_string(), original);
        assert_eq!(app.project.features[0].scenarios[0].steps[0].text, "one");

        app.runner_rx = None;
        app.last_external_check = Instant::now() - Duration::from_secs(2);
        app.poll_external_feature_changes();
        assert_eq!(app.buffer.as_string(), updated);
        assert_eq!(
            app.project.features[0].scenarios[0].steps[0].text,
            "changed during run"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_external_scan_failure_keeps_loaded_project() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        let path = project.path().join("kept.feature");
        fs::write(&path, "Feature: Kept\n  Scenario: S\n    Given retained\n").unwrap();
        let mut app = App::from_directory(
            project.path(),
            crate::config::load_config().unwrap(),
            true,
            store.path().to_path_buf(),
        )
        .unwrap();
        // A missing scan root deterministically exercises the traversal error path
        // without changing permissions or deleting anything outside the fixture.
        app.project.root_dir = project.path().join("missing-scan-root");
        app.handle_action(Action::RefreshFeatures).unwrap();
        assert_eq!(app.project.features.len(), 1);
        assert_eq!(app.project.features[0].file_path, path);
        assert_eq!(
            app.project.features[0].scenarios[0].steps[0].text,
            "retained"
        );
        assert!(
            app.status
                .contains("Feature refresh incomplete; retained project")
        );
        assert!(app.status_message.as_deref().is_some_and(|message| {
            message.contains("Feature refresh incomplete; retained project")
        }));
    }

    #[test]
    fn background_external_scan_failure_is_visible() {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        let path = project.path().join("kept.feature");
        fs::write(&path, "Feature: Kept\n  Scenario: S\n    Given retained\n").unwrap();
        let mut app = App::from_project_directory(
            project.path(),
            crate::config::load_config().unwrap(),
            true,
            false,
            store.path().to_path_buf(),
        )
        .unwrap();
        fs::write(project.path().join("unreadable.feature"), [0xff, 0xfe]).unwrap();
        await_catalog(&mut app, 2);
        load_catalog_file(&mut app, 0);
        app.select_catalog_feature(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll_lazy_features();
            if app.status.contains("Feature loading failed") {
                break;
            }
            assert!(Instant::now() < deadline, "content error was not shown");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.project.features[0].name, "Kept");
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

    // ── Exploration state machine tests ──

    #[test]
    fn test_start_exploration_sets_initial_state() {
        let mut app = editor_test_app();
        let steps = vec![
            (1, "Given".into(), "I am on the login page".into()),
            (2, "When".into(), "I enter my password".into()),
        ];
        app.start_exploration("features/login.feature".into(), steps.clone(), 0);

        let state = app
            .explore_state
            .as_ref()
            .expect("explore_state should be set");
        assert_eq!(state.feature_path, "features/login.feature");
        assert_eq!(state.steps.len(), 2);
        assert_eq!(state.current_step, 0);
        assert_eq!(state.step_count, 0);
        assert_eq!(state.max_steps, 15);
        assert!(!state.completed);
        assert!(!state.terminated);
        assert!(state.termination_reason.is_none());
        assert!(state.url_whitelist.is_empty());
        assert!(state.current_url.is_empty());
        assert!(state.trace.is_empty());
    }

    #[test]
    fn test_record_exploration_action_increments_step_count() {
        let mut app = editor_test_app();
        app.start_exploration("test.feature".into(), vec![], 0);

        assert_eq!(app.explore_state.as_ref().unwrap().step_count, 0);

        app.record_exploration_action("browser_click".into(), Some("e1".into()), None, true, None);
        assert_eq!(app.explore_state.as_ref().unwrap().step_count, 1);

        app.record_exploration_action(
            "browser_type".into(),
            Some("e2".into()),
            Some("hello".into()),
            true,
            None,
        );
        assert_eq!(app.explore_state.as_ref().unwrap().step_count, 2);
    }

    #[test]
    fn test_record_exploration_action_adds_trace_entry() {
        let mut app = editor_test_app();
        app.start_exploration("test.feature".into(), vec![], 0);

        app.record_exploration_action("browser_click".into(), Some("e15".into()), None, true, None);

        let state = app.explore_state.as_ref().unwrap();
        assert_eq!(state.trace.len(), 1);
        assert_eq!(state.trace[0].action, "browser_click");
        assert_eq!(state.trace[0].ref_id, Some("e15".into()));
        assert!(state.trace[0].ok);
        assert!(state.trace[0].error.is_none());
    }

    #[test]
    fn test_record_exploration_action_no_state_does_not_panic() {
        let mut app = editor_test_app();
        // No explore_state set
        app.record_exploration_action("browser_click".into(), None, None, true, None);
        // Should not panic
    }

    #[test]
    fn test_check_exploration_limits_no_state() {
        let mut app = editor_test_app();
        assert!(!app.check_exploration_limits());
    }

    #[test]
    fn test_check_exploration_limits_step_limit_terminates() {
        let mut app = editor_test_app();
        app.start_exploration("test.feature".into(), vec![], 0);

        // Manually set step_count to exceed max_steps
        let state = app.explore_state.as_mut().unwrap();
        state.step_count = 15;
        state.max_steps = 15;

        let terminated = app.check_exploration_limits();
        assert!(terminated);

        let state = app.explore_state.as_ref().unwrap();
        assert!(state.terminated);
        assert_eq!(
            state.termination_reason.as_deref(),
            Some("Step limit exceeded (max 15)")
        );
    }

    #[test]
    fn test_check_exploration_limits_below_limit_does_not_terminate() {
        let mut app = editor_test_app();
        app.start_exploration("test.feature".into(), vec![], 0);

        let state = app.explore_state.as_mut().unwrap();
        state.step_count = 10;
        state.max_steps = 15;

        assert!(!app.check_exploration_limits());
        assert!(!app.explore_state.as_ref().unwrap().terminated);
    }

    #[test]
    fn test_check_exploration_limits_already_terminated_returns_false() {
        let mut app = editor_test_app();
        app.start_exploration("test.feature".into(), vec![], 0);
        {
            let state = app.explore_state.as_mut().unwrap();
            state.terminated = true;
            state.step_count = 99;
            state.max_steps = 15;
        }
        assert!(!app.check_exploration_limits());
    }

    #[test]
    fn test_check_exploration_limits_already_completed_returns_false() {
        let mut app = editor_test_app();
        app.start_exploration("test.feature".into(), vec![], 0);
        {
            let state = app.explore_state.as_mut().unwrap();
            state.completed = true;
            state.step_count = 99;
            state.max_steps = 15;
        }
        assert!(!app.check_exploration_limits());
    }

    // ── URL sandbox tests ──

    #[test]
    fn test_is_url_allowed_empty_whitelist() {
        assert!(App::is_url_allowed("https://example.com", &[]));
        assert!(App::is_url_allowed("https://evil.com", &[]));
    }

    #[test]
    fn test_is_url_allowed_matching_substring() {
        let whitelist = vec!["example.com".to_string()];
        assert!(App::is_url_allowed("https://example.com/login", &whitelist));
        assert!(App::is_url_allowed("http://example.com", &whitelist));
        assert!(App::is_url_allowed(
            "https://sub.example.com/page",
            &whitelist
        ));
    }

    #[test]
    fn test_is_url_allowed_non_matching() {
        let whitelist = vec!["example.com".to_string()];
        assert!(!App::is_url_allowed("https://evil.com", &whitelist));
        assert!(!App::is_url_allowed("https://malicious.org", &whitelist));
    }

    #[test]
    fn test_is_url_allowed_multiple_patterns() {
        let whitelist = vec!["example.com".to_string(), "trusted.org".to_string()];
        assert!(App::is_url_allowed("https://example.com", &whitelist));
        assert!(App::is_url_allowed("https://trusted.org/page", &whitelist));
        assert!(!App::is_url_allowed("https://evil.com", &whitelist));
    }

    #[test]
    fn screenshot_panel_opens_and_closes_on_mindmap_i() {
        let mut app = editor_test_app();
        app.mindmap_focus = MindMapFocus::Main;

        assert!(app.handle_screenshot_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)));
        assert!(app.screenshot.visible);
        assert!(app.handle_screenshot_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(!app.screenshot.visible);

        // The binding stays inert outside the MindMap tab and off the tree.
        app.active_tab = MainTab::Ai;
        assert!(!app.handle_screenshot_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)));
        app.active_tab = MainTab::MindMap;
        app.mindmap_focus = MindMapFocus::AiPanel;
        assert!(!app.handle_screenshot_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)));
    }

    #[test]
    fn screenshot_zoom_keys_do_not_switch_tabs_and_close_cancels_loading() {
        let mut app = editor_test_app();
        app.screenshot.visible = true;
        app.screenshot.loading = true;
        for (key, label) in [('1', "100%"), ('+', "125%"), ('-', "100%"), ('0', "Fit")] {
            assert!(
                app.handle_screenshot_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
            );
            assert_eq!(app.screenshot.view.label(), label);
            assert_eq!(app.active_tab, MainTab::MindMap);
        }
        app.screenshot.loading = true;
        assert!(app.handle_screenshot_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(!app.screenshot.visible);
        assert!(!app.screenshot.loading);
    }

    /// Writes one replay run whose captures cover the given steps in order.
    fn write_capture_run(project: &Path, feature: &str, scenario: &str, steps: &[&str]) {
        let run =
            project.join(".teshi/logs/replay-screenshots/11111111-1111-4111-8111-111111111111");
        fs::create_dir_all(&run).unwrap();
        let events: Vec<serde_json::Value> = steps
            .iter()
            .enumerate()
            .map(|(index, text)| {
                serde_json::json!({
                    "type": "step_screenshot",
                    "capture_status": "available",
                    "screenshot_file": format!("step-{index}.png"),
                    "feature": feature,
                    "scenario": scenario,
                    "step_text": text,
                    "step_line": index + 3,
                    "status": "passed",
                    "captured_at": "2026-10-31T00:00:00Z",
                })
            })
            .collect();
        fs::write(
            run.join("run.json"),
            serde_json::json!({
                "started_at": "2026-10-31T00:00:00Z",
                "events": events,
            })
            .to_string(),
        )
        .unwrap();
    }

    /// App with one Feature whose two steps have captures.
    fn explore_capture_test_app() -> (App, tempfile::TempDir, tempfile::TempDir) {
        let project = tempdir().unwrap();
        let store = tempdir().unwrap();
        fs::write(
            project.path().join("marked.feature"),
            "Feature: Marked\n  Scenario: Detail\n    Given a visible step\n    Then broken step\n",
        )
        .unwrap();
        write_capture_run(
            project.path(),
            "marked.feature",
            "Detail",
            &["a visible step", "broken step"],
        );
        let mut app = App::from_directory(
            project.path(),
            crate::config::load_config().unwrap(),
            false,
            store.path().to_path_buf(),
        )
        .unwrap();
        app.active_tab = MainTab::Explore;
        (app, project, store)
    }

    #[test]
    fn explore_panes_mark_the_scenario_and_steps_that_have_captures() {
        let (mut app, _project, _store) = explore_capture_test_app();

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = (0..30)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|cell| buffer[cell].symbol())
            .collect();
        assert!(text.contains("Detail ●2"), "Explore was: {text}");
        assert!(text.contains("a visible step ●1"), "Explore was: {text}");
        assert!(text.contains("broken step ●1"), "Explore was: {text}");
    }

    #[test]
    fn explore_steps_open_the_viewer_at_the_selected_step() {
        let (mut app, _project, _store) = explore_capture_test_app();
        app.explore_focus = ColumnFocus::Step;
        app.explore_selected_step = 1;

        assert!(app.handle_screenshot_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)));
        assert!(app.screenshot.visible);
        assert_eq!(
            app.screenshot.origin,
            crate::image_view::ScreenshotOrigin::Explore
        );
        assert_eq!(app.screenshot.refs.len(), 2);
        assert_eq!(app.screenshot.index, 1);

        // Closing hands the last viewed capture back to the Steps pane.
        app.screenshot.index = 0;
        assert!(app.handle_screenshot_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(!app.screenshot.visible);
        assert_eq!(app.explore_selected_step, 0);
    }

    #[test]
    fn explore_i_stays_inert_off_the_steps_pane() {
        let (mut app, _project, _store) = explore_capture_test_app();
        app.explore_focus = ColumnFocus::Scenario;

        assert!(!app.handle_screenshot_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)));
        assert!(!app.screenshot.visible);

        app.explore_edit_mode = true;
        app.explore_focus = ColumnFocus::Step;
        assert!(!app.handle_screenshot_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)));
        assert!(!app.screenshot.visible);
    }

    #[test]
    fn failure_details_open_the_capture_of_the_failed_step() {
        let (mut app, _project, _store) = explore_capture_test_app();
        let mut detail = super::empty_running_detail("case-1".to_string());
        detail.status = RunStatus::Failed;
        detail.steps = vec![
            super::StepRunRecord {
                step_id: "s1".into(),
                text: "a visible step".into(),
                status: Some("passed".into()),
                message: None,
            },
            super::StepRunRecord {
                step_id: "s2".into(),
                text: "broken step".into(),
                status: Some("failed".into()),
                message: Some("boom".into()),
            },
        ];
        app.explore_case_details.insert((0, 0), detail);
        app.explore_detail_open = true;
        app.explore_detail_case = Some((0, 0));
        // The popup owns `i` from any Explore pane.
        app.explore_focus = ColumnFocus::Scenario;

        assert!(app.handle_screenshot_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)));
        assert!(app.screenshot.visible);
        assert_eq!(app.screenshot.index, 1);
        assert_eq!(app.screenshot.refs[1].step_text, "broken step");
    }

    #[test]
    fn screenshot_overlay_lists_scenarios_that_have_captures() {
        let mut app = editor_test_app();
        app.active_tab = MainTab::MindMap;
        app.screenshot.visible = true;
        app.screenshot.support = crate::image_view::GraphicsSupport::None;
        app.capture_index = crate::image_view::CaptureIndex {
            entries: vec![crate::image_view::CaptureEntry {
                feature: "features/demo.feature".into(),
                scenario: "First".into(),
                count: 4,
                step_counts: vec![("Given a".into(), 4)],
                started_at: "2026-10-30T00:00:00Z".into(),
            }],
        };

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = (0..30)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|cell| buffer[cell].symbol())
            .collect();
        assert!(
            text.contains("features/demo.feature · First (4)"),
            "overlay was: {text}"
        );
    }

    #[test]
    fn screenshot_overlay_explains_a_missing_pixel_protocol() {
        let mut app = editor_test_app();
        app.active_tab = MainTab::MindMap;
        app.screenshot.visible = true;
        app.screenshot.support = crate::image_view::GraphicsSupport::None;
        app.screenshot.refs = vec![crate::image_view::ScreenshotRef {
            run_id: "run-1".into(),
            file: "frame.png".into(),
            scenario: "First".into(),
            step_text: "Given a".into(),
            step_line: Some(4),
            captured_at: "2026-01-01T00:00:00Z".into(),
            status: "passed".into(),
        }];

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = (0..30)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|cell| buffer[cell].symbol())
            .collect();
        assert!(text.contains("no image protocol"), "overlay was: {text}");
        assert!(text.contains("frame.png"), "overlay was: {text}");
    }

    #[test]
    fn test_is_url_allowed_case_sensitive() {
        let whitelist = vec!["Example".to_string()];
        // substring match is case-sensitive by default
        assert!(App::is_url_allowed("https://Example.com", &whitelist));
        assert!(!App::is_url_allowed("https://example.com", &whitelist));
    }
}
