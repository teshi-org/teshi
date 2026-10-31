//! Real terminal automation for Teshi scenario runners.

pub mod scenario;

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tui_test::{
    KeyAction, MouseAction, MouseButton, MouseOptions, OpenOptions, Operation, OperationResult,
    RunOptions, Session,
};

/// Delay after writing a lone Escape so the application consumes it before the
/// next key byte can arrive in the same read and be parsed as an Alt prefix.
// Crossterm folds a following byte into the Escape read (ESC + q becomes
// Alt+q), which silently swallows both keys. The settling delay must outlast a
// busy frame, so it is generous rather than minimal.
const ESCAPE_SETTLE_MS: u64 = 250;

/// Executable configuration; arguments are passed directly without a shell.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    /// Executable path or name resolved through PATH.
    pub program: String,
    /// Individual executable arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional application working directory.
    pub cwd: Option<String>,
    /// Environment overrides for the child only.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Terminal width in cells.
    pub cols: u16,
    /// Terminal height in cells.
    pub rows: u16,
}

/// An owned application session, closed when dropped.
pub struct Terminal {
    session: Session,
    process_id: Option<u32>,
}

/// Artifact filenames written for one captured terminal viewport.
#[derive(Debug, Clone)]
pub struct Frame {
    /// SVG viewport written by the terminal backend.
    pub svg: String,
    /// PNG preview rendered from the SVG.
    pub png: String,
    /// Plain-text viewport.
    pub text: String,
}

impl Terminal {
    /// Launches an application in a real pseudo-terminal.
    ///
    /// # Errors
    /// Returns an error for invalid dimensions, an empty program, or PTY/spawn failure.
    pub fn launch(config: Launch) -> Result<Self> {
        validate_size(config.cols, config.rows)?;
        if config.program.trim().is_empty() {
            bail!("TUI executable must not be empty");
        }
        let mut terminal = Self {
            session: Session::new("teshi-tui"),
            process_id: None,
        };
        let defaults = OpenOptions::default();
        let opened = terminal.session.run(RunOptions {
            program: config.program,
            args: config.args,
            // Windows ConPTY can start a child with an unusable console when
            // handed a verbatim canonical cwd (\\?\); simplify only safe paths.
            cwd: config.cwd.map(|cwd| {
                dunce::simplified(std::path::Path::new(&cwd))
                    .to_string_lossy()
                    .into_owned()
            }),
            env: config.env,
            cols: config.cols,
            rows: config.rows,
            backend: defaults.backend,
            profile: defaults.profile,
            // Full-screen applications do not emit shell readiness markers.
            wait_ready: Some(false),
            restart: false,
            timeouts: defaults.timeouts,
            recording: defaults.recording,
        })?;
        terminal.process_id = opened.shell_pid;
        Ok(terminal)
    }

    /// Returns the owned application PID when the PTY backend provides it.
    pub fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    /// Writes text verbatim to the application.
    ///
    /// # Errors
    /// Returns a PTY input or session error.
    pub fn write(&self, text: &str) -> Result<()> {
        self.session
            .execute(Operation::Write { data: text.into() })?;
        Ok(())
    }

    /// Presses a named key or key combination supported by tui-test.
    ///
    /// # Errors
    /// Returns an error for unsupported keys or failed input.
    pub fn key(&self, key: &str) -> Result<()> {
        validate_key(key)?;
        self.session.execute(Operation::Key {
            keys: vec![key.into()],
            action: KeyAction::Press,
        })?;
        // A lone Escape is the byte 0x1b. Crossterm parses it as an Alt prefix
        // when the next key byte lands in the same read (`\x1b` + `q` becomes
        // Alt+q), which swallows the Escape and misinterprets the follow-up
        // key. Byte-stream PTYs (Linux) race here; ConPTY (Windows) delivers
        // discrete key records and cannot merge. Let the application consume
        // the Escape before the scenario writes the next key.
        if key == "Escape" {
            std::thread::sleep(Duration::from_millis(ESCAPE_SETTLE_MS));
        }
        Ok(())
    }

    /// Changes the PTY viewport dimensions.
    ///
    /// # Errors
    /// Returns an error for invalid dimensions or failed resizing.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        validate_size(cols, rows)?;
        self.session.execute(Operation::Resize { cols, rows })?;
        Ok(())
    }

    /// Returns the rendered viewport, excluding scrollback history.
    ///
    /// # Errors
    /// Returns a session or terminal observation error.
    pub fn screen(&self) -> Result<String> {
        match self.session.execute(Operation::Text { full: false })? {
            OperationResult::Text(text) => Ok(text),
            _ => bail!("terminal backend returned an unexpected screen result"),
        }
    }

    /// Saves the current styled terminal viewport as an SVG screenshot.
    ///
    /// # Errors
    /// Returns terminal observation or screenshot file errors.
    pub fn screenshot(&self, path: &std::path::Path) -> Result<()> {
        self.session.execute(Operation::Screenshot {
            full: false,
            path: Some(path.to_string_lossy().into_owned()),
            zoom: None,
            background: None,
        })?;
        Ok(())
    }

    /// Writes `<stem>.svg`, a `<stem>.png` preview, and `<stem>.txt` into `directory`.
    ///
    /// The text viewport proves what the step displayed even when a terminal
    /// client cannot render graphics, so every capture carries all three.
    ///
    /// # Errors
    /// Returns errors for an invalid stem, terminal observation failures, an
    /// oversized viewport, or unreadable SVG output.
    pub fn capture_frame(&self, directory: &std::path::Path, stem: &str) -> Result<Frame> {
        anyhow::ensure!(
            !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "invalid capture stem"
        );
        std::fs::create_dir_all(directory)?;
        let frame = Frame {
            svg: format!("{stem}.svg"),
            png: format!("{stem}.png"),
            text: format!("{stem}.txt"),
        };
        std::fs::write(directory.join(&frame.text), self.screen()?)?;
        self.screenshot(&directory.join(&frame.svg))?;
        let mut options = resvg::usvg::Options::default();
        static FONTS: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
            std::sync::OnceLock::new();
        options.fontdb = FONTS
            .get_or_init(|| {
                let mut database = resvg::usvg::fontdb::Database::new();
                database.load_system_fonts();
                std::sync::Arc::new(database)
            })
            .clone();
        let tree =
            resvg::usvg::Tree::from_data(&std::fs::read(directory.join(&frame.svg))?, &options)?;
        let size = tree.size().to_int_size();
        anyhow::ensure!(
            size.width() <= 16384
                && size.height() <= 16384
                && u64::from(size.width()) * u64::from(size.height()) <= 32 * 1024 * 1024,
            "terminal preview exceeds the pixel budget"
        );
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
            .context("cannot allocate terminal preview")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::default(),
            &mut pixmap.as_mut(),
        );
        pixmap.save_png(directory.join(&frame.png))?;
        Ok(frame)
    }

    /// Waits for text to become visible or absent in the current viewport.
    ///
    /// # Errors
    /// Returns an error for timeouts outside 1–120000ms, observation failure, or expiry.
    pub fn expect_text(&self, text: &str, visible: bool, timeout_ms: u64) -> Result<()> {
        validate_timeout(timeout_ms)?;
        if text.is_empty() {
            bail!("screen assertion requires nonempty text and a positive timeout");
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let screen = self.screen()?;
            if screen.contains(text) == visible {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "timed out after {timeout_ms}ms waiting for {text:?} visible={visible}\n{screen}"
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits for a viewport condition to stay true across consecutive frames.
    ///
    /// # Errors
    /// Returns an error for invalid timing, observation failure, or expiry.
    pub fn expect_stable_text(
        &self,
        text: &str,
        visible: bool,
        timeout_ms: u64,
        stable_ms: u64,
    ) -> Result<()> {
        validate_timeout(timeout_ms)?;
        if text.is_empty() || stable_ms == 0 || stable_ms > timeout_ms {
            bail!("invalid stable viewport condition");
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut matched_at = None;
        loop {
            if self.screen()?.contains(text) == visible {
                let matched = matched_at.get_or_insert_with(Instant::now);
                if matched.elapsed() >= Duration::from_millis(stable_ms) {
                    return Ok(());
                }
            } else {
                matched_at = None;
            }
            if Instant::now() >= deadline {
                bail!("viewport did not stabilize for {text:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Clicks the leftmost viewport cell rendering `text`.
    ///
    /// The coordinate is resolved from the live viewport rather than from the
    /// binding, so a click follows the layout across terminal resizes. The
    /// first matching line wins; close any overlay that duplicates `text`
    /// before clicking or the topmost copy receives the press.
    ///
    /// # Errors
    /// Returns errors for timeouts outside 1–120000ms, empty text, expiry
    /// without a match, or PTY write failure.
    pub fn click_text(&self, text: &str, timeout_ms: u64) -> Result<()> {
        validate_timeout(timeout_ms)?;
        if text.is_empty() {
            bail!("click target requires nonempty text");
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let screen = self.screen()?;
            if let Some((row, col)) = locate(&screen, text) {
                // SGR 1006 encodes 1-based row/column. Crossterm's mouse
                // parser needs a press and release pair for one click.
                self.write(&format!("\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m"))?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("timed out after {timeout_ms}ms waiting for {text:?} to click\n{screen}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Drags the left mouse button over the first viewport occurrence of `text`.
    ///
    /// The drag is resolved from the live viewport and emits SGR mouse press,
    /// motion, and release events. It is intended for text-selection acceptance
    /// tests; the selected text remains owned by the application under test.
    ///
    /// # Errors
    /// Returns errors for timeouts outside 1–120000ms, empty text, expiry
    /// without a match, or PTY write failure.
    pub fn drag_text(&self, text: &str, timeout_ms: u64) -> Result<()> {
        validate_timeout(timeout_ms)?;
        if text.is_empty() {
            bail!("drag target requires nonempty text");
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let screen = self.screen()?;
            if let Some((row, col)) = locate(&screen, text) {
                let end_col = col + text.chars().count();
                self.session.execute(Operation::Mouse {
                    action: MouseAction::Drag {
                        x1: col.saturating_sub(1) as u16,
                        y1: row.saturating_sub(1) as u16,
                        x2: end_col.saturating_sub(1) as u16,
                        y2: row.saturating_sub(1) as u16,
                        options: MouseOptions::default(),
                    },
                })?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("timed out after {timeout_ms}ms waiting for {text:?} to drag\n{screen}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Right-clicks the first viewport occurrence of `text`.
    ///
    /// The coordinate is resolved from the live viewport so the click follows
    /// the layout across terminal resizes. This is useful for pointer actions
    /// whose meaning depends on the clicked pane or selection.
    ///
    /// # Errors
    /// Returns errors for timeouts outside 1–120000ms, empty text, expiry
    /// without a match, or PTY write failure.
    pub fn right_click_text(&self, text: &str, timeout_ms: u64) -> Result<()> {
        validate_timeout(timeout_ms)?;
        if text.is_empty() {
            bail!("right-click target requires nonempty text");
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let screen = self.screen()?;
            if let Some((row, col)) = locate(&screen, text) {
                self.session.execute(Operation::Mouse {
                    action: MouseAction::Click {
                        x: Some(col.saturating_sub(1) as u16),
                        y: Some(row.saturating_sub(1) as u16),
                        on_text: None,
                        options: MouseOptions::new(MouseButton::Right),
                        clicks: 1,
                    },
                })?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "timed out after {timeout_ms}ms waiting for {text:?} to right-click\n{screen}"
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Sends a wheel event at a zero-based viewport cell.
    ///
    /// `up` selects the wheel-up direction. When `shift` is true, the event
    /// carries the terminal Shift modifier so applications that support
    /// horizontal wheel scrolling can distinguish it from vertical scrolling.
    /// The coordinates are intentionally explicit: unlike the framework's
    /// generic scroll action, this keeps wheel routing under the pointer
    /// observable in end-to-end tests.
    ///
    /// # Errors
    /// Returns a PTY input error.
    pub fn scroll_at(&self, col: u16, row: u16, up: bool, shift: bool) -> Result<()> {
        let direction = if up { 64 } else { 65 };
        let modifier = if shift { 4 } else { 0 };
        self.write(&format!(
            "\x1b[<{};{};{}M",
            direction + modifier,
            u32::from(col) + 1,
            u32::from(row) + 1
        ))
    }

    /// Waits for the application to exit with the expected code.
    ///
    /// # Errors
    /// Returns an error for timeouts outside 1–120000ms, expiry, or unexpected exit code.
    pub fn expect_exit(&self, code: i32, timeout_ms: u64) -> Result<()> {
        validate_timeout(timeout_ms)?;
        // ExpectExitCode observes shell command markers; a directly spawned TUI
        // must instead use the actual process exit status.
        self.session.execute(Operation::WaitExit {
            timeout_ms: Some(timeout_ms),
        })?;
        match self.session.execute(Operation::State)? {
            OperationResult::State(state) if state.exited == Some(code) => Ok(()),
            OperationResult::State(state) => {
                bail!(
                    "expected application exit code {code}, got {:?}",
                    state.exited
                )
            }
            _ => bail!("terminal backend returned an unexpected process state"),
        }
    }

    /// Closes the owned PTY and application.
    ///
    /// # Errors
    /// Returns backend cleanup errors.
    pub fn close(&self) -> Result<()> {
        self.session.close()?;
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Drop must also release a session after an assertion or launch error.
        let _ = self.session.close();
    }
}

fn validate_size(cols: u16, rows: u16) -> Result<()> {
    if cols == 0 || rows == 0 {
        bail!("terminal dimensions must be positive");
    }
    Ok(())
}

/// Resolves `text` to a 1-based (row, column) cell in `screen`.
///
/// The column counts characters, not bytes: control labels carry glyphs such as
/// `▾`, and a byte offset would aim the press to the right of the target.
fn locate(screen: &str, text: &str) -> Option<(usize, usize)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        line.find(text)
            .map(|index| (row + 1, line[..index].chars().count() + 1))
    })
}

fn validate_timeout(timeout_ms: u64) -> Result<()> {
    if !(1..=120_000).contains(&timeout_ms) {
        bail!("assertion timeout must be between 1 and 120000 milliseconds");
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<()> {
    let presses = tui_test::input::keys::token_to_presses(key, KeyAction::Down)?;
    // The framework treats unknown multi-character names as literal typing.
    // Our typed key action must represent one key; text belongs to Write.
    if presses.len() != 1 {
        bail!("expected one named key or character, got {key:?}; use write for text");
    }
    Ok(())
}
