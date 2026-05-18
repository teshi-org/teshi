mod agent;
mod app;
mod auth;
mod bdd_nav;
mod cli;
mod config;
mod diff;
mod editor_buffer;
mod gherkin;
mod gherkin_keywords;
mod gherkin_lang;
mod highlight;
mod keymap;
mod llm;
mod markdown;
mod mindmap;
mod profiles;
mod runner;
mod session;
mod step_index;
mod ui;

use std::fmt;
use std::io;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::Command;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// Enable mouse tracking for in-app selection (click, drag, scroll) using
/// ANSI escape sequences — basic tracking (`?1000h`) for click/release,
/// button-event tracking (`?1002h`) for drag-based selection, and SGR extended
/// coordinates (`?1006h`) for positions > 223. Omits any-event tracking
/// (`?1003h`) to avoid flooding the event loop with free-motion reports.
///
/// Uses raw ANSI writes for all platforms so the same SGR mode is guaranteed.
struct EnableAppMouseCapture;

impl Command for EnableAppMouseCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1000h\x1b[?1002h\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        // Modern Windows terminals (Windows Terminal, ConEmu, Alacritty,
        // VS Code terminal) all support ANSI escape codes, so the
        // write_ansi path works fine. This stub keeps the trait happy.
        Ok(())
    }
}

/// Disable mouse tracking that was enabled by [`EnableAppMouseCapture`].
struct DisableAppMouseCapture;

impl Command for DisableAppMouseCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1006l\x1b[?1002l\x1b[?1000l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
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

fn main() -> Result<()> {
    if let Ok(path) = std::env::var("TESHI_DIAG_PATH")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
    {
        let _ = writeln!(file, "pid {}: entered main", std::process::id());
    }

    let cli_args = cli::Cli::parse();

    match cli_args.command {
        Some(cli::Command::Auth { action }) => {
            return cli::auth::handle_auth_command(&action);
        }
        Some(cli::Command::Run {
            feature,
            scenario,
            runner_cmd,
            runner_arg,
            runner_cwd,
        }) => {
            let mut args: Vec<String> = Vec::new();
            if let Some(f) = feature {
                args.push("--feature".into());
                args.push(f);
            }
            if let Some(s) = scenario {
                args.push("--scenario".into());
                args.push(s);
            }
            if let Some(c) = runner_cmd {
                args.push("--runner-cmd".into());
                args.push(c);
            }
            for a in runner_arg.unwrap_or_default() {
                args.push("--runner-arg".into());
                args.push(a);
            }
            if let Some(cwd) = runner_cwd {
                args.push("--runner-cwd".into());
                args.push(cwd);
            }
            return runner::run_cli(&args);
        }
        None => {}
    }

    let _guard = TerminalGuard::setup()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::from_cli(&cli_args)?;

    while !app.should_quit {
        app.poll_runner_events();
        app.poll_llm_events();
        app.poll_external_feature_changes();
        app.poll_status_message_expiry();
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
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
                            model_panel_adding: app.model_panel_mode == app::ModelPanelMode::Adding,
                            session_panel_active: app.session_panel_active,
                            change_summary_visible: app.change_summary_visible,
                            ai_status_waiting: app.agent().status == crate::app::AiStatus::Waiting,
                            scenario_dropdown_open: app.scenario_dropdown_open,
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
