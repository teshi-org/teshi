mod app;
mod editor_buffer;
mod highlight;
mod keymap;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};
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
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn main() -> Result<()> {
    let _guard = TerminalGuard::setup()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::from_args()?;

    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key_event) = event::read()?
            && let Some(action) = Action::from_key_event(
                key_event,
                KeyContext {
                    step_input_active: app.step_input_active,
                    active_tab: app.active_tab,
                },
            )
        {
            app.handle_action(action)?;
        }
    }

    Ok(())
}
