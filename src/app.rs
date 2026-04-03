use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::editor_buffer::EditorBuffer;
use crate::keymap::Action;

pub struct App {
    pub buffer: EditorBuffer,
    pub file_path: Option<PathBuf>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub desired_col: usize,
    pub scroll_row: usize,
    pub should_quit: bool,
    pub dirty: bool,
    pub status: String,
    pub step_input_active: bool,
    step_input_row: usize,
    step_input_min_col: usize,
    quit_pending_confirm: bool,
}

impl App {
    pub fn from_args() -> Result<Self> {
        let path = std::env::args().nth(1).map(PathBuf::from);
        if let Some(path) = path {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            Ok(Self {
                buffer: EditorBuffer::from_string(content),
                file_path: Some(path),
                cursor_row: 0,
                cursor_col: 0,
                desired_col: 0,
                scroll_row: 0,
                should_quit: false,
                dirty: false,
                status: "Opened file".to_string(),
                step_input_active: false,
                step_input_row: 0,
                step_input_min_col: 0,
                quit_pending_confirm: false,
            })
        } else {
            Ok(Self {
                buffer: EditorBuffer::from_string(String::new()),
                file_path: None,
                cursor_row: 0,
                cursor_col: 0,
                desired_col: 0,
                scroll_row: 0,
                should_quit: false,
                dirty: false,
                status: "New buffer".to_string(),
                step_input_active: false,
                step_input_row: 0,
                step_input_min_col: 0,
                quit_pending_confirm: false,
            })
        }
    }

    pub fn handle_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::MoveUp => self.move_up(),
            Action::MoveDown => self.move_down(),
            Action::MoveLeft => self.move_left(),
            Action::MoveRight => self.move_right(),
            Action::MoveHome => self.move_home(),
            Action::MoveEnd => self.move_end(),
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
                    self.status = "Step input committed".to_string();
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
            Action::ActivateStepInput => {
                let line = self.buffer.line(self.cursor_row);
                if let Some(min_col) = step_edit_start_col(&line) {
                    self.step_input_active = true;
                    self.step_input_row = self.cursor_row;
                    self.step_input_min_col = min_col;
                    self.cursor_col = self.cursor_col.max(min_col);
                    self.desired_col = self.cursor_col;
                    self.status = "Step input active".to_string();
                } else {
                    self.status = "Current line is not a BDD step".to_string();
                }
                self.quit_pending_confirm = false;
            }
            Action::ClearInputState => {
                self.step_input_active = false;
                self.status = "Input state cleared".to_string();
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

    fn move_up(&mut self) {
        if self.step_input_active {
            return;
        }
        self.cursor_row = self.cursor_row.saturating_sub(1);
        self.cursor_col = self.buffer.clamp_col(self.cursor_row, self.desired_col);
        self.quit_pending_confirm = false;
    }

    fn move_down(&mut self) {
        if self.step_input_active {
            return;
        }
        self.cursor_row = (self.cursor_row + 1).min(self.buffer.line_count().saturating_sub(1));
        self.cursor_col = self.buffer.clamp_col(self.cursor_row, self.desired_col);
        self.quit_pending_confirm = false;
    }

    fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.buffer.line_len_chars(self.cursor_row);
        }
        if self.step_input_active {
            self.cursor_row = self.step_input_row;
            self.cursor_col = self.cursor_col.max(self.step_input_min_col);
        }
        self.desired_col = self.cursor_col;
        self.quit_pending_confirm = false;
    }

    fn move_right(&mut self) {
        let line_len = self.buffer.line_len_chars(self.cursor_row);
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.buffer.line_count() {
            if self.step_input_active {
                return;
            }
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
        self.desired_col = self.cursor_col;
        self.quit_pending_confirm = false;
    }

    fn move_home(&mut self) {
        self.cursor_col = if self.step_input_active {
            self.step_input_min_col
        } else {
            0
        };
        self.desired_col = 0;
        self.quit_pending_confirm = false;
    }

    fn move_end(&mut self) {
        self.cursor_col = self.buffer.line_len_chars(self.cursor_row);
        self.desired_col = self.cursor_col;
        self.quit_pending_confirm = false;
    }

    fn page_up(&mut self) {
        if self.step_input_active {
            return;
        }
        self.cursor_row = self.cursor_row.saturating_sub(10);
        self.cursor_col = self.buffer.clamp_col(self.cursor_row, self.desired_col);
        self.quit_pending_confirm = false;
    }

    fn page_down(&mut self) {
        if self.step_input_active {
            return;
        }
        let last_row = self.buffer.line_count().saturating_sub(1);
        self.cursor_row = (self.cursor_row + 10).min(last_row);
        self.cursor_col = self.buffer.clamp_col(self.cursor_row, self.desired_col);
        self.quit_pending_confirm = false;
    }
}

pub(crate) fn step_edit_start_col(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let leading = line.len().saturating_sub(trimmed.len());
    for kw in ["Given", "When", "Then", "And", "But"] {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            let mut col = leading + kw.chars().count();
            if rest.starts_with(' ') {
                col += 1;
            }
            return Some(col);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{App, step_edit_start_col};
    use crate::keymap::Action;

    #[test]
    fn test_step_edit_boundary_detection() {
        assert_eq!(step_edit_start_col("  Given I log in"), Some(8));
        assert_eq!(step_edit_start_col("When x"), Some(5));
        assert_eq!(step_edit_start_col("Feature: x"), None);
    }

    #[test]
    fn test_activate_step_input_and_block_prefix_backspace() {
        let mut app = App::from_args().expect("app init should work");
        app.buffer = crate::editor_buffer::EditorBuffer::from_string("Given hello".to_string());
        app.handle_action(Action::ActivateStepInput)
            .expect("activate should work");
        assert!(app.step_input_active);
        assert_eq!(app.cursor_col, 6);
        app.handle_action(Action::Backspace)
            .expect("backspace should work");
        assert_eq!(app.buffer.as_string(), "Given hello");
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
}
