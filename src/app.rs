use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

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
use crate::runner::{self, RunCase, RunEvent, RunRequest, RunnerConfig};
use crate::step_index::StepIndex;

/// Step keywords in cycle order (re-exported for UI pickers).
pub use crate::bdd_nav::STEP_KEYWORDS_CYCLE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    MindMap,
    Explore,
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

/// Focused column in the Explore tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFocus {
    Feature,
    Scenario,
    Step,
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
pub struct FailureDetail {
    pub message: String,
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

pub struct App {
    // ── Multi-file project ──────────────────────────────────────────
    pub project: BddProject,
    pub step_index: StepIndex,
    pub mindmap_index: mindmap::MindMapIndex,
    pub mindmap_location_selection: HashMap<String, usize>,
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
    pub runner_config: Option<RunnerConfig>,
    runner_rx: Option<Receiver<RunEvent>>,
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
    pub explore_failure_details: HashMap<(usize, usize), FailureDetail>,
    pub explore_detail_open: bool,
    pub explore_detail_case: Option<(usize, usize)>,
    pub explore_run_summary: Option<RunSummary>,
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

    fn from_directory(dir: &Path) -> Result<Self> {
        let project = gherkin::parse_project(dir);
        let step_index = StepIndex::build(&project);
        let mindmap_index = mindmap::build_index(&project);
        let buffers: Vec<EditorBuffer> = project
            .features
            .iter()
            .map(|f| {
                let content = fs::read_to_string(&f.file_path).unwrap_or_default();
                EditorBuffer::from_string(content)
            })
            .collect();
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
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_failure_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            quit_pending_confirm: false,
        };
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
        let tree_state = mindmap::init_tree_state(&mindmap_index);
        let mut app = Self {
            project,
            step_index,
            mindmap_index,
            mindmap_location_selection: HashMap::new(),
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
            preview_buffer: None,
            preview_title: String::new(),
            preview_cursor_row: 0,
            preview_scroll_row: 0,
            should_quit: false,
            active_tab: MainTab::Explore,
            dirty: false,
            status: "Opened file".to_string(),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_failure_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            quit_pending_confirm: false,
        };
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
            status: "New buffer".to_string(),
            step_input_active: false,
            step_input_row: 0,
            step_input_min_col: 0,
            step_keyword_picker: None,
            runner_config: runner::load_runner_config(None).ok(),
            runner_rx: None,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_failure_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            quit_pending_confirm: false,
        };
        app.sync_cursor_to_first_node();
        app.normalize_explore_selection();
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
                }
            }
            RunEvent::CasePassed { case_id, .. } => {
                if let Some(key) = self.explore_case_map.get(&case_id).copied() {
                    self.explore_case_status.insert(key, RunStatus::Passed);
                    self.explore_failure_details.remove(&key);
                    if let Some(summary) = self.explore_run_summary.as_mut() {
                        summary.passed = summary.passed.saturating_add(1);
                    }
                }
            }
            RunEvent::CaseFailed { case_id, error, .. } => {
                if let Some(key) = self.explore_case_map.get(&case_id).copied() {
                    self.explore_case_status.insert(key, RunStatus::Failed);
                    self.explore_failure_details.insert(
                        key,
                        FailureDetail {
                            message: error.message,
                            stack: error.stack,
                            attachments: error.attachments,
                            logs: Vec::new(),
                        },
                    );
                    if let Some(summary) = self.explore_run_summary.as_mut() {
                        summary.failed = summary.failed.saturating_add(1);
                    }
                }
            }
            RunEvent::CaseSkipped { case_id, .. } => {
                if let Some(key) = self.explore_case_map.get(&case_id).copied() {
                    self.explore_case_status.insert(key, RunStatus::Skipped);
                    if let Some(summary) = self.explore_run_summary.as_mut() {
                        summary.skipped = summary.skipped.saturating_add(1);
                    }
                }
            }
            RunEvent::Log { case_id, message } => {
                if let Some(case_id) = case_id
                    && let Some(key) = self.explore_case_map.get(&case_id).copied()
                    && let Some(detail) = self.explore_failure_details.get_mut(&key)
                {
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
                    && let Some(detail) = self.explore_failure_details.get_mut(&key)
                {
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
        self.explore_failure_details.clear();
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
        if self.explore_case_status.get(&key) == Some(&RunStatus::Failed)
            && self.explore_failure_details.contains_key(&key)
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
        self.step_index = StepIndex::build(&self.project);
        self.mindmap_index = mindmap::build_index(&self.project);
        self.tree_state = mindmap::init_tree_state(&self.mindmap_index);
        self.mindmap_location_selection.clear();
        self.normalize_explore_selection();
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
                } else if self.explore_detail_open {
                    self.explore_detail_open = false;
                    self.explore_detail_case = None;
                } else if self.active_tab == MainTab::Explore && self.explore_edit_mode {
                    self.explore_exit_edit();
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
        if self.active_tab == MainTab::Explore {
            self.explore_edit_mode = false;
            self.explore_detail_open = false;
            self.explore_detail_case = None;
        }
        self.quit_pending_confirm = false;
        self.active_tab = tab;
        if self.active_tab == MainTab::MindMap {
            self.view_stage = ViewStage::TreeOnly;
        }
        self.status = match tab {
            MainTab::MindMap => "Switched to MindMap tab",
            MainTab::Explore => "Switched to Explore tab",
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
        (self.active_tab == MainTab::MindMap && self.view_stage == ViewStage::EditorAndPanel)
            || (self.active_tab == MainTab::Explore && self.explore_edit_mode)
    }

    pub fn is_editor_nav_mode(&self) -> bool {
        self.is_editor_active() && !self.step_input_active && self.step_keyword_picker.is_none()
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
            if self.focus_slot == BddFocusSlot::Keyword {
                let line = self.buffer.line(self.cursor_row);
                if keyword_char_range(&line).is_some() {
                    if self.active_tab == MainTab::Explore && self.explore_edit_mode {
                        self.explore_exit_edit();
                        self.quit_pending_confirm = false;
                    } else if self.active_tab == MainTab::MindMap
                        && self.view_stage == ViewStage::EditorAndPanel
                    {
                        self.stage_back();
                    }
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
    use std::collections::HashMap;

    use super::{
        App, BddFocusSlot, ColumnFocus, MainTab, ViewStage, current_step_keyword_index,
        replace_step_keyword_line,
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
            runner_config: None,
            runner_rx: None,
            explore_focus: ColumnFocus::Step,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_failure_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            quit_pending_confirm: false,
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
            runner_config: None,
            runner_rx: None,
            explore_focus: ColumnFocus::Feature,
            explore_selected_feature: 0,
            explore_selected_scenario: 0,
            explore_selected_step: 0,
            explore_edit_mode: false,
            explore_feature_scenario_memory: HashMap::new(),
            explore_scenario_step_memory: HashMap::new(),
            explore_case_map: HashMap::new(),
            explore_case_status: HashMap::new(),
            explore_failure_details: HashMap::new(),
            explore_detail_open: false,
            explore_detail_case: None,
            explore_run_summary: None,
            quit_pending_confirm: false,
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
}
