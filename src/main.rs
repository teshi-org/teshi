mod agent;
mod app;
mod bdd_nav;
mod editor_buffer;
mod gherkin;
mod gherkin_keywords;
mod highlight;
mod keymap;
mod llm;
mod mindmap;
mod runner;
mod step_index;
mod ui;

use std::env;
use std::io;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

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
        execute!(io::stdout(), EnableMouseCapture)?;
        guard.mouse_capture = true;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.mouse_capture {
            let _ = execute!(io::stdout(), DisableMouseCapture);
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

    let mut args: Vec<String> = env::args().skip(1).collect();
    if matches!(args.first().map(|s| s.as_str()), Some("run")) {
        args.remove(0);
        return runner::run_cli(&args);
    }
    let _guard = TerminalGuard::setup()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::from_args()?;

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
                        },
                    ) {
                        app.handle_action(action)?;
                    }
                }
                Event::Mouse(mouse_event) => {
                    let in_ai_context =
                        app.active_tab == app::MainTab::Ai
                            || (app.active_tab == app::MainTab::MindMap
                                && app.mindmap_focus == app::MindMapFocus::AiPanel);
                    if in_ai_context {
                        match mouse_event.kind {
                            MouseEventKind::ScrollUp => {
                                app.handle_action(Action::AiScrollUp)?;
                            }
                            MouseEventKind::ScrollDown => {
                                app.handle_action(Action::AiScrollDown)?;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}
