//! Test Points 2D MindMap canvas, live execution status, direct CRUD, and Linked Inspector state.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use teshi_core::authoring::{
    AuthoringArtifacts, HierarchyPath, ResolutionState, ReviewState, TestPoint, TextRange,
    parse_teshi_tp_tags, slice_by_char_range,
};
use teshi_core::gherkin::BddProject;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::RunStatus;
use crate::mindmap::HighlightCategory;
use crate::theme;

const MAX_UNDO_HISTORY: usize = 50;
const NODE_HEIGHT: u16 = 3;
const MIN_NODE_WIDTH: u16 = 14;
const MAX_NODE_WIDTH: u16 = 44;
const GAP_X: u16 = 5;
const GAP_Y: u16 = 1;
const CANVAS_PAD_X: u16 = 2;
const CANVAS_PAD_Y: u16 = 1;

const DIR_LEFT: u8 = 1;
const DIR_RIGHT: u8 = 2;
const DIR_UP: u8 = 4;
const DIR_DOWN: u8 = 8;

/// Pane focus on the Test Points tab.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TestPointsFocus {
    /// 2D MindMap Canvas (primary left workspace).
    #[default]
    Tree,
    /// Right Linked Inspector metadata/fields.
    Details,
    /// Right Linked Inspector scenarios / requirement excerpts.
    Excerpts,
}

impl TestPointsFocus {
    pub fn next(self) -> Self {
        match self {
            Self::Tree => Self::Details,
            Self::Details => Self::Excerpts,
            Self::Excerpts => Self::Excerpts,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Tree => Self::Tree,
            Self::Details => Self::Tree,
            Self::Excerpts => Self::Details,
        }
    }
}

/// Editable fields in the right Linked Inspector.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DetailField {
    #[default]
    Title,
    Objective,
    Preconditions,
    ExpectedOutcomes,
    Hierarchy,
}

impl DetailField {
    pub fn next(self) -> Self {
        match self {
            Self::Title => Self::Hierarchy,
            Self::Hierarchy => Self::Objective,
            Self::Objective => Self::Preconditions,
            Self::Preconditions => Self::ExpectedOutcomes,
            Self::ExpectedOutcomes => Self::ExpectedOutcomes,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Title => Self::Title,
            Self::Hierarchy => Self::Title,
            Self::Objective => Self::Hierarchy,
            Self::Preconditions => Self::Objective,
            Self::ExpectedOutcomes => Self::Preconditions,
        }
    }
}

/// Live coverage & BDD execution state shown on canvas nodes and inspector.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TestPointExecStatus {
    #[default]
    Uncovered,
    Linked,
    Running,
    Passed,
    Failed,
}

impl TestPointExecStatus {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Uncovered => "○",
            Self::Linked => "◐",
            Self::Running => "⠋",
            Self::Passed => "✔",
            Self::Failed => "✖",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Uncovered => crate::i18n::t("tp.status.uncovered"),
            Self::Linked => crate::i18n::t("tp.status.linked"),
            Self::Running => crate::i18n::t("tp.status.running"),
            Self::Passed => crate::i18n::t("tp.status.passed"),
            Self::Failed => crate::i18n::t("tp.status.failed"),
        }
    }

    pub fn style(self) -> Style {
        match self {
            Self::Uncovered => Style::default().fg(theme::MUTED),
            Self::Linked => Style::default().fg(theme::PRIMARY),
            Self::Running => Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
            Self::Passed => Style::default().fg(theme::SUCCESS),
            Self::Failed => Style::default()
                .fg(theme::ERROR)
                .add_modifier(Modifier::BOLD),
        }
    }
}

/// Filter on Test Points 2D MindMap canvas.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    #[default]
    All,
    Uncovered,
    Linked,
    Passed,
    Failed,
}

/// Compatibility alias retained for existing callers.
pub type ReviewFilter = StatusFilter;

impl StatusFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Uncovered,
            Self::Uncovered => Self::Linked,
            Self::Linked => Self::Passed,
            Self::Passed => Self::Failed,
            Self::Failed => Self::All,
        }
    }
}

/// Directional movement in 2D spatial canvas navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialDir {
    Left,
    Right,
    Up,
    Down,
}

/// Kind of node placed on the 2D MindMap canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TpNodeKind {
    Root,
    Branch { path: Vec<String> },
    Leaf { tp_id: String },
}

/// A laid-out box node on the 2D MindMap canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedTpNode {
    pub id: String,
    pub kind: TpNodeKind,
    pub label: String,
    pub note: String,
    pub parent_id: Option<String>,
    pub children_ids: Vec<String>,
    pub depth: usize,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub exec_status: TestPointExecStatus,
    pub scenario_count: usize,
    pub req_count: usize,
    pub collapsed_hidden_count: usize,
    pub highlight: HighlightCategory,
}

impl PlacedTpNode {
    pub fn mid_y(&self) -> u16 {
        self.y + self.height / 2
    }

    pub fn right_x(&self) -> u16 {
        self.x + self.width.saturating_sub(1)
    }
}

/// Active inline editing session inside a canvas node box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineNodeEdit {
    pub node_id: String,
    pub kind: TpNodeKind,
    pub buffer: String,
    pub cursor: usize,
    pub is_new: bool,
}

/// Linked requirement document excerpt displayed in the Inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementExcerpt {
    pub document_id: String,
    pub document_title: String,
    pub revision: String,
    pub resolution: ResolutionState,
    pub position: TextRange,
    pub resolved: bool,
    pub quote: String,
}

/// 2D sparse character + style + 4-bit junction canvas modeled after `icode`'s diagram canvas.
#[derive(Debug, Clone)]
pub struct TpCanvas {
    pub width: usize,
    pub height: usize,
    chars: Vec<String>,
    styles: Vec<Style>,
    dirs: Vec<u8>,
    rounded: Vec<bool>,
}

impl TpCanvas {
    pub fn new(width: usize, height: usize, default_bg: Color) -> Self {
        let len = width.saturating_mul(height);
        Self {
            width,
            height,
            chars: vec![" ".to_string(); len],
            styles: vec![Style::default().fg(theme::FG).bg(default_bg); len],
            dirs: vec![0; len],
            rounded: vec![false; len],
        }
    }

    fn idx(&self, x: usize, y: usize) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(y * self.width + x)
        } else {
            None
        }
    }

    /// Put a single ASCII / 1-column character at `(x, y)` without touching `dirs`.
    pub fn put_char(&mut self, x: usize, y: usize, ch: char, style: Style) {
        if let Some(i) = self.idx(x, y) {
            self.chars[i] = ch.to_string();
            self.styles[i] = style;
            self.dirs[i] = 0;
        }
    }

    /// Accumulate directional bits onto `(x, y)` and synthesize box-drawing junctions.
    pub fn add_dir(&mut self, x: usize, y: usize, mask: u8, style: Style, rounded: bool) {
        if let Some(i) = self.idx(x, y) {
            self.dirs[i] |= mask;
            self.rounded[i] |= rounded;
            self.chars[i] = junction_char(self.dirs[i], self.rounded[i]).to_string();
            self.styles[i] = style;
        }
    }

    /// Place display text starting at `(x, y)` with wide-character continuation cells (`""`).
    /// Clips strictly to `[x, max_x)`.
    pub fn put_text(&mut self, x: usize, y: usize, text: &str, max_x: usize, style: Style) {
        if y >= self.height {
            return;
        }
        let limit = max_x.min(self.width);
        let mut col = x;
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' || ch == '\t' {
                continue;
            }
            let w = ch.width().unwrap_or(0);
            if w == 0 {
                continue;
            }
            if col + w > limit {
                break;
            }
            if let Some(i) = self.idx(col, y) {
                self.chars[i] = ch.to_string();
                self.styles[i] = style;
                self.dirs[i] = 0;
            }
            if w == 2
                && let Some(cont) = self.idx(col + 1, y)
            {
                self.chars[cont].clear();
                self.styles[cont] = style;
                self.dirs[cont] = 0;
            }
            col += w;
        }
    }

    /// Draw a 3-row node box with explicit background fill on every interior cell.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_node_box(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        icon: &str,
        label: &str,
        badge: &str,
        rounded: bool,
        border_style: Style,
        icon_style: Style,
        title_style: Style,
        badge_style: Style,
        fill_bg: Color,
    ) {
        let x = x as usize;
        let y = y as usize;
        let w = w as usize;
        let h = h as usize;
        if w < 4 || h < 3 {
            return;
        }
        let border_style = border_style.bg(fill_bg);
        let icon_style = icon_style.bg(fill_bg);
        let title_style = title_style.bg(fill_bg);
        let badge_style = badge_style.bg(fill_bg);
        let fill_style = Style::default().bg(fill_bg).fg(theme::FG);

        // Fill interior cells first so no terminal default bg leaks.
        for ry in y..y + h {
            for rx in x..x + w {
                if let Some(i) = self.idx(rx, ry) {
                    self.chars[i] = " ".to_string();
                    self.styles[i] = fill_style;
                    self.dirs[i] = 0;
                }
            }
        }

        let (tl, tr, bl, br) = if rounded {
            ('╭', '╮', '╰', '╯')
        } else {
            ('┌', '┐', '└', '┘')
        };
        let right = x + w - 1;
        let bottom = y + h - 1;

        self.put_char(x, y, tl, border_style);
        self.put_char(right, y, tr, border_style);
        self.put_char(x, bottom, bl, border_style);
        self.put_char(right, bottom, br, border_style);
        for rx in (x + 1)..right {
            self.put_char(rx, y, '─', border_style);
            self.put_char(rx, bottom, '─', border_style);
        }
        for ry in (y + 1)..bottom {
            self.put_char(x, ry, '│', border_style);
            self.put_char(right, ry, '│', border_style);
        }

        let mid_y = y + h / 2;
        let inner_start = x + 2;
        let inner_end = right.saturating_sub(1);
        if inner_start >= inner_end {
            return;
        }
        let avail = inner_end - inner_start;
        let badge_w = badge.width();
        let icon_w = icon.width();
        let prefix_w = if icon_w > 0 { icon_w + 1 } else { 0 };
        let show_badge = badge_w > 0 && avail > prefix_w + badge_w + 2;
        let badge_reserve = if show_badge { badge_w + 1 } else { 0 };
        let title_budget = avail.saturating_sub(prefix_w + badge_reserve);

        if icon_w > 0 && inner_start + icon_w <= inner_end {
            self.put_text(inner_start, mid_y, icon, inner_end, icon_style);
        }
        let title_x = inner_start + prefix_w;
        let truncated = truncate_display(label, title_budget);
        self.put_text(
            title_x,
            mid_y,
            &truncated,
            title_x + title_budget,
            title_style,
        );

        if show_badge {
            let badge_x = inner_end - badge_w;
            self.put_text(badge_x, mid_y, badge, inner_end, badge_style);
        }
    }

    /// Draw shared-trunk orthogonal connectors from parent right edge to each child left edge.
    pub fn draw_tree_connector(
        &mut self,
        p_right_x: u16,
        p_mid_y: u16,
        children: &[(u16, u16, Style)],
        default_bg: Color,
    ) {
        if children.is_empty() {
            return;
        }
        let px = (p_right_x + 1) as usize;
        let py = p_mid_y as usize;
        let trunk_x = (p_right_x + 2) as usize;

        // Horizontal stub out of parent box.
        let parent_style = children
            .iter()
            .map(|(_, _, s)| *s)
            .max_by_key(|s| s.add_modifier.contains(Modifier::BOLD))
            .unwrap_or_else(|| Style::default().fg(theme::BORDER))
            .bg(default_bg);

        if children.len() == 1 && children[0].1 == p_mid_y {
            let c_left = children[0].0 as usize;
            let style = children[0].2.bg(default_bg);
            for cx in px..c_left {
                self.add_dir(cx, py, DIR_LEFT | DIR_RIGHT, style, false);
            }
            return;
        }

        self.add_dir(px, py, DIR_LEFT | DIR_RIGHT, parent_style, false);
        self.add_dir(trunk_x, py, DIR_LEFT, parent_style, false);

        for &(c_left_x, c_mid_y, c_style) in children {
            let cy = c_mid_y as usize;
            let cx_end = c_left_x as usize;
            let style = c_style.bg(default_bg);
            if cy == py {
                self.add_dir(trunk_x, py, DIR_LEFT | DIR_RIGHT, style, false);
            } else if cy < py {
                self.add_dir(trunk_x, py, DIR_UP, style, false);
                self.add_dir(trunk_x, cy, DIR_DOWN | DIR_RIGHT, style, true);
                for vy in (cy + 1)..py {
                    self.add_dir(trunk_x, vy, DIR_UP | DIR_DOWN, style, false);
                }
            } else {
                self.add_dir(trunk_x, py, DIR_DOWN, style, false);
                self.add_dir(trunk_x, cy, DIR_UP | DIR_RIGHT, style, true);
                for vy in (py + 1)..cy {
                    self.add_dir(trunk_x, vy, DIR_UP | DIR_DOWN, style, false);
                }
            }
            for hx in (trunk_x + 1)..cx_end {
                self.add_dir(hx, cy, DIR_LEFT | DIR_RIGHT, style, false);
            }
        }
    }

    /// Extract a viewport slice with CJK wide-character edge clipping and explicit background fills.
    pub fn to_ratatui_lines(
        &self,
        scroll_x: u16,
        scroll_y: u16,
        vp_w: u16,
        vp_h: u16,
        default_bg: Color,
    ) -> Vec<Line<'static>> {
        let sx = scroll_x as usize;
        let sy = scroll_y as usize;
        let w = vp_w as usize;
        let h = vp_h as usize;
        let bg_fill = Style::default().fg(theme::FG).bg(default_bg);
        let mut lines = Vec::with_capacity(h);

        for vy in 0..h {
            let cy = sy + vy;
            if cy >= self.height {
                lines.push(Line::from(Span::styled(" ".repeat(w), bg_fill)));
                continue;
            }
            let row_base = cy * self.width;
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut cur_style: Option<Style> = None;
            let mut cur_text = String::new();

            let push_cell = |spans: &mut Vec<Span<'static>>,
                             cur_style: &mut Option<Style>,
                             cur_text: &mut String,
                             text: &str,
                             style: Style| {
                if let Some(prev) = *cur_style {
                    if prev == style {
                        cur_text.push_str(text);
                        return;
                    }
                    spans.push(Span::styled(std::mem::take(cur_text), prev));
                }
                *cur_style = Some(style);
                cur_text.push_str(text);
            };

            let end_x = sx + w;
            let mut cx = sx;
            while cx < end_x {
                if cx >= self.width {
                    let pad = end_x - cx;
                    push_cell(
                        &mut spans,
                        &mut cur_style,
                        &mut cur_text,
                        &" ".repeat(pad),
                        bg_fill,
                    );
                    break;
                }
                let idx = row_base + cx;
                let cell = &self.chars[idx];
                let mut cell_style = self.styles[idx];
                if cell_style.bg.is_none() {
                    cell_style = cell_style.bg(default_bg);
                }

                if cell.is_empty() {
                    // Split wide char entering at the left viewport boundary: paint space.
                    push_cell(&mut spans, &mut cur_style, &mut cur_text, " ", cell_style);
                    cx += 1;
                    continue;
                }
                let cell_w = cell.width();
                if cell_w == 2 && cx + 2 > end_x {
                    // Wide char does not fit at the right viewport boundary: replace with space.
                    push_cell(&mut spans, &mut cur_style, &mut cur_text, " ", bg_fill);
                    cx += 1;
                    continue;
                }
                push_cell(&mut spans, &mut cur_style, &mut cur_text, cell, cell_style);
                cx += cell_w.max(1);
            }

            if let Some(style) = cur_style.take()
                && !cur_text.is_empty()
            {
                spans.push(Span::styled(cur_text, style));
            }
            lines.push(Line::from(spans));
        }

        lines
    }
}

/// Convert 4-bit directional mask into a crisp Unicode box-drawing glyph.
pub fn junction_char(mask: u8, rounded: bool) -> char {
    match mask {
        0 => ' ',
        15 => '┼', // L | R | U | D
        7 => '┴',  // L | R | U
        11 => '┬', // L | R | D
        14 => '├', // U | D | R
        13 => '┤', // U | D | L
        3 => '─',  // L | R
        12 => '│', // U | D
        10 => {
            if rounded {
                '╭'
            } else {
                '┌'
            }
        } // R | D
        9 => {
            if rounded {
                '╮'
            } else {
                '┐'
            }
        } // L | D
        6 => {
            if rounded {
                '╰'
            } else {
                '└'
            }
        } // R | U
        5 => {
            if rounded {
                '╯'
            } else {
                '┘'
            }
        } // L | U
        1 | 2 => '─',
        _ => '│',
    }
}

/// Truncate a string cleanly by terminal column width, adding `"…"` when shortened.
pub fn truncate_display(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    let budget = max_width - 1;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// UI state for the Test Points 2D MindMap canvas and Inspector tab.
#[derive(Debug, Clone)]
pub struct TestPointsUiState {
    pub focus: TestPointsFocus,
    pub fullscreen_canvas: bool,
    pub selected_node_id: Option<String>,
    pub hovered_node_id: Option<String>,
    pub selected_test_point_id: Option<String>,
    pub detail_field: DetailField,
    pub editing: bool,
    pub field_buffer: String,
    pub field_cursor: usize,
    pub field_dirty: bool,
    pub inline_edit: Option<InlineNodeEdit>,
    pub review_filter: ReviewFilter,
    pub selected_excerpt_index: usize,
    pub selected_scenario_ref_index: usize,
    pub scroll_offsets: [usize; 3],
    pub exec_statuses: HashMap<String, TestPointExecStatus>,
    pub collapsed_dirs: HashSet<String>,
    pub placed_nodes: Vec<PlacedTpNode>,
    pub canvas_width: u16,
    pub canvas_height: u16,
    pub canvas_scroll_x: u16,
    pub canvas_scroll_y: u16,
    pub canvas_viewport: (u16, u16),
    pub canvas_drag: Option<(u16, u16, u16, u16)>,
    pub last_click: Option<(String, Instant)>,
    undo_stack: Vec<Vec<TestPoint>>,
    redo_stack: Vec<Vec<TestPoint>>,
}

impl Default for TestPointsUiState {
    fn default() -> Self {
        Self::empty()
    }
}

impl TestPointsUiState {
    pub fn empty() -> Self {
        Self {
            focus: TestPointsFocus::Tree,
            fullscreen_canvas: false,
            selected_node_id: None,
            hovered_node_id: None,
            selected_test_point_id: None,
            detail_field: DetailField::Title,
            editing: false,
            field_buffer: String::new(),
            field_cursor: 0,
            field_dirty: false,
            inline_edit: None,
            review_filter: StatusFilter::All,
            selected_excerpt_index: 0,
            selected_scenario_ref_index: 0,
            scroll_offsets: [0; 3],
            exec_statuses: HashMap::new(),
            collapsed_dirs: HashSet::new(),
            placed_nodes: Vec::new(),
            canvas_width: 40,
            canvas_height: 12,
            canvas_scroll_x: 0,
            canvas_scroll_y: 0,
            canvas_viewport: (80, 24),
            canvas_drag: None,
            last_click: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn from_artifacts(artifacts: Option<&AuthoringArtifacts>) -> Self {
        let mut state = Self::empty();
        state.rebuild_tree(artifacts);
        state
    }

    /// Recompute live test-point execution status from linked feature scenarios and runner results.
    pub fn compute_exec_statuses(
        &mut self,
        project: &BddProject,
        case_status: &HashMap<(usize, usize), RunStatus>,
        runner_active: bool,
        artifacts: Option<&AuthoringArtifacts>,
    ) {
        let mut linked_cases: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (fi, feature) in project.features.iter().enumerate() {
            for si in 0..feature.scenario_count() {
                if let Some(sc) = feature.scenario_at(si) {
                    for tp_id in parse_teshi_tp_tags(&sc.tags) {
                        linked_cases.entry(tp_id).or_default().push((fi, si));
                    }
                }
            }
        }

        self.exec_statuses.clear();
        let Some(artifacts) = artifacts else {
            return;
        };
        for tp in &artifacts.test_points.test_points {
            let cases = linked_cases.get(&tp.id);
            let has_refs = !tp.scenario_refs.is_empty();
            let status = match cases {
                None if !has_refs => TestPointExecStatus::Uncovered,
                None => TestPointExecStatus::Linked,
                Some(pairs) => {
                    let mut any_failed = false;
                    let mut any_running = false;
                    let mut passed_count = 0usize;
                    let mut counted = 0usize;
                    for pair in pairs {
                        if let Some(st) = case_status.get(pair) {
                            counted += 1;
                            match st {
                                RunStatus::Failed => any_failed = true,
                                RunStatus::Running => any_running = true,
                                RunStatus::Passed => passed_count += 1,
                                RunStatus::Idle | RunStatus::Skipped | RunStatus::Blocked => {}
                            }
                        } else if runner_active {
                            // Preserve Linked until this specific scenario transitions to Running.
                        }
                    }
                    if any_failed {
                        TestPointExecStatus::Failed
                    } else if any_running {
                        TestPointExecStatus::Running
                    } else if counted > 0 && passed_count == pairs.len() {
                        TestPointExecStatus::Passed
                    } else {
                        TestPointExecStatus::Linked
                    }
                }
            };
            self.exec_statuses.insert(tp.id.clone(), status);
        }
    }

    pub fn exec_status_for(&self, tp: &TestPoint) -> TestPointExecStatus {
        if let Some(&status) = self.exec_statuses.get(&tp.id) {
            return status;
        }
        if tp.scenario_refs.is_empty() {
            TestPointExecStatus::Uncovered
        } else {
            TestPointExecStatus::Linked
        }
    }

    pub fn cycle_filter(&mut self, artifacts: Option<&AuthoringArtifacts>) {
        self.review_filter = self.review_filter.next();
        self.rebuild_tree(artifacts);
    }

    pub fn rebuild_tree(&mut self, artifacts: Option<&AuthoringArtifacts>) {
        self.rebuild_canvas_layout(artifacts, "Test Points");
    }

    /// Build the 2D left-to-right orthogonal MindMap tree layout.
    pub fn rebuild_canvas_layout(
        &mut self,
        artifacts: Option<&AuthoringArtifacts>,
        root_label: &str,
    ) {
        let all_points = artifacts
            .map(|a| a.test_points.test_points.as_slice())
            .unwrap_or(&[]);
        let points = filtered_test_points(all_points, &self.exec_statuses, self.review_filter);

        let mut tree: HashMap<String, LayoutDraft> = HashMap::new();
        let root_id = "root".to_string();
        tree.insert(
            root_id.clone(),
            LayoutDraft {
                id: root_id.clone(),
                kind: TpNodeKind::Root,
                label: root_label.to_string(),
                note: String::new(),
                parent_id: None,
                children_ids: Vec::new(),
                exec_status: TestPointExecStatus::Uncovered,
                scenario_count: 0,
                req_count: 0,
                collapsed_hidden_count: 0,
            },
        );

        let root_collapsed = self.collapsed_dirs.contains(&root_id);
        let mut hidden_under_root = 0usize;

        for tp in &points {
            let status = self.exec_status_for(tp);
            let leaf_id = format!("tp:{}", tp.id);
            let leaf_note = tp.concise_note();
            let scenario_count = tp.scenario_refs.len();
            let req_count = tp.requirement_links.len();

            if root_collapsed {
                hidden_under_root += 1;
                continue;
            }

            let mut parent_id = root_id.clone();
            let segments = tp.hierarchy_path.as_slice();
            let mut collapsed_ancestor: Option<String> = None;

            for depth in 0..segments.len() {
                let prefix = segments[..=depth].to_vec();
                let dir_id = format!("dir:{}", prefix.join("/"));
                if !tree.contains_key(&dir_id) {
                    tree.insert(
                        dir_id.clone(),
                        LayoutDraft {
                            id: dir_id.clone(),
                            kind: TpNodeKind::Branch {
                                path: prefix.clone(),
                            },
                            label: segments[depth].clone(),
                            note: String::new(),
                            parent_id: Some(parent_id.clone()),
                            children_ids: Vec::new(),
                            exec_status: TestPointExecStatus::Uncovered,
                            scenario_count: 0,
                            req_count: 0,
                            collapsed_hidden_count: 0,
                        },
                    );
                    if let Some(p) = tree.get_mut(&parent_id) {
                        p.children_ids.push(dir_id.clone());
                    }
                }
                if collapsed_ancestor.is_none() && self.collapsed_dirs.contains(&dir_id) {
                    collapsed_ancestor = Some(dir_id.clone());
                }
                if collapsed_ancestor.is_some() {
                    // Keep branch counts updated on the collapsed ancestor only once per leaf below.
                    break;
                }
                parent_id = dir_id;
            }

            if let Some(col_id) = collapsed_ancestor {
                if let Some(col_node) = tree.get_mut(&col_id) {
                    col_node.collapsed_hidden_count += 1;
                    col_node.scenario_count += scenario_count;
                    col_node.req_count += req_count;
                    col_node.exec_status = merge_status(col_node.exec_status, status);
                }
                continue;
            }

            tree.insert(
                leaf_id.clone(),
                LayoutDraft {
                    id: leaf_id.clone(),
                    kind: TpNodeKind::Leaf {
                        tp_id: tp.id.clone(),
                    },
                    label: tp.title.clone(),
                    note: leaf_note,
                    parent_id: Some(parent_id.clone()),
                    children_ids: Vec::new(),
                    exec_status: status,
                    scenario_count,
                    req_count,
                    collapsed_hidden_count: 0,
                },
            );
            if let Some(p) = tree.get_mut(&parent_id) {
                p.children_ids.push(leaf_id);
            }
        }

        if root_collapsed && let Some(root) = tree.get_mut(&root_id) {
            root.collapsed_hidden_count = hidden_under_root;
        }

        aggregate_branch_metrics(&root_id, &mut tree);

        // Compute subtree heights & assign depth, y positions bottom-up / top-down.
        let mut subtree_heights: HashMap<String, u16> = HashMap::new();
        compute_subtree_heights(&root_id, &tree, &mut subtree_heights);

        let mut depth_widths: HashMap<usize, u16> = HashMap::new();
        let mut widths: HashMap<String, u16> = HashMap::new();
        assign_node_widths(&root_id, 0, &tree, &mut widths, &mut depth_widths);

        let max_depth = depth_widths.keys().copied().max().unwrap_or(0);
        let mut depth_x: Vec<u16> = vec![CANVAS_PAD_X; max_depth + 1];
        for d in 1..=max_depth {
            let prev_w = depth_widths
                .get(&(d - 1))
                .copied()
                .unwrap_or(MIN_NODE_WIDTH);
            depth_x[d] = depth_x[d - 1] + prev_w + GAP_X;
        }

        let mut placed: Vec<PlacedTpNode> = Vec::new();
        place_subtree(
            &root_id,
            0,
            CANVAS_PAD_Y,
            &tree,
            &subtree_heights,
            &widths,
            &depth_x,
            &mut placed,
        );

        let max_right = placed
            .iter()
            .map(|n| n.x + n.width + CANVAS_PAD_X)
            .max()
            .unwrap_or(40);
        let max_bottom = placed
            .iter()
            .map(|n| n.y + n.height + CANVAS_PAD_Y)
            .max()
            .unwrap_or(12);

        self.placed_nodes = placed;
        self.canvas_width = max_right;
        self.canvas_height = max_bottom;

        // Maintain valid selection.
        let valid_selected = self
            .selected_node_id
            .as_ref()
            .is_some_and(|sid| self.placed_nodes.iter().any(|n| &n.id == sid));
        if !valid_selected {
            let first_leaf = self
                .placed_nodes
                .iter()
                .find(|n| matches!(n.kind, TpNodeKind::Leaf { .. }))
                .map(|n| n.id.clone());
            self.selected_node_id = first_leaf.or(Some(root_id));
        }

        self.sync_selected_test_point_id();
        self.recompute_highlights();
        self.load_selected_field_buffer(artifacts);
    }

    fn sync_selected_test_point_id(&mut self) {
        let new_tp_id = self
            .selected_node_id
            .as_ref()
            .and_then(|nid| self.placed_nodes.iter().find(|n| &n.id == nid))
            .and_then(|node| match &node.kind {
                TpNodeKind::Leaf { tp_id } => Some(tp_id.clone()),
                _ => None,
            });
        if new_tp_id != self.selected_test_point_id {
            self.selected_test_point_id = new_tp_id;
            self.selected_excerpt_index = 0;
            self.selected_scenario_ref_index = 0;
            self.scroll_offsets[1] = 0;
            self.scroll_offsets[2] = 0;
        }
    }

    /// Recompute `HighlightCategory` for every node relative to `selected_node_id`.
    pub fn recompute_highlights(&mut self) {
        let Some(selected_id) = self.selected_node_id.clone() else {
            return;
        };
        let parent_map: HashMap<String, Option<String>> = self
            .placed_nodes
            .iter()
            .map(|n| (n.id.clone(), n.parent_id.clone()))
            .collect();
        let selected_parent = parent_map.get(&selected_id).cloned().flatten();

        let mut ancestors: HashSet<String> = HashSet::new();
        let mut cursor = parent_map.get(&selected_id).cloned().flatten();
        while let Some(pid) = cursor {
            ancestors.insert(pid.clone());
            cursor = parent_map.get(&pid).cloned().flatten();
        }

        let mut descendants: HashSet<String> = HashSet::new();
        let mut stack = vec![selected_id.clone()];
        let children_map: HashMap<String, Vec<String>> = self
            .placed_nodes
            .iter()
            .map(|n| (n.id.clone(), n.children_ids.clone()))
            .collect();
        while let Some(id) = stack.pop() {
            if let Some(ch) = children_map.get(&id) {
                for cid in ch {
                    if descendants.insert(cid.clone()) {
                        stack.push(cid.clone());
                    }
                }
            }
        }

        for node in &mut self.placed_nodes {
            node.highlight = if node.id == selected_id {
                HighlightCategory::Selected
            } else if ancestors.contains(&node.id) {
                HighlightCategory::Ancestor
            } else if descendants.contains(&node.id) {
                HighlightCategory::Descendant
            } else if selected_parent.is_some() && node.parent_id == selected_parent {
                HighlightCategory::Sibling
            } else {
                HighlightCategory::GrayedOut
            };
        }
    }

    pub fn selected_node(&self) -> Option<&PlacedTpNode> {
        let id = self.selected_node_id.as_ref()?;
        self.placed_nodes.iter().find(|n| &n.id == id)
    }

    pub fn selected_test_point<'a>(
        &self,
        artifacts: Option<&'a AuthoringArtifacts>,
    ) -> Option<&'a TestPoint> {
        let id = self.selected_test_point_id.as_ref()?;
        artifacts?
            .test_points
            .test_points
            .iter()
            .find(|tp| &tp.id == id)
    }

    pub fn select_node_by_id(
        &mut self,
        node_id: &str,
        artifacts: Option<&AuthoringArtifacts>,
    ) -> bool {
        if self.placed_nodes.iter().any(|n| n.id == node_id) {
            self.selected_node_id = Some(node_id.to_string());
            self.sync_selected_test_point_id();
            self.recompute_highlights();
            self.load_selected_field_buffer(artifacts);
            self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
            true
        } else {
            false
        }
    }

    pub fn node_at_canvas(&self, cx: u16, cy: u16) -> Option<&PlacedTpNode> {
        self.placed_nodes
            .iter()
            .find(|n| cx >= n.x && cx < n.x + n.width && cy >= n.y && cy < n.y + n.height)
    }

    pub fn toggle_collapse_selected(&mut self, artifacts: Option<&AuthoringArtifacts>) -> bool {
        let Some(node) = self.selected_node().cloned() else {
            return false;
        };
        let key = match node.kind {
            TpNodeKind::Root | TpNodeKind::Branch { .. } => node.id.clone(),
            TpNodeKind::Leaf { .. } => {
                if let Some(pid) = node.parent_id {
                    pid
                } else {
                    return false;
                }
            }
        };
        if !self.collapsed_dirs.remove(&key) {
            self.collapsed_dirs.insert(key.clone());
            self.selected_node_id = Some(key);
        }
        self.rebuild_tree(artifacts);
        self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
        true
    }

    /// Navigate spatially in the 2D MindMap canvas (`Left/Right/Up/Down`).
    pub fn navigate_spatial(
        &mut self,
        dir: SpatialDir,
        artifacts: Option<&AuthoringArtifacts>,
    ) -> bool {
        let Some(cur) = self.selected_node().cloned() else {
            return false;
        };
        let cur_mid_y = cur.mid_y();
        let target: Option<String> = match dir {
            SpatialDir::Left => cur.parent_id.clone(),
            SpatialDir::Right => {
                if cur.children_ids.is_empty() {
                    None
                } else {
                    cur.children_ids
                        .iter()
                        .filter_map(|cid| self.placed_nodes.iter().find(|n| &n.id == cid))
                        .min_by_key(|n| n.mid_y().abs_diff(cur_mid_y))
                        .map(|n| n.id.clone())
                }
            }
            SpatialDir::Up | SpatialDir::Down => {
                // First walk within the same depth column sorted vertically so movement feels natural.
                let mut same_col: Vec<&PlacedTpNode> = self
                    .placed_nodes
                    .iter()
                    .filter(|n| n.depth == cur.depth)
                    .collect();
                same_col.sort_by_key(|n| n.y);
                let pos = same_col.iter().position(|n| n.id == cur.id).unwrap_or(0);
                match dir {
                    SpatialDir::Up if pos > 0 => Some(same_col[pos - 1].id.clone()),
                    SpatialDir::Down if pos + 1 < same_col.len() => {
                        Some(same_col[pos + 1].id.clone())
                    }
                    _ => None,
                }
            }
        };

        if let Some(next_id) = target {
            self.selected_node_id = Some(next_id);
            self.sync_selected_test_point_id();
            self.recompute_highlights();
            self.load_selected_field_buffer(artifacts);
            self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
            true
        } else {
            false
        }
    }

    pub fn ensure_selected_visible(&mut self, vp_w: u16, vp_h: u16) {
        if vp_w == 0 || vp_h == 0 {
            return;
        }
        let Some(node) = self.selected_node().cloned() else {
            return;
        };
        let margin_x = 2u16.min(vp_w / 4);
        let margin_y = 1u16.min(vp_h / 4);

        if node.x < self.canvas_scroll_x + margin_x {
            self.canvas_scroll_x = node.x.saturating_sub(margin_x);
        } else if node.x + node.width + margin_x > self.canvas_scroll_x + vp_w {
            self.canvas_scroll_x = (node.x + node.width + margin_x).saturating_sub(vp_w);
        }

        if node.y < self.canvas_scroll_y + margin_y {
            self.canvas_scroll_y = node.y.saturating_sub(margin_y);
        } else if node.y + node.height + margin_y > self.canvas_scroll_y + vp_h {
            self.canvas_scroll_y = (node.y + node.height + margin_y).saturating_sub(vp_h);
        }

        let max_sx = self.canvas_width.saturating_sub(vp_w);
        let max_sy = self.canvas_height.saturating_sub(vp_h);
        self.canvas_scroll_x = self.canvas_scroll_x.min(max_sx);
        self.canvas_scroll_y = self.canvas_scroll_y.min(max_sy);
    }

    pub fn pan_canvas(&mut self, dx: i32, dy: i32) {
        let (vp_w, vp_h) = self.canvas_viewport;
        let max_sx = self.canvas_width.saturating_sub(vp_w) as i32;
        let max_sy = self.canvas_height.saturating_sub(vp_h) as i32;
        self.canvas_scroll_x = (self.canvas_scroll_x as i32 + dx).clamp(0, max_sx) as u16;
        self.canvas_scroll_y = (self.canvas_scroll_y as i32 + dy).clamp(0, max_sy) as u16;
    }

    /// Render the current layout and connectors onto a `TpCanvas`.
    pub fn build_canvas(&self, default_bg: Color) -> TpCanvas {
        let mut canvas = TpCanvas::new(
            self.canvas_width as usize,
            self.canvas_height as usize,
            default_bg,
        );

        let by_id: HashMap<&str, &PlacedTpNode> = self
            .placed_nodes
            .iter()
            .map(|n| (n.id.as_str(), n))
            .collect();

        // Connectors first so boxes draw cleanly above stubs.
        for node in &self.placed_nodes {
            if node.children_ids.is_empty() {
                continue;
            }
            let children: Vec<(u16, u16, Style)> = node
                .children_ids
                .iter()
                .filter_map(|cid| by_id.get(cid.as_str()))
                .map(|child| {
                    let style = connector_style(node.highlight, child.highlight, child.exec_status);
                    (child.x, child.mid_y(), style)
                })
                .collect();
            canvas.draw_tree_connector(node.right_x(), node.mid_y(), &children, default_bg);
        }

        // Draw node boxes.
        for node in &self.placed_nodes {
            let is_selected = node.highlight == HighlightCategory::Selected;
            let is_hovered = self.hovered_node_id.as_deref() == Some(node.id.as_str());
            let fill_bg = if is_selected {
                theme::SELECTION_INACTIVE_BG
            } else if is_hovered {
                theme::HOVER_BG
            } else {
                default_bg
            };

            let border_style = node_border_style(node.highlight, node.exec_status);
            let title_style = node_title_style(node.highlight, &node.kind);
            let icon_style = if node.highlight == HighlightCategory::GrayedOut {
                Style::default().fg(theme::MUTED)
            } else {
                node.exec_status.style()
            };
            let badge_style = if node.highlight == HighlightCategory::GrayedOut {
                Style::default().fg(theme::MUTED)
            } else {
                Style::default().fg(theme::INFO)
            };

            let (icon, badge, rounded) = match &node.kind {
                TpNodeKind::Root => {
                    let b = if node.collapsed_hidden_count > 0 {
                        format!("[+{}]", node.collapsed_hidden_count)
                    } else {
                        format!("[{}]", node.req_count.max(node.children_ids.len()))
                    };
                    ("◆", b, true)
                }
                TpNodeKind::Branch { .. } => {
                    let ic = if self.collapsed_dirs.contains(&node.id) {
                        "▸"
                    } else {
                        "▾"
                    };
                    let b = if node.collapsed_hidden_count > 0 {
                        format!("[+{}]", node.collapsed_hidden_count)
                    } else if node.scenario_count > 0 {
                        format!("[{}]", node.scenario_count)
                    } else {
                        String::new()
                    };
                    (ic, b, false)
                }
                TpNodeKind::Leaf { .. } => {
                    let b = if node.scenario_count > 0 {
                        format!("[{}]", node.scenario_count)
                    } else {
                        String::new()
                    };
                    (node.exec_status.glyph(), b, true)
                }
            };

            let mut display_label = node.label.clone();
            if let Some(edit) = &self.inline_edit
                && edit.node_id == node.id
            {
                display_label = format!("{}▏", edit.buffer);
            }

            canvas.draw_node_box(
                node.x,
                node.y,
                node.width,
                node.height,
                icon,
                &display_label,
                &badge,
                rounded,
                border_style,
                icon_style,
                title_style,
                badge_style,
                fill_bg,
            );
        }

        canvas
    }

    pub fn push_undo(&mut self, artifacts: &AuthoringArtifacts) {
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack
            .push(artifacts.test_points.test_points.clone());
        self.redo_stack.clear();
    }

    /// Add a child TestPoint beneath the selected node and start inline editing (`a` / `Tab`).
    pub fn start_add_child(&mut self, artifacts: &mut Option<AuthoringArtifacts>) -> bool {
        let artifacts = artifacts.get_or_insert_with(AuthoringArtifacts::default);
        let target_path = self.target_child_hierarchy(artifacts);
        self.create_node_and_edit(artifacts, target_path)
    }

    /// Add a sibling TestPoint alongside the selected node and start inline editing (`n` / `O`).
    pub fn start_add_sibling(&mut self, artifacts: &mut Option<AuthoringArtifacts>) -> bool {
        let artifacts = artifacts.get_or_insert_with(AuthoringArtifacts::default);
        let target_path = self.target_sibling_hierarchy(artifacts);
        self.create_node_and_edit(artifacts, target_path)
    }

    fn target_child_hierarchy(&self, artifacts: &AuthoringArtifacts) -> HierarchyPath {
        match self.selected_node().map(|n| &n.kind) {
            Some(TpNodeKind::Branch { path }) => {
                let mut p = path.clone();
                if p.is_empty() {
                    p.push("General".into());
                }
                HierarchyPath::new(p)
            }
            Some(TpNodeKind::Leaf { tp_id }) => {
                if let Some(tp) = artifacts
                    .test_points
                    .test_points
                    .iter()
                    .find(|t| t.id == *tp_id)
                {
                    tp.hierarchy_path.clone()
                } else {
                    HierarchyPath::new(vec!["General".into()])
                }
            }
            Some(TpNodeKind::Root) | None => HierarchyPath::new(vec!["General".into()]),
        }
    }

    fn target_sibling_hierarchy(&self, artifacts: &AuthoringArtifacts) -> HierarchyPath {
        match self.selected_node().map(|n| &n.kind) {
            Some(TpNodeKind::Leaf { tp_id }) => artifacts
                .test_points
                .test_points
                .iter()
                .find(|t| t.id == *tp_id)
                .map(|t| t.hierarchy_path.clone())
                .unwrap_or_else(|| HierarchyPath::new(vec!["General".into()])),
            Some(TpNodeKind::Branch { path }) => {
                let mut parent = path.clone();
                parent.pop();
                parent.push("Module".into());
                HierarchyPath::new(parent)
            }
            Some(TpNodeKind::Root) | None => HierarchyPath::new(vec!["General".into()]),
        }
    }

    fn create_node_and_edit(
        &mut self,
        artifacts: &mut AuthoringArtifacts,
        hierarchy_path: HierarchyPath,
    ) -> bool {
        self.push_undo(artifacts);
        // Expand any ancestor dirs covering hierarchy_path.
        self.collapsed_dirs.remove("root");
        let segs = hierarchy_path.as_slice();
        for i in 0..segs.len() {
            self.collapsed_dirs
                .remove(&format!("dir:{}", segs[..=i].join("/")));
        }

        let id = next_test_point_id(&artifacts.test_points.test_points);
        let tp = TestPoint {
            id: id.clone(),
            title: "New test point".into(),
            objective: String::new(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path,
            review_state: ReviewState::Approved,
            requirement_links: Vec::new(),
            scenario_refs: Vec::new(),
        };
        artifacts.test_points.test_points.push(tp);
        let node_id = format!("tp:{id}");
        self.selected_node_id = Some(node_id.clone());
        self.rebuild_tree(Some(artifacts));
        self.inline_edit = Some(InlineNodeEdit {
            node_id,
            kind: TpNodeKind::Leaf { tp_id: id },
            buffer: String::new(),
            cursor: 0,
            is_new: true,
        });
        self.editing = true;
        self.focus = TestPointsFocus::Tree;
        self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
        true
    }

    /// Start inline box editing of the currently selected canvas node (`i` / `e` / `Enter`).
    pub fn start_inline_edit(&mut self, artifacts: Option<&AuthoringArtifacts>) -> bool {
        let Some(node) = self.selected_node().cloned() else {
            return false;
        };
        let (buffer, kind) = match node.kind {
            TpNodeKind::Root => return false,
            TpNodeKind::Branch { ref path } => {
                (path.last().cloned().unwrap_or_default(), node.kind.clone())
            }
            TpNodeKind::Leaf { ref tp_id } => {
                let title = artifacts
                    .and_then(|a| a.test_points.test_points.iter().find(|t| &t.id == tp_id))
                    .map(|t| t.title.clone())
                    .unwrap_or_else(|| node.label.clone());
                (title, node.kind.clone())
            }
        };
        let cursor = buffer.chars().count();
        self.inline_edit = Some(InlineNodeEdit {
            node_id: node.id,
            kind,
            buffer,
            cursor,
            is_new: false,
        });
        self.editing = true;
        true
    }

    /// Commit (or cancel if newly added & empty) the active inline edit.
    ///
    /// Returns `true` when `artifacts.test_points` changed and should be saved.
    pub fn commit_inline_edit(
        &mut self,
        artifacts: &mut AuthoringArtifacts,
        cancelled: bool,
    ) -> bool {
        let Some(edit) = self.inline_edit.take() else {
            self.editing = false;
            return false;
        };
        self.editing = false;
        let trimmed = edit.buffer.trim().to_string();

        if (cancelled || trimmed.is_empty()) && edit.is_new {
            if let TpNodeKind::Leaf { ref tp_id } = edit.kind {
                artifacts.test_points.test_points.retain(|t| &t.id != tp_id);
                // Discard the placeholder undo entry since creation was cancelled.
                self.undo_stack.pop();
            }
            self.rebuild_tree(Some(artifacts));
            return false;
        }
        if cancelled || trimmed.is_empty() {
            self.rebuild_tree(Some(artifacts));
            return false;
        }

        let mut changed = false;
        match edit.kind {
            TpNodeKind::Root => {}
            TpNodeKind::Branch { path } => {
                if !path.is_empty() && path.last().map(String::as_str) != Some(trimmed.as_str()) {
                    self.push_undo(artifacts);
                    let idx = path.len() - 1;
                    for tp in &mut artifacts.test_points.test_points {
                        let segs = tp.hierarchy_path.as_slice();
                        if segs.len() >= path.len() && segs[..path.len()] == path[..] {
                            let mut next = segs.to_vec();
                            next[idx] = trimmed.clone();
                            tp.hierarchy_path = HierarchyPath::new(next);
                        }
                    }
                    let mut new_path = path;
                    new_path[idx] = trimmed;
                    self.selected_node_id = Some(format!("dir:{}", new_path.join("/")));
                    changed = true;
                }
            }
            TpNodeKind::Leaf { tp_id } => {
                if !edit.is_new {
                    self.push_undo(artifacts);
                }
                if let Some(tp) = artifacts
                    .test_points
                    .test_points
                    .iter_mut()
                    .find(|t| t.id == tp_id)
                {
                    if trimmed.contains('/') {
                        let parts: Vec<String> = trimmed
                            .split('/')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        if parts.len() >= 2 {
                            let leaf_title = parts.last().cloned().unwrap_or(trimmed.clone());
                            let prefix_extra = &parts[..parts.len() - 1];
                            let mut segs = if tp.hierarchy_path.as_slice() == ["General"] {
                                Vec::new()
                            } else {
                                tp.hierarchy_path.as_slice().to_vec()
                            };
                            segs.extend(prefix_extra.iter().cloned());
                            if !segs.is_empty() {
                                tp.hierarchy_path = HierarchyPath::new(segs);
                            }
                            tp.title = leaf_title;
                        } else if let Some(only) = parts.into_iter().next() {
                            tp.title = only;
                        }
                    } else {
                        tp.title = trimmed;
                    }
                    changed = true;
                }
                self.selected_node_id = Some(format!("tp:{tp_id}"));
            }
        }

        self.rebuild_tree(Some(artifacts));
        self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
        changed
    }

    /// Delete the selected leaf or entire branch subtree (`d` / `Delete`).
    pub fn delete_selected(&mut self, artifacts: &mut AuthoringArtifacts) -> Option<String> {
        let node = self.selected_node()?.clone();
        match node.kind {
            TpNodeKind::Root => None,
            TpNodeKind::Leaf { tp_id } => {
                let title = node.label.clone();
                self.push_undo(artifacts);
                artifacts.test_points.test_points.retain(|t| t.id != tp_id);
                self.selected_node_id = node.parent_id.clone();
                self.rebuild_tree(Some(artifacts));
                self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
                Some(title)
            }
            TpNodeKind::Branch { path } => {
                let before = artifacts.test_points.test_points.len();
                self.push_undo(artifacts);
                artifacts.test_points.test_points.retain(|t| {
                    let segs = t.hierarchy_path.as_slice();
                    !(segs.len() >= path.len() && segs[..path.len()] == path[..])
                });
                if artifacts.test_points.test_points.len() == before {
                    self.undo_stack.pop();
                    return None;
                }
                self.selected_node_id = node.parent_id.clone();
                self.rebuild_tree(Some(artifacts));
                self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
                Some(node.label)
            }
        }
    }

    pub fn undo(&mut self, artifacts: &mut AuthoringArtifacts) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        if self.redo_stack.len() >= MAX_UNDO_HISTORY {
            self.redo_stack.remove(0);
        }
        self.redo_stack
            .push(artifacts.test_points.test_points.clone());
        artifacts.test_points.test_points = prev;
        self.rebuild_tree(Some(artifacts));
        self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
        true
    }

    pub fn redo(&mut self, artifacts: &mut AuthoringArtifacts) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack
            .push(artifacts.test_points.test_points.clone());
        artifacts.test_points.test_points = next;
        self.rebuild_tree(Some(artifacts));
        self.ensure_selected_visible(self.canvas_viewport.0, self.canvas_viewport.1);
        true
    }

    /// Return all `TestPoint` indices (into `artifacts.test_points.test_points`) under `selected`.
    pub fn selected_test_point_ids(&self, artifacts: &AuthoringArtifacts) -> Vec<String> {
        let Some(node) = self.selected_node() else {
            return Vec::new();
        };
        match &node.kind {
            TpNodeKind::Leaf { tp_id } => vec![tp_id.clone()],
            TpNodeKind::Branch { path } => artifacts
                .test_points
                .test_points
                .iter()
                .filter(|t| {
                    let segs = t.hierarchy_path.as_slice();
                    segs.len() >= path.len() && segs[..path.len()] == path[..]
                })
                .map(|t| t.id.clone())
                .collect(),
            TpNodeKind::Root => artifacts
                .test_points
                .test_points
                .iter()
                .map(|t| t.id.clone())
                .collect(),
        }
    }

    pub fn load_selected_field_buffer(&mut self, artifacts: Option<&AuthoringArtifacts>) {
        if self.inline_edit.is_some()
            || self.field_dirty
            || (self.focus == TestPointsFocus::Details && self.editing)
        {
            return;
        }
        match self.selected_test_point(artifacts) {
            Some(tp) => self.load_field_buffer_from(tp),
            None => self.load_field_buffer(),
        }
    }

    pub fn load_field_buffer(&mut self) {
        self.field_buffer.clear();
        self.field_cursor = 0;
        self.field_dirty = false;
    }

    pub fn load_field_buffer_from(&mut self, tp: &TestPoint) {
        let value = match self.detail_field {
            DetailField::Title => tp.title.clone(),
            DetailField::Objective => tp.objective.clone(),
            DetailField::Preconditions => tp.preconditions.clone().unwrap_or_default(),
            DetailField::ExpectedOutcomes => tp.expected_outcomes.clone().unwrap_or_default(),
            DetailField::Hierarchy => tp.hierarchy_path.as_slice().join(" / "),
        };
        self.field_cursor = value.chars().count();
        self.field_buffer = value;
        self.field_dirty = false;
    }

    pub fn move_detail_field(&mut self, delta: isize, artifacts: Option<&AuthoringArtifacts>) {
        self.detail_field = if delta < 0 {
            self.detail_field.prev()
        } else {
            self.detail_field.next()
        };
        self.load_selected_field_buffer(artifacts);
    }

    pub fn move_excerpt_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.selected_excerpt_index = 0;
            return;
        }
        let current = self.selected_excerpt_index as isize;
        self.selected_excerpt_index =
            (current + delta).clamp(0, count.saturating_sub(1) as isize) as usize;
    }

    fn active_buffer_mut(&mut self) -> (&mut String, &mut usize, Option<&mut bool>) {
        if let Some(edit) = &mut self.inline_edit {
            (&mut edit.buffer, &mut edit.cursor, None)
        } else {
            (
                &mut self.field_buffer,
                &mut self.field_cursor,
                Some(&mut self.field_dirty),
            )
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        let (buf, cur, dirty) = self.active_buffer_mut();
        let byte = char_to_byte(buf, *cur);
        buf.insert(byte, ch);
        *cur += 1;
        if let Some(d) = dirty {
            *d = true;
        }
    }

    pub fn backspace(&mut self) {
        let (buf, cur, dirty) = self.active_buffer_mut();
        if *cur == 0 {
            return;
        }
        let end = char_to_byte(buf, *cur);
        let start = char_to_byte(buf, *cur - 1);
        buf.replace_range(start..end, "");
        *cur -= 1;
        if let Some(d) = dirty {
            *d = true;
        }
    }

    pub fn delete_forward(&mut self) {
        let (buf, cur, dirty) = self.active_buffer_mut();
        let len = buf.chars().count();
        if *cur >= len {
            return;
        }
        let start = char_to_byte(buf, *cur);
        let end = char_to_byte(buf, *cur + 1);
        buf.replace_range(start..end, "");
        if let Some(d) = dirty {
            *d = true;
        }
    }

    pub fn move_field_cursor(&mut self, delta: isize) {
        let (buf, cur, _) = self.active_buffer_mut();
        let len = buf.chars().count() as isize;
        *cur = (*cur as isize + delta).clamp(0, len) as usize;
    }

    pub fn field_line_boundary(&mut self, end: bool) {
        let (buf, cur, _) = self.active_buffer_mut();
        *cur = if end { buf.chars().count() } else { 0 };
    }

    pub fn field_line_vertical(&mut self, down: bool) {
        let (buf, cur, _) = self.active_buffer_mut();
        let lines: Vec<&str> = buf.split('\n').collect();
        if lines.len() <= 1 {
            return;
        }
        let mut remaining = *cur;
        let mut row = 0usize;
        let mut col = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let count = line.chars().count();
            if remaining <= count {
                row = idx;
                col = remaining;
                break;
            }
            remaining -= count + 1;
        }
        let target_row = if down {
            (row + 1).min(lines.len() - 1)
        } else {
            row.saturating_sub(1)
        };
        let target_col = col.min(lines[target_row].chars().count());
        let mut offset = 0usize;
        for line in &lines[..target_row] {
            offset += line.chars().count() + 1;
        }
        *cur = offset + target_col;
    }

    /// Apply an Inspector field edit to `tp`.
    pub fn apply_field_buffer(tp: &mut TestPoint, field: DetailField, value: &str) {
        match field {
            DetailField::Title => {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    tp.title = trimmed.to_string();
                }
            }
            DetailField::Objective => {
                tp.objective = value.trim().to_string();
            }
            DetailField::Preconditions => {
                let trimmed = value.trim();
                tp.preconditions = (!trimmed.is_empty()).then(|| trimmed.to_string());
            }
            DetailField::ExpectedOutcomes => {
                let trimmed = value.trim();
                tp.expected_outcomes = (!trimmed.is_empty()).then(|| trimmed.to_string());
            }
            DetailField::Hierarchy => {
                let segments: Vec<String> = value
                    .split('/')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !segments.is_empty() {
                    tp.hierarchy_path = HierarchyPath::new(segments);
                }
            }
        }
    }

    pub fn save_test_points(project_root: &Path, artifacts: &AuthoringArtifacts) -> Result<()> {
        teshi_engine::save_test_points(project_root, &artifacts.test_points)
    }

    pub fn focus_next_column(&mut self) {
        self.focus = self.focus.next();
    }

    pub fn focus_prev_column(&mut self) {
        self.focus = self.focus.prev();
    }

    pub fn page_canvas(&mut self, down: bool) {
        let step = self.canvas_viewport.1.saturating_sub(2).max(4) as i32;
        self.pan_canvas(0, if down { step } else { -step });
    }

    pub fn move_canvas_boundary(&mut self, end: bool) {
        if end {
            let max_y = self.canvas_height.saturating_sub(self.canvas_viewport.1);
            self.canvas_scroll_y = max_y;
        } else {
            self.canvas_scroll_x = 0;
            self.canvas_scroll_y = 0;
        }
    }

    pub fn scroll_pane(&mut self, pane: usize, delta: isize) {
        if pane == 0 {
            self.pan_canvas(0, delta as i32);
            return;
        }
        let idx = pane.min(2);
        let cur = self.scroll_offsets[idx] as isize;
        self.scroll_offsets[idx] = (cur + delta).max(0) as usize;
    }

    pub fn commit_field_if_dirty(&mut self) {
        self.field_dirty = false;
    }
}

/// Extract display excerpts for each requirement link attached to `tp`.
pub fn excerpts_for_test_point(
    tp: &TestPoint,
    artifacts: &AuthoringArtifacts,
) -> Vec<RequirementExcerpt> {
    tp.requirement_links
        .iter()
        .map(|link| {
            let doc = artifacts
                .documents
                .iter()
                .find(|d| d.meta.id == link.document_id);
            let title = doc
                .map(|d| d.meta.title.clone())
                .unwrap_or_else(|| link.document_id.clone());
            let is_doc_level = !link.position.is_non_empty();
            let quote = if !link.quote.quote.trim().is_empty() {
                link.quote.quote.clone()
            } else if let Some(d) = doc {
                if is_doc_level {
                    d.body
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .trim()
                        .to_string()
                } else {
                    slice_by_char_range(&d.body, link.position).unwrap_or_default()
                }
            } else {
                String::new()
            };
            RequirementExcerpt {
                document_id: link.document_id.clone(),
                document_title: title,
                revision: link.document_revision.clone(),
                resolution: link.resolution,
                position: link.position,
                resolved: link.resolution == ResolutionState::Resolved,
                quote,
            }
        })
        .collect()
}

fn filtered_test_points<'a>(
    test_points: &'a [TestPoint],
    exec_statuses: &HashMap<String, TestPointExecStatus>,
    filter: StatusFilter,
) -> Vec<&'a TestPoint> {
    test_points
        .iter()
        .filter(|tp| {
            if tp.review_state == ReviewState::Rejected {
                return false;
            }
            let st = exec_statuses
                .get(&tp.id)
                .copied()
                .unwrap_or(if tp.scenario_refs.is_empty() {
                    TestPointExecStatus::Uncovered
                } else {
                    TestPointExecStatus::Linked
                });
            match filter {
                StatusFilter::All => true,
                StatusFilter::Uncovered => st == TestPointExecStatus::Uncovered,
                StatusFilter::Linked => st == TestPointExecStatus::Linked,
                StatusFilter::Passed => st == TestPointExecStatus::Passed,
                StatusFilter::Failed => st == TestPointExecStatus::Failed,
            }
        })
        .collect()
}

fn merge_status(a: TestPointExecStatus, b: TestPointExecStatus) -> TestPointExecStatus {
    use TestPointExecStatus as S;
    match (a, b) {
        (S::Running, _) | (_, S::Running) => S::Running,
        (S::Failed, _) | (_, S::Failed) => S::Failed,
        (S::Passed, S::Passed) => S::Passed,
        (S::Uncovered, other) | (other, S::Uncovered) => other,
        _ => S::Linked,
    }
}

#[derive(Clone)]
struct LayoutDraft {
    id: String,
    kind: TpNodeKind,
    label: String,
    note: String,
    parent_id: Option<String>,
    children_ids: Vec<String>,
    exec_status: TestPointExecStatus,
    scenario_count: usize,
    req_count: usize,
    collapsed_hidden_count: usize,
}

fn aggregate_branch_metrics(node_id: &str, tree: &mut HashMap<String, LayoutDraft>) {
    let children = tree
        .get(node_id)
        .map(|n| n.children_ids.clone())
        .unwrap_or_default();
    if children.is_empty() {
        return;
    }
    for cid in &children {
        aggregate_branch_metrics(cid, tree);
    }

    let mut total_scenarios = 0usize;
    let mut total_reqs = 0usize;
    let mut agg_status: Option<TestPointExecStatus> = None;
    for cid in &children {
        if let Some(child) = tree.get(cid) {
            total_scenarios += child.scenario_count;
            total_reqs += child.req_count;
            agg_status = Some(match agg_status {
                None => child.exec_status,
                Some(prev) => {
                    use TestPointExecStatus as S;
                    match (prev, child.exec_status) {
                        (S::Running, _) | (_, S::Running) => S::Running,
                        (S::Failed, _) | (_, S::Failed) => S::Failed,
                        (S::Passed, S::Passed) => S::Passed,
                        (S::Uncovered, S::Uncovered) => S::Uncovered,
                        _ => S::Linked,
                    }
                }
            });
        }
    }
    if let Some(node) = tree.get_mut(node_id) {
        node.scenario_count += total_scenarios;
        node.req_count += total_reqs;
        if let Some(st) = agg_status {
            node.exec_status = st;
        }
    }
}

fn compute_subtree_heights(
    node_id: &str,
    tree: &HashMap<String, LayoutDraft>,
    out: &mut HashMap<String, u16>,
) -> u16 {
    let Some(node) = tree.get(node_id) else {
        return NODE_HEIGHT;
    };
    if node.children_ids.is_empty() {
        out.insert(node_id.to_string(), NODE_HEIGHT);
        return NODE_HEIGHT;
    }
    let mut sum = 0u16;
    for (idx, cid) in node.children_ids.iter().enumerate() {
        if idx > 0 {
            sum += GAP_Y;
        }
        sum += compute_subtree_heights(cid, tree, out);
    }
    let h = sum.max(NODE_HEIGHT);
    out.insert(node_id.to_string(), h);
    h
}

fn assign_node_widths(
    node_id: &str,
    depth: usize,
    tree: &HashMap<String, LayoutDraft>,
    widths: &mut HashMap<String, u16>,
    depth_widths: &mut HashMap<usize, u16>,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    let badge_w = match &node.kind {
        TpNodeKind::Root => 5usize,
        TpNodeKind::Branch { .. } => {
            if node.collapsed_hidden_count > 0 || node.scenario_count > 0 {
                5
            } else {
                0
            }
        }
        TpNodeKind::Leaf { .. } => {
            if node.scenario_count > 0 {
                5
            } else {
                0
            }
        }
    };
    let w = (node.label.width() + 6 + badge_w)
        .clamp(MIN_NODE_WIDTH as usize, MAX_NODE_WIDTH as usize) as u16;
    widths.insert(node_id.to_string(), w);
    depth_widths
        .entry(depth)
        .and_modify(|prev| *prev = (*prev).max(w))
        .or_insert(w);

    for cid in &node.children_ids {
        assign_node_widths(cid, depth + 1, tree, widths, depth_widths);
    }
}

#[allow(clippy::too_many_arguments)]
fn place_subtree(
    node_id: &str,
    depth: usize,
    y_top: u16,
    tree: &HashMap<String, LayoutDraft>,
    subtree_heights: &HashMap<String, u16>,
    widths: &HashMap<String, u16>,
    depth_x: &[u16],
    out: &mut Vec<PlacedTpNode>,
) -> u16 {
    let Some(node) = tree.get(node_id) else {
        return y_top + NODE_HEIGHT / 2;
    };
    let x = depth_x[depth];
    let width = widths.get(node_id).copied().unwrap_or(MIN_NODE_WIDTH);

    let out_idx = out.len();
    out.push(PlacedTpNode {
        id: node.id.clone(),
        kind: node.kind.clone(),
        label: node.label.clone(),
        note: node.note.clone(),
        parent_id: node.parent_id.clone(),
        children_ids: node.children_ids.clone(),
        depth,
        x,
        y: y_top,
        width,
        height: NODE_HEIGHT,
        exec_status: node.exec_status,
        scenario_count: node.scenario_count,
        req_count: node.req_count,
        collapsed_hidden_count: node.collapsed_hidden_count,
        highlight: HighlightCategory::GrayedOut,
    });

    if node.children_ids.is_empty() {
        return y_top + NODE_HEIGHT / 2;
    }

    let mut cur_y = y_top;
    let mut first_mid = 0u16;
    let mut last_mid = 0u16;
    for (idx, cid) in node.children_ids.iter().enumerate() {
        let ch_h = subtree_heights.get(cid).copied().unwrap_or(NODE_HEIGHT);
        let mid = place_subtree(
            cid,
            depth + 1,
            cur_y,
            tree,
            subtree_heights,
            widths,
            depth_x,
            out,
        );
        if idx == 0 {
            first_mid = mid;
        }
        last_mid = mid;
        cur_y += ch_h + GAP_Y;
    }

    let center_mid = ((first_mid as u32 + last_mid as u32) / 2) as u16;
    let node_y = center_mid.saturating_sub(NODE_HEIGHT / 2).max(y_top);
    out[out_idx].y = node_y;
    node_y + NODE_HEIGHT / 2
}

fn node_border_style(highlight: HighlightCategory, status: TestPointExecStatus) -> Style {
    match highlight {
        HighlightCategory::Selected => match status {
            TestPointExecStatus::Failed => Style::default()
                .fg(theme::ERROR)
                .add_modifier(Modifier::BOLD),
            TestPointExecStatus::Running => Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
            TestPointExecStatus::Passed => Style::default()
                .fg(theme::SUCCESS)
                .add_modifier(Modifier::BOLD),
            _ => Style::default()
                .fg(theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        },
        HighlightCategory::Ancestor | HighlightCategory::Descendant => {
            Style::default().fg(theme::INFO)
        }
        HighlightCategory::Sibling => Style::default().fg(theme::BORDER),
        HighlightCategory::GrayedOut => Style::default().fg(theme::MUTED),
    }
}

fn node_title_style(highlight: HighlightCategory, kind: &TpNodeKind) -> Style {
    let bold_kind = matches!(kind, TpNodeKind::Root | TpNodeKind::Branch { .. });
    match highlight {
        HighlightCategory::Selected => Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
        HighlightCategory::Ancestor | HighlightCategory::Descendant => {
            if bold_kind {
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::FG)
            }
        }
        HighlightCategory::Sibling => Style::default().fg(theme::FG),
        HighlightCategory::GrayedOut => Style::default().fg(theme::MUTED),
    }
}

fn connector_style(
    parent_hl: HighlightCategory,
    child_hl: HighlightCategory,
    status: TestPointExecStatus,
) -> Style {
    if matches!(
        child_hl,
        HighlightCategory::Selected | HighlightCategory::Descendant
    ) || matches!(
        (parent_hl, child_hl),
        (
            HighlightCategory::Ancestor,
            HighlightCategory::Selected | HighlightCategory::Ancestor
        )
    ) {
        match status {
            TestPointExecStatus::Failed => Style::default()
                .fg(theme::ERROR)
                .add_modifier(Modifier::BOLD),
            TestPointExecStatus::Running => Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
            TestPointExecStatus::Passed => Style::default().fg(theme::SUCCESS),
            _ => Style::default().fg(theme::PRIMARY),
        }
    } else if child_hl == HighlightCategory::GrayedOut {
        Style::default().fg(theme::MUTED)
    } else {
        Style::default().fg(theme::BORDER)
    }
}

fn next_test_point_id(existing: &[TestPoint]) -> String {
    let mut max_n = 0u64;
    for tp in existing {
        if let Some(n) = tp.id.strip_prefix("tp-").and_then(|s| s.parse().ok()) {
            max_n = max_n.max(n);
        }
    }
    format!("tp-{}", max_n + 1)
}

fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::authoring::{QuoteSelector, RequirementLink, TextRange};

    fn sample_tp(id: &str, title: &str, path: &[&str], scenarios: usize) -> TestPoint {
        let scenario_refs = (0..scenarios)
            .map(|i| teshi_core::authoring::ScenarioRef {
                feature_path: "auth.feature".into(),
                scenario_name: Some(format!("Scenario {i}")),
                scenario_line: Some(i + 2),
            })
            .collect();
        TestPoint {
            id: id.into(),
            title: title.into(),
            objective: String::new(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(path.iter().map(|s| s.to_string()).collect()),
            review_state: ReviewState::Approved,
            requirement_links: vec![RequirementLink {
                store_id: None,
                document_id: "doc-1".into(),
                document_revision: "rev-1".into(),
                position: TextRange::new(0, 0),
                quote: QuoteSelector::default(),
                resolution: ResolutionState::Resolved,
            }],
            scenario_refs,
        }
    }

    fn artifacts_with(tps: Vec<TestPoint>) -> AuthoringArtifacts {
        let mut artifacts = AuthoringArtifacts::default();
        artifacts.test_points.test_points = tps;
        artifacts
    }

    #[test]
    fn junction_synthesis_handles_multi_child_trunks_without_artifacts() {
        let mut canvas = TpCanvas::new(16, 9, theme::SCREEN_BG);
        let style = Style::default().fg(theme::PRIMARY);
        // Parent at right edge x=3, mid_y=4; three children at y=1, y=4, y=7, left_x=8.
        canvas.draw_tree_connector(
            2,
            4,
            &[(8, 1, style), (8, 4, style), (8, 7, style)],
            theme::SCREEN_BG,
        );
        let rendered: Vec<String> = canvas
            .to_ratatui_lines(0, 0, 16, 9, theme::SCREEN_BG)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|s| s.content.into_owned())
                    .collect::<String>()
            })
            .collect();

        // trunk_x = 2 + 2 = 4.
        assert_eq!(rendered[1].chars().nth(4), Some('╭'));
        assert_eq!(rendered[2].chars().nth(4), Some('│'));
        assert_eq!(rendered[4].chars().nth(3), Some('─'));
        assert_eq!(rendered[4].chars().nth(4), Some('┼'));
        assert_eq!(rendered[7].chars().nth(4), Some('╰'));
    }

    #[test]
    fn wide_cjk_viewport_clipping_never_leaks_half_width_or_reset_bg() {
        let mut canvas = TpCanvas::new(12, 3, theme::SCREEN_BG);
        canvas.put_text(
            1,
            1,
            "中文测试节点",
            12,
            Style::default().fg(theme::FG).bg(theme::SCREEN_BG),
        );
        // Clip inside second char of '中' (x=2) and before second char of '点' (max col=10 -> w=8).
        let lines = canvas.to_ratatui_lines(2, 0, 8, 3, theme::SCREEN_BG);
        for line in &lines {
            let line_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
            assert_eq!(line_width, 8);
            for span in &line.spans {
                assert_eq!(span.style.bg, Some(theme::SCREEN_BG));
            }
        }
        let middle: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(middle.starts_with(' '));
        assert!(middle.contains("文测试"));
    }

    #[test]
    fn layout_boxes_never_overlap_and_spatial_navigation_works() {
        let artifacts = artifacts_with(vec![
            sample_tp("tp-1", "Valid login", &["Auth", "Login"], 1),
            sample_tp("tp-2", "Bad password", &["Auth", "Login"], 0),
            sample_tp("tp-3", "Reset token", &["Auth", "Recovery"], 2),
        ]);
        let mut ui = TestPointsUiState::from_artifacts(Some(&artifacts));
        assert_eq!(ui.selected_test_point_id.as_deref(), Some("tp-1"));

        // Verify zero pairwise box overlap.
        for (i, a) in ui.placed_nodes.iter().enumerate() {
            for b in ui.placed_nodes.iter().skip(i + 1) {
                let x_overlap = a.x < b.x + b.width && b.x < a.x + a.width;
                let y_overlap = a.y < b.y + b.height && b.y < a.y + a.height;
                assert!(
                    !(x_overlap && y_overlap),
                    "nodes {} and {} overlap",
                    a.id,
                    b.id
                );
            }
        }

        // Move Down -> tp-2; Left -> Login branch; Up/Down -> Recovery/Login; Left -> Auth -> Root.
        assert!(ui.navigate_spatial(SpatialDir::Down, Some(&artifacts)));
        assert_eq!(ui.selected_test_point_id.as_deref(), Some("tp-2"));

        assert!(ui.navigate_spatial(SpatialDir::Left, Some(&artifacts)));
        assert_eq!(ui.selected_node_id.as_deref(), Some("dir:Auth/Login"));
        assert_eq!(ui.selected_test_point_id, None);

        assert!(ui.navigate_spatial(SpatialDir::Down, Some(&artifacts)));
        assert_eq!(ui.selected_node_id.as_deref(), Some("dir:Auth/Recovery"));

        assert!(ui.navigate_spatial(SpatialDir::Right, Some(&artifacts)));
        assert_eq!(ui.selected_test_point_id.as_deref(), Some("tp-3"));
    }

    #[test]
    fn crud_add_slash_split_rename_delete_and_undo_redo_work() {
        let mut maybe = Some(artifacts_with(vec![sample_tp(
            "tp-1",
            "Existing",
            &["Auth"],
            0,
        )]));
        let mut ui = TestPointsUiState::from_artifacts(maybe.as_ref());

        // Add child under Auth with `/` auto-split.
        ui.select_node_by_id("dir:Auth", maybe.as_ref());
        assert!(ui.start_add_child(&mut maybe));
        for ch in "OAuth / Callback expires".chars() {
            ui.insert_char(ch);
        }
        assert!(ui.commit_inline_edit(maybe.as_mut().unwrap(), false));
        let added = maybe
            .as_ref()
            .unwrap()
            .test_points
            .test_points
            .iter()
            .find(|t| t.id == "tp-2")
            .expect("tp-2");
        assert_eq!(added.title, "Callback expires");
        assert_eq!(added.hierarchy_path.as_slice(), &["Auth", "OAuth"]);
        assert_eq!(added.review_state, ReviewState::Approved);

        // Delete tp-2, then undo -> restored, then redo -> deleted again.
        let deleted = ui.delete_selected(maybe.as_mut().unwrap());
        assert_eq!(deleted.as_deref(), Some("Callback expires"));
        assert_eq!(maybe.as_ref().unwrap().test_points.test_points.len(), 1);

        assert!(ui.undo(maybe.as_mut().unwrap()));
        assert_eq!(maybe.as_ref().unwrap().test_points.test_points.len(), 2);

        assert!(ui.redo(maybe.as_mut().unwrap()));
        assert_eq!(maybe.as_ref().unwrap().test_points.test_points.len(), 1);
    }

    #[test]
    fn exec_statuses_reflect_uncovered_linked_running_passed_failed() {
        let artifacts = artifacts_with(vec![
            sample_tp("tp-1", "Unlinked", &["A"], 0),
            sample_tp("tp-2", "Linked", &["A"], 1),
            sample_tp("tp-3", "Pass", &["A"], 1),
            sample_tp("tp-4", "Fail", &["A"], 1),
            sample_tp("tp-5", "Run", &["A"], 1),
        ]);
        let mut feature = teshi_core::gherkin::parse_feature(
            "Feature: Auth\n\
             @teshi-tp:tp-2\n  Scenario: S2\n    Given a\n\
             @teshi-tp:tp-3\n  Scenario: S3\n    Given b\n\
             @teshi-tp:tp-4\n  Scenario: S4\n    Given c\n\
             @teshi-tp:tp-5\n  Scenario: S5\n    Given d\n",
            std::path::PathBuf::from("auth.feature"),
        );
        feature.scenarios.iter_mut().for_each(|_| {});
        let project = BddProject {
            root_dir: std::path::PathBuf::from("."),
            features: vec![feature],
        };

        let mut statuses = HashMap::new();
        statuses.insert((0, 1), RunStatus::Passed);
        statuses.insert((0, 2), RunStatus::Failed);
        statuses.insert((0, 3), RunStatus::Running);

        let mut ui = TestPointsUiState::empty();
        ui.compute_exec_statuses(&project, &statuses, true, Some(&artifacts));

        assert_eq!(
            ui.exec_statuses.get("tp-1").copied(),
            Some(TestPointExecStatus::Uncovered)
        );
        assert_eq!(
            ui.exec_statuses.get("tp-2").copied(),
            Some(TestPointExecStatus::Linked)
        );
        assert_eq!(
            ui.exec_statuses.get("tp-3").copied(),
            Some(TestPointExecStatus::Passed)
        );
        assert_eq!(
            ui.exec_statuses.get("tp-4").copied(),
            Some(TestPointExecStatus::Failed)
        );
        assert_eq!(
            ui.exec_statuses.get("tp-5").copied(),
            Some(TestPointExecStatus::Running)
        );
    }
}
