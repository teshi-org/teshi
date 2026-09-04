mod agent;
mod app;
mod auth;
mod authoring_tab;
mod bdd_nav;
mod cli;
mod config;
mod diff;
mod editor_buffer;
mod engine;
mod generation_state;
mod highlight;
mod input;
mod keymap;
mod llm;
pub mod markdown;
mod mindmap;
mod profiles;
mod runner;
mod session;
mod test_points_tab;
mod ui;

use std::fmt;
use std::io;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::Command;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// Enable mouse tracking for in-app selection (click, drag, scroll) using
/// ANSI escape sequences — basic tracking (`?1000h`) for click/release,
/// any-event tracking (`?1003h`) for drag and hover detection, RXVT extended
/// coordinates (`?1015h`) as a compatibility fallback, and SGR extended
/// coordinates (`?1006h`) as the preferred encoding.
///
/// This matches Textual's widely compatible mode sequence. Any-event tracking
/// already includes button motion, so enabling `?1002h` as well is redundant
/// and needlessly introduces another tracking-mode transition for multiplexers.
struct EnableAppMouseCapture;

impl Command for EnableAppMouseCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1000h\x1b[?1003h\x1b[?1015h\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        // Mouse tracking is an output protocol between teshi and the terminal
        // (or multiplexer). Force the ANSI path even when crossterm cannot
        // detect VT support through a Zellij pseudoterminal.
        true
    }
}

/// Disable mouse tracking that was enabled by [`EnableAppMouseCapture`].
struct DisableAppMouseCapture;

impl Command for DisableAppMouseCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1000l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

use app::App;
use keymap::{Action, KeyContext};

struct TerminalGuard {
    raw_mode: bool,
    alt_screen: bool,
    cursor_hidden: bool,
    mouse_capture: bool,
}

impl TerminalGuard {
    fn setup() -> Result<Self> {
        let no_raw = std::env::var_os("TESHI_NO_RAW").is_some();
        let no_alt = std::env::var_os("TESHI_NO_ALT").is_some();
        let mut guard = Self {
            raw_mode: false,
            alt_screen: false,
            cursor_hidden: false,
            mouse_capture: false,
        };
        if !no_raw {
            enable_raw_mode()?;
            guard.raw_mode = true;
        }
        if !no_alt {
            execute!(io::stdout(), EnterAlternateScreen)?;
            guard.alt_screen = true;
        }
        execute!(io::stdout(), Hide)?;
        guard.cursor_hidden = true;
        execute!(io::stdout(), EnableAppMouseCapture)?;
        guard.mouse_capture = true;
        execute!(io::stdout(), EnableBracketedPaste)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.mouse_capture {
            let _ = execute!(io::stdout(), DisableAppMouseCapture);
            let _ = execute!(io::stdout(), DisableBracketedPaste);
        }
        if self.cursor_hidden {
            let _ = execute!(io::stdout(), Show);
        }
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
        if self.alt_screen {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
    }
}

fn write_diagnostic_event(writer: &mut impl Write, event: &Event) -> io::Result<()> {
    match event {
        Event::Key(key_event) if matches!(key_event.code, KeyCode::Char(_)) => writeln!(
            writer,
            "event: Key {{ code: Char(<redacted>), modifiers: {:?}, kind: {:?}, state: {:?} }}",
            key_event.modifiers, key_event.kind, key_event.state
        ),
        Event::Paste(_) => writeln!(writer, "event: Paste(<redacted>)"),
        _ => writeln!(writer, "event: {event:?}"),
    }
}

/// Runs the terminal application and non-daemon CLI commands.
pub fn run(version: &str) -> Result<()> {
    let mut diag_file = std::env::var("TESHI_DIAG_PATH").ok().and_then(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    if let Some(file) = diag_file.as_mut() {
        let _ = writeln!(file, "pid {}: entered main", std::process::id());
    }

    let cli_args = cli::Cli::parse();
    let requirements_root = cli_args.requirements_root.clone();

    match cli_args.command {
        Some(cli::Command::Auth { action }) => {
            return cli::auth::handle_auth_command(&action);
        }
        Some(cli::Command::Desktop {
            project,
            path,
            start_embedded,
        }) => {
            return cli::desktop::spawn_desktop(
                project.as_deref(),
                path.as_deref(),
                start_embedded,
                requirements_root.as_deref(),
            );
        }
        Some(cli::Command::Requirements { action }) => {
            return cli::requirements::handle_requirements_command(
                &action,
                requirements_root.as_deref(),
            );
        }
        Some(cli::Command::Run {
            path,
            scenario,
            runner_cmd,
            runner_arg,
            runner_cwd,
            feature,
        }) => {
            let opts = cli::Command::run_options(
                path, scenario, runner_cmd, runner_arg, runner_cwd, feature,
            );
            return runner::run_with_options(opts);
        }
        Some(cli::Command::Steps { action }) => {
            return cli::steps::handle_steps_command(&action);
        }
        Some(cli::Command::Browser { action }) => {
            return cli::browser::handle_browser_command(&action);
        }
        Some(cli::Command::Mcp { action }) => {
            return cli::mcp::handle_mcp_command(&action);
        }
        Some(cli::Command::InstallSkill { dry_run, yes }) => {
            return cli::install_skill::handle_install_skill(dry_run, yes);
        }
        Some(cli::Command::Trace { action }) => {
            return cli::trace::handle_trace_command(&action);
        }
        Some(cli::Command::WinApp { action }) => {
            return cli::winapp::handle_winapp_command(&action);
        }
        Some(cli::Command::Api { action }) => {
            return cli::api::handle_api_command(&action);
        }
        Some(cli::Command::Terminal { action }) => {
            return cli::terminal::handle_terminal_command(&action);
        }
        Some(cli::Command::Export { args }) => {
            return cli::export::handle_export_command(&args);
        }
        Some(cli::Command::Daemon { action }) => {
            return cli::daemon::handle_daemon_command(&action);
        }
        Some(cli::Command::Record {
            url,
            feature,
            auto_propose,
        }) => {
            return engine::handle_record_command(&url, feature.as_deref(), auto_propose);
        }
        Some(cli::Command::Generate { action }) => {
            return engine::handle_generate_command(&action);
        }
        None => {}
    }

    let _guard = TerminalGuard::setup()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::from_cli(&cli_args)?;
    app.version = version.to_string();
    let mut event_source = input::EventSource::new()?;

    while !app.should_quit {
        app.poll_runner_events();
        app.poll_llm_events();
        app.poll_external_feature_changes();
        app.poll_status_message_expiry();
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        if let Some(terminal_event) = event_source.next(Duration::from_millis(50))? {
            if let Some(file) = diag_file.as_mut() {
                let _ = write_diagnostic_event(file, &terminal_event);
                let _ = file.flush();
            }
            match terminal_event {
                Event::Key(key_event) => {
                    if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        continue;
                    }
                    if let Some(action) = Action::from_key_event(
                        key_event,
                        KeyContext {
                            step_keyword_picker_active: app.step_keyword_picker.is_some(),
                            step_input_active: app.step_input_active,
                            external_change_prompt_active: app.has_external_change_prompt(),
                            agent_change_prompt_active: app.has_agent_change_prompt(),
                            active_tab: app.active_tab,
                            view_stage: app.view_stage,
                            explore_edit_mode: app.explore_edit_mode,
                            pending_char: app.pending_char,
                            mindmap_focus: app.mindmap_focus,
                            mindmap_ai_panel_visible: app.mindmap_ai_panel_visible,
                            ai_input_focused: app.ai_input_focused,
                            slash_suggestion_active: app.slash_suggestion_active,
                            auth_panel_active: app.auth_panel_active,
                            model_panel_active: app.model_panel_active,
                            model_panel_adding: app.model_panel_mode == app::ModelPanelMode::Adding
                                || app.model_panel_mode == app::ModelPanelMode::Editing,
                            session_panel_active: app.session_panel_active,
                            change_summary_visible: app.change_summary_visible,
                            ai_status_waiting: app.agent().status == crate::app::AiStatus::Waiting,
                            scenario_dropdown_open: app.scenario_dropdown_open,
                            approval_panel_active: app.approval_panel_active,
                            agent_profile_panel_active: app.agent_profile_panel_active,
                            requirements_focus: app.authoring_ui.focus,
                            test_points_focus: app.test_points_ui.focus,
                            requirements_overlay_active: app.authoring_ui.overlay_active(),
                            generation_scope_prompt_active: app.generation_scope_prompt.is_some(),
                            quit_pending_confirm: app.quit_pending_confirm,
                        },
                    ) {
                        app.handle_action(action)?;
                    }
                }
                Event::Mouse(mouse_event) => {
                    app.handle_mouse_event(
                        mouse_event.kind,
                        mouse_event.column,
                        mouse_event.row,
                        mouse_event.modifiers,
                    )?;
                }
                Event::Paste(text) if app.ai_input_focused => {
                    app.handle_action(Action::AiPaste(text))?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DisableAppMouseCapture, EnableAppMouseCapture, write_diagnostic_event};
    use crossterm::Command;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn mouse_capture_uses_multiplexer_compatible_modes() {
        let mut enable = String::new();
        EnableAppMouseCapture.write_ansi(&mut enable).unwrap();
        assert_eq!(enable, "\x1b[?1000h\x1b[?1003h\x1b[?1015h\x1b[?1006h");

        let mut disable = String::new();
        DisableAppMouseCapture.write_ansi(&mut disable).unwrap();
        assert_eq!(disable, "\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1000l");
    }

    #[test]
    fn diagnostic_events_redact_typed_and_pasted_text() {
        let secret = "sk-secret-value";
        let mut output = Vec::new();

        write_diagnostic_event(
            &mut output,
            &Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        )
        .unwrap();
        write_diagnostic_event(&mut output, &Event::Paste(secret.to_string())).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Char(<redacted>)"));
        assert!(output.contains("Paste(<redacted>)"));
        assert!(!output.contains(secret));
    }
}
