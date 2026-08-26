//! VTE-backed terminal screen grid for structured output parsing.
//!
//! This module provides a [`ScreenGrid`] that feeds raw PTY output into a
//! `vte::Parser` and builds a character-cell grid with SGR attributes,
//! scrollback, dirty-row tracking, and heuristic process-state detection.

use serde_json::{json, Value};
use std::time::Instant;
use vte::Perform;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single character cell in the terminal grid.
#[derive(Debug, Clone)]
pub struct Cell {
    pub ch: char,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub fg: Color,
    pub bg: Color,
}

/// Terminal color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            fg: Color::Default,
            bg: Color::Default,
        }
    }
}

/// Process state of the shell running inside the PTY.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessState {
    Spawned,
    Running,
    Idle,
    WaitingForInput,
    Exited(i32),
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Cursor position within the grid (0-based).
#[derive(Debug, Clone, Copy)]
struct Cursor {
    row: u16,
    col: u16,
}

// ---------------------------------------------------------------------------
// GridPerformer — implements vte::Perform
// ---------------------------------------------------------------------------

/// VTE performer that builds the screen grid from ANSI byte sequences.
struct GridPerformer {
    /// The visible grid: rows × cols.
    grid: Vec<Vec<Cell>>,
    /// Current cursor position (0-based).
    cursor: Cursor,
    /// Saved cursor position (DECSC / DECRC).
    saved_cursor: Cursor,
    /// Grid dimensions.
    rows: u16,
    cols: u16,
    /// Rows that have scrolled off the top of the visible grid.
    scrollback: Vec<Vec<Cell>>,
    /// Maximum scrollback rows to keep.
    max_scrollback: usize,
    /// Per-row dirty flag.
    dirty: Vec<bool>,
    /// Timestamp of the last output byte processed.
    last_output_at: Instant,
    /// Whether new content arrived since the last `clear_dirty`.
    has_new_content: bool,
    /// Current heuristic process state.
    state: ProcessState,
    /// Whether the shell has produced any output yet.
    has_seen_output: bool,

    // ── SGR state ─────────────────────────────────────────────────────────
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    fg: Color,
    bg: Color,

    // ── Scroll margins ────────────────────────────────────────────────────
    scroll_region_top: u16,
    scroll_region_bottom: u16,

    // ── Tab stops ─────────────────────────────────────────────────────────
    tab_stops: Vec<bool>,
}

impl GridPerformer {
    fn new(rows: u16, cols: u16) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let grid = vec![vec![Cell::default(); cols as usize]; rows as usize];
        let dirty = vec![false; rows as usize];
        let tab_stops = Self::default_tab_stops(cols);

        Self {
            grid,
            cursor: Cursor { row: 0, col: 0 },
            saved_cursor: Cursor { row: 0, col: 0 },
            rows,
            cols,
            scrollback: Vec::new(),
            max_scrollback: 10_000,
            dirty,
            last_output_at: Instant::now(),
            has_new_content: false,
            state: ProcessState::Spawned,
            has_seen_output: false,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            fg: Color::Default,
            bg: Color::Default,
            scroll_region_top: 0,
            scroll_region_bottom: rows.saturating_sub(1),
            tab_stops,
        }
    }

    fn default_tab_stops(cols: u16) -> Vec<bool> {
        let mut stops = vec![false; cols as usize];
        for i in (0..cols as usize).step_by(8) {
            stops[i] = true;
        }
        stops
    }

    /// Mark the current cursor row as dirty.
    fn mark_dirty(&mut self) {
        let r = self.cursor.row as usize;
        if r < self.dirty.len() {
            self.dirty[r] = true;
        }
        self.has_new_content = true;
        self.last_output_at = Instant::now();
    }

    /// Mark every row in the grid as dirty.
    #[allow(dead_code)]
    fn mark_all_dirty(&mut self) {
        self.dirty.fill(true);
        self.has_new_content = true;
    }

    /// Write a character at the cursor, advancing the cursor.
    fn put_char(&mut self, c: char) {
        let r = self.cursor.row as usize;
        let c_idx = self.cursor.col as usize;

        if r < self.grid.len() && c_idx < self.grid[r].len() {
            let cell = &mut self.grid[r][c_idx];
            cell.ch = c;
            cell.bold = self.bold;
            cell.dim = self.dim;
            cell.italic = self.italic;
            cell.underline = self.underline;
            cell.fg = self.fg;
            cell.bg = self.bg;
            self.mark_dirty();
        }

        // Advance column, wrapping if necessary
        self.cursor.col += 1;
        if self.cursor.col >= self.cols {
            self.cursor.col = 0;
            self.newline();
        }
    }

    /// Move cursor down one row, scrolling if needed.
    fn newline(&mut self) {
        if self.cursor.row == self.scroll_region_bottom {
            self.scroll_up(1);
        } else if (self.cursor.row as usize) < self.grid.len().saturating_sub(1) {
            self.cursor.row += 1;
        }
    }

    /// Reverse index (move up, or scroll down).
    #[allow(dead_code)]
    fn reverse_index(&mut self) {
        if self.cursor.row == self.scroll_region_top {
            self.scroll_down(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    /// Scroll content within the scroll region up by `n` lines.
    fn scroll_up(&mut self, n: u16) {
        let n = n.max(1) as usize;
        let top = self.scroll_region_top as usize;
        let bot = self.scroll_region_bottom as usize;

        if top > bot {
            return;
        }
        let region_height = bot - top + 1;
        let n = n.min(region_height);

        // Save scrolled-off rows to scrollback
        for r in top..top + n {
            if let Some(row) = self.grid.get(r) {
                // Only capture non-empty rows (with content)
                let is_empty = row.iter().all(|c| c.ch == ' ');
                if !is_empty && self.scrollback.len() < self.max_scrollback {
                    self.scrollback.push(row.clone());
                }
            }
        }
        if self.scrollback.len() > self.max_scrollback {
            self.scrollback
                .drain(0..self.scrollback.len() - self.max_scrollback);
        }

        // Shift rows up within the scroll region
        for r in top..bot.saturating_add(1).saturating_sub(n) {
            let src = r + n;
            let dst = r;
            if dst < self.grid.len() && src < self.grid.len() {
                self.grid[dst] = std::mem::take(&mut self.grid[src]);
                self.dirty[dst] = true;
            }
        }
        // Clear newly exposed rows at the bottom of the scroll region
        for r in (bot.saturating_add(1).saturating_sub(n))..=bot {
            if r < self.grid.len() {
                self.grid[r] = vec![Cell::default(); self.cols as usize];
                self.dirty[r] = true;
            }
        }
    }

    /// Scroll content within the scroll region down by `n` lines.
    fn scroll_down(&mut self, n: u16) {
        let n = n.max(1) as usize;
        let top = self.scroll_region_top as usize;
        let bot = self.scroll_region_bottom as usize;

        if top > bot {
            return;
        }
        let region_height = bot - top + 1;
        let n = n.min(region_height);

        // Shift rows down within the scroll region (from bottom to top)
        for r in (top + n..=bot).rev() {
            let src = r - n;
            let dst = r;
            if dst < self.grid.len() && src < self.grid.len() {
                self.grid[dst] = std::mem::take(&mut self.grid[src]);
                self.dirty[dst] = true;
            }
        }
        // Clear newly exposed rows at the top of the scroll region
        for r in top..top + n {
            if r < self.grid.len() {
                self.grid[r] = vec![Cell::default(); self.cols as usize];
                self.dirty[r] = true;
            }
        }
    }

    /// Erase in line (CSI K).
    fn erase_in_line(&mut self, param: i64) {
        let r = self.cursor.row as usize;
        if r >= self.grid.len() {
            return;
        }
        let row = &mut self.grid[r];
        let blank = Cell::default();
        match param {
            0 => {
                // Erase from cursor to end of line
                for cell in row.iter_mut().skip(self.cursor.col as usize) {
                    *cell = blank.clone();
                }
            }
            1 => {
                // Erase from beginning of line to cursor
                for cell in row.iter_mut().take(self.cursor.col as usize + 1) {
                    *cell = blank.clone();
                }
            }
            2 => {
                // Erase entire line
                for c in row.iter_mut() {
                    *c = blank.clone();
                }
            }
            _ => {}
        }
        self.dirty[r] = true;
    }

    /// Erase in display (CSI J).
    fn erase_in_display(&mut self, param: i64) {
        let blank_row = vec![Cell::default(); self.cols as usize];
        match param {
            0 => {
                // Erase from cursor to end of screen
                let r = self.cursor.row as usize;
                // Current row from cursor
                self.erase_in_line(0);
                // Rows below
                for r in (r + 1)..self.grid.len() {
                    self.grid[r] = blank_row.clone();
                    self.dirty[r] = true;
                }
            }
            1 => {
                // Erase from beginning of screen to cursor
                let r = self.cursor.row as usize;
                // Current row to cursor
                self.erase_in_line(1);
                // Rows above
                for r in 0..r {
                    self.grid[r] = blank_row.clone();
                    self.dirty[r] = true;
                }
            }
            2 | 3 => {
                // Erase entire display (and scrollback for 3)
                for r in 0..self.grid.len() {
                    self.grid[r] = blank_row.clone();
                    self.dirty[r] = true;
                }
                if param == 3 {
                    self.scrollback.clear();
                }
            }
            _ => {}
        }
    }

    /// Delete character (CSI P).
    fn delete_chars(&mut self, n: i64) {
        let n = n.max(1) as usize;
        let r = self.cursor.row as usize;
        let c = self.cursor.col as usize;
        if r >= self.grid.len() {
            return;
        }
        let row = &mut self.grid[r];
        if c < row.len() {
            let end = row.len();
            // Shift characters left
            for i in c..end.saturating_sub(n) {
                row[i] = row[i + n].clone();
            }
            // Fill vacated positions with blanks
            let blank = Cell::default();
            for cell in row.iter_mut().take(end).skip(end.saturating_sub(n)) {
                *cell = blank.clone();
            }
            self.dirty[r] = true;
        }
    }

    /// Insert character (CSI @).
    fn insert_chars(&mut self, n: i64) {
        let n = n.max(1) as usize;
        let r = self.cursor.row as usize;
        let c = self.cursor.col as usize;
        if r >= self.grid.len() {
            return;
        }
        let row = &mut self.grid[r];
        let end = row.len();
        if c < end {
            // Shift characters right
            for i in (c..end.saturating_sub(n)).rev() {
                row[i + n] = row[i].clone();
            }
            // Insert blanks at cursor
            let blank = Cell::default();
            for cell in row.iter_mut().take((c + n).min(end)).skip(c) {
                *cell = blank.clone();
            }
            self.dirty[r] = true;
        }
    }

    /// Set a single SGR parameter.
    fn set_sgr(&mut self, param: i64, params: &[i64], param_idx: &mut usize) {
        match param {
            0 => {
                self.bold = false;
                self.dim = false;
                self.italic = false;
                self.underline = false;
                self.fg = Color::Default;
                self.bg = Color::Default;
            }
            1 => self.bold = true,
            2 => self.dim = true,
            3 => self.italic = true,
            4 => self.underline = true,
            22 => {
                self.bold = false;
                self.dim = false;
            }
            23 => self.italic = false,
            24 => self.underline = false,
            // Foreground 3-bit
            30..=37 => {
                self.fg = Color::Indexed((param - 30) as u8);
            }
            38 => {
                // Extended foreground: 38;5;N or 38;2;R;G;B
                if let Some(mode) = params.get(*param_idx + 1).copied() {
                    *param_idx += 1;
                    if mode == 5 {
                        // 256-color
                        if let Some(idx) = params.get(*param_idx + 1).copied() {
                            *param_idx += 1;
                            self.fg = Color::Indexed(idx as u8);
                        }
                    } else if mode == 2 {
                        // 24-bit
                        let r = params.get(*param_idx + 1).copied().unwrap_or(0) as u8;
                        let g = params.get(*param_idx + 2).copied().unwrap_or(0) as u8;
                        let b = params.get(*param_idx + 3).copied().unwrap_or(0) as u8;
                        *param_idx += 3;
                        self.fg = Color::Rgb(r, g, b);
                    }
                }
            }
            39 => self.fg = Color::Default,
            // Background 3-bit
            40..=47 => {
                self.bg = Color::Indexed((param - 40) as u8);
            }
            48 => {
                // Extended background: 48;5;N or 48;2;R;G;B
                if let Some(mode) = params.get(*param_idx + 1).copied() {
                    *param_idx += 1;
                    if mode == 5 {
                        if let Some(idx) = params.get(*param_idx + 1).copied() {
                            *param_idx += 1;
                            self.bg = Color::Indexed(idx as u8);
                        }
                    } else if mode == 2 {
                        let r = params.get(*param_idx + 1).copied().unwrap_or(0) as u8;
                        let g = params.get(*param_idx + 2).copied().unwrap_or(0) as u8;
                        let b = params.get(*param_idx + 3).copied().unwrap_or(0) as u8;
                        *param_idx += 3;
                        self.bg = Color::Rgb(r, g, b);
                    }
                }
            }
            49 => self.bg = Color::Default,
            // Bright foreground
            90..=97 => self.fg = Color::Indexed(param as u8),
            // Bright background
            100..=107 => self.bg = Color::Indexed(param as u8),
            _ => {}
        }
    }

    /// Process a CSI dispatch by action byte.
    fn handle_csi(&mut self, params: &[i64], action: u8) {
        // Helper: get param with default value.
        let p = |idx: usize, default: i64| -> i64 { params.get(idx).copied().unwrap_or(default) };

        match action {
            b'A' => {
                // CUU – Cursor Up
                let n = p(0, 1).max(1);
                self.cursor.row = self.cursor.row.saturating_sub(n as u16);
            }
            b'B' => {
                // CUD – Cursor Down
                let n = p(0, 1).max(1);
                let new_row = self.cursor.row.saturating_add(n as u16);
                self.cursor.row = new_row.min(self.rows.saturating_sub(1));
            }
            b'C' => {
                // CUF – Cursor Forward
                let n = p(0, 1).max(1);
                let new_col = self.cursor.col.saturating_add(n as u16);
                self.cursor.col = new_col.min(self.cols.saturating_sub(1));
            }
            b'D' => {
                // CUB – Cursor Backward
                let n = p(0, 1).max(1);
                self.cursor.col = self.cursor.col.saturating_sub(n as u16);
            }
            b'H' | b'f' => {
                // CUP / HVP – Cursor Position
                let row = (p(0, 1) - 1).max(0).min((self.rows - 1) as i64) as u16;
                let col = (p(1, 1) - 1).max(0).min((self.cols - 1) as i64) as u16;
                self.cursor.row = row;
                self.cursor.col = col;
            }
            b'G' => {
                // CHA – Cursor Horizontal Absolute
                let col = (p(0, 1) - 1).max(0).min((self.cols - 1) as i64) as u16;
                self.cursor.col = col;
            }
            b'J' => {
                // ED – Erase in Display
                self.erase_in_display(p(0, 0));
            }
            b'K' => {
                // EL – Erase in Line
                self.erase_in_line(p(0, 0));
            }
            b'P' => {
                // DCH – Delete Character
                self.delete_chars(p(0, 1));
            }
            b'@' => {
                // ICH – Insert Character
                self.insert_chars(p(0, 1));
            }
            b'm' => {
                // SGR – Select Graphic Rendition
                let mut i = 0;
                while i < params.len() {
                    let param = params[i];
                    self.set_sgr(param, params, &mut i);
                    i += 1;
                }
            }
            b's' => {
                // SCP – Save Cursor Position
                self.saved_cursor = self.cursor;
            }
            b'u' => {
                // RCP – Restore Cursor Position
                self.cursor = self.saved_cursor;
            }
            b'S' => {
                // SU – Scroll Up
                let n = p(0, 1).max(1);
                self.scroll_up(n as u16);
            }
            b'T' => {
                // SD – Scroll Down
                let n = p(0, 1).max(1);
                self.scroll_down(n as u16);
            }
            b'n' => {
                // DSR – Device Status Report (CPR response ignored)
            }
            b'l' | b'h' => {
                // SM / RM – Set/Reset Mode (DEC private modes etc.)
                // We handle scroll region reset here minimally.
                let mode = p(0, 0);
                if action == b'h' && mode == 7 {
                    // DECAWM – Auto-wrap; not tracked yet, ignore
                }
            }
            b'r' => {
                // DECSTBM – Set Top and Bottom Margins
                let top = (p(0, 1) - 1).max(0) as u16;
                let bot = if params.len() > 1 {
                    (p(1, 1) - 1).max(0).min(self.rows.saturating_sub(1) as i64) as u16
                } else {
                    self.rows.saturating_sub(1)
                };
                self.scroll_region_top = top.min(bot);
                self.scroll_region_bottom = bot.max(top);
                // Cursor to home position (1,1) per DECSTBM
                self.cursor.row = 0;
                self.cursor.col = 0;
            }
            _ => {}
        }
    }

    /// Check whether the cursor is currently sitting on a row that looks like a
    /// shell prompt (ends with `$`, `#`, `>`, `%`, `:`, `]` etc.).
    fn last_row_looks_like_prompt(&self) -> bool {
        let r = self.cursor.row as usize;
        if r >= self.grid.len() {
            return false;
        }
        let row = &self.grid[r];

        // Find the last non-space character
        let last_ch = row.iter().rev().find(|c| c.ch != ' ').map(|c| c.ch);
        match last_ch {
            Some(c) => matches!(c, '$' | '#' | '>' | '%' | ':' | ']' | '❯' | 'λ' | '→'),
            None => false,
        }
    }

    /// Update the heuristic process state based on time since last output and
    /// the visible grid content.
    fn update_process_state(&mut self) {
        // Transition: Spawned -> Running on first output
        if !self.has_seen_output {
            self.state = ProcessState::Spawned;
            return;
        }

        // If we've been idle for 500ms, check for prompt
        let idle_ms = self.last_output_at.elapsed().as_millis();
        if idle_ms >= 500 {
            if self.last_row_looks_like_prompt() {
                self.state = ProcessState::WaitingForInput;
            } else {
                self.state = ProcessState::Idle;
            }
        } else {
            self.state = ProcessState::Running;
        }
    }
}

impl Perform for GridPerformer {
    fn print(&mut self, c: char) {
        self.has_seen_output = true;
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        self.has_seen_output = true;
        match byte {
            b'\n' => {
                // Line feed
                self.cursor.col = 0; // implicit carriage return on LF (common terminal behaviour)
                self.newline();
            }
            b'\r' => {
                // Carriage return
                self.cursor.col = 0;
            }
            b'\t' => {
                // Horizontal tab
                let col = self.cursor.col as usize;
                // Find next tab stop
                for i in col + 1..self.tab_stops.len() {
                    if self.tab_stops[i] {
                        self.cursor.col = i as u16;
                        return;
                    }
                }
                // No more tab stops – go to end of line
                self.cursor.col = self.cols.saturating_sub(1);
            }
            0x08 => {
                // Backspace
                if self.cursor.col > 0 {
                    self.cursor.col -= 1;
                }
            }
            0x07 => {
                // Bell – ignore
            }
            0x0b | 0x0c => {
                // Vertical tab / Form feed – treat as LF
                self.newline();
            }
            _ => {}
        }
        self.last_output_at = Instant::now();
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
        // DCS hook – not handling device control strings for now
    }

    fn put(&mut self, _byte: u8) {
        // DCS data – ignored
    }

    fn unhook(&mut self) {
        // DCS unhook – ignored
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC – operating system command (e.g. set window title) – ignored
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        self.has_seen_output = true;
        // Extract flat parameter list: each Params group may have sub-params (separated by ';'),
        // but for standard CSI we take the first value from each group.
        let p: Vec<i64> = params
            .iter()
            .map(|sub| sub.first().copied().unwrap_or(0) as i64)
            .collect();
        self.handle_csi(&p, action as u8);
        self.last_output_at = Instant::now();
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _action: u8) {
        // Escape sequences (e.g. DECSET, DECRST) – mostly ignored for now
    }
}

// ---------------------------------------------------------------------------
// ScreenGrid — public thread-safe wrapper
// ---------------------------------------------------------------------------

/// Thread-safe screen grid wrapper.
pub struct ScreenGrid {
    inner: std::sync::Mutex<GridPerformer>,
}

impl ScreenGrid {
    /// Create a new screen grid with the given viewport dimensions.
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            inner: std::sync::Mutex::new(GridPerformer::new(rows, cols)),
        }
    }

    /// Feed raw PTY bytes into the VTE parser, updating the grid.
    pub fn feed(&self, bytes: &[u8]) {
        let mut perf = self.inner.lock().unwrap();
        let mut parser = vte::Parser::new();
        parser.advance(&mut *perf, bytes);
    }

    /// Resize the grid to new dimensions. Content is preserved/clipped.
    pub fn resize(&self, rows: u16, cols: u16) {
        let mut perf = self.inner.lock().unwrap();
        let cols = cols.max(1);
        let rows = rows.max(1);

        if cols == perf.cols && rows == perf.rows {
            return;
        }

        // Resize each existing row to the new column count
        for row in &mut perf.grid {
            if cols > row.len() as u16 {
                // Extend
                row.resize(cols as usize, Cell::default());
            } else {
                // Truncate
                row.truncate(cols as usize);
            }
        }

        // Adjust row count
        if rows > perf.rows {
            // Add new blank rows at the bottom
            let extra = (rows - perf.rows) as usize;
            let blank_row = vec![Cell::default(); cols as usize];
            for _ in 0..extra {
                perf.grid.push(blank_row.clone());
            }
        } else if rows < perf.rows {
            // Remove rows from the bottom
            perf.grid.truncate(rows as usize);
            // Also trim scrollback if the visible area shrinks
        }

        perf.rows = rows;
        perf.cols = cols;

        // Update dirty tracking
        perf.dirty = vec![true; rows as usize];

        // Update scroll region to new dimensions if old bounds were at the edges
        if perf.scroll_region_top == 0 && perf.scroll_region_bottom >= perf.rows.saturating_sub(1) {
            perf.scroll_region_bottom = rows.saturating_sub(1);
        } else {
            perf.scroll_region_top = perf.scroll_region_top.min(rows.saturating_sub(1));
            perf.scroll_region_bottom = perf.scroll_region_bottom.min(rows.saturating_sub(1));
        }

        // Rebuild tab stops
        perf.tab_stops = GridPerformer::default_tab_stops(cols);

        perf.has_new_content = true;
    }

    /// Return a JSON snapshot of the grid.
    ///
    /// When `full` is false only dirty rows are included; the caller is expected
    /// to call `clear_dirty` after consuming.
    pub fn snapshot(&self, full: bool) -> Value {
        let perf = self.inner.lock().unwrap();
        let mut rows = Vec::new();

        for (r, row) in perf.grid.iter().enumerate() {
            if !full && !perf.dirty[r] {
                continue;
            }
            let cells: Vec<Value> = row
                .iter()
                .map(|c| {
                    let fg_val = color_to_value(c.fg);
                    let bg_val = color_to_value(c.bg);
                    json!({
                        "ch": c.ch.to_string(),
                        "bold": c.bold,
                        "dim": c.dim,
                        "italic": c.italic,
                        "underline": c.underline,
                        "fg": fg_val,
                        "bg": bg_val,
                    })
                })
                .collect();

            rows.push(json!({
                "row": r,
                "cells": cells,
            }));
        }

        json!({
            "rows": perf.rows,
            "cols": perf.cols,
            "cursor": {
                "row": perf.cursor.row,
                "col": perf.cursor.col,
            },
            "data": rows,
        })
    }

    /// Clear the dirty flags for all rows.
    pub fn clear_dirty(&self) {
        let mut perf = self.inner.lock().unwrap();
        perf.dirty.fill(false);
        perf.has_new_content = false;
    }

    /// Return a lightweight status JSON (no cell data).
    pub fn status(&self) -> Value {
        let perf = self.inner.lock().unwrap();
        let state_str = match perf.state {
            ProcessState::Spawned => "spawned",
            ProcessState::Running => "running",
            ProcessState::Idle => "idle",
            ProcessState::WaitingForInput => "waiting-for-input",
            ProcessState::Exited(_) => "exited",
        };

        json!({
            "rows": perf.rows,
            "cols": perf.cols,
            "cursor": {
                "row": perf.cursor.row,
                "col": perf.cursor.col,
            },
            "state": state_str,
            "scrollback_len": perf.scrollback.len(),
            "dirty_count": perf.dirty.iter().filter(|&&d| d).count(),
        })
    }

    /// Return the current heuristic process state.
    pub fn process_state(&self) -> ProcessState {
        let mut perf = self.inner.lock().unwrap();
        perf.update_process_state();
        perf.state
    }

    /// Whether new content has arrived since the last `clear_dirty`.
    pub fn has_new_content(&self) -> bool {
        let perf = self.inner.lock().unwrap();
        perf.has_new_content
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn color_to_value(c: Color) -> Value {
    match c {
        Color::Default => json!("default"),
        Color::Indexed(n) => json!(n),
        Color::Rgb(r, g, b) => json!([r, g, b]),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_grid_has_correct_dimensions() {
        let sg = ScreenGrid::new(24, 80);
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid.len(), 24);
        assert_eq!(perf.grid[0].len(), 80);
        assert_eq!(perf.rows, 24);
        assert_eq!(perf.cols, 80);
        assert_eq!(perf.state, ProcessState::Spawned);
    }

    #[test]
    fn feed_printable_text() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"hello");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].ch, 'h');
        assert_eq!(perf.grid[0][1].ch, 'e');
        assert_eq!(perf.grid[0][2].ch, 'l');
        assert_eq!(perf.grid[0][3].ch, 'l');
        assert_eq!(perf.grid[0][4].ch, 'o');
        assert_eq!(perf.cursor.col, 5);
    }

    #[test]
    fn newline_advances_row() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"abc\ndef");
        let perf = sg.inner.lock().unwrap();
        // First row: "abc"
        assert_eq!(perf.grid[0][0].ch, 'a');
        assert_eq!(perf.grid[0][1].ch, 'b');
        assert_eq!(perf.grid[0][2].ch, 'c');
        // Second row: "def"
        assert_eq!(perf.grid[1][0].ch, 'd');
        assert_eq!(perf.grid[1][1].ch, 'e');
        assert_eq!(perf.grid[1][2].ch, 'f');
        assert_eq!(perf.cursor.row, 1);
        assert_eq!(perf.cursor.col, 3);
    }

    #[test]
    fn carriage_return_moves_cursor_to_col_0() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"abcdef\rxyz");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].ch, 'x');
        assert_eq!(perf.grid[0][1].ch, 'y');
        assert_eq!(perf.grid[0][2].ch, 'z');
        assert_eq!(perf.grid[0][3].ch, 'd');
        assert_eq!(perf.cursor.col, 3);
    }

    #[test]
    fn cursor_up_and_down() {
        let sg = ScreenGrid::new(10, 10);
        sg.feed(b"\x1b[5B"); // CUD 5
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.row, 5);
        drop(perf);

        sg.feed(b"\x1b[3A"); // CUU 3
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.row, 2);
    }

    #[test]
    fn cursor_forward_backward() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"\x1b[5C"); // CUF 5
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.col, 5);
        drop(perf);

        sg.feed(b"\x1b[2D"); // CUB 2
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.col, 3);
    }

    #[test]
    fn cursor_position_absolute() {
        let sg = ScreenGrid::new(10, 20);
        sg.feed(b"\x1b[5;10H"); // CUP row=5, col=10
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.row, 4);
        assert_eq!(perf.cursor.col, 9);
    }

    #[test]
    fn erase_in_line() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"abcdefghi");
        sg.feed(b"\x1b[2D"); // CUB 2 → col 7 (9-2)
        sg.feed(b"\x1b[K"); // EL 0 — erase from col 7 to end of line
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].ch, 'a');
        assert_eq!(perf.grid[0][5].ch, 'f');
        assert_eq!(perf.grid[0][6].ch, 'g');
        assert_eq!(perf.grid[0][7].ch, ' '); // erased
        assert_eq!(perf.grid[0][8].ch, ' '); // erased
        assert_eq!(perf.grid[0][9].ch, ' '); // erased
    }

    #[test]
    fn sgr_bold_reset() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"\x1b[1mhello\x1b[0m world");
        let perf = sg.inner.lock().unwrap();
        assert!(perf.grid[0][0].bold);
        assert!(perf.grid[0][4].bold);
        assert!(!perf.grid[0][6].bold);
        assert_eq!(perf.grid[0][6].ch, 'w');
        assert_eq!(perf.grid[0][7].ch, 'o');
    }

    #[test]
    fn sgr_256_color() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"\x1b[38;5;200mhello");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].fg, Color::Indexed(200));
        assert_eq!(perf.grid[0][4].fg, Color::Indexed(200));
    }

    #[test]
    fn sgr_24_bit_color() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"\x1b[38;2;255;128;64mtest");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].fg, Color::Rgb(255, 128, 64));
    }

    #[test]
    fn scroll_up_shifts_content() {
        let sg = ScreenGrid::new(3, 10);
        sg.feed(b"line1\nline2\nline3\nline4");
        let perf = sg.inner.lock().unwrap();
        // After 3 newlines, the first line should have scrolled off
        assert_eq!(perf.grid[0][0].ch, 'l');
        assert_eq!(perf.grid[0][4].ch, '2');
        assert_eq!(perf.grid[1][0].ch, 'l');
        assert_eq!(perf.grid[1][4].ch, '3');
        assert_eq!(perf.grid[2][0].ch, 'l');
        assert_eq!(perf.grid[2][4].ch, '4');
        // line1 should be in scrollback
        assert!(!perf.scrollback.is_empty());
    }

    #[test]
    fn resize_expands_grid() {
        let sg = ScreenGrid::new(3, 5);
        sg.feed(b"hello");
        sg.resize(5, 10);
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.rows, 5);
        assert_eq!(perf.cols, 10);
        assert_eq!(perf.grid.len(), 5);
        assert_eq!(perf.grid[0].len(), 10);
        assert_eq!(perf.grid[0][0].ch, 'h');
        assert!(perf.dirty.iter().all(|&d| d));
    }

    #[test]
    fn resize_truncates_grid() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"hello world!");
        sg.resize(3, 5);
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.rows, 3);
        assert_eq!(perf.cols, 5);
        assert_eq!(perf.grid.len(), 3);
        assert_eq!(perf.grid[0].len(), 5);
        assert_eq!(perf.grid[0][0].ch, 'h');
    }

    #[test]
    fn process_state_transitions() {
        let sg = ScreenGrid::new(5, 10);
        assert_eq!(sg.process_state(), ProcessState::Spawned);
        sg.feed(b"echo hello");
        // Immediately after output it should be Running
        assert_eq!(sg.process_state(), ProcessState::Running);
    }

    #[test]
    fn has_new_content_tracking() {
        let sg = ScreenGrid::new(5, 10);
        assert!(!sg.has_new_content());
        sg.feed(b"hello");
        assert!(sg.has_new_content());
        sg.clear_dirty();
        assert!(!sg.has_new_content());
        sg.feed(b" ");
        assert!(sg.has_new_content());
    }

    #[test]
    fn snapshot_returns_json() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"hi");
        let snap = sg.snapshot(true);
        assert_eq!(snap["rows"], 5);
        assert_eq!(snap["cols"], 10);
        assert_eq!(snap["cursor"]["row"], 0);
        assert_eq!(snap["cursor"]["col"], 2);
        assert!(snap["data"].is_array());
    }

    #[test]
    fn tab_stops() {
        let sg = ScreenGrid::new(5, 20);
        sg.feed(b"\t"); // tab to col 8
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.col, 8);
    }

    #[test]
    fn backspace() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"ab");
        sg.feed(b"\x08"); // backspace
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.col, 1);
    }

    #[test]
    fn save_restore_cursor() {
        let sg = ScreenGrid::new(10, 20);
        sg.feed(b"\x1b[5;10H\x1b[s\x1b[2;3H");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.row, 1);
        assert_eq!(perf.cursor.col, 2);
        drop(perf);

        sg.feed(b"\x1b[u");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.cursor.row, 4);
        assert_eq!(perf.cursor.col, 9);
    }

    #[test]
    fn scroll_up_down() {
        let sg = ScreenGrid::new(10, 10);
        sg.feed(b"1\n2\n3\n4\n5");
        sg.feed(b"\x1b[2T"); // SD 2 — scroll down 2 lines (blank lines at top)
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].ch, ' ');
        assert_eq!(perf.grid[1][0].ch, ' ');
        assert_eq!(perf.grid[2][0].ch, '1');
    }

    #[test]
    fn delete_chars() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"abcdefghi");
        // cursor at col 9. Move to col 3 (0-indexed = CHA 4)
        sg.feed(b"\x1b[4G"); // CHA to col 3 (1-indexed: 4)
        sg.feed(b"\x1b[3P"); // DCH 3 — delete cols 3,4,5 (d,e,f)
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].ch, 'a');
        assert_eq!(perf.grid[0][1].ch, 'b');
        assert_eq!(perf.grid[0][2].ch, 'c');
        assert_eq!(perf.grid[0][3].ch, 'g'); // g shifted from col 6 → 3
        assert_eq!(perf.grid[0][4].ch, 'h'); // h shifted from col 7 → 4
        assert_eq!(perf.grid[0][5].ch, 'i'); // i shifted from col 8 → 5
        assert_eq!(perf.grid[0][6].ch, ' '); // blank
        assert_eq!(perf.grid[0][7].ch, ' '); // blank
        assert_eq!(perf.grid[0][8].ch, ' '); // blank
        assert_eq!(perf.grid[0][9].ch, ' '); // blank
    }

    #[test]
    fn insert_chars() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"abcdefghi");
        sg.feed(b"\x1b[3G"); // CHA to col 2 (0-indexed)
        sg.feed(b"\x1b[2@"); // ICH 2 — insert 2 blanks at col 2
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].ch, 'a');
        assert_eq!(perf.grid[0][1].ch, 'b');
        assert_eq!(perf.grid[0][2].ch, ' '); // inserted blank
        assert_eq!(perf.grid[0][3].ch, ' '); // inserted blank
        assert_eq!(perf.grid[0][4].ch, 'c'); // c shifted from col 2 → 4
        assert_eq!(perf.grid[0][5].ch, 'd');
        assert_eq!(perf.grid[0][6].ch, 'e');
        assert_eq!(perf.grid[0][7].ch, 'f');
        assert_eq!(perf.grid[0][8].ch, 'g');
        assert_eq!(perf.grid[0][9].ch, 'h'); // i would go off the end
    }

    #[test]
    fn erase_in_display_full() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"line1\nline2\nline3\nline4");
        sg.feed(b"\x1b[2J"); // ED 2 - clear entire display
        let perf = sg.inner.lock().unwrap();
        for row in &perf.grid {
            for cell in row {
                assert_eq!(cell.ch, ' ');
            }
        }
    }

    #[test]
    fn sgr_bright_foreground() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"\x1b[91mtest");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].fg, Color::Indexed(91));
    }

    #[test]
    fn sgr_bright_background() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"\x1b[105mtest");
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.grid[0][0].bg, Color::Indexed(105));
    }

    #[test]
    fn status_returns_scrollback_count() {
        let sg = ScreenGrid::new(3, 10);
        sg.feed(b"1\n2\n3\n4\n5");
        let st = sg.status();
        assert!(st["scrollback_len"].as_i64().unwrap_or(0) > 0);
    }

    #[test]
    fn process_state_starts_as_spawned() {
        let sg = ScreenGrid::new(24, 80);
        assert_eq!(sg.process_state(), ProcessState::Spawned);
    }

    #[test]
    fn process_state_changes_to_running_after_feed() {
        let sg = ScreenGrid::new(24, 80);
        sg.feed(b"something");
        assert_eq!(sg.process_state(), ProcessState::Running);
    }

    #[test]
    fn snap_without_full_only_includes_dirty() {
        let sg = ScreenGrid::new(5, 10);
        sg.feed(b"hello");
        sg.clear_dirty();
        sg.feed(b"world");
        let snap = sg.snapshot(false);
        // Only row 0 should be in the snapshot (row 1 hasn't been touched)
        assert_eq!(snap["data"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn scroll_region_bounds() {
        let sg = ScreenGrid::new(10, 10);
        sg.feed(b"\x1b[3;8r"); // DECSTBM top=3, bottom=8
        let perf = sg.inner.lock().unwrap();
        assert_eq!(perf.scroll_region_top, 2);
        assert_eq!(perf.scroll_region_bottom, 7);
    }
}
