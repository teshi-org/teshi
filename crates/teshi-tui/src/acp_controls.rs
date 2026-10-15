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
                    args: vec![script.display().to_string(), "settings".into()],
                    cwd: project.path().to_path_buf(),
                    env: HashMap::new(),
                },
            ))
            .unwrap();
        app.agent_mut().backend.connect_acp().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.agent().backend.acp_controls().unwrap().ready {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        (app, project, store)
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
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use teshi_acp::session_config::ConfigOption;
use unicode_width::UnicodeWidthStr;

use crate::{app::App, theme};

#[derive(Debug, Clone)]
enum Target {
    Settings,
    Option(String),
    Agent,
    Approvals,
    Row(usize),
    Back,
}

#[derive(Debug, Clone)]
struct Picker {
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
}

impl ControlsUi {
    pub(crate) fn clear_regions(&mut self) {
        self.regions.clear();
    }
}

impl App {
    pub(crate) fn effective_approval_mode(&self) -> teshi_agent::approval::ApprovalMode {
        use teshi_agent::approval::ApprovalMode;
        match self.agent().backend.acp_controls().and_then(|s| s.policy) {
            Some(teshi_acp::PermissionPolicy::Manual) => ApprovalMode::Manual,
            Some(teshi_acp::PermissionPolicy::Auto) => ApprovalMode::Auto,
            Some(teshi_acp::PermissionPolicy::Bypass) => ApprovalMode::Bypass,
            None => self.approval_mode,
        }
    }
    pub(crate) fn connect_acp_controls(&mut self) {
        let index = self.selected_agent;
        if self.ensure_agent_backend(index)
            && let Err(error) = self.agents[index].backend.connect_acp()
        {
            self.status = error.to_string();
        }
    }

    pub(crate) fn open_acp_controls(&mut self, id: Option<String>) {
        if self.agent().backend.acp_controls().is_none() {
            self.status = "Select an ACP agent with /agent first".into();
            return;
        }
        if !self
            .agent()
            .backend
            .acp_controls()
            .is_some_and(|s| s.ready || s.pending)
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
            option,
            revision: state.revision,
            selected,
            error: None,
        });
    }

    pub(crate) fn sync_acp_policy(&mut self) {
        use teshi_agent::approval::ApprovalMode;
        let policy = if std::env::var("TESHI_ACP_PERMISSION_POLICY").as_deref() == Ok("auto") {
            teshi_acp::PermissionPolicy::Auto
        } else {
            match self.approval_mode {
                ApprovalMode::Manual => teshi_acp::PermissionPolicy::Manual,
                ApprovalMode::Auto => teshi_acp::PermissionPolicy::Auto,
                ApprovalMode::Bypass => teshi_acp::PermissionPolicy::Bypass,
            }
        };
        for thread in &mut self.agents {
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
            Target::Approvals => {
                self.agent_mut().acp_controls_ui.picker = None;
                let _ = self.execute_slash_command("approval");
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
                    && let Some(option) = state.config.options.get(index)
                {
                    self.open_acp_controls(Some(option.id.clone()));
                }
            }
        }
    }

    pub(crate) fn handle_acp_controls_key(&mut self, event: KeyEvent) -> bool {
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
                    .map_or(0, |s| s.config.options.len())
            });
        let selected = picker.selected;
        match event.code {
            KeyCode::Esc => {
                self.agent_mut().acp_controls_ui.picker = None;
            }
            KeyCode::Left | KeyCode::Backspace => self.open_acp_controls(None),
            KeyCode::Enter => self.activate_control(Target::Row(selected)),
            KeyCode::Char('a') => self.activate_control(Target::Agent),
            KeyCode::Char('p') => self.activate_control(Target::Approvals),
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
    frame.render_widget(
        Paragraph::new(label).style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
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
    let policy = match state.policy {
        Some(teshi_acp::PermissionPolicy::Manual) => "Ask",
        Some(teshi_acp::PermissionPolicy::Auto) => "Auto",
        Some(teshi_acp::PermissionPolicy::Bypass) => "Bypass",
        None => "Connecting",
    };
    let busy = matches!(
        app.agent().backend.status(),
        teshi_agent::backend::AgentBackendStatus::Running
            | teshi_agent::backend::AgentBackendStatus::WaitingForPermission
    );
    let status = if state.pending {
        "Applying"
    } else if state.error.is_some() {
        "Error"
    } else if busy {
        "Busy"
    } else if state.ready {
        "More"
    } else {
        "Connect"
    };
    let more = format!("[{status} ▾]");
    let more_width = (more.width() as u16).min(area.width);
    let ui = &mut app.agent_mut().acp_controls_ui;
    ui.regions.clear();
    if area.height == 0 || area.width == 0 {
        return;
    }
    chip(
        frame,
        Rect::new(
            area.right().saturating_sub(more_width),
            area.y,
            more_width,
            1,
        ),
        more,
        Target::Settings,
        ui,
    );
    let mut x = area.x;
    let end = area.right().saturating_sub(more_width + 1);
    let controls = std::iter::once((format!("{agent} ▾"), Target::Agent))
        .chain(state.config.options.iter().map(|o| {
            (
                format!("{}: {} ▾", o.name, o.value_label()),
                Target::Option(o.id.clone()),
            )
        }))
        .chain(std::iter::once((
            format!("Approvals: {policy} ▾"),
            Target::Approvals,
        )));
    for (label, target) in controls {
        let width = label.width().min(u16::MAX as usize) as u16;
        if x.saturating_add(width) > end {
            break;
        }
        chip(frame, Rect::new(x, area.y, width, 1), label, target, ui);
        x = x.saturating_add(width + 2);
    }
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
    let width = screen.width.min(76);
    let height = screen.height.min(20);
    if width < 4 || height < 5 {
        return;
    }
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let title = picker
        .option
        .as_ref()
        .map_or("ACP settings", |o| o.name.as_str());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .style(Style::default().bg(theme::CHIP_FG).fg(theme::FG));
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
        state
            .config
            .options
            .iter()
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
            Style::default().fg(theme::CHIP_FG).bg(theme::ACCENT)
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
    let message = picker
        .error
        .as_deref()
        .or(state.error.as_deref())
        .or(description)
        .unwrap_or(if state.pending {
            "Applying setting..."
        } else if !state.ready {
            "Connecting to discover settings..."
        } else if rows.is_empty() {
            "No editable settings advertised by this agent"
        } else {
            "↑/↓ select · Enter apply · ← settings · Esc close"
        });
    frame.render_widget(
        Paragraph::new(message).style(Style::default().fg(theme::FG)),
        Rect::new(inner.x, inner.bottom().saturating_sub(3), inner.width, 2),
    );
    let footer = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    frame.render_widget(
        Paragraph::new(Line::from("[Back]  [Agent]  [Approvals]")),
        footer,
    );
    for (offset, width, target) in [
        (0, 6, Target::Back),
        (8, 7, Target::Agent),
        (17, 11, Target::Approvals),
    ] {
        if offset + width <= inner.width {
            ui.regions
                .push((Rect::new(inner.x + offset, footer.y, width, 1), target));
        }
    }
}
