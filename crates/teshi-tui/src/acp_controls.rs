//! Conversation-local bottom controls and a keyboard-accessible ACP picker.

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::{
        collections::HashMap,
        path::PathBuf,
        time::{Duration, Instant},
    };

    fn app_with_agent() -> (App, tempfile::TempDir, tempfile::TempDir) {
        configured_app(true)
    }

    fn configured_app(connect: bool) -> (App, tempfile::TempDir, tempfile::TempDir) {
        configured_mode(connect, "settings")
    }

    fn configured_mode(connect: bool, mode: &str) -> (App, tempfile::TempDir, tempfile::TempDir) {
        let project = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        teshi_engine::initialize_requirement_store(store.path()).unwrap();
        let feature = project.path().join("sample.feature");
        std::fs::write(&feature, "Feature: Sample\n").unwrap();
        let mut app = App::from_file(
            &feature,
            crate::config::load_config().unwrap(),
            store.path().to_path_buf(),
        )
        .unwrap();
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py");
        app.agent_mut().backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
            teshi_agent::backend::AgentBackendKind::Acp,
            1,
        );
        app.agent_mut()
            .backend
            .configure_acp(teshi_agent_runtime::backend::AcpBackendConfig::new(
                teshi_acp::AcpAgentCommand {
                    program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
                    args: vec![script.display().to_string(), mode.into()],
                    cwd: project.path().to_path_buf(),
                    env: HashMap::new(),
                },
            ))
            .unwrap();
        if !connect {
            return (app, project, store);
        }
        app.agent_mut().backend.connect_acp().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.agent().backend.acp_controls().unwrap().ready {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        (app, project, store)
    }

    fn wait_for_setting(app: &App) -> teshi_agent_runtime::AcpControls {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = app.agent().backend.acp_controls().unwrap();
            if !state.pending {
                return state;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn approval_command_changes_remote_policy_and_retains_rejected_value() {
        let (mut app, _project, _store) = configured_mode(true, "settings_approval");
        let original = app.agent().backend.acp_controls().unwrap();
        app.execute_slash_command("approval").unwrap();
        assert!(!app.approval_panel_active);
        let picker = app.agent().acp_controls_ui.picker.as_ref().unwrap();
        assert_eq!(picker.option.as_ref().unwrap().id, "approval_policy");
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let changed = wait_for_setting(&app);
        assert_eq!(changed.config.approval_options()[0].current, "auto");
        assert_eq!(changed.session_id, original.session_id);
        assert_eq!(changed.policy, original.policy);
        assert_eq!(
            app.approval_mode,
            teshi_agent::approval::ApprovalMode::Manual
        );
        app.execute_slash_command("approval").unwrap();
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let rejected = wait_for_setting(&app);
        assert_eq!(rejected.config.approval_options()[0].current, "auto");
        assert!(
            rejected
                .error
                .as_deref()
                .unwrap()
                .contains("setting rejected")
        );
    }

    #[test]
    fn approval_command_supports_legacy_modes() {
        let (mut app, _project, _store) = configured_mode(true, "approval_legacy");
        app.execute_slash_command("approve").unwrap();
        assert_eq!(
            app.agent()
                .acp_controls_ui
                .picker
                .as_ref()
                .unwrap()
                .option
                .as_ref()
                .unwrap()
                .route,
            teshi_acp::session_config::ConfigRoute::Mode
        );
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            wait_for_setting(&app).config.approval_options()[0].current,
            "auto"
        );
    }

    #[test]
    fn approval_command_waits_for_discovery_and_multiple_options_stay_filtered() {
        let (mut app, _project, _store) = configured_mode(false, "settings_approval_delayed");
        app.execute_slash_command("approval").unwrap();
        assert!(
            app.agent()
                .acp_controls_ui
                .picker
                .as_ref()
                .unwrap()
                .option
                .is_none()
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .agent()
            .acp_controls_ui
            .picker
            .as_ref()
            .unwrap()
            .option
            .is_none()
        {
            assert!(Instant::now() < deadline);
            app.refresh_acp_controls();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.agent()
                .acp_controls_ui
                .picker
                .as_ref()
                .unwrap()
                .option
                .as_ref()
                .unwrap()
                .id,
            "approval_policy"
        );
        let (mut app, _project, _store) = configured_mode(true, "settings_approval_multiple");
        app.execute_slash_command("app").unwrap();
        let picker = app.agent().acp_controls_ui.picker.as_ref().unwrap();
        assert!(picker.option.is_none());
        assert_eq!(
            picker
                .scope
                .options(&app.agent().backend.acp_controls().unwrap().config)
                .len(),
            2
        );
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.agent()
                .acp_controls_ui
                .picker
                .as_ref()
                .unwrap()
                .option
                .as_ref()
                .unwrap()
                .id,
            "permissions"
        );
    }

    #[test]
    fn unavailable_remote_approval_is_visible_and_local_policy_remains_accessible() {
        let (mut app, _project, _store) = app_with_agent();
        app.execute_slash_command("approval").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| render_picker(frame, &mut app))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("does not advertise approval"), "{content}");
        assert!(!app.approval_panel_active);
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.execute_slash_command("approval local").unwrap();
        assert!(app.approval_panel_active);
        app.approval_panel_active = false;
        app.agent_mut().backend = teshi_agent_runtime::backend::AgentBackendRuntime::new(
            teshi_agent::backend::AgentBackendKind::Native,
            1,
        );
        app.execute_slash_command("approval").unwrap();
        assert!(app.approval_panel_active);
    }

    #[test]
    fn browsing_chat_does_not_connect_until_settings_are_explicitly_opened() {
        let (mut app, _project, _store) = configured_app(false);
        for tab in [
            crate::app::MainTab::Explore,
            crate::app::MainTab::Ai,
            crate::app::MainTab::MindMap,
            crate::app::MainTab::Ai,
        ] {
            app.active_tab = tab;
            app.mindmap_ai_panel_visible = true;
            app.poll_llm_events();
            assert!(!app.agent().acp_controls_ui.discovery_started);
            assert!(!app.agent().backend.is_connected());
            assert!(!app.agent().backend.acp_controls().unwrap().ready);
        }
        app.open_acp_controls(None);
        assert!(app.agent().acp_controls_ui.discovery_started);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.agent().backend.acp_controls().unwrap().ready {
            assert!(Instant::now() < deadline);
            app.refresh_acp_controls();
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = app.agent().backend.acp_controls().unwrap();
        app.refresh_acp_controls();
        assert_eq!(
            app.agent().backend.acp_controls().unwrap().revision,
            state.revision
        );
        assert!(!state.config.options.is_empty());
        assert!(app.agent().acp_controls_ui.picker.is_some());
        assert!(app.agent().messages().is_empty());
    }

    #[test]
    fn input_focus_connects_without_starting_a_turn_on_both_chat_surfaces() {
        use crate::{
            app::{AiStatus, ClickableRegion, MainTab, MindMapFocus},
            keymap::Action,
        };
        for tab in [MainTab::Ai, MainTab::MindMap] {
            for mouse in [false, true] {
                let (mut app, _project, _store) = configured_app(false);
                app.active_tab = tab;
                app.mindmap_ai_panel_visible = true;
                app.mindmap_focus = MindMapFocus::AiPanel;
                app.ai_input_focused = false;
                if mouse {
                    let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
                    terminal
                        .draw(|frame| crate::ui::render(frame, &mut app))
                        .unwrap();
                    let rect = app
                        .clickable_regions
                        .iter()
                        .find_map(|region| match region {
                            ClickableRegion::ChatInput { rect } => Some(*rect),
                            _ => None,
                        })
                        .expect("visible chat input");
                    app.handle_mouse_event(
                        MouseEventKind::Down(MouseButton::Left),
                        rect.x + 1,
                        rect.y + 1,
                        KeyModifiers::NONE,
                    )
                    .unwrap();
                } else {
                    app.handle_action(Action::AiFocusInput).unwrap();
                }
                assert!(app.ai_input_focused);
                let deadline = Instant::now() + Duration::from_secs(5);
                while !app.agent().backend.acp_controls().unwrap().ready {
                    assert!(Instant::now() < deadline);
                    app.poll_llm_events();
                    std::thread::sleep(Duration::from_millis(10));
                }
                let connected = app.agent().backend.acp_controls().unwrap();
                assert!(app.agent().messages().is_empty());
                assert_eq!(app.agent().status, AiStatus::Idle);
                assert_eq!(
                    app.agent().backend.status(),
                    teshi_agent::backend::AgentBackendStatus::Idle
                );
                app.handle_action(Action::AiBlurInput).unwrap();
                app.handle_action(Action::AiFocusInput).unwrap();
                assert_eq!(
                    app.agent().backend.acp_controls().unwrap().session_id,
                    connected.session_id
                );
                app.agent_mut().backend.start().unwrap();
                app.agent_mut().status = AiStatus::Waiting;
                app.agent_mut().partial_response_mut().push_str("streaming");
                app.handle_action(Action::AiFocusInput).unwrap();
                assert_eq!(app.agent().status, AiStatus::Waiting);
                assert_eq!(app.agent().partial_response(), "streaming");
            }
        }
    }

    #[test]
    fn switching_and_closing_conversations_preserves_focused_connection() {
        use crate::keymap::Action;
        let (mut app, _project, _store) = configured_app(false);
        let (mut other, _other_project, _other_store) = configured_app(false);
        app.agents.push(other.agents.remove(0));
        app.ai_input_focused = true;
        app.handle_action(Action::AgentSelect(1)).unwrap();
        assert!(app.agent().backend.is_connected());
        assert!(!app.agents[0].backend.is_connected());
        app.handle_action(Action::AgentClose).unwrap();
        assert!(app.ai_input_focused);
        assert!(app.agent().backend.is_connected());
        assert!(app.agent().messages().is_empty());
        assert_eq!(app.agent().status, crate::app::AiStatus::Idle);
    }

    #[test]
    fn discovery_animation_rotates_and_is_visible_in_the_bar() {
        assert_ne!(
            loading_frame(Duration::ZERO),
            loading_frame(Duration::from_millis(120))
        );
        let (mut app, _project, _store) = configured_app(false);
        app.agent_mut().acp_controls_ui.discovery_started = true;
        let mut terminal = Terminal::new(TestBackend::new(40, 4)).unwrap();
        terminal
            .draw(|frame| render_bar(frame, &mut app, Rect::new(0, 2, 40, 1)))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("◐ Loading"), "{content}");
    }

    #[test]
    fn opening_restore_error_preserves_it_and_wraps_the_message() {
        let (mut app, project, _store) = app_with_agent();
        let preference = project.path().join("model.json");
        std::fs::write(&preference, "\"b\"").unwrap();
        let mut config =
            teshi_agent_runtime::backend::AcpBackendConfig::new(teshi_acp::AcpAgentCommand {
                program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
                args: vec![
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py")
                        .display()
                        .to_string(),
                    "settings_timeout".into(),
                ],
                cwd: project.path().into(),
                env: HashMap::new(),
            });
        config.model_preference_path = Some(preference);
        config.client.request_timeout = Duration::from_millis(200);
        app.agent_mut().backend.configure_acp(config).unwrap();
        app.agent_mut().backend.connect_acp().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.agent().backend.acp_controls().unwrap().error.is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let before = app.agent().backend.acp_controls().unwrap();
        app.open_acp_controls(None);
        let after = app.agent().backend.acp_controls().unwrap();
        assert_eq!(before.revision, after.revision);
        assert_eq!(before.error, after.error);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| render_picker(frame, &mut app))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("timed out"), "{content}");
    }

    #[test]
    fn controls_click_keyboard_and_narrow_overflow() {
        let (mut app, _project, _store) = app_with_agent();
        app.agent_mut().selected_acp_id = Some("模型 Agent".into());
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| render_bar(frame, &mut app, Rect::new(0, 22, 100, 1)))
            .unwrap();
        let (rect, _) = app
            .agent()
            .acp_controls_ui
            .regions
            .iter()
            .find(|(_, t)| matches!(t, Target::Option(id) if id == "model"))
            .unwrap()
            .clone();
        assert!(app.handle_acp_controls_mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x,
            rect.y
        ));
        assert_eq!(
            app.agent()
                .acp_controls_ui
                .picker
                .as_ref()
                .unwrap()
                .option
                .as_ref()
                .unwrap()
                .id,
            "model"
        );
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.agent().backend.acp_controls().unwrap().pending {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.agent().backend.acp_controls().unwrap().config.options[0].current,
            "b"
        );
        terminal
            .draw(|frame| render_bar(frame, &mut app, Rect::new(0, 22, 12, 1)))
            .unwrap();
        let (rect, _) = app
            .agent()
            .acp_controls_ui
            .regions
            .iter()
            .find(|(_, t)| matches!(t, Target::Settings))
            .unwrap()
            .clone();
        assert!(rect.right() <= 12);
        app.handle_acp_controls_mouse(MouseEventKind::Down(MouseButton::Left), rect.x, rect.y);
        terminal
            .draw(|frame| render_picker(frame, &mut app))
            .unwrap();
        assert!(
            app.agent()
                .acp_controls_ui
                .picker
                .as_ref()
                .unwrap()
                .option
                .is_none()
        );
        app.handle_acp_controls_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.agent().acp_controls_ui.picker.is_none());
    }
}

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use teshi_acp::session_config::{ConfigOption, SessionConfig};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{app::App, theme};

#[derive(Debug, Clone)]
enum Target {
    Settings,
    Option(String),
    Agent,
    Row(usize),
    Back,
}

#[derive(Debug, Clone)]
enum PickerScope {
    Settings,
    Approvals,
}

impl PickerScope {
    fn options<'a>(&self, config: &'a SessionConfig) -> Vec<&'a ConfigOption> {
        match self {
            Self::Settings => config.options.iter().collect(),
            Self::Approvals => config.approval_options(),
        }
    }
}

#[derive(Debug, Clone)]
struct Picker {
    scope: PickerScope,
    option: Option<ConfigOption>,
    revision: u64,
    selected: usize,
    error: Option<String>,
}

/// UI state belongs to its conversation, preventing cross-session picker replies.
#[derive(Debug, Default)]
pub(crate) struct ControlsUi {
    picker: Option<Picker>,
    regions: Vec<(Rect, Target)>,
    discovery_started: bool,
    animation_started: Option<std::time::Instant>,
    discovery_error: Option<String>,
}

impl ControlsUi {
    /// Record a connection requested by a user action, never by panel visibility.
    pub(crate) fn begin_connection(&mut self) {
        self.discovery_started = true;
        self.animation_started = Some(std::time::Instant::now());
        self.discovery_error = None;
    }

    /// Whether settings own text/paste input ahead of the underlying chat.
    pub(crate) fn picker_active(&self) -> bool {
        self.picker.is_some()
    }

    pub(crate) fn clear_regions(&mut self) {
        self.regions.clear();
    }
}

impl App {
    /// Prepare ACP once when the user enters the input, preserving startup errors.
    pub(crate) fn connect_acp_on_input_focus(&mut self) {
        // A failed attempt stays visible until the user explicitly reconnects.
        if !self.agent().acp_controls_ui.discovery_started {
            self.connect_acp_controls();
        }
    }

    pub(crate) fn refresh_acp_controls(&mut self) {
        // Polling updates an existing picker; displaying chat never connects it.
        self.refresh_acp_approval_picker();
    }

    pub(crate) fn effective_approval_mode(&self) -> teshi_agent::approval::ApprovalMode {
        use teshi_agent::approval::ApprovalMode;
        match self.agent().backend.acp_controls().and_then(|s| s.policy) {
            Some(teshi_acp::PermissionPolicy::Manual) => ApprovalMode::Manual,
            Some(teshi_acp::PermissionPolicy::Auto) => ApprovalMode::Auto,
            Some(teshi_acp::PermissionPolicy::Bypass) => ApprovalMode::Bypass,
            None => self.thread_approval_mode(self.selected_agent),
        }
    }

    /// Approval mode governing one conversation: its ACP agent's remembered
    /// mode when present, otherwise the application-wide mode.
    pub(crate) fn thread_approval_mode(&self, index: usize) -> teshi_agent::approval::ApprovalMode {
        self.agents
            .get(index)
            .and_then(|thread| thread.acp_approval)
            .unwrap_or(self.approval_mode)
    }

    /// Apply a local approval choice to the current conversation.
    ///
    /// Installed ACP agents remember it per agent for their future sessions;
    /// their other open conversations keep their current mode. Backends without
    /// an agent identity change the application-wide mode instead.
    pub(crate) fn select_approval_mode(&mut self, mode: teshi_agent::approval::ApprovalMode) {
        let index = self.selected_agent;
        let old = self.thread_approval_mode(index);
        self.approval_panel_active = false;
        self.status = format!(
            "Approval: {} → {} ({})",
            old.display_name(),
            mode.display_name(),
            mode.description()
        );
        let thread = &self.agents[index];
        let path = if thread.backend.kind() == teshi_agent::backend::AgentBackendKind::Acp {
            thread.acp_preference_path("approval-preferences")
        } else {
            Ok(None)
        };
        match path {
            Ok(Some(path)) => {
                self.agents[index].acp_approval = Some(mode);
                if let Err(error) = teshi_engine::write_atomic(&path, &mode) {
                    self.status = format!(
                        "Approval set to {}, but saving its preference failed: {error}",
                        mode.display_name()
                    );
                }
            }
            Ok(None) => self.approval_mode = mode,
            Err(error) => {
                self.agents[index].acp_approval = Some(mode);
                self.status = format!(
                    "Approval set to {}, but its preference location is unavailable: {error}",
                    mode.display_name()
                );
            }
        }
    }
    pub(crate) fn connect_acp_controls(&mut self) {
        // A failed discovery stays inspectable instead of restarting every frame.
        self.agent_mut().backend_selected = true;
        self.agent_mut().acp_controls_ui.begin_connection();
        let index = self.selected_agent;
        if !self.ensure_agent_backend(index) {
            self.agent_mut().acp_controls_ui.discovery_error = Some(self.status.clone());
        } else if let Err(error) = self.agents[index].backend.connect_acp() {
            self.status = error.to_string();
            self.agent_mut().acp_controls_ui.discovery_error = Some(error.to_string());
        }
    }

    pub(crate) fn open_acp_controls(&mut self, id: Option<String>) {
        if self.agent().backend.acp_controls().is_none() {
            self.status = "Select an ACP agent with /agent first".into();
            return;
        }
        // Opening an error must preserve it for inspection. Reconnecting here
        // would clear the failed snapshot before the picker can display it.
        if !self
            .agent()
            .backend
            .acp_controls()
            .is_some_and(|s| s.ready || s.pending || s.error.is_some())
        {
            self.connect_acp_controls();
        }
        let Some(state) = self.agent().backend.acp_controls() else {
            return;
        };
        let option = id.and_then(|id| state.config.options.iter().find(|o| o.id == id).cloned());
        let selected = option
            .as_ref()
            .and_then(|o| o.choices.iter().position(|c| c.value == o.current))
            .unwrap_or(0);
        self.agent_mut().acp_controls_ui.picker = Some(Picker {
            scope: PickerScope::Settings,
            option,
            revision: state.revision,
            selected,
            error: None,
        });
    }

    /// Open the current ACP session's advertised approval settings.
    pub(crate) fn open_acp_approvals(&mut self) {
        self.open_acp_controls(None);
        if let Some(picker) = &mut self.agent_mut().acp_controls_ui.picker {
            picker.scope = PickerScope::Approvals;
        }
        self.refresh_acp_approval_picker();
    }

    fn refresh_acp_approval_picker(&mut self) {
        if !self
            .agent()
            .acp_controls_ui
            .picker
            .as_ref()
            .is_some_and(|p| matches!(p.scope, PickerScope::Approvals) && p.option.is_none())
        {
            return;
        }
        let Some(state) = self.agent().backend.acp_controls() else {
            return;
        };
        let Some(picker) = &mut self.agent_mut().acp_controls_ui.picker else {
            return;
        };
        if !state.ready || state.pending {
            return;
        }
        // Discovery is asynchronous; keep /approval's intent until the agent's
        // confirmed options arrive, including when it was the first command.
        let options = state.config.approval_options();
        picker.revision = state.revision;
        if let [option] = options.as_slice() {
            picker.selected = option
                .choices
                .iter()
                .position(|c| c.value == option.current)
                .unwrap_or(0);
            picker.option = Some((*option).clone());
        } else {
            picker.selected = picker.selected.min(options.len().saturating_sub(1));
        }
    }

    pub(crate) fn sync_acp_policy(&mut self) {
        use teshi_agent::approval::ApprovalMode;
        let forced_auto = std::env::var("TESHI_ACP_PERMISSION_POLICY").as_deref() == Ok("auto");
        for index in 0..self.agents.len() {
            // Each conversation follows its own agent's mode, so threads of
            // different agents can hold different policies at once.
            let policy = if forced_auto {
                teshi_acp::PermissionPolicy::Auto
            } else {
                match self.thread_approval_mode(index) {
                    ApprovalMode::Manual => teshi_acp::PermissionPolicy::Manual,
                    ApprovalMode::Auto => teshi_acp::PermissionPolicy::Auto,
                    ApprovalMode::Bypass => teshi_acp::PermissionPolicy::Bypass,
                }
            };
            let thread = &mut self.agents[index];
            if thread
                .backend
                .acp_controls()
                .is_some_and(|s| s.policy != Some(policy))
                && let Err(error) = thread.backend.set_acp_policy(policy)
            {
                self.status = error.to_string();
            }
        }
    }

    fn activate_control(&mut self, target: Target) {
        match target {
            Target::Settings | Target::Back => self.open_acp_controls(None),
            Target::Option(id) => self.open_acp_controls(Some(id)),
            Target::Agent => {
                self.agent_mut().acp_controls_ui.picker = None;
                let _ = self.execute_slash_command("agent");
            }
            Target::Row(index) => {
                let Some(picker) = self.agent().acp_controls_ui.picker.clone() else {
                    return;
                };
                if let Some(option) = picker.option {
                    let Some(choice) = option.choices.get(index) else {
                        return;
                    };
                    match self.agent_mut().backend.set_acp_option(
                        &option.id,
                        choice.value.clone(),
                        picker.revision,
                    ) {
                        Ok(()) => {
                            self.agent_mut().acp_controls_ui.picker = None;
                            self.status = format!("Updating {}...", option.name);
                        }
                        Err(error) => {
                            self.status = error.to_string();
                            if let Some(picker) = &mut self.agent_mut().acp_controls_ui.picker {
                                picker.error = Some(error.to_string());
                            }
                        }
                    }
                } else if let Some(state) = self.agent().backend.acp_controls()
                    && let Some(option) = picker.scope.options(&state.config).get(index)
                {
                    self.open_acp_controls(Some(option.id.clone()));
                    if let Some(current) = &mut self.agent_mut().acp_controls_ui.picker {
                        current.scope = picker.scope;
                    }
                }
            }
        }
    }

    pub(crate) fn handle_acp_controls_key(&mut self, event: KeyEvent) -> bool {
        if self.pending_delete.is_some()
            || self.quit_pending_confirm
            || self.ai_interrupt_confirm
            || self.authoring_ui.overlay_active()
            || self.project_switch.overlay != crate::project_context::SwitchOverlay::None
            || self.has_acp_permission_prompt()
            || self.has_agent_change_prompt()
            || self.has_external_change_prompt()
            || self.model_panel_active
            || self.session_panel_active
            || self.agent_profile_panel_active
            || self.approval_panel_active
        {
            return false;
        }
        if event.code == KeyCode::Char(',')
            && event.modifiers.contains(KeyModifiers::CONTROL)
            && self.agent().backend.acp_controls().is_some()
        {
            self.open_acp_controls(None);
            return true;
        }
        let Some(picker) = self.agent().acp_controls_ui.picker.as_ref() else {
            return false;
        };
        let count = picker
            .option
            .as_ref()
            .map(|o| o.choices.len())
            .unwrap_or_else(|| {
                self.agent()
                    .backend
                    .acp_controls()
                    .map_or(0, |s| picker.scope.options(&s.config).len())
            });
        let selected = picker.selected;
        let top = event.code == KeyCode::Home
            || (event.code == KeyCode::Char('g') && self.pending_char == Some('g'));
        let last = event.code == KeyCode::End || event.code == KeyCode::Char('G');
        let page_up = event.code == KeyCode::PageUp
            || (event.code == KeyCode::Char('b') && event.modifiers == KeyModifiers::CONTROL);
        let page_down = event.code == KeyCode::PageDown
            || (event.code == KeyCode::Char('f') && event.modifiers == KeyModifiers::CONTROL);
        if top || last || page_up || page_down {
            if let Some(p) = &mut self.agent_mut().acp_controls_ui.picker {
                p.selected = if top {
                    0
                } else if last {
                    count.saturating_sub(1)
                } else if page_up {
                    selected.saturating_sub(10)
                } else {
                    selected.saturating_add(10).min(count.saturating_sub(1))
                };
            }
            self.pending_char = None;
            return true;
        }
        if event.code == KeyCode::Char('g') && event.modifiers.is_empty() {
            self.pending_char = Some('g');
            return true;
        }
        self.pending_char = None;
        match event.code {
            KeyCode::Esc => {
                self.agent_mut().acp_controls_ui.picker = None;
            }
            KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => self.open_acp_controls(None),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                self.activate_control(Target::Row(selected))
            }
            KeyCode::Char('a') => self.activate_control(Target::Agent),
            KeyCode::Char('p') => {
                self.agent_mut().acp_controls_ui.picker = None;
                let _ = self.execute_slash_command("approval");
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = &mut self.agent_mut().acp_controls_ui.picker {
                    p.selected = selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = &mut self.agent_mut().acp_controls_ui.picker {
                    p.selected = (selected + 1).min(count.saturating_sub(1));
                }
            }
            _ => {}
        }
        true
    }

    pub(crate) fn handle_acp_controls_mouse(
        &mut self,
        kind: MouseEventKind,
        col: u16,
        row: u16,
    ) -> bool {
        if self.approval_panel_active
            || self.agent_profile_panel_active
            || self.model_panel_active
            || self.session_panel_active
            || self.quit_pending_confirm
            || self.ai_interrupt_confirm
        {
            return false;
        }
        let open = self.agent().acp_controls_ui.picker.is_some();
        if matches!(kind, MouseEventKind::Down(MouseButton::Left)) {
            let target = self
                .agent()
                .acp_controls_ui
                .regions
                .iter()
                .rev()
                .find(|(rect, _)| rect.contains((col, row).into()))
                .map(|(_, target)| target.clone());
            if let Some(target) = target {
                self.activate_control(target);
                return true;
            }
        }
        if open && matches!(kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) {
            let key = if kind == MouseEventKind::ScrollDown {
                KeyCode::Down
            } else {
                KeyCode::Up
            };
            self.handle_acp_controls_key(KeyEvent::new(key, KeyModifiers::NONE));
        }
        open
    }
}

fn chip(frame: &mut Frame<'_>, area: Rect, label: String, target: Target, ui: &mut ControlsUi) {
    let content = if let Some((name, value)) = label.split_once(": ") {
        Line::from(vec![
            Span::styled(
                format!(" {name} "),
                Style::default().fg(theme::MUTED).bg(theme::BLOCK_BG),
            ),
            Span::styled(
                value.to_owned(),
                Style::default().fg(theme::PRIMARY).bg(theme::CHAT_CHIP_BG),
            ),
            Span::raw(" "),
        ])
    } else {
        Line::from(
            if label.width().saturating_add(2) <= usize::from(area.width) {
                format!(" {label} ")
            } else {
                label
            },
        )
    };
    frame.render_widget(
        Paragraph::new(content).style(Style::default().fg(theme::PRIMARY).bg(theme::CHAT_CHIP_BG)),
        area,
    );
    ui.regions.push((area, target));
}

/// Draw every fitting control, retaining a settings entry for overflow.
pub(crate) fn render_bar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(state) = app.agent().backend.acp_controls() else {
        return;
    };
    let agent = app
        .agent()
        .selected_acp_id
        .as_ref()
        .map(|id| {
            app.installed_acp_agents
                .iter()
                .find(|agent| &agent.id == id)
                .map(|agent| agent.name.clone())
                .unwrap_or_else(|| id.clone())
        })
        .unwrap_or_else(|| "ACP".into());
    let elapsed = app
        .agent()
        .acp_controls_ui
        .animation_started
        .map_or(std::time::Duration::ZERO, |start| start.elapsed());
    let status = if state.pending {
        format!("{} Applying", loading_frame(elapsed))
    } else if state.error.is_some() || app.agent().acp_controls_ui.discovery_error.is_some() {
        "Error".into()
    } else if !state.ready && app.agent().acp_controls_ui.discovery_started {
        format!("{} Loading", loading_frame(elapsed))
    } else if state.ready {
        "Settings".into()
    } else {
        "Not connected".into()
    };
    let more = if area.width < 32 && status == "Settings" {
        "⋯".into()
    } else if status.width().saturating_add(4) <= usize::from(area.width) {
        format!("{status} ▾")
    } else {
        status
    };
    let more_width = (more.width() as u16 + 2).min(if area.width >= 8 {
        area.width / 2
    } else {
        area.width
    });
    let activity_on_left = state.pending
        || (!state.ready
            && state.error.is_none()
            && app.agent().acp_controls_ui.discovery_error.is_none()
            && app.agent().acp_controls_ui.discovery_started);
    let ui = &mut app.agent_mut().acp_controls_ui;
    ui.regions.clear();
    if area.height == 0 || area.width == 0 {
        return;
    }
    chip(
        frame,
        Rect::new(
            if activity_on_left {
                area.x
            } else {
                area.right().saturating_sub(more_width)
            },
            area.y,
            more_width,
            1,
        ),
        more,
        Target::Settings,
        ui,
    );
    let mut x = if activity_on_left {
        area.x.saturating_add(more_width + 1)
    } else {
        area.x
    };
    let end = if activity_on_left {
        area.right()
    } else {
        area.right().saturating_sub(more_width + 1)
    };
    let controls = std::iter::once((format!("{agent} ▾"), Target::Agent)).chain(
        state.config.options.iter().map(|o| {
            (
                format!("{}: {} ▾", o.name, o.value_label()),
                Target::Option(o.id.clone()),
            )
        }),
    );
    for (mut label, target) in controls {
        let mut width = label.width().saturating_add(2).min(u16::MAX as usize) as u16;
        if matches!(&target, Target::Agent) && x.saturating_add(width) > end {
            let available = end.saturating_sub(x);
            if available >= 3 {
                let budget = available.saturating_sub(3) as usize;
                let mut used = 0;
                label = agent
                    .chars()
                    .take_while(|ch| {
                        used += ch.width().unwrap_or(0);
                        used <= budget
                    })
                    .collect();
                label.push('▾');
                width = available;
            }
        }
        if x.saturating_add(width) > end {
            continue;
        }
        chip(frame, Rect::new(x, area.y, width, 1), label, target, ui);
        x = x.saturating_add(width + 1);
    }
    if let Some((x, y)) = app.mouse_position {
        for (rect, _) in &app.agent().acp_controls_ui.regions {
            if rect.contains((x, y).into()) {
                frame
                    .buffer_mut()
                    .set_style(*rect, Style::default().bg(theme::HOVER_BG));
                break;
            }
        }
    }
}

/// Read an agent's remembered approval mode; a missing file means none is saved.
pub(crate) fn read_approval_preference(
    path: &std::path::Path,
) -> anyhow::Result<Option<teshi_agent::approval::ApprovalMode>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn loading_frame(elapsed: std::time::Duration) -> &'static str {
    const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
    FRAMES[(elapsed.as_millis() / 120 % FRAMES.len() as u128) as usize]
}

/// Render the settings picker above all underlying controls.
pub(crate) fn render_picker(frame: &mut Frame<'_>, app: &mut App) {
    let Some(picker) = app.agent().acp_controls_ui.picker.clone() else {
        return;
    };
    let Some(state) = app.agent().backend.acp_controls() else {
        return;
    };
    let screen = frame.area();
    let count = picker.option.as_ref().map_or_else(
        || picker.scope.options(&state.config).len(),
        |o| o.choices.len(),
    );
    let width = screen.width.min(60);
    let height = screen.height.min((count.saturating_add(6).min(20)) as u16);
    if width < 4 || height < 5 {
        return;
    }
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.bottom().saturating_sub(height + 3).max(screen.y),
        width,
        height,
    );
    let title = match (&picker.scope, &picker.option) {
        (PickerScope::Approvals, Some(o)) => format!("ACP approvals: {}", o.name),
        (PickerScope::Approvals, None) => "ACP approvals".into(),
        (PickerScope::Settings, Some(o)) => o.name.clone(),
        (PickerScope::Settings, None) => "ACP settings".into(),
    };
    let block = theme::popup_block(format!(" {title} "));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let rows: Vec<String> = if let Some(option) = &picker.option {
        option
            .choices
            .iter()
            .map(|c| {
                format!(
                    "{} {}{}",
                    if c.value == option.current {
                        "●"
                    } else {
                        "○"
                    },
                    c.group
                        .as_ref()
                        .map(|g| format!("{g} / "))
                        .unwrap_or_default(),
                    c.name
                )
            })
            .collect()
    } else {
        picker
            .scope
            .options(&state.config)
            .into_iter()
            .map(|o| {
                format!(
                    "{}: {}{}",
                    o.name,
                    o.value_label(),
                    if o.choices.is_empty() {
                        " (unsupported type)"
                    } else {
                        ""
                    }
                )
            })
            .collect()
    };
    let selected = picker.selected.min(rows.len().saturating_sub(1));
    let visible = inner.height.saturating_sub(4) as usize;
    let start = selected.saturating_sub(visible.saturating_sub(1));
    let ui = &mut app.agent_mut().acp_controls_ui;
    ui.regions.clear();
    for (offset, (index, text)) in rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let rect = Rect::new(inner.x, inner.y + offset as u16, inner.width, 1);
        let style = if index == selected {
            theme::selected_row(true)
        } else {
            Style::default().fg(theme::FG)
        };
        frame.render_widget(Paragraph::new(text.clone()).style(style), rect);
        ui.regions.push((rect, Target::Row(index)));
    }
    let description = picker.option.as_ref().and_then(|o| {
        o.choices
            .get(selected)
            .and_then(|c| c.description.as_deref())
            .or(o.description.as_deref())
    });
    let elapsed = ui
        .animation_started
        .map_or(std::time::Duration::ZERO, |start| start.elapsed());
    let fallback = if state.pending {
        format!("{} Applying setting...", loading_frame(elapsed))
    } else if !state.ready {
        format!("{} Loading agent settings...", loading_frame(elapsed))
    } else if rows.is_empty() {
        if picker.option.is_some() {
            "This agent setting uses an unsupported type".into()
        } else if matches!(picker.scope, PickerScope::Approvals) {
            "This ACP agent does not advertise approval settings or modes".into()
        } else {
            "No editable settings advertised by this agent".into()
        }
    } else {
        "↑/↓ select · Enter apply · ← settings · Esc close".into()
    };
    let message = picker
        .error
        .as_deref()
        .or(state.error.as_deref())
        .or(ui.discovery_error.as_deref())
        .or(description)
        .unwrap_or(&fallback);
    frame.render_widget(
        Paragraph::new(message)
            .style(Style::default().fg(theme::FG))
            .wrap(ratatui::widgets::Wrap { trim: true }),
        Rect::new(inner.x, inner.bottom().saturating_sub(3), inner.width, 2),
    );
    let footer = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    frame.render_widget(Paragraph::new(Line::from("[Back]  [Agent]")), footer);
    for (offset, width, target) in [(0, 6, Target::Back), (8, 7, Target::Agent)] {
        if offset + width <= inner.width {
            ui.regions
                .push((Rect::new(inner.x + offset, footer.y, width, 1), target));
        }
    }
}
