mod app;
mod bdd_nav;
mod editor_buffer;
mod gherkin;
mod gherkin_keywords;
mod highlight;
mod keymap;
mod mindmap;
mod runner;
mod step_index;
mod ui;

use std::env;
use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::App;
use keymap::{Action, KeyContext};

struct TerminalGuard;

impl TerminalGuard {
    fn setup() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show);
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn main() -> Result<()> {
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
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key_event) = event::read()?
        {
            if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                continue;
            }
            if let Some(action) = Action::from_key_event(
                key_event,
                KeyContext {
                    step_keyword_picker_active: app.step_keyword_picker.is_some(),
                    step_input_active: app.step_input_active,
                    active_tab: app.active_tab,
                    view_stage: app.view_stage,
                    explore_edit_mode: app.explore_edit_mode,
                },
            ) {
                app.handle_action(action)?;
            }
        }
    }

    Ok(())
}
