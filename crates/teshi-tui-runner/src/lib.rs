//! Real terminal automation for Teshi scenario runners.

pub mod scenario;

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tui_test::{KeyAction, OpenOptions, Operation, OperationResult, RunOptions, Session};

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
