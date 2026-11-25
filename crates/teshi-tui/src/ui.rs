use crate::theme;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use tui_tree_widget::{Tree, TreeItem, TreeState};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{
    App, CaseDetail, ChangedStep, ClickableRegion, ColumnFocus, MainTab, MindMapFocus, RunStatus,
};
use crate::bdd_nav::{
    nav_body_char_range_in_buffer, row_is_in_background_block, structural_prefix_char_range,
};
use crate::highlight::{
    StepHighlightState, highlight_line_with_state, kind_to_style, leading_whitespace_chars,
    step_keyword_gutter_pad, step_keyword_gutter_styled_span,
};
use crate::keymap::Action;
use teshi_core::gherkin::{BddScenario, ScenarioKind};
use teshi_core::gherkin_lang::{
    GherkinLanguage, GherkinLanguages, StepKeywordType, StructuralType,
};
use teshi_core::highlight::{HighlightKind, highlight_line_spans};

pub(crate) mod chat;
mod dialog;
pub(crate) mod help;
mod overlay;
mod pointer_controls;

use dialog::{Dialog, DialogButton, FormDialog, FormField, ListDialog};
use theme::ButtonTone;

/// iCode-style size rules for the overlays migrated to [`overlay`].
///
/// Each axis keeps a preferred size, a viewport share and an absolute cap, so
/// a 200-column terminal does not stretch a card that was designed for 120.
const CHANGE_SUMMARY_SPEC: overlay::OverlaySpec =
    overlay::OverlaySpec::viewer(65, 30, 110, 55, 10, 40);
const SETTINGS_SPEC: overlay::OverlaySpec = overlay::OverlaySpec {
    width: overlay::Axis::cells(80, 12, 92),
    height: overlay::Axis::cells(22, 8, 85),
};
const AGENT_PROFILE_SPEC: overlay::OverlaySpec = overlay::OverlaySpec::viewer(
    95,
    AGENT_PROFILE_PANEL_MIN_WIDTH,
    AGENT_PROFILE_PANEL_MAX_WIDTH,
    85,
    AGENT_PROFILE_PANEL_MIN_HEIGHT,
    AGENT_PROFILE_PANEL_MAX_HEIGHT,
);

/// Stage-2 preview: one solid style for the tree-selected line (avoids span-patch gaps that read as bright blocks).
fn main_tabs(width: u16) -> [(MainTab, &'static str); 5] {
    let compact = width < 85;
    [
        (
            MainTab::Explore,
            crate::i18n::t(if compact {
                "tab.explore.compact"
            } else {
                "tab.explore"
            }),
        ),
        (
            MainTab::MindMap,
            crate::i18n::t(if compact {
                "tab.mindmap.compact"
            } else {
                "tab.mindmap"
            }),
        ),
        (
            MainTab::Ai,
            crate::i18n::t(if compact { "tab.ai.compact" } else { "tab.ai" }),
        ),
        (
            MainTab::Requirements,
            crate::i18n::t(if compact {
                "tab.requirements.compact"
            } else {
                "tab.requirements"
            }),
        ),
        (
            MainTab::TestPoints,
            crate::i18n::t(if compact {
                "tab.test_points.compact"
            } else {
                "tab.test_points"
            }),
        ),
    ]
}
const TAB_PADDING_LEFT: &str = " ";
const TAB_PADDING_RIGHT: &str = " ";
const TAB_DIVIDER: &str = " ";

fn register_tab_regions(app: &mut App, area: Rect) {
    if area.is_empty() {
        return;
    }

    // Mirror `ratatui::Tabs` rendering: left padding, visible title, right
    // padding, then a divider. Keeping these values beside `main_tabs` and
    // applying them explicitly to the widget keeps rendering and hit-testing
    // on the same coordinate system.
    let tabs = main_tabs(area.width);
    let mut x = area.left();
    let right = area.right();
    let padding_left_width = TAB_PADDING_LEFT.width() as u16;
    let padding_right_width = TAB_PADDING_RIGHT.width() as u16;
    let divider_width = TAB_DIVIDER.width() as u16;

    for (index, &(tab, title)) in tabs.iter().enumerate() {
        x = x.saturating_add(padding_left_width.min(right.saturating_sub(x)));
        if x >= right {
            break;
        }

        let width = (title.width() as u16).min(right.saturating_sub(x));
        if width == 0 {
            break;
        }
        app.clickable_regions.push(ClickableRegion::Tab {
            tab,
            rect: Rect::new(x, area.top(), width, 1),
        });
        x = x.saturating_add(width);
        if x >= right {
            break;
        }

        x = x.saturating_add(padding_right_width.min(right.saturating_sub(x)));
        if x >= right || index == tabs.len() - 1 {
            break;
        }
        x = x.saturating_add(divider_width.min(right.saturating_sub(x)));
    }
}

/// Format token counts with thousands separators for display.
fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if (s.len() - i).is_multiple_of(3) && i > 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

/// Returns a short token-usage label for the input area, or empty if no tokens used yet.
fn token_usage_label(in_tokens: u64, out_tokens: u64) -> String {
    if in_tokens == 0 && out_tokens == 0 {
        return String::new();
    }
    format!(
        "in: {} · out: {}",
        fmt_count(in_tokens),
        fmt_count(out_tokens)
    )
}

/// Applies `patch` on UTF-8 character indices `[range.start, range.end)` within each span.
fn apply_patch_to_char_range(
    line: Line<'static>,
    range: std::ops::Range<usize>,
    patch: Style,
) -> Line<'static> {
    if range.start >= range.end {
        return line;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut idx = 0usize;
    let spans = line.spans;
    for span in spans {
        let text = span.content.to_string();
        let n = text.chars().count();
        let start_i = idx;
        let end_i = idx + n;
        let lo = range.start.max(start_i);
        let hi = range.end.min(end_i);
        if hi <= lo {
            out.push(span);
        } else {
            let lo_rel = lo - start_i;
            let hi_rel = hi - start_i;
            let chars: Vec<char> = text.chars().collect();
            if lo_rel > 0 {
                let before: String = chars[..lo_rel].iter().collect();
                out.push(Span::styled(before, span.style));
            }
            let mid: String = chars[lo_rel..hi_rel].iter().collect();
            let mid_style = span.style.patch(patch);
            out.push(Span::styled(mid, mid_style));
            if hi_rel < chars.len() {
                let after: String = chars[hi_rel..].iter().collect();
                out.push(Span::styled(after, span.style));
            }
        }
        idx = end_i;
    }
    Line::from(out)
}

/// Truncates styled spans so total display width does not exceed `max_cols` (Unicode columns).
///
/// When content is wider than the editor inner width, `pad_line_to_width` intentionally does not
/// pad; `Buffer::set_line` then clips visually while the `Line` still reports a larger width. That
/// mismatch breaks ratatui's terminal diff for trailing cells (observed as garbling on Windows).
fn truncate_line_to_cols(line: Line<'static>, max_cols: u16) -> Line<'static> {
    let max = max_cols as usize;
    if line.width() <= max {
        return line;
    }
    let line_style = line.style;
    let alignment = line.alignment;
    let mut budget = max;
    let mut out_spans: Vec<Span<'static>> = Vec::new();
    for span in line.spans {
        if budget == 0 {
            break;
        }
        let s = span.content.to_string();
        let mut acc = String::new();
        for ch in s.chars() {
            let w = ch.width().unwrap_or(0);
            if w == 0 {
                acc.push(ch);
                continue;
            }
            if w > budget {
                break;
            }
            acc.push(ch);
            budget -= w;
        }
        if !acc.is_empty() {
            out_spans.push(Span::styled(acc, span.style));
        }
        if budget == 0 {
            break;
        }
    }
    let mut out = Line::from(out_spans);
    out.style = line_style;
    out.alignment = alignment;
    out
}

/// Pads a line to `target_cols` display width using a trailing span (Unicode column widths).
fn pad_line_to_width(mut line: Line<'static>, target_cols: u16, trail: Style) -> Line<'static> {
    let t = target_cols as usize;
    let w = line.width();
    if w >= t {
        return line;
    }
    line.push_span(Span::styled(" ".repeat(t - w), trail));
    line
}

fn step_line_display(
    line: &str,
    in_doc_string: bool,
    lang: &GherkinLanguage,
) -> (String, usize, usize) {
    if in_doc_string {
        return (line.to_string(), 0, 0);
    }
    let trimmed = line.trim_start();
    let leading = leading_whitespace_chars(line);
    let Some((keyword, _ty)) = lang.match_step_prefix(trimmed) else {
        return (line.to_string(), 0, 0);
    };
    let pad = step_keyword_gutter_pad(keyword);
    if pad == 0 {
        return (line.to_string(), 0, 0);
    }
    let rest: String = trimmed.chars().skip(keyword.chars().count()).collect();
    let mut out = String::new();
    let lead: String = line.chars().take(leading).collect();
    out.push_str(&lead);
    out.push_str(&" ".repeat(pad));
    out.push_str(keyword);
    out.push_str(&rest);
    // Padding is inserted before the keyword; buffer columns from indent onward shift visually.
    let pad_start = leading;
    (out, pad, pad_start)
}

/// Workspace identity pinned to the right of the tab row: project name, then
/// build version.
///
/// The project label is the only affordance for the switcher, so it keeps the
/// row; the version yields entirely when the two cannot share it.
fn render_workspace_meta(frame: &mut Frame<'_>, app: &mut App, row: Rect) {
    let tabs_end = app
        .clickable_regions
        .iter()
        .filter_map(|region| match region {
            ClickableRegion::Tab { rect, .. } => Some(rect.right()),
            _ => None,
        })
        .max()
        .unwrap_or(row.x);
    // One column of breathing room keeps the last tab's padding intact.
    let room = row.right().saturating_sub(tabs_end.saturating_add(1));
    if room == 0 {
        return;
    }

    let version_text = format!("v{}", app.version);
    let project_label = crate::i18n::t("footer.project")
        .replace("{name}", &app.project_switch.active.identity.display_name());
    const META_GAP: u16 = 1;
    let version_width = if app.version.is_empty() {
        0
    } else {
        version_text.width() as u16
    };
    let project_width = project_label.width() as u16;
    let show_version = version_width > 0 && project_width + META_GAP + version_width <= room;
    let version_width = if show_version { version_width } else { 0 };
    let project_room = room.saturating_sub(version_width).saturating_sub(META_GAP);
    let project_width = project_width.min(project_room);
    if project_width > 0 {
        let rect = Rect::new(
            row.right() - version_width - project_width,
            row.y,
            project_width,
            1,
        );
        frame.render_widget(
            Paragraph::new(project_label).style(theme::project_chip_style()),
            rect,
        );
        app.clickable_regions
            .push(ClickableRegion::ProjectControl { rect });
    }
    if version_width > 0 {
        frame.render_widget(
            Paragraph::new(version_text)
                .style(theme::muted().bg(theme::TOPBAR_BG))
                .alignment(Alignment::Right),
            Rect::new(row.right() - version_width, row.y, version_width, 1),
        );
    }
}

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    // Paint the screen surface first, the way iCode's `MainScreen` paints
    // `$background`: the palette then reads the same in any terminal profile.
    // Component fills draw on top; a region cleared later repaints it with
    // `clear_to` because `Clear` alone resets cells to the terminal's defaults.
    frame.render_widget(
        Block::default().style(Style::default().fg(theme::FG).bg(theme::SCREEN_BG)),
        frame.area(),
    );
    // Validate the raw editor buffer on every redraw.  The parser-backed
    // project remains permissive; these diagnostics are only an editor aid
    // and never prevent Browse/Insert rendering or input handling.
    app.refresh_buffer_diagnostics();
    app.clickable_regions.clear();
    app.vertical_scrollbars.clear();
    app.modal_card_rect = None;
    app.chat_input_text_rect = None;
    app.req_tree_row_spans.clear();
    app.feature_tags.begin_frame();
    app.agent_mut().acp_controls_ui.clear_regions();
    app.chat_history_rect = None;
    app.ai_sidebar_rect = None;
    app.ai_sidebar_working_rect = None;
    app.ai_sidebar_settled_rect = None;

    let [tabs_row, content_row, footer_row] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

    frame.render_widget(Block::default().style(theme::topbar_style()), tabs_row);
    let active_tab_accent = theme::tab_accent(app.active_tab);
    let top_tabs = Tabs::new(
        main_tabs(tabs_row.width)
            .iter()
            .map(|&(_, title)| Line::from(title))
            .collect::<Vec<_>>(),
    )
    .select(match app.active_tab {
        MainTab::Explore => 0,
        MainTab::MindMap => 1,
        MainTab::Ai => 2,
        MainTab::Requirements => 3,
        MainTab::TestPoints => 4,
    })
    .style(theme::topbar_style())
    .highlight_style(
        Style::default()
            .fg(active_tab_accent)
            .bg(theme::TAB_ACTIVE_BG)
            .add_modifier(Modifier::BOLD),
    )
    .padding(TAB_PADDING_LEFT, TAB_PADDING_RIGHT)
    .divider(TAB_DIVIDER);
    frame.render_widget(top_tabs, tabs_row);

    register_tab_regions(app, tabs_row);
    render_workspace_meta(frame, app, tabs_row);

    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(theme::FOOTER_BG).fg(theme::FOOTER_DESC)),
        footer_row,
    );
    // Quit stays pinned to the bottom-right of every tab so it is always
    // discoverable and clickable. The label advertises Ctrl+Q (matching the
    // host-safe `:q` docs) so the chord is visible without leaving the strip.
    // Tab footers render into the remaining width and cannot cover the control.
    let exit_label = format!(" {} ", crate::i18n::t("footer.exit"));
    let exit_label_width = exit_label.width() as u16;
    let full_footer = footer_row;
    // Very narrow terminals keep their tab footer intact and drop Exit so
    // contextual controls stay reachable; Ctrl+Q / :q remain available.
    let exit_width = if full_footer.width >= 60 {
        exit_label_width.min(full_footer.width)
    } else {
        0
    };
    let footer = Rect {
        width: full_footer.width.saturating_sub(exit_width),
        ..full_footer
    };
    // Every tab footer below renders into `footer`, leaving the right edge to
    // the exit control.
    render_main_panel(frame, app, content_row);
    render_tree_pointer_feedback(frame, app);

    // Agent change approval status is shown only in the AI chat panel's status
    // bar (above the input area) — never in the bottom command line.
    // Tab workflows live in pane strips; the global strip is Settings only.
    // Give the global strip the full remaining width when it fits; narrow
    // terminals drop it rather than back-filling contextual controls.
    const APP_ACTIONS_WIDTH: u16 = 20;
    let app_footer = (footer.width >= APP_ACTIONS_WIDTH).then_some(footer);

    if app.command_line.is_some() {
        render_command_line(frame, app, footer);
    } else if let Some(ref msg) = app.status_message {
        let status_line = Line::from(vec![Span::styled(
            msg.as_str(),
            Style::default()
                .fg(theme::WARNING)
                .bg(theme::FOOTER_BG)
                .add_modifier(Modifier::BOLD),
        )]);
        frame.render_widget(Paragraph::new(status_line), footer);
    } else if let Some(app_footer) = app_footer {
        render_app_actions(frame, app, app_footer);
    }

    if exit_width > 0 {
        let exit_area = Rect::new(
            full_footer.right().saturating_sub(exit_width),
            full_footer.y,
            exit_width,
            1,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: exit_area,
            action: Action::Quit,
            modal: false,
        });
        frame.render_widget(
            Paragraph::new(theme::footer_line([Span::styled(
                exit_label,
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )])),
            exit_area,
        );
    }

    if app.auth_panel_active
        || app.model_panel_active
        || app.session_panel_active
        || app.history_search.is_some()
        || app.composer_editor.is_some()
        || app.quit_pending_confirm
        || app.ai_interrupt_confirm
        || app.approval_panel_active
        || app.settings_panel_active
        || app.help_guide.active
        || app.agent_profile_panel_active
        || app.change_summary_visible
        || app.screenshot.visible
        || app.has_external_change_prompt()
        || app.step_keyword_picker.is_some()
        || app.has_agent_change_prompt()
        || app.has_acp_permission_prompt()
        || app.authoring_ui.overlay_active()
        || app.generation_scope_prompt.is_some()
        || !matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::None
        )
    {
        // Modal rendering replaces the interactive layer, not just its pixels.
        app.clickable_regions.clear();
    }
    render_external_change_prompt(frame, app, content_row);

    // Change Summary overlay (MindMap tab)
    render_change_summary_panel(frame, app, content_row);
    if let Some(editor_area) = app.editor_panel_rect
        && app.step_keyword_picker.is_some()
    {
        render_step_keyword_picker(frame, app, editor_area);
    }

    if app.auth_panel_active {
        render_auth_panel(frame, app, frame.area());
    }

    if app.model_panel_active {
        render_model_panel(frame, app, frame.area());
    }

    if app.session_panel_active {
        render_session_panel(frame, app, frame.area());
    }

    if app.history_search.is_some() {
        render_history_search_panel(frame, app, frame.area());
    }

    if app.composer_editor.is_some() {
        render_composer_editor_panel(frame, app, frame.area());
    }

    if app.quit_pending_confirm {
        render_quit_panel(frame, app, frame.area());
    }

    if app.ai_interrupt_confirm {
        render_interrupt_panel(frame, app, frame.area());
    }

    if app.approval_panel_active {
        render_approval_panel(frame, app, frame.area());
    }

    if app.settings_panel_active {
        render_settings_panel(frame, app, frame.area());
    }

    if app.help_guide.active {
        help::render_help_guide(frame, app, frame.area());
    }

    if app.agent_profile_panel_active {
        if app.agent_panel_mode == crate::app::AgentPanelMode::List {
            render_agent_profile_panel(frame, app, frame.area());
        } else {
            render_acp_agent_panel(frame, app, frame.area());
        }
    }

    if !matches!(
        app.authoring_ui.overlay,
        crate::authoring_tab::RequirementsOverlay::None
    ) || app.generation_scope_prompt.is_some()
        || !matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::None
        )
    {
        render_requirements_overlay(frame, app, frame.area());
    }
    render_screenshot_overlay(frame, app, content_row);
    crate::acp_controls::render_picker(frame, app);
    pointer_controls::render_pending_review(frame, app, content_row);
    if app.pending_delete.is_some() {
        // The confirmation is the only interactive layer, including its mouse
        // targets; covered list rows must not change the captured target.
        app.clickable_regions.clear();
        render_delete_confirmation(frame, app, frame.area());
    }
    render_pointer_feedback(frame, app);
}

/// Current Vim/GUI interaction mode badge for the global status bar.
fn active_mode_badge(app: &App) -> (&'static str, Color) {
    match app.active_tab {
        MainTab::Explore => {
            if app.explore_edit_mode {
                if app.step_input_active {
                    (crate::i18n::t("mode.badge.insert"), theme::ACCENT)
                } else {
                    (crate::i18n::t("mode.badge.normal"), theme::PRIMARY)
                }
            } else if app.feature_browser.searching {
                (crate::i18n::t("mode.badge.search"), theme::INFO)
            } else {
                (crate::i18n::t("mode.badge.normal"), theme::PRIMARY)
            }
        }
        MainTab::MindMap => {
            if app.mindmap_focus == MindMapFocus::AiPanel && app.ai_input_focused {
                (crate::i18n::t("mode.badge.input"), theme::ACCENT)
            } else {
                (crate::i18n::t("mode.badge.normal"), theme::INFO)
            }
        }
        MainTab::Ai => {
            if app.ai_input_focused {
                (crate::i18n::t("mode.badge.input"), theme::ACCENT)
            } else {
                (crate::i18n::t("mode.badge.normal"), theme::PRIMARY)
            }
        }
        MainTab::Requirements => {
            if app.authoring_ui.search_active {
                (crate::i18n::t("mode.badge.search"), theme::INFO)
            } else if app.authoring_ui.focus == crate::authoring_tab::RequirementsFocus::Editor {
                match app.authoring_ui.editor_mode {
                    crate::authoring_tab::RequirementsEditorMode::Preview => {
                        (crate::i18n::t("mode.badge.preview"), theme::SUCCESS)
                    }
                    crate::authoring_tab::RequirementsEditorMode::Browse => {
                        (crate::i18n::t("mode.badge.browse"), theme::PRIMARY)
                    }
                    crate::authoring_tab::RequirementsEditorMode::Insert => {
                        (crate::i18n::t("mode.badge.insert"), theme::ACCENT)
                    }
                    crate::authoring_tab::RequirementsEditorMode::VisualChar => {
                        (crate::i18n::t("mode.badge.visual"), theme::WARNING)
                    }
                    crate::authoring_tab::RequirementsEditorMode::VisualLine => {
                        (crate::i18n::t("mode.badge.visual_line"), theme::WARNING)
                    }
                }
            } else {
                (crate::i18n::t("mode.badge.normal"), theme::SUCCESS)
            }
        }
        MainTab::TestPoints => {
            if app.test_points_ui.editing {
                (crate::i18n::t("mode.badge.insert"), theme::ACCENT)
            } else {
                (crate::i18n::t("mode.badge.normal"), theme::WARNING)
            }
        }
    }
}

/// Active tab and focused pane breadcrumb for the global status bar.
fn active_pane_breadcrumb(app: &App) -> &'static str {
    match app.active_tab {
        MainTab::Explore => {
            if app.explore_edit_mode {
                crate::i18n::t("pane.crumb.explore.source")
            } else {
                match app.explore_focus {
                    ColumnFocus::Feature => crate::i18n::t("pane.crumb.explore.features"),
                    ColumnFocus::Scenario => crate::i18n::t("pane.crumb.explore.scenarios"),
                    ColumnFocus::Step => crate::i18n::t("pane.crumb.explore.steps"),
                    ColumnFocus::Details => crate::i18n::t("pane.crumb.explore.details"),
                }
            }
        }
        MainTab::MindMap => match app.mindmap_focus {
            MindMapFocus::Main => crate::i18n::t("pane.crumb.mindmap.tree"),
            MindMapFocus::AiPanel => crate::i18n::t("pane.crumb.mindmap.ai"),
        },
        MainTab::Ai => {
            if app.ai_sidebar_focused {
                crate::i18n::t("pane.crumb.ai.chats")
            } else {
                crate::i18n::t("pane.crumb.ai.chat")
            }
        }
        MainTab::Requirements => match app.authoring_ui.focus {
            crate::authoring_tab::RequirementsFocus::Tree => crate::i18n::t("pane.crumb.req.tree"),
            crate::authoring_tab::RequirementsFocus::Editor => crate::i18n::t("pane.crumb.req.doc"),
            crate::authoring_tab::RequirementsFocus::LinkedTestPoints => {
                crate::i18n::t("pane.crumb.req.linked")
            }
        },
        MainTab::TestPoints => match app.test_points_ui.focus {
            crate::test_points_tab::TestPointsFocus::Tree => crate::i18n::t("pane.crumb.tp.tree"),
            crate::test_points_tab::TestPointsFocus::Details => {
                crate::i18n::t("pane.crumb.tp.details")
            }
            crate::test_points_tab::TestPointsFocus::Excerpts => {
                crate::i18n::t("pane.crumb.tp.excerpts")
            }
        },
    }
}

/// Draw the global status strip: semantic mode badge, active pane breadcrumb,
/// and the clickable Settings `[F10]` control.
fn render_app_actions(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let (mode_label, mode_color) = active_mode_badge(app);
    let crumb = active_pane_breadcrumb(app);
    let settings_label = crate::i18n::t("footer.settings");
    let mode_w = mode_label.width() as u16;
    let crumb_text = format!(" {crumb} ");
    let crumb_w = crumb_text.width() as u16;
    let sep_w = 2u16;
    let settings_w = settings_label.width() as u16;

    let mut x = area.x;
    let right = area.right();

    // Paint the mode chip first when Settings still fits beside it.
    if x + mode_w + 1 + settings_w <= right {
        frame.render_widget(
            Paragraph::new(Line::from(theme::chip(mode_label, mode_color))),
            Rect::new(x, area.y, mode_w, 1),
        );
        x += mode_w;
    }
    // Paint the active pane breadcrumb when width allows.
    if x + crumb_w + sep_w + settings_w <= right {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    crumb_text,
                    Style::default()
                        .fg(theme::tab_accent(app.active_tab))
                        .bg(theme::FOOTER_BG)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "│ ",
                    Style::default().fg(theme::BORDER).bg(theme::FOOTER_BG),
                ),
            ])),
            Rect::new(x, area.y, crumb_w + sep_w, 1),
        );
        x += crumb_w + sep_w;
    } else if x > area.x && x + 1 + settings_w <= right {
        x += 1;
    }

    let remaining = Rect::new(x, area.y, right.saturating_sub(x), 1);
    let help_label = crate::i18n::t("footer.help");
    let help_w = help_label.width() as u16;
    if remaining.width >= settings_w + 1 + help_w {
        let actions = [
            (settings_label, Action::SettingsPanelOpen),
            (help_label, Action::HelpPanelOpen),
        ];
        render_action_bar(
            frame,
            &mut app.clickable_regions,
            remaining,
            "",
            &actions,
            false,
        );
    } else {
        let actions = [(settings_label, Action::SettingsPanelOpen)];
        render_action_bar(
            frame,
            &mut app.clickable_regions,
            remaining,
            "",
            &actions,
            false,
        );
    }
}

/// Single-line `:` prompt in the footer strip; the block caret marks the cursor.
fn render_command_line(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(input) = app.command_line.as_ref() else {
        return;
    };
    let (before, after) = input.split_at_cursor();
    let mut rest = after.chars();
    let under = rest.next().map_or_else(|| " ".to_string(), String::from);
    let rest_str = rest.as_str();
    let mut spans = vec![
        Span::styled(
            ":",
            Style::default()
                .fg(theme::PRIMARY)
                .bg(theme::FOOTER_BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            before.to_string(),
            Style::default().fg(theme::FOOTER_DESC).bg(theme::FOOTER_BG),
        ),
        Span::styled(
            under.clone(),
            Style::default().fg(theme::CURSOR_FG).bg(theme::CURSOR_BG),
        ),
        Span::styled(
            rest_str.to_string(),
            Style::default().fg(theme::FOOTER_DESC).bg(theme::FOOTER_BG),
        ),
    ];

    let used_w = 1 + before.width() + under.width() + rest_str.width();
    let matches = app.matching_ex_commands();
    if !matches.is_empty() {
        let hint = format!("  [Tab] {}", matches.join(" · "));
        if used_w + 6 < area.width as usize {
            spans.push(Span::styled(
                hint,
                Style::default().fg(theme::MUTED).bg(theme::FOOTER_BG),
            ));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_tree_pointer_feedback(frame: &mut Frame<'_>, app: &mut App) {
    let Some((x, y)) = app.mouse_position else {
        return;
    };
    if app.pending_delete.is_some()
        || app.has_external_change_prompt()
        || app.auth_panel_active
        || app.model_panel_active
        || app.session_panel_active
        || app.agent_profile_panel_active
        || app.approval_panel_active
        || app.settings_panel_active
        || app.help_guide.active
        || app.quit_pending_confirm
        || !matches!(
            app.authoring_ui.overlay,
            crate::authoring_tab::RequirementsOverlay::None
        )
        || !matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::None
        )
    {
        return;
    }
    let mut rows = Vec::new();
    let inner = |area: Rect| {
        area.inner(ratatui::layout::Margin {
            horizontal: 1,
            vertical: 1,
        })
    };
    match app.active_tab {
        MainTab::Requirements => {
            let ui = &mut app.authoring_ui;
            let tree_inner = inner(ui.pane_areas[0]);
            if tree_inner.contains((x, y).into()) {
                for &(row_y, row_h) in &app.req_tree_row_spans {
                    if y >= row_y && y < row_y.saturating_add(row_h) {
                        paint_hover(
                            frame,
                            Rect::new(tree_inner.x, row_y, tree_inner.width, row_h),
                        );
                        break;
                    }
                }
            }
            rows.push((
                inner(ui.pane_areas[2]),
                ui.filtered_linked_test_points().len(),
                ui.linked_scroll,
            ));
        }
        MainTab::MindMap => {
            if let Some(area) = app.tree_panel_rect {
                rows.push((
                    area,
                    app.tree_state.flatten(&app.mindmap_index.items).len(),
                    app.tree_state.get_offset(),
                ));
            }
        }
        _ => {}
    }
    for (area, count, offset) in rows {
        if area.contains((x, y).into()) && offset + usize::from(y - area.y) < count {
            paint_hover(frame, Rect::new(area.x, y, area.width, 1));
        }
    }
}

/// Clears `area` before new content and repaints it with `bg`.
///
/// `Clear` resets every cell to the terminal's own defaults, which would cut a
/// transparent hole through the screen surface. Any cleared region that does not
/// immediately draw its own fill must be cleared through here instead.
pub(crate) fn clear_to(frame: &mut Frame<'_>, area: Rect, bg: Color) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().fg(theme::FG).bg(bg)),
        area,
    );
}

/// [`clear_to`] on the screen surface.
pub(crate) fn clear_to_surface(frame: &mut Frame<'_>, area: Rect) {
    clear_to(frame, area, theme::SCREEN_BG);
}

/// Tints a hovered target. Fills are lifted by `theme::hover_fill`, so an
/// already selected row still answers the pointer without losing its label.
fn paint_hover(frame: &mut Frame<'_>, area: Rect) {
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buffer[(x, y)];
            // Half-block button edges carry the button color in the glyph;
            // lifting their cell background would tint the dialog around them.
            if matches!(cell.symbol(), "▄" | "▀") {
                cell.fg = theme::hover_fill(cell.fg);
            } else {
                cell.bg = theme::hover_fill(cell.bg);
            }
        }
    }
}

fn render_pointer_feedback(frame: &mut Frame<'_>, app: &App) {
    let Some((x, y)) = app.mouse_position else {
        return;
    };
    // Only registered, currently visible targets get feedback. Modal rendering
    // already removes covered targets, so hover cannot imply a blocked action.
    for region in app.clickable_regions.iter().rev() {
        let rect = match region {
            ClickableRegion::ActionButton { rect, .. }
            | ClickableRegion::RequirementFilterOption { rect, .. }
            | ClickableRegion::TestPointField { rect, .. }
            | ClickableRegion::TestPointExcerpt { rect, .. }
            | ClickableRegion::TestPointScenario { rect, .. }
            | ClickableRegion::ModelProfileRow { rect, .. }
            | ClickableRegion::ModelFormField { rect, .. }
            | ClickableRegion::AgentProfileRow { rect, .. }
            | ClickableRegion::ChangeSummaryRow { rect, .. }
            | ClickableRegion::StepKeywordRow { rect, .. }
            | ClickableRegion::SessionRow { rect, .. }
            | ClickableRegion::MindMapLocation { rect, .. }
            | ClickableRegion::Tab { rect, .. }
            | ClickableRegion::ProjectControl { rect }
            | ClickableRegion::ProjectRecent { rect, .. }
            | ClickableRegion::ProjectOpenPath { rect, .. }
            | ClickableRegion::QuitConfirmYes { rect }
            | ClickableRegion::QuitConfirmNo { rect }
            | ClickableRegion::AiInterruptConfirm { rect }
            | ClickableRegion::AiInterruptContinue { rect }
            | ClickableRegion::ExploreHorizontalScrollbar { rect, .. }
            | ClickableRegion::ExploreDetailTab { rect, .. }
            | ClickableRegion::ExploreDetailItem { rect, .. }
            | ClickableRegion::ExploreSearchTrigger { rect }
            | ClickableRegion::ExploreSearchConfirm { rect }
            | ClickableRegion::ExploreSearchClear { rect }
            | ClickableRegion::ReqSearchTrigger { rect }
            | ClickableRegion::ReqSearchConfirm { rect }
            | ClickableRegion::ReqSearchClear { rect } => *rect,
            ClickableRegion::ExploreFeature {
                row_y,
                col_x,
                col_right,
                ..
            }
            | ClickableRegion::ExploreDeletedFeature {
                row_y,
                col_x,
                col_right,
                ..
            }
            | ClickableRegion::ExploreScenario {
                row_y,
                col_x,
                col_right,
                ..
            }
            | ClickableRegion::ExploreStep {
                row_y,
                col_x,
                col_right,
                ..
            }
            | ClickableRegion::ApprovalBadge {
                row_y,
                col_x,
                col_right,
                ..
            }
            | ClickableRegion::ApprovalOption {
                row_y,
                col_x,
                col_right,
                ..
            } => Rect::new(*col_x, *row_y, col_right.saturating_sub(*col_x), 1),
            _ => continue,
        };
        if rect.contains((x, y).into()) {
            let visible = rect.intersection(frame.area());
            // A tinted cell under inline pixels makes the whole image drop out
            // of the frame, so image rows answer the pointer only around it.
            let pixels: Vec<Rect> = app.inline_images.pixel_areas().collect();
            if pixels.iter().any(|area| area.intersects(visible)) {
                for position in visible.positions() {
                    if !pixels.iter().any(|area| area.contains(position)) {
                        paint_hover(frame, Rect::new(position.x, position.y, 1, 1));
                    }
                }
            } else {
                paint_hover(frame, visible);
            }
            break;
        }
    }
}

fn render_delete_confirmation(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(request) = app.pending_delete.clone() else {
        return;
    };
    // Closing a conversation saves it first, so it stays resumable and the
    // prompt must not claim the loss is permanent.
    let closing = request.action == Action::AgentClose;
    let (title, verb, consequence, confirm, tone) = if closing {
        (
            crate::i18n::t("dialog.close.title"),
            crate::i18n::t("dialog.close.verb"),
            crate::i18n::t("dialog.close.body"),
            crate::i18n::t("dialog.close.confirm"),
            ButtonTone::Primary,
        )
    } else {
        (
            crate::i18n::t("dialog.delete.title"),
            crate::i18n::t("dialog.delete.verb"),
            crate::i18n::t("dialog.delete.body"),
            crate::i18n::t("dialog.delete.confirm"),
            ButtonTone::Danger,
        )
    };
    let mut body = vec![
        dialog_question(format!("{verb} {}?", request.label)),
        Line::styled(consequence, theme::muted()),
    ];
    if let Some(error) = request.error {
        body.push(Line::raw(""));
        body.push(Line::styled(
            crate::i18n::t("dialog.error")
                .replace("{action}", &verb.to_lowercase())
                .replace("{error}", &error.to_string()),
            Style::default().fg(theme::ERROR),
        ));
    }
    render_confirm_dialog(
        frame,
        app,
        area,
        Dialog {
            title,
            warning: !closing,
            body,
            buttons: vec![
                DialogButton {
                    label: confirm,
                    tone,
                    focused: true,
                },
                DialogButton {
                    label: crate::i18n::t("dialog.cancel"),
                    tone: ButtonTone::Warning,
                    focused: false,
                },
            ],
        },
        &[Action::DeleteConfirm, Action::DeleteCancel],
    );
}

/// The bold question that opens a confirmation message.
fn dialog_question(text: String) -> Line<'static> {
    Line::styled(
        text,
        Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
    )
}

/// A destructive confirmation whose Enter-activated first button deletes and
/// whose second button cancels with `Esc`.
fn delete_dialog<'a>(title: &'a str, body: Vec<Line<'a>>, confirm: &'a str) -> Dialog<'a> {
    Dialog {
        title,
        warning: true,
        body,
        buttons: vec![
            DialogButton {
                label: confirm,
                tone: ButtonTone::Danger,
                focused: true,
            },
            DialogButton {
                label: crate::i18n::t("dialog.cancel.esc"),
                tone: ButtonTone::Warning,
                focused: false,
            },
        ],
    }
}

/// Renders a picker whose rows select their option index and whose buttons
/// dispatch `actions`, in order.
fn render_picker_dialog(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    dialog: ListDialog<'_>,
    actions: &[Action],
) {
    let targets = dialog.render(frame, area);
    app.modal_card_rect = Some(targets.panel);
    for (index, rect) in targets.rows {
        app.clickable_regions
            .push(ClickableRegion::RequirementFilterOption { rect, index });
    }
    register_dialog_buttons(app, targets.buttons, actions);
}

/// Style of a picker row; the dialog extends the selected fill across the row.
fn picker_row_style(selected: bool) -> Style {
    if selected {
        theme::selected_row(true)
    } else {
        Style::default()
    }
}

fn picker_row(text: String, selected: bool) -> Line<'static> {
    Line::styled(text, picker_row_style(selected))
}

/// The Enter-activated button of a picker.
fn confirm_button(label: &str) -> DialogButton<'_> {
    DialogButton {
        label,
        tone: ButtonTone::Primary,
        focused: true,
    }
}

/// The `Esc` way out of a picker.
fn cancel_button(label: &str) -> DialogButton<'_> {
    DialogButton {
        label,
        tone: ButtonTone::Warning,
        focused: false,
    }
}

/// A picker edit that keeps the dialog open.
fn neutral_button(label: &str) -> DialogButton<'_> {
    DialogButton {
        label,
        tone: ButtonTone::Neutral,
        focused: false,
    }
}

fn register_dialog_buttons(app: &mut App, rects: Vec<Rect>, actions: &[Action]) {
    for (rect, action) in rects.into_iter().zip(actions) {
        // A clipped button is not an actionable control.
        if !rect.is_empty() {
            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect,
                action: action.clone(),
                modal: true,
            });
        }
    }
}

/// Renders a confirmation dialog whose buttons dispatch `actions`, in order.
fn render_confirm_dialog(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    dialog: Dialog<'_>,
    actions: &[Action],
) {
    let rects = dialog.render(frame, area);
    register_dialog_buttons(app, rects, actions);
}

fn render_main_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    match app.active_tab {
        MainTab::MindMap => render_mindmap_panel(frame, app, area),
        MainTab::Explore => render_explore_panel(frame, app, area),
        MainTab::Ai => render_ai_panel(frame, app, area),
        MainTab::Requirements => render_requirements_panel(frame, app, area),
        MainTab::TestPoints => render_test_points_panel(frame, app, area),
    }
}

/// Shared left-pane band for Explore / Requirements / Test Points / AI:
/// 30% of the row, never under 42 columns (titles stay readable) and never
/// over 60 (the main content keeps the extra width). MindMap keeps its own
/// 55/45 split because the tree is the primary surface there.
const LEFT_PANE_MIN_WIDTH: u16 = 42;
/// Beyond this width extra columns belong to the main content, not the list.
const LEFT_PANE_MAX_WIDTH: u16 = 60;
/// The AI sidebar never appears when the conversation would fall below this.
const AI_CHAT_MIN_WIDTH: u16 = 50;

/// Resolve the shared left-pane width (30%, clamped to 42..=60).
pub(super) fn left_pane_width(panel_width: u16) -> u16 {
    (panel_width * 3 / 10).clamp(LEFT_PANE_MIN_WIDTH, LEFT_PANE_MAX_WIDTH)
}

/// Resolve the AI sidebar width, or `None` when the chat should own the row.
pub(super) fn ai_sidebar_width(panel_width: u16) -> Option<u16> {
    let width = left_pane_width(panel_width);
    (panel_width.saturating_sub(width) >= AI_CHAT_MIN_WIDTH).then_some(width)
}

/// Renders the AI chat panel with a sidebar for agent management.
fn render_ai_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 25 || area.height < 3 {
        return;
    }

    let sidebar_width = if app.ai_sidebar_collapsed {
        None
    } else {
        ai_sidebar_width(area.width)
    };
    let Some(sidebar_width) = sidebar_width else {
        // Layout: chat only, keeping the full reading width.
        app.ai_sidebar_visible = false;
        app.ai_sidebar_focused = false;
        // The chat is the only pane here, so it always owns the keyboard; passing
        // `false` would also dim a focused composer.
        chat::render_chat_panel(frame, app, area, true, chat::ChatPlacement::Tab);
        return;
    };
    app.ai_sidebar_visible = true;
    // Layout: sidebar (left) + main panel (right)
    let [sidebar_area, main_area] =
        Layout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(10)]).areas(area);

    chat::render_agent_sidebar(frame, app, sidebar_area);
    // The sidebar paints its own focus outline when it owns the keyboard.
    chat::render_chat_panel(
        frame,
        app,
        main_area,
        !app.ai_sidebar_focused,
        chat::ChatPlacement::Tab,
    );
}

/// Renders the MindMap layout: tree (55%) + right panel (45%) split vertically into
/// scenario preview (top) and AI chat (bottom).
fn render_mindmap_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.mindmap_ai_panel_visible && area.width >= 30 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        let tree_focused = app.mindmap_focus == MindMapFocus::Main;
        render_tree_panel(frame, app, cols[0], tree_focused);

        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(cols[1]);

        let preview_focused = app.mindmap_focus == MindMapFocus::AiPanel;
        render_mindmap_scenario_preview(frame, app, right_chunks[0], preview_focused);
        render_mindmap_agent_chat(frame, app, right_chunks[1]);
    } else {
        render_tree_panel(frame, app, area, true);
    }
}

/// Two columns with supporting details below the main content; tiny terminals
/// show only the focused pane, preserving all keyboard navigation targets.
/// Left width matches the AI tab band via [`left_pane_width`].
fn compact_panes(area: Rect, focus: usize) -> [Rect; 3] {
    if area.width < 70 || area.height < 12 {
        let mut panes = [Rect::default(); 3];
        panes[focus.min(2)] = area;
        return panes;
    }
    let [left, right] = Layout::horizontal([
        Constraint::Length(left_pane_width(area.width)),
        Constraint::Min(10),
    ])
    .areas(area);
    let [main, detail] =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(right);
    [left, main, detail]
}

/// Explore keeps the Feature column on the shared left-pane band, gives the
/// Scenario list only the rows it needs (capped at 40%), and splits the rest
/// into Steps and Run Details. Tiny terminals still show only the focused pane.
#[cfg_attr(not(test), allow(dead_code))]
fn explore_panes(area: Rect, focus: usize, scenario_rows: usize) -> [Rect; 4] {
    explore_panes_with_details(area, focus, scenario_rows, false)
}

fn explore_panes_with_details(
    area: Rect,
    focus: usize,
    scenario_rows: usize,
    details_collapsed: bool,
) -> [Rect; 4] {
    if area.width < 70 || area.height < 12 {
        let mut panes = [Rect::default(); 4];
        panes[focus.min(3)] = area;
        return panes;
    }
    let [left, right] = Layout::horizontal([
        Constraint::Length(left_pane_width(area.width)),
        Constraint::Min(10),
    ])
    .areas(area);
    let max_scenario = (right.height * 40 / 100).max(3);
    // Every row needs a slot: two borders, the in-pane action strip, and one
    // row per scenario. Undercounting hides the last scenario behind the strip.
    let desired = (scenario_rows.min(u16::MAX as usize) as u16)
        .saturating_add(3)
        .clamp(4, max_scenario);
    let scenario_height = desired.min(right.height.saturating_sub(3).max(3));
    let [scenario, bottom] =
        Layout::vertical([Constraint::Length(scenario_height), Constraint::Min(3)]).areas(right);
    if details_collapsed {
        return [left, scenario, bottom, Rect::default()];
    }
    let min_details = 24.min(bottom.width / 2);
    let steps_width = 38
        .min(bottom.width.saturating_sub(min_details))
        .max(bottom.width / 4);
    let [steps, details] =
        Layout::horizontal([Constraint::Length(steps_width), Constraint::Min(10)]).areas(bottom);
    [left, scenario, steps, details]
}

fn render_explore_details_toggle_chip(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    expanded: bool,
    show_key_hint: bool,
) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    let label = match (expanded, show_key_hint) {
        (true, true) => crate::i18n::t("explore.details.hide.key"),
        (true, false) => crate::i18n::t("explore.details.hide"),
        (false, true) => crate::i18n::t("explore.details.show.key"),
        (false, false) => crate::i18n::t("explore.details.show"),
    };
    let width = label.width() as u16;
    if width == 0 {
        return;
    }
    let chip_x = area.right().saturating_sub(width + 1);
    if chip_x <= area.x + 1 {
        return;
    }
    let rect = Rect::new(chip_x, area.y, width, 1);
    app.clickable_regions.push(ClickableRegion::ActionButton {
        rect,
        action: Action::ExploreToggleDetails,
        modal: false,
    });
    let mut style = Style::default().fg(theme::PRIMARY).bg(theme::CHAT_CHIP_BG);
    if app
        .mouse_position
        .is_some_and(|(x, y)| rect.contains(ratatui::layout::Position::new(x, y)))
    {
        style = style.bg(theme::HOVER_BG);
    }
    frame.render_widget(Paragraph::new(Line::styled(label, style)), rect);
}

fn render_explore_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.explore_edit_mode {
        if app.explore_details_collapsed {
            render_editor_panel(frame, app, area, false);
            render_explore_details_toggle_chip(frame, app, area, false, false);
        } else {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            render_editor_panel(frame, app, cols[0], false);
            render_run_details(frame, app, cols[1], false);
        }
        return;
    }
    let scenario_rows = explore_scenario_count(app);
    let cols = explore_panes_with_details(
        area,
        app.explore_focus.index(),
        scenario_rows,
        app.explore_details_collapsed,
    );
    if !app.explore_details_collapsed {
        app.ensure_detail_shots();
    }

    render_explore_features(frame, app, cols[0]);
    render_explore_scenarios(frame, app, cols[1]);
    render_explore_steps(frame, app, cols[2]);
    if app.explore_details_collapsed {
        render_explore_details_toggle_chip(frame, app, cols[2], false, true);
    } else {
        render_run_details(
            frame,
            app,
            cols[3],
            app.explore_focus == ColumnFocus::Details,
        );
    }
    // Scenario/step bindings still use logical rows. Feature bindings are
    // emitted directly for the viewport and already have screen coordinates.
    let views = app.explore_viewports;
    app.clickable_regions.retain_mut(|region| {
        let (column, row) = match region {
            ClickableRegion::ExploreScenario { row_y, .. } => (1, row_y),
            ClickableRegion::ExploreStep { row_y, .. } => (2, row_y),
            _ => return true,
        };
        let view = views[column];
        let logical_row = row.saturating_sub(view.rect.y) as usize;
        if logical_row < view.offset || logical_row >= view.offset + view.rect.height as usize {
            return false;
        }
        *row = view.rect.y + (logical_row - view.offset) as u16;
        true
    });
}

fn explore_select_style(focused: bool) -> Style {
    theme::selected_row(focused)
}

/// Capture marker for the Explore panes; the MindMap tree uses the same shape.
fn capture_badge(count: usize) -> Span<'static> {
    Span::styled(
        format!(" ●{count}"),
        Style::default()
            .fg(theme::SUCCESS)
            .add_modifier(Modifier::DIM),
    )
}

fn explore_block<'a>(title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
    theme::panel_block(title, focused)
}

/// `/query` spans for a search title, with a block caret at the cursor while
/// the field is active. Uses the same caret colors as the `:` prompt and form
/// fields so an active search reads as a focused input rather than static text.
fn search_query_spans(query: &crate::authoring_tab::LineInput, active: bool) -> Vec<Span<'static>> {
    let (before, after) = query.split_at_cursor();
    let mut spans = vec![Span::raw("/".to_string())];
    if active {
        if let Some(selection) = query.selection() {
            let text = query.text();
            let start = selection.start.min(text.chars().count());
            let end = selection.end.min(text.chars().count()).max(start);
            let before_sel: String = text.chars().take(start).collect();
            let selected: String = text.chars().skip(start).take(end - start).collect();
            let after_sel: String = text.chars().skip(end).collect();
            spans.push(Span::raw(before_sel));
            spans.push(Span::styled(
                selected,
                Style::default()
                    .fg(theme::FG)
                    .bg(theme::EDITOR_SELECTION_BG),
            ));
            spans.push(Span::raw(after_sel));
        } else {
            spans.push(Span::raw(before.to_string()));
            let mut rest = after.chars();
            let under = rest.next().map_or_else(|| " ".to_string(), String::from);
            spans.push(Span::styled(
                under,
                Style::default().fg(theme::CURSOR_FG).bg(theme::CURSOR_BG),
            ));
            spans.push(Span::raw(rest.as_str().to_string()));
        }
    } else {
        spans.push(Span::raw(format!("{before}{after}")));
    }
    spans
}

/// Render one Explore column with an independent, two-row horizontal scrollbar.
fn render_explore_lines<'a>(
    frame: &mut Frame<'_>,
    app: &mut App,
    column: ColumnFocus,
    inner: Rect,
    lines: Vec<Line<'a>>,
) -> Rect {
    render_explore_lines_selected(frame, app, column, inner, lines, None)
}

fn render_explore_lines_selected<'a>(
    frame: &mut Frame<'_>,
    app: &mut App,
    column: ColumnFocus,
    inner: Rect,
    lines: Vec<Line<'a>>,
    selected_row: Option<usize>,
) -> Rect {
    let content_width = lines.iter().map(Line::width).max().unwrap_or(0);
    render_explore_viewport(
        frame,
        app,
        column,
        inner,
        lines.len(),
        content_width,
        selected_row,
        |_, range| {
            lines
                .into_iter()
                .skip(range.start)
                .take(range.len())
                .collect()
        },
    )
}

// Build styled lines only after the viewport is known. Feature catalogs can be
// huge, and resolving Git status may touch the filesystem for each visible row.
#[allow(clippy::too_many_arguments)]
fn render_explore_viewport<'a>(
    frame: &mut Frame<'_>,
    app: &mut App,
    column: ColumnFocus,
    inner: Rect,
    row_count: usize,
    content_width: usize,
    selected_row: Option<usize>,
    render_rows: impl FnOnce(&App, std::ops::Range<usize>) -> Vec<Line<'a>>,
) -> Rect {
    let base_height = inner.height as usize;
    let preliminary_v_overflow = row_count > base_height && inner.width >= 3;
    let prelim_width = if preliminary_v_overflow {
        inner.width.saturating_sub(1) as usize
    } else {
        inner.width as usize
    };
    let h_overflow = content_width > prelim_width && inner.height >= 2;
    let body_height = if h_overflow {
        inner.height - 1
    } else {
        inner.height
    };
    let v_overflow = row_count > body_height as usize && inner.width >= 3;
    let content_width_cols = if v_overflow {
        inner.width - 1
    } else {
        inner.width
    };
    let content_area = Rect::new(inner.x, inner.y, content_width_cols, body_height);
    let viewport_width = content_area.width as usize;
    let max_scroll = content_width.saturating_sub(viewport_width);
    let offset = app.explore_horizontal_scroll[column.index()].min(max_scroll);
    app.explore_horizontal_scroll[column.index()] = offset;
    let selected = selected_row.unwrap_or_else(|| match column {
        ColumnFocus::Feature => app.explore_selected_feature_row().unwrap_or(0),
        ColumnFocus::Scenario => app.explore_selected_scenario,
        ColumnFocus::Step => app.explore_selected_step,
        // Run Details has no list viewport; it never reaches this renderer.
        ColumnFocus::Details => 0,
    });
    let view = &mut app.explore_viewports[column.index()];
    let owner = match column {
        ColumnFocus::Feature => (0, None, None),
        ColumnFocus::Scenario => (
            app.explore_selected_feature,
            app.explore_selected_deleted_feature,
            None,
        ),
        ColumnFocus::Step => (
            app.explore_selected_feature,
            app.explore_selected_deleted_feature,
            Some(app.explore_selected_scenario),
        ),
        ColumnFocus::Details => (0, None, None),
    };
    if view.last_owner != Some(owner) {
        view.offset = 0;
        view.last_selected = None;
        view.last_owner = Some(owner);
    }
    view.rect = content_area;
    view.max_vertical = row_count.saturating_sub(content_area.height as usize);
    view.max_horizontal = max_scroll;
    view.offset = view.offset.min(view.max_vertical);
    if view.last_selected != Some(selected) {
        if selected < view.offset {
            view.offset = selected;
        } else if selected >= view.offset + content_area.height as usize {
            view.offset = selected
                .saturating_add(1)
                .saturating_sub(content_area.height as usize)
                .min(view.max_vertical);
        }
        view.last_selected = Some(selected);
    }
    let v_offset = view.offset;
    // Pad and render only the viewport, rather than allocating full-width
    // backgrounds for every file in a large project.
    let range = v_offset..(v_offset + content_area.height as usize).min(row_count);
    let visible_lines = render_rows(app, range);
    frame.render_widget(
        Paragraph::new(Text::from(theme::popup_rows(
            visible_lines,
            content_width.max(viewport_width).min(u16::MAX as usize) as u16,
        )))
        .scroll((0, offset.min(u16::MAX as usize) as u16)),
        content_area,
    );

    if v_overflow {
        let vbar = Rect::new(content_area.right(), content_area.y, 1, content_area.height);
        render_vertical_scrollbar(
            frame,
            app,
            crate::app::VerticalScrollTarget::ExploreColumn(column),
            vbar,
            row_count,
            v_offset,
        );
    }

    if h_overflow {
        let scrollbar = Rect::new(
            inner.x,
            inner.bottom().saturating_sub(1),
            content_area.width,
            1,
        );
        let bar = crate::app::ScrollbarBar {
            rect: scrollbar,
            axis: theme::ScrollAxis::Horizontal,
            total: content_width,
        };
        let grabbed = app
            .explore_horizontal_scroll_grab
            .is_some_and(|(dragged, _)| dragged == column);
        let state = crate::app::scrollbar_state(app.mouse_position, &bar, grabbed);
        let line = theme::scrollbar_lines(
            content_width,
            viewport_width,
            offset,
            theme::ScrollAxis::Horizontal,
            state,
        )
        .into_iter()
        .next()
        .unwrap_or_default();
        frame.render_widget(Paragraph::new(vec![line]), scrollbar);
        app.clickable_regions
            .push(ClickableRegion::ExploreHorizontalScrollbar {
                column,
                rect: scrollbar,
                content_width,
                viewport_width,
            });
    }
    content_area
}

fn git_file_status_style(status: teshi_core::git::FileGitStatus) -> Style {
    use teshi_core::git::{DiffStatus, FileGitStatus};
    theme::diff_style(match status {
        FileGitStatus::Added | FileGitStatus::Untracked => DiffStatus::Added,
        FileGitStatus::Modified => DiffStatus::Modified,
        FileGitStatus::Deleted => DiffStatus::Deleted,
        FileGitStatus::Unmodified => DiffStatus::Unchanged,
    })
    .add_modifier(Modifier::BOLD)
}

fn git_diff_status_style(status: teshi_core::git::DiffStatus) -> Style {
    theme::diff_style(status).add_modifier(Modifier::BOLD)
}

fn truncate_string_to_cols(text: &str, max_cols: u16) -> String {
    let max = max_cols as usize;
    let mut budget = max;
    let mut out = String::new();
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if w == 0 {
            out.push(ch);
            continue;
        }
        if w > budget {
            break;
        }
        out.push(ch);
        budget -= w;
    }
    out
}

fn input_with_cursor(text: &str, cursor: usize, width: u16) -> String {
    let chars: Vec<_> = text
        .chars()
        .map(|ch| match ch {
            '\n' => '↵',
            '\t' => '→',
            ch => ch,
        })
        .collect();
    let cursor = cursor.min(chars.len());
    let mut start = 0;
    let mut prefix_width: usize = chars[..cursor]
        .iter()
        .map(|ch| ch.width().unwrap_or(0))
        .sum();
    while start < cursor && prefix_width + 1 > usize::from(width) {
        prefix_width = prefix_width.saturating_sub(chars[start].width().unwrap_or(0));
        start += 1;
    }
    let mut display: String = chars[start..cursor].iter().collect();
    display.push('▏');
    display.extend(chars[cursor..].iter());
    truncate_string_to_cols(&display, width)
}

/// Keeps both ends of a tag that cannot fit one line, with an ellipsis between.
fn truncate_middle(text: &str, max_cols: usize) -> String {
    let total: usize = text.chars().map(|ch| ch.width().unwrap_or(0)).sum();
    if total <= max_cols {
        return text.to_string();
    }
    if max_cols < 2 {
        return "…".to_string();
    }
    let head_budget = (max_cols - 1) / 2;
    let tail_budget = max_cols - 1 - head_budget;
    let mut head = String::new();
    let mut head_w = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if head_w + w > head_budget {
            break;
        }
        head.push(ch);
        head_w += w;
    }
    let mut tail = String::new();
    let mut tail_w = 0;
    for ch in text.chars().rev() {
        let w = ch.width().unwrap_or(0);
        if tail_w + w > tail_budget {
            break;
        }
        tail.insert(0, ch);
        tail_w += w;
    }
    format!("{head}…{tail}")
}

/// Truncates a styled title from the right, keeping per-tag colors intact.
fn truncate_spans(spans: Vec<Span<'static>>, max_cols: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut remaining = max_cols;
    for span in spans {
        let width = UnicodeWidthStr::width(span.content.as_ref());
        if width <= remaining {
            remaining -= width;
            out.push(span);
        } else {
            let text = truncate_string_to_cols(&span.content, remaining as u16);
            if !text.is_empty() {
                out.push(Span::styled(text, span.style));
            }
            break;
        }
    }
    out
}

/// Flow-wraps the tag pool into at most two rows inside the Features pane.
fn render_explore_tags(frame: &mut Frame<'_>, app: &mut App, area: Rect, tags: &[String]) {
    // One column at the right is reserved for the scroll arrows.
    let content_width = area.width.saturating_sub(1) as usize;
    if content_width == 0 || area.height == 0 {
        return;
    }
    let mut lines: Vec<Vec<(usize, usize, String)>> = vec![Vec::new()];
    let mut line_width = 0usize;
    for (index, tag) in tags.iter().enumerate() {
        let label = truncate_middle(tag, content_width);
        let width = UnicodeWidthStr::width(label.as_str());
        let gap = usize::from(line_width > 0);
        if line_width > 0 && line_width + gap + width > content_width {
            lines.push(Vec::new());
            line_width = 0;
        }
        let col = line_width + usize::from(line_width > 0);
        // `lines` always starts with one empty row and only grows via push.
        if let Some(line) = lines.last_mut() {
            line.push((index, col, label));
        }
        line_width = col + width;
    }
    let line_count = lines.len();
    let visible_rows = area.height as usize;
    let max_scroll = line_count.saturating_sub(visible_rows);
    app.feature_tags.scroll_line = app.feature_tags.scroll_line.min(max_scroll);
    let scroll = app.feature_tags.scroll_line;

    let mut slots = Vec::new();
    for (line, entries) in lines.iter().enumerate() {
        for (index, col, _) in entries {
            slots.push(crate::feature_tags::TagSlot {
                index: *index,
                line,
                col: *col,
            });
        }
    }
    app.feature_tags.slots = slots;
    app.feature_tags.line_count = line_count;
    app.feature_tags.visible_rows = visible_rows;
    app.feature_tags.area = Some(area);

    let hover = app.mouse_position;
    for (row, line_index) in (scroll..(scroll + visible_rows).min(line_count)).enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut last_col = 0usize;
        for (index, col, label) in &lines[line_index] {
            if *col > last_col {
                spans.push(Span::raw(" ".repeat(*col - last_col)));
            }
            let width = UnicodeWidthStr::width(label.as_str()) as u16;
            let rect = Rect::new(area.x + *col as u16, area.y + row as u16, width, 1);
            app.clickable_regions.push(ClickableRegion::ExploreTag {
                index: *index,
                rect,
            });
            let selected = app.feature_tags.selected.contains(tags[*index].as_str());
            let highlighted = (app.feature_tags.focused && app.feature_tags.cursor == *index)
                || hover.is_some_and(|(x, y)| rect.contains((x, y).into()));
            let style = if selected {
                Style::default()
                    .bg(crate::feature_tags::tag_accent(&tags[*index]))
                    .fg(theme::CHIP_FG)
                    .add_modifier(Modifier::BOLD)
            } else if highlighted {
                // Hover/focus tint only; never underline (tui-style-guide rule 4).
                Style::default().fg(theme::FG).bg(theme::HOVER_BG)
            } else {
                Style::default().fg(theme::MUTED)
            };
            spans.push(Span::styled(label.clone(), style));
            last_col = *col + width as usize;
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(area.x, area.y + row as u16, content_width as u16, 1),
        );
    }

    let arrow_x = area.x + content_width as u16;
    if scroll > 0 {
        let rect = Rect::new(arrow_x, area.y, 1, 1);
        app.feature_tags.up_hit = Some(rect);
        app.clickable_regions
            .push(ClickableRegion::ExploreTagScroll { down: false, rect });
        frame.render_widget(Paragraph::new("↑"), rect);
    }
    if scroll + visible_rows < line_count {
        let rect = Rect::new(arrow_x, area.y + visible_rows as u16 - 1, 1, 1);
        app.feature_tags.down_hit = Some(rect);
        app.clickable_regions
            .push(ClickableRegion::ExploreTagScroll { down: true, rect });
        frame.render_widget(Paragraph::new("↓"), rect);
    }
}

fn render_explore_features(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use crate::feature_browser::Row;
    let focused = app.explore_focus == ColumnFocus::Feature;
    let searching = app.feature_browser.searching;
    let mut title_spans: Vec<Span<'static>> =
        if searching || !app.feature_browser.query.text().is_empty() {
            let mut spans = vec![Span::raw(
                crate::i18n::t("features.title.prefix").to_string(),
            )];
            spans.extend(search_query_spans(&app.feature_browser.query, searching));
            spans
        } else if let Some(catalog) = &app.lazy_features {
            vec![Span::raw(
                crate::i18n::t("features.title.count")
                    .replace("{count}", &catalog.paths.len().to_string()),
            )]
        } else {
            vec![Span::raw(
                crate::i18n::t("features.title.search").to_string(),
            )]
        };
    let filter_active = app.filter_active();
    if filter_active {
        title_spans.push(Span::raw(" · ".to_string()));
        for (index, tag) in app.feature_tags.selected.iter().enumerate() {
            if index > 0 {
                title_spans.push(Span::raw(" ".to_string()));
            }
            title_spans.push(Span::styled(
                tag.clone(),
                Style::default()
                    .fg(crate::feature_tags::tag_accent(tag))
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    let badge_x = area.right().saturating_sub(2);
    let mut title = if filter_active {
        let max_title = badge_x.saturating_sub(area.x + 1).saturating_sub(1) as usize;
        Line::from(truncate_spans(title_spans, max_title))
    } else {
        Line::from(title_spans)
    };
    if app.features_loading() {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let frame = (START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis()
            / 100) as usize;
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        title.spans.push(Span::raw(format!(
            " {} {}",
            spinner[frame % spinner.len()],
            crate::i18n::t("features.loading")
        )));
    }
    let title_width = title.width() as u16;
    let block = explore_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let search_has_text = !app.feature_browser.query.text().is_empty();
    let mut title_right_limit = area.right().saturating_sub(1);
    if filter_active && badge_x > area.x {
        let rect = Rect::new(badge_x, area.y, 1, 1);
        app.feature_tags.clear_hit = Some(rect);
        app.clickable_regions
            .push(ClickableRegion::ExploreTagClear { rect });
        let mut style = Style::default()
            .fg(theme::PRIMARY)
            .add_modifier(Modifier::BOLD);
        if app
            .mouse_position
            .is_some_and(|(x, y)| rect.contains((x, y).into()))
        {
            style = style.add_modifier(Modifier::REVERSED);
        }
        frame.render_widget(Paragraph::new(Line::from(Span::styled("✗", style))), rect);
        title_right_limit = badge_x.saturating_sub(1);
    }
    if (searching || search_has_text) && title_right_limit > area.x + 4 {
        let clear_rect = Rect::new(title_right_limit.saturating_sub(3), area.y, 3, 1);
        app.clickable_regions
            .push(ClickableRegion::ExploreSearchClear { rect: clear_rect });
        let (_, clear_style) = theme::action_chip_styles(theme::ActionTone::Warning);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" ✗ ", clear_style))),
            clear_rect,
        );
        title_right_limit = clear_rect.x.saturating_sub(1);
    }
    if searching && title_right_limit > area.x + 4 {
        let confirm_rect = Rect::new(title_right_limit.saturating_sub(3), area.y, 3, 1);
        app.clickable_regions
            .push(ClickableRegion::ExploreSearchConfirm { rect: confirm_rect });
        let (_, confirm_style) = theme::action_chip_styles(theme::ActionTone::Success);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" ✓ ", confirm_style))),
            confirm_rect,
        );
        title_right_limit = confirm_rect.x.saturating_sub(1);
    }
    let trigger_w = title_width.min(title_right_limit.saturating_sub(area.x + 1));
    if trigger_w > 0 {
        let trigger_rect = Rect::new(area.x + 1, area.y, trigger_w, 1);
        app.clickable_regions
            .push(ClickableRegion::ExploreSearchTrigger { rect: trigger_rect });
    }
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let tags = app.feature_tags.index.all_tags();
    let tag_rows = if tags.is_empty() || inner.height < 4 {
        0
    } else if inner.height >= 8 {
        crate::feature_tags::MAX_TAG_ROWS
    } else {
        1
    };
    let separator = usize::from(tag_rows > 0);
    let tree_height = (inner.height as usize)
        .saturating_sub(tag_rows + separator)
        .max(1);
    let tree_inner = Rect::new(inner.x, inner.y, inner.width, tree_height as u16);
    if tag_rows > 0 {
        let tag_area = Rect::new(
            inner.x,
            inner.y + tree_height as u16 + separator as u16,
            inner.width,
            tag_rows as u16,
        );
        render_explore_tags(frame, app, tag_area, &tags);
    }
    let rows = app.feature_browser_rows();
    if rows.is_empty() {
        // Search/filter empty-states only apply when the catalog actually has
        // files; a truly empty project must keep "(no features)" even after the
        // runner types `/.feature` to force an external refresh.
        let has_any_files = app
            .lazy_features
            .as_ref()
            .map(|catalog| !catalog.paths.is_empty())
            .unwrap_or_else(|| !app.project.features.is_empty())
            || !app.git_status.deleted.is_empty();
        let searching = !app.feature_browser.query.text().is_empty();
        let text = if app.features_loading() {
            crate::i18n::t("features.empty.scanning")
        } else if !has_any_files {
            app.lazy_features
                .as_ref()
                .and_then(|state| state.scan_error.as_deref())
                .unwrap_or(crate::i18n::t("features.empty.none"))
        } else if filter_active && !app.feature_tags.index.is_complete() {
            crate::i18n::t("features.empty.indexing")
        } else if filter_active && searching {
            crate::i18n::t("features.empty.tags_search")
        } else if filter_active {
            crate::i18n::t("features.empty.tags")
        } else if searching {
            crate::i18n::t("features.empty.search")
        } else {
            app.lazy_features
                .as_ref()
                .and_then(|state| state.scan_error.as_deref())
                .unwrap_or(crate::i18n::t("features.empty.none"))
        };
        frame.render_widget(Paragraph::new(text), tree_inner);
        return;
    }
    let selected = app.explore_selected_feature_row();
    let content = render_explore_viewport(
        frame,
        app,
        ColumnFocus::Feature,
        tree_inner,
        rows.len(),
        app.feature_browser_width(),
        selected,
        |app, range| {
            rows.iter()
                .enumerate()
                .skip(range.start)
                .take(range.len())
                .map(|(row, item)| {
                    let label = match item {
                        Row::Directory(path, depth) => format!(
                            " {}{} {}/",
                            "  ".repeat(*depth),
                            if app.feature_directory_collapsed(path) {
                                "▸"
                            } else {
                                "▾"
                            },
                            path.rsplit('/').next().unwrap_or(path)
                        ),
                        Row::File(index, label, deleted, _) => {
                            let status = if *deleted {
                                app.git_status.deleted[*index].file_status
                            } else {
                                let path = match &app.lazy_features {
                                    Some(catalog) => catalog.paths.get(*index),
                                    None => app
                                        .project
                                        .features
                                        .get(*index)
                                        .map(|feature| &feature.file_path),
                                };
                                path.map_or(teshi_core::git::FileGitStatus::Unmodified, |path| {
                                    app.git_status.file_status_at_path(path)
                                })
                            };
                            return Line::from(vec![
                                Span::styled(
                                    format!(" {:<2} ", status.marker()),
                                    git_file_status_style(status),
                                ),
                                Span::styled(
                                    label.clone(),
                                    if selected == Some(row) {
                                        explore_select_style(focused)
                                    } else {
                                        Style::default()
                                    },
                                ),
                            ]);
                        }
                    };
                    Line::styled(
                        label,
                        if selected == Some(row) {
                            explore_select_style(focused)
                        } else {
                            Style::default()
                        },
                    )
                })
                .collect()
        },
    );
    let offset = app.explore_viewports[ColumnFocus::Feature.index()].offset;
    for (row, item) in rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(content.height as usize)
    {
        let region = match item {
            Row::File(index, _, true, _) => ClickableRegion::ExploreDeletedFeature {
                deleted_idx: *index,
                row_y: content.y + (row - offset) as u16,
                col_x: content.x,
                col_right: content.right(),
            },
            _ => ClickableRegion::ExploreFeature {
                feature_idx: row,
                row_y: content.y + (row - offset) as u16,
                col_x: content.x,
                col_right: content.right(),
            },
        };
        app.clickable_regions.push(region);
    }
}

fn render_explore_scenarios(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let explore_actions = [
        ActionItem::new(
            crate::i18n::t("explore.run.key"),
            crate::i18n::t("explore.run"),
            Action::RunScenario,
            theme::ActionTone::Success,
        ),
        ActionItem::new(
            crate::i18n::t("explore.detail.key"),
            crate::i18n::t("explore.detail"),
            Action::FocusRunDetails,
            theme::ActionTone::Warning,
        ),
        ActionItem::new(
            crate::i18n::t("explore.edit.key"),
            crate::i18n::t("explore.edit"),
            Action::EnterEdit,
            theme::ActionTone::Info,
        ),
    ];
    if !app.catalog_selection_ready() {
        let block = explore_block(crate::i18n::t("pane.feature_content"), false);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let (list_area, action_area) = split_pane_action_row(inner);
        frame.render_widget(
            Paragraph::new(
                app.lazy_features
                    .as_ref()
                    .and_then(|state| state.error.as_deref())
                    .unwrap_or(crate::i18n::t("scenarios.empty.select")),
            )
            .wrap(ratatui::widgets::Wrap { trim: false }),
            list_area,
        );
        if let Some(action_area) = action_area {
            render_chip_action_bar(
                frame,
                &mut app.clickable_regions,
                action_area,
                &explore_actions,
                false,
            );
        }
        return;
    }
    let focused = app.explore_focus == ColumnFocus::Scenario;
    let scenarios_title = explore_scenarios_title(app);
    let run_summary = explore_run_summary_label(app);
    let block = explore_block(scenarios_title.as_str(), focused).title_top(
        Line::from(Span::styled(run_summary, Style::default().fg(theme::MUTED))).right_aligned(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (list_area, action_area) = split_pane_action_row(inner);

    if let Some(deleted_idx) = app.explore_selected_deleted_feature {
        let deleted_scenarios: Vec<String> = app
            .git_status
            .deleted
            .get(deleted_idx)
            .map(|view| {
                view.deleted_scenarios()
                    .map(|scenario| scenario.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut lines = Vec::new();
        if deleted_scenarios.is_empty() {
            lines.push(Line::styled(
                crate::i18n::t("scenarios.deleted"),
                Style::default().fg(theme::MUTED),
            ));
        } else {
            for (scenario_idx, name) in deleted_scenarios.iter().enumerate() {
                let selected = scenario_idx == app.explore_selected_scenario;
                let mut line = Line::from(vec![
                    Span::styled(
                        "- ",
                        git_diff_status_style(teshi_core::git::DiffStatus::Deleted),
                    ),
                    Span::styled(
                        format!("Scenario: {name}"),
                        if selected {
                            explore_select_style(focused)
                        } else {
                            Style::default().fg(theme::ERROR)
                        },
                    ),
                ]);
                line = pad_line_to_width(
                    line,
                    list_area.width,
                    if selected {
                        explore_select_style(focused)
                    } else {
                        Style::default()
                    },
                );
                lines.push(line);
            }
        }
        let content_area =
            render_explore_lines(frame, app, ColumnFocus::Scenario, list_area, lines);
        for (scenario_idx, _name) in deleted_scenarios.iter().enumerate() {
            app.clickable_regions
                .push(ClickableRegion::ExploreScenario {
                    scenario_idx,
                    row_y: content_area.y + scenario_idx as u16,
                    col_x: content_area.x,
                    col_right: content_area.right(),
                });
        }
        if let Some(action_area) = action_area {
            render_chip_action_bar(
                frame,
                &mut app.clickable_regions,
                action_area,
                &explore_actions,
                false,
            );
        }
        return;
    }

    let normal = Style::default();
    let mut lines: Vec<Line> = Vec::new();

    let visible = app.explore_visible_scenario_indices();
    let scenarios = app
        .project
        .features
        .get(app.explore_selected_feature)
        .map(|f| f.all_scenarios());
    let deleted_scenarios: Vec<_> = if app.filter_active() {
        Vec::new()
    } else {
        app.git_status
            .current_at(app.explore_selected_feature)
            .map(|view| view.deleted_scenarios().cloned().collect())
            .unwrap_or_default()
    };

    if visible.is_empty() && deleted_scenarios.is_empty() {
        lines.push(Line::styled(
            " (no scenarios)",
            Style::default().fg(theme::MUTED),
        ));
    }
    let current_scenario_count = visible.len();
    if let Some(scenarios) = scenarios.as_ref() {
        for &i in &visible {
            let Some(scenario) = scenarios.get(i) else {
                continue;
            };
            let status = app
                .explore_case_status
                .get(&(app.explore_selected_feature, i))
                .copied()
                .unwrap_or(RunStatus::Idle);
            let status_dot = Span::styled("●", theme::status_style(status));
            let tp_ids = app
                .authoring_ui
                .artifacts
                .as_ref()
                .map(|artifacts| {
                    teshi_core::authoring::parse_teshi_tp_tags_for_test_points(
                        &scenario.tags,
                        &artifacts.test_points.test_points,
                    )
                })
                .unwrap_or_else(|| teshi_core::authoring::parse_teshi_tp_tags(&scenario.tags));
            let tp_badge = if tp_ids.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tp_ids.join(","))
            };
            let git_status = app
                .git_status
                .current_at(app.explore_selected_feature)
                .map(|view| view.scenario_status(scenario))
                .unwrap_or_default();
            let git_marker = Span::styled(
                format!("{} ", git_status.marker()),
                git_diff_status_style(git_status),
            );
            let name = Span::styled(format!(" {}{}", scenario.name, tp_badge), normal);
            let mut spans = vec![git_marker, status_dot, name];
            let captures = app.explore_scenario_captures(i);
            if captures > 0 {
                spans.push(capture_badge(captures));
            }
            let mut line = Line::from(spans);
            if i == app.explore_selected_scenario {
                line = apply_line_background(line, explore_select_style(focused));
            }
            let trail = if i == app.explore_selected_scenario {
                explore_select_style(focused)
            } else {
                Style::default()
            };
            line = pad_line_to_width(line, list_area.width, trail);
            lines.push(line);
        }
    }

    for (deleted_idx, deleted) in deleted_scenarios.iter().enumerate() {
        let scenario_idx = current_scenario_count + deleted_idx;
        let selected = scenario_idx == app.explore_selected_scenario;
        let mut line = Line::from(vec![
            Span::styled(
                "- ",
                git_diff_status_style(teshi_core::git::DiffStatus::Deleted),
            ),
            Span::styled(
                format!("Scenario: {}", deleted.name),
                if selected {
                    explore_select_style(focused)
                } else {
                    Style::default().fg(theme::ERROR)
                },
            ),
        ]);
        line = pad_line_to_width(
            line,
            inner.width,
            if selected {
                explore_select_style(focused)
            } else {
                Style::default()
            },
        );
        lines.push(line);
    }

    let selected_position = visible
        .iter()
        .position(|&index| index == app.explore_selected_scenario)
        .unwrap_or(0);
    let content_area = render_explore_lines_selected(
        frame,
        app,
        ColumnFocus::Scenario,
        list_area,
        lines,
        Some(selected_position),
    );

    // Register clickable regions for each scenario row
    if app
        .project
        .features
        .get(app.explore_selected_feature)
        .is_some()
    {
        for (row, &i) in visible.iter().enumerate() {
            app.clickable_regions
                .push(ClickableRegion::ExploreScenario {
                    scenario_idx: i,
                    row_y: content_area.y + row as u16,
                    col_x: content_area.x,
                    col_right: content_area.right(),
                });
        }
        for (deleted_idx, _deleted) in deleted_scenarios.iter().enumerate() {
            let scenario_idx = current_scenario_count + deleted_idx;
            app.clickable_regions
                .push(ClickableRegion::ExploreScenario {
                    scenario_idx,
                    row_y: content_area.y + scenario_idx as u16,
                    col_x: content_area.x,
                    col_right: content_area.right(),
                });
        }
    } else {
        for (deleted_idx, _deleted) in deleted_scenarios.iter().enumerate() {
            let scenario_idx = deleted_idx;
            app.clickable_regions
                .push(ClickableRegion::ExploreScenario {
                    scenario_idx,
                    row_y: content_area.y + scenario_idx as u16,
                    col_x: content_area.x,
                    col_right: content_area.right(),
                });
        }
    }
    if let Some(action_area) = action_area {
        render_chip_action_bar(
            frame,
            &mut app.clickable_regions,
            action_area,
            &explore_actions,
            false,
        );
    }
}

fn explore_scenario_count(app: &App) -> usize {
    if let Some(deleted_idx) = app.explore_selected_deleted_feature {
        return app
            .git_status
            .deleted
            .get(deleted_idx)
            .map(|view| view.deleted_scenarios().count())
            .unwrap_or(0);
    }
    let current_count = app.explore_visible_scenario_indices().len();
    let deleted_count = if app.filter_active() {
        0
    } else {
        app.git_status
            .current_at(app.explore_selected_feature)
            .map(|view| view.deleted_scenarios().count())
            .unwrap_or(0)
    };
    current_count + deleted_count
}

fn explore_scenarios_title(app: &App) -> String {
    crate::i18n::t("scenarios.title").replace("{count}", &explore_scenario_count(app).to_string())
}

/// Tally shown on the right edge of the Scenarios pane title.
///
/// The tab footer no longer carries tab workflows, so the pane border keeps the
/// last run's summary visible instead of dropping it.
fn explore_run_summary_label(app: &App) -> String {
    let total = explore_scenario_count(app);
    match &app.explore_run_summary {
        Some(summary) => {
            let template = if summary.blocked > 0 {
                crate::i18n::t("explore.summary.blocked")
            } else {
                crate::i18n::t("explore.summary")
            };
            template
                .replace("{passed}", &summary.passed.to_string())
                .replace("{total}", &summary.total.to_string())
                .replace("{failed}", &summary.failed.to_string())
                .replace("{blocked}", &summary.blocked.to_string())
        }
        None => crate::i18n::t("explore.summary.idle").replace("{total}", &total.to_string()),
    }
}

/// Gherkin syntax spans for one source line. `keyword_kind` overrides the
/// keyword colour when the caller already resolved it from the AST.
///
/// Diff rows must show the file exactly as written, so the highlighter's
/// keyword gutter padding is removed and any line the highlighter would
/// re-space (tags) is returned unstyled instead.
fn gherkin_syntax_spans(
    line: &str,
    state: &mut StepHighlightState,
    lang: &GherkinLanguage,
    keyword_kind: Option<HighlightKind>,
) -> Vec<Span<'static>> {
    let mut tokens = highlight_line_spans(line, state, lang);
    for token in &mut tokens {
        if matches!(
            token.kind,
            HighlightKind::Given
                | HighlightKind::When
                | HighlightKind::Then
                | HighlightKind::AndBut
        ) {
            token.text = token.text.trim_start().to_string();
            if let Some(kind) = keyword_kind {
                token.kind = kind;
            }
        }
    }
    let text: String = tokens.iter().map(|token| token.text.as_str()).collect();
    if text != line {
        return vec![Span::styled(
            line.to_string(),
            kind_to_style(HighlightKind::Default),
        )];
    }
    tokens
        .into_iter()
        .map(|token| Span::styled(token.text, kind_to_style(token.kind)))
        .collect()
}

/// Spans for a changed Step row; folded doc-string and table lines keep their
/// own Gherkin colours.
fn changed_step_spans(change: &ChangedStep, lang: &GherkinLanguage) -> Vec<Span<'static>> {
    let mut state = StepHighlightState::default();
    let mut lines = change.text.split('\n');
    let mut spans = gherkin_syntax_spans(
        lines.next().unwrap_or_default(),
        &mut state,
        lang,
        change.keyword_kind,
    );
    for line in lines {
        spans.push(Span::styled(" ↳ ", Style::default().fg(theme::MUTED)));
        spans.extend(gherkin_syntax_spans(line, &mut state, lang, None));
    }
    spans
}

/// Shared diff garnish for one Explore row: a `+`/`-`/`~` gutter marker. The
/// caller keeps the Gherkin syntax foregrounds, so a changed row never loses
/// its colours and no row paints a background.
fn decorate_diff_row(
    spans: Vec<Span<'static>>,
    status: teshi_core::git::DiffStatus,
) -> Line<'static> {
    let mut line = Vec::with_capacity(spans.len() + 1);
    line.push(Span::styled(
        format!("{} ", status.marker()),
        theme::diff_style(status).add_modifier(Modifier::BOLD),
    ));
    line.extend(spans);
    Line::from(line)
}

/// Maps the buffer-diff kind of a changed Step onto the Git diff status.
fn change_status(kind: crate::app::ChangeKind) -> teshi_core::git::DiffStatus {
    use teshi_core::git::DiffStatus;
    match kind {
        crate::app::ChangeKind::Added => DiffStatus::Added,
        crate::app::ChangeKind::Modified => DiffStatus::Modified,
        crate::app::ChangeKind::Deleted => DiffStatus::Deleted,
        crate::app::ChangeKind::Unchanged => DiffStatus::Unchanged,
    }
}

/// Gherkin language of the Feature currently open in Explore; removed Features
/// fall back to the `HEAD` content.
fn explore_gherkin_language(app: &App) -> &'static GherkinLanguage {
    let code = app
        .explore_selected_deleted_feature
        .and_then(|index| app.git_status.deleted.get(index))
        .and_then(|view| view.diff.as_ref()?.old_content.as_deref())
        .map(GherkinLanguages::detect_from_content)
        .or_else(|| {
            app.project
                .features
                .get(app.explore_selected_feature)
                .map(|feature| feature.language.as_str())
        })
        .unwrap_or("en");
    GherkinLanguages::global().get(code)
}

fn render_git_diff_line(
    diff_line: &teshi_core::git::GitDiffLine,
    state: &mut StepHighlightState,
    lang: &GherkinLanguage,
) -> Line<'static> {
    let spans = gherkin_syntax_spans(&diff_line.text, state, lang, None);
    decorate_diff_row(spans, diff_line.status)
}

fn render_git_diff_lines(
    lines: &[teshi_core::git::GitDiffLine],
    state: &mut StepHighlightState,
    lang: &GherkinLanguage,
    output: &mut Vec<Line<'static>>,
) {
    for diff_line in lines {
        output.push(render_git_diff_line(diff_line, state, lang));
    }
}

fn render_git_steps(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    scenario: &BddScenario,
    view: &teshi_core::git::FeatureGitView,
) -> Vec<(usize, u16)> {
    let focused = app.explore_focus == ColumnFocus::Step;
    let block = explore_block("Steps (Git diff)", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return Vec::new();
    }

    let scenario_status = view.scenario_status(scenario);
    let lang = explore_gherkin_language(app);
    let mut highlight = StepHighlightState::default();
    let old_scenario_line = view
        .scenarios
        .iter()
        .find(|item| {
            item.new_line_number == Some(scenario.line_number) && item.name == scenario.name
        })
        .and_then(|item| item.old_line_number);
    let is_scenario_header = |diff_line: &teshi_core::git::GitDiffLine| {
        diff_line.new_line_number == Some(scenario.line_number)
            || old_scenario_line.is_some_and(|line| diff_line.old_line_number == Some(line))
    };
    let mut lines = Vec::new();
    let mut step_rows = Vec::new();
    if !view.feature_diff_lines.is_empty() {
        lines.push(Line::styled(
            crate::i18n::t("explore.feature"),
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
        ));
        render_git_diff_lines(&view.feature_diff_lines, &mut highlight, lang, &mut lines);
        lines.push(Line::raw(""));
    }
    if let Some(background) = &view.background
        && background.status != teshi_core::git::DiffStatus::Unchanged
    {
        lines.push(Line::styled(
            crate::i18n::t("explore.background"),
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
        ));
        for diff_line in &background.diff_lines {
            lines.push(render_git_diff_line(diff_line, &mut highlight, lang));
        }
        lines.push(Line::raw(""));
    }

    lines.push(Line::styled(
        format!(
            " {} {}",
            scenario_status.marker(),
            match scenario.kind {
                ScenarioKind::Scenario => format!("Scenario: {}", scenario.name),
                ScenarioKind::ScenarioOutline => format!("Scenario Outline: {}", scenario.name),
            }
        ),
        git_diff_status_style(scenario_status),
    ));
    if scenario_status != teshi_core::git::DiffStatus::Unchanged {
        let diff_lines = view.scenario_diff_lines(scenario).unwrap_or_default();
        if diff_lines.is_empty() {
            if let Some(diff) = &view.diff {
                for diff_line in &diff.lines {
                    if is_scenario_header(diff_line) {
                        continue;
                    }
                    let row_y = inner.y + lines.len() as u16;
                    lines.push(render_git_diff_line(diff_line, &mut highlight, lang));
                    if let Some(new_line) = diff_line.new_line_number
                        && let Some(step_idx) = scenario
                            .steps
                            .iter()
                            .position(|step| step.line_number == new_line)
                    {
                        step_rows.push((step_idx, row_y));
                    }
                }
            }
        } else {
            for diff_line in diff_lines {
                if is_scenario_header(diff_line) {
                    continue;
                }
                let row_y = inner.y + lines.len() as u16;
                lines.push(render_git_diff_line(diff_line, &mut highlight, lang));
                if let Some(new_line) = diff_line.new_line_number
                    && let Some(step_idx) = scenario
                        .steps
                        .iter()
                        .position(|step| step.line_number == new_line)
                {
                    step_rows.push((step_idx, row_y));
                }
            }
        }
    } else {
        for step in &scenario.steps {
            let row_y = inner.y + lines.len() as u16;
            let spans = gherkin_syntax_spans(
                &format!("  {} {}", step.keyword, step.text),
                &mut highlight,
                lang,
                None,
            );
            lines.push(decorate_diff_row(
                spans,
                teshi_core::git::DiffStatus::Unchanged,
            ));
            if let Some(step_idx) = scenario
                .steps
                .iter()
                .position(|candidate| candidate.line_number == step.line_number)
            {
                step_rows.push((step_idx, row_y));
            }
        }
    }
    for deleted in view.deleted_scenarios() {
        if deleted.diff_lines.is_empty() {
            continue;
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(" - Scenario: {}", deleted.name),
            git_diff_status_style(teshi_core::git::DiffStatus::Deleted),
        ));
        for diff_line in &deleted.diff_lines {
            if diff_line.old_line_number == deleted.old_line_number {
                continue;
            }
            lines.push(render_git_diff_line(diff_line, &mut highlight, lang));
        }
    }
    render_explore_lines(frame, app, ColumnFocus::Step, inner, lines);
    step_rows
}

fn render_git_file_fallback(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    view: &teshi_core::git::FeatureGitView,
) {
    let focused = app.explore_focus == ColumnFocus::Step;
    let block = explore_block("Feature (Git diff)", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut lines = Vec::new();
    if let Some(diff) = &view.diff {
        let mut highlight = StepHighlightState::default();
        render_git_diff_lines(
            &diff.lines,
            &mut highlight,
            explore_gherkin_language(app),
            &mut lines,
        );
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            " (Git diff unavailable)",
            Style::default().fg(theme::MUTED),
        ));
    }
    render_explore_lines(frame, app, ColumnFocus::Step, inner, lines);
}

fn render_explore_steps(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let changes = app.explore_changed_steps();
    // An explicitly opened pending-agent diff owns the pane until it is closed;
    // the Git-diff view must not shadow the Diff action behind it.
    if !changes.is_empty() && app.explore_diff_lines.is_none() {
        let block = explore_block(
            "Steps (Git diff) · j/k select · Shift+←/→ scroll",
            app.explore_focus == ColumnFocus::Step,
        );
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        // Keep document-level changes visible alongside independently selectable
        // steps; replacing the old diff renderer must not hide title changes.
        let view = app
            .explore_selected_deleted_feature
            .and_then(|index| app.git_status.deleted.get(index))
            .or_else(|| app.git_status.current_at(app.explore_selected_feature));
        let mut lines = Vec::new();
        let lang = explore_gherkin_language(app);
        if let Some(view) = view {
            let mut highlight = StepHighlightState::default();
            render_git_diff_lines(&view.feature_diff_lines, &mut highlight, lang, &mut lines);
        }
        let prefix_rows = lines.len();
        lines.extend(changes.iter().enumerate().map(|(index, change)| {
            let mut line =
                decorate_diff_row(changed_step_spans(change, lang), change_status(change.kind));
            if index == app.explore_selected_step {
                // The selection fill replaces the diff tint, matching the other
                // Explore columns and the MindMap tree.
                let selection = theme::selected_row(true);
                line.style = selection;
                for span in &mut line.spans {
                    span.style = span.style.patch(selection);
                }
            }
            line
        }));
        let content = render_explore_lines_selected(
            frame,
            app,
            ColumnFocus::Step,
            inner,
            lines,
            Some(prefix_rows + app.explore_selected_step),
        );
        for index in 0..changes.len() {
            // render_explore_panel translates logical rows once and removes
            // off-screen hit targets after every pane has been rendered.
            app.clickable_regions.push(ClickableRegion::ExploreStep {
                step_idx: index,
                row_y: content
                    .y
                    .saturating_add((prefix_rows + index).min(u16::MAX as usize) as u16),
                col_x: content.x,
                col_right: content.right(),
            });
        }
        return;
    }
    if !app.catalog_selection_ready() {
        frame.render_widget(
            Paragraph::new(
                app.lazy_features
                    .as_ref()
                    .and_then(|state| state.error.as_deref())
                    .unwrap_or(crate::i18n::t("scenarios.empty.select")),
            )
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(explore_block(crate::i18n::t("pane.feature_content"), false)),
            area,
        );
        return;
    }
    let focused = app.explore_focus == ColumnFocus::Step;
    if app.explore_edit_mode {
        render_editor_panel(frame, app, area, false);
        return;
    }

    // ── Diff mode (from pending agent change) ──────────────────────────
    if let Some(ref diff_lines) = app.explore_diff_lines {
        let block = explore_block("Steps (diff)", focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let feature = app.project.features.get(app.explore_selected_feature);
        let scenario = feature.and_then(|f| f.scenario_at(app.explore_selected_scenario));
        let scenario_steps = scenario.map(|s| s.steps.as_slice()).unwrap_or(&[]);

        // Determine line-number range of the current scenario's steps
        let first_line = scenario_steps.first().map(|s| s.line_number).unwrap_or(0);
        let last_line = scenario_steps.last().map(|s| s.line_number).unwrap_or(0);

        let mut lines: Vec<Line> = Vec::new();

        // Diff-mode header
        lines.push(Line::styled(
            format!(" {} changed line(s) — [D] close", diff_lines.len()),
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(""));

        // Filter diff lines to those relevant to the current scenario
        let relevant: Vec<&crate::app::DiffLine> = diff_lines
            .iter()
            .filter(|dl| {
                dl.line_number_1based == 0
                    || (dl.line_number_1based >= first_line && dl.line_number_1based <= last_line)
            })
            .collect();

        if relevant.is_empty() {
            lines.push(Line::styled(
                " (no changes in this scenario)",
                Style::default().fg(theme::MUTED),
            ));
        } else {
            let lang = explore_gherkin_language(app);
            for dl in relevant {
                let mut state = StepHighlightState::default();
                let spans = gherkin_syntax_spans(&dl.text, &mut state, lang, None);
                lines.push(decorate_diff_row(spans, change_status(dl.kind)));
            }
        }

        render_explore_lines(frame, app, ColumnFocus::Step, inner, lines);
        return;
    }

    if let Some(deleted_idx) = app.explore_selected_deleted_feature {
        if let Some(view) = app.git_status.deleted.get(deleted_idx).cloned() {
            render_git_file_fallback(frame, app, area, &view);
        }
        return;
    }

    let feature = app.project.features.get(app.explore_selected_feature);
    let scenario = feature.and_then(|f| f.scenario_at(app.explore_selected_scenario));
    if let Some(view) = app
        .git_status
        .current_at(app.explore_selected_feature)
        .cloned()
    {
        let feature_changed = !view.feature_diff_lines.is_empty();
        if let Some(scenario) = scenario {
            let scenario_status = view.scenario_status(scenario);
            let background_changed = view.background.as_ref().is_some_and(|background| {
                background.status != teshi_core::git::DiffStatus::Unchanged
            });
            let deleted_scenario = view.deleted_scenarios().next().is_some();
            if scenario_status != teshi_core::git::DiffStatus::Unchanged
                || background_changed
                || deleted_scenario
                || feature_changed
            {
                let scenario = scenario.clone();
                let step_rows = render_git_steps(frame, app, area, &scenario, &view);
                for (step_idx, row_y) in step_rows {
                    app.clickable_regions.push(ClickableRegion::ExploreStep {
                        step_idx,
                        row_y,
                        col_x: area.x,
                        col_right: area.right(),
                    });
                }
                return;
            }
            let has_mapped_change = view
                .scenarios
                .iter()
                .any(|item| item.status != teshi_core::git::DiffStatus::Unchanged);
            if view.file_status != teshi_core::git::FileGitStatus::Unmodified && !has_mapped_change
            {
                render_git_file_fallback(frame, app, area, &view);
                return;
            }
        } else if view.file_status != teshi_core::git::FileGitStatus::Unmodified {
            render_git_file_fallback(frame, app, area, &view);
            return;
        }
    }

    // ── Normal (browse) mode ──────────────────────────────────────────
    let block = explore_block(crate::i18n::t("steps.pane"), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let highlight_style = explore_select_style(focused);
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx = inner.y;

    let feature = app.project.features.get(app.explore_selected_feature);
    let scenario = feature.and_then(|f| f.scenario_at(app.explore_selected_scenario));
    let background_steps = feature
        .and_then(|f| f.background.as_ref())
        .map(|bg| bg.steps.as_slice())
        .unwrap_or(&[]);
    let scenario_steps = scenario.map(|s| s.steps.as_slice()).unwrap_or(&[]);

    if background_steps.is_empty() && scenario_steps.is_empty() {
        lines.push(Line::styled(
            crate::i18n::t("steps.empty"),
            Style::default().fg(theme::MUTED),
        ));
        line_idx += 1;
    } else {
        let mut last_major: Option<Color> = None;
        if !background_steps.is_empty() {
            let background_label = feature
                .map(|f| {
                    let lang = GherkinLanguages::global().get(&f.language);
                    format!(" {}", lang.primary_structural(StructuralType::Background))
                })
                .unwrap_or_else(|| crate::i18n::t("explore.background_label").to_string());
            lines.push(Line::styled(
                background_label,
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::BOLD),
            ));
            line_idx += 1;
            for step in background_steps {
                let kw_color = match step.keyword_type {
                    StepKeywordType::Given => {
                        last_major = Some(theme::PRIMARY);
                        theme::PRIMARY
                    }
                    StepKeywordType::When => {
                        last_major = Some(theme::WARNING);
                        theme::WARNING
                    }
                    StepKeywordType::Then => {
                        last_major = Some(theme::SUCCESS);
                        theme::SUCCESS
                    }
                    StepKeywordType::And => last_major.unwrap_or(theme::TAB_INACTIVE),
                    StepKeywordType::But => last_major.unwrap_or(theme::TAB_INACTIVE),
                };
                let (kw_span, keyword_pad) =
                    step_keyword_gutter_styled_span(&step.keyword, Style::default().fg(kw_color));
                let body_span =
                    Span::styled(format!(" {}", step.text), Style::default().fg(theme::MUTED));
                let mut line = Line::from(vec![kw_span, body_span]);
                line = pad_line_to_width(line, inner.width, Style::default());
                lines.push(line);
                line_idx += 1;
                if let Some(rows) = step.data_table.as_deref() {
                    let style = Style::default().fg(theme::MUTED);
                    line_idx += push_step_table_rows(
                        &mut lines,
                        &mut app.clickable_regions,
                        rows,
                        step_table_indent(keyword_pad, &step.keyword),
                        inner,
                        None,
                        style,
                    );
                }
            }
            lines.push(Line::raw(""));
            line_idx += 1;
        }

        if let Some(scenario) = scenario
            && !scenario.tags.is_empty()
        {
            lines.push(Line::styled(
                format!(" {}", scenario.tags.join(" ")),
                Style::default().fg(theme::MUTED),
            ));
            line_idx += 1;
        }

        for (i, step) in scenario_steps.iter().enumerate() {
            let kw_color = match step.keyword_type {
                StepKeywordType::Given => {
                    last_major = Some(theme::PRIMARY);
                    theme::PRIMARY
                }
                StepKeywordType::When => {
                    last_major = Some(theme::WARNING);
                    theme::WARNING
                }
                StepKeywordType::Then => {
                    last_major = Some(theme::SUCCESS);
                    theme::SUCCESS
                }
                StepKeywordType::And => last_major.unwrap_or(theme::TAB_INACTIVE),
                StepKeywordType::But => last_major.unwrap_or(theme::TAB_INACTIVE),
            };
            let (kw_span, keyword_pad) =
                step_keyword_gutter_styled_span(&step.keyword, Style::default().fg(kw_color));
            let body_span = if i == app.explore_selected_step {
                Span::styled(format!(" {}", step.text), highlight_style)
            } else {
                Span::raw(format!(" {}", step.text))
            };
            let captures = app.explore_step_captures(&step.text);
            let mut spans = vec![kw_span, body_span];
            if captures > 0 {
                spans.push(capture_badge(captures));
            }
            let mut line = Line::from(spans);
            let trail = if i == app.explore_selected_step {
                highlight_style
            } else {
                Style::default()
            };
            line = pad_line_to_width(line, inner.width, trail);
            lines.push(line);
            line_idx += 1;
            app.clickable_regions.push(ClickableRegion::ExploreStep {
                step_idx: i,
                row_y: line_idx - 1,
                col_x: inner.x,
                col_right: inner.right(),
            });
            if let Some(rows) = step.data_table.as_deref() {
                line_idx += push_step_table_rows(
                    &mut lines,
                    &mut app.clickable_regions,
                    rows,
                    step_table_indent(keyword_pad, &step.keyword),
                    inner,
                    Some(i),
                    trail,
                );
            }
        }

        if let Some(scenario) = scenario
            && !scenario.examples.is_empty()
        {
            lines.push(Line::raw(""));
            line_idx += 1;
            for table in &scenario.examples {
                if !table.tags.is_empty() {
                    lines.push(Line::styled(
                        format!(" {}", table.tags.join(" ")),
                        Style::default().fg(theme::MUTED),
                    ));
                    line_idx += 1;
                }
                lines.push(Line::styled(
                    crate::i18n::t("explore.examples"),
                    Style::default()
                        .fg(theme::MUTED)
                        .add_modifier(Modifier::BOLD),
                ));
                line_idx += 1;
                for row in render_gherkin_table_lines(&table.headers, &table.rows) {
                    lines.push(Line::raw(format!("   {row}")));
                    line_idx += 1;
                }
            }
        }
    }

    let _ = line_idx;
    render_explore_lines(frame, app, ColumnFocus::Step, inner, lines);
}

/// Indent that lines a step's data table up with the step body.
///
/// The keyword gutter pads short keywords, so the body starts past the padded
/// keyword plus the separating space. Table rows use the same column to read as
/// part of their step instead of floating under the gutter.
fn step_table_indent(keyword_pad: usize, keyword: &str) -> usize {
    keyword_pad + UnicodeWidthStr::width(keyword) + 1
}

/// Appends one step's Gherkin data table below its step line.
///
/// The first row is the header. Every rendered row becomes an
/// [`ClickableRegion::ExploreStep`] hit target for `step_idx`, so clicking a
/// table cell selects the step that owns it. Returns the number of rows added.
fn push_step_table_rows(
    lines: &mut Vec<Line<'static>>,
    regions: &mut Vec<ClickableRegion>,
    rows: &[Vec<String>],
    indent: usize,
    inner: Rect,
    step_idx: Option<usize>,
    style: Style,
) -> u16 {
    let Some((headers, body)) = rows.split_first() else {
        return 0;
    };
    let prefix = " ".repeat(indent);
    let mut appended = 0;

    for row in render_gherkin_table_lines(headers, body) {
        let line = pad_line_to_width(
            Line::styled(format!("{prefix}{row}"), style),
            inner.width,
            style,
        );
        lines.push(line);
        if let Some(step_idx) = step_idx {
            regions.push(ClickableRegion::ExploreStep {
                step_idx,
                row_y: inner.y + (lines.len() as u16).saturating_sub(1),
                col_x: inner.x,
                col_right: inner.right(),
            });
        }
        appended += 1;
    }
    u16::try_from(appended).unwrap_or(u16::MAX)
}

fn render_gherkin_table_lines(headers: &[String], rows: &[Vec<String>]) -> Vec<String> {
    if headers.is_empty() {
        return Vec::new();
    }
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|h| UnicodeWidthStr::width(h.as_str()))
        .collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(i) {
                *width = (*width).max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }
    let format_row = |cells: &[String]| {
        let mut out = String::from("|");
        for (i, width) in widths.iter().enumerate() {
            let cell = cells.get(i).map_or("", String::as_str);
            let cell_w = UnicodeWidthStr::width(cell);
            let pad = width.saturating_sub(cell_w);
            out.push(' ');
            out.push_str(cell);
            out.push_str(&" ".repeat(pad));
            out.push(' ');
            out.push('|');
        }
        out
    };

    let mut out = Vec::with_capacity(rows.len() + 2);
    out.push(format_row(headers));
    for row in rows {
        out.push(format_row(row));
    }
    out
}

fn render_external_change_prompt(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if !app.has_external_change_prompt() {
        return;
    }
    if area.width < 10 || area.height < 3 {
        return;
    }
    let title = app
        .external_change_prompt_title()
        .unwrap_or(crate::i18n::t("external_change.title"));
    let file_name = app
        .external_change_prompt_path()
        .unwrap_or_else(|| "feature".to_string());

    // iCode's notice card: a readable fixed width and a height measured from
    // the wrapped message, capped at 90% of the screen.
    let body = crate::i18n::t("external_change.body").replace("{file}", &file_name);
    let spec = overlay::OverlaySpec::prompt(64, 5, 8);
    let width = spec.width.resolve(area.width, 0);
    let body_rows =
        chat::wrap_line_to_width(Line::raw(body.clone()), width.saturating_sub(2) as usize).len();
    // Message + blank separator + action row, inside two borders.
    let rows = body_rows as u16 + 4;
    let popup = overlay::centered(area, width, spec.height.resolve(area.height, rows));

    frame.render_widget(Clear, popup);
    let block = theme::popup_block(title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let [content, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let lines = vec![Line::raw(body), Line::raw("")];
    frame.render_widget(Paragraph::new(Text::from(lines)), content);
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[
            (
                crate::i18n::t("external_change.reload"),
                Action::ExternalChangeReload,
            ),
            (
                crate::i18n::t("external_change.keep"),
                Action::ExternalChangeKeepLocal,
            ),
        ],
        true,
    );
}

/// Renders the Change Summary overlay with pending added/modified/deleted nodes.
fn render_change_summary_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if !app.change_summary_visible || app.pending_change_summary.is_empty() {
        return;
    }
    if area.width < 20 || area.height < 5 {
        return;
    }

    let popup = CHANGE_SUMMARY_SPEC.rect(area, (0, 0));
    app.modal_card_rect = Some(popup);
    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, popup);
    let block = theme::popup_block(crate::i18n::t("change_summary.title"));
    let [inner, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(block.inner(popup));
    frame.render_widget(block, popup);
    if inner.width < 10 || inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let scroll = app
        .change_summary_selection
        .saturating_add(3)
        .saturating_sub(inner.height as usize);
    app.clickable_regions.push(ClickableRegion::ModalList {
        rect: inner,
        up: Action::ChangeSummaryUp,
        down: Action::ChangeSummaryDown,
    });

    // Header
    let header = format!(
        " {} change(s) pending — ↑↓ navigate, Enter jump, Esc close",
        app.pending_change_summary.len()
    );
    lines.push(Line::styled(header, Style::default().fg(theme::MUTED)));
    lines.push(Line::raw(""));

    // List each change node
    for (i, node) in app.pending_change_summary.iter().enumerate() {
        let marker = if i == app.change_summary_selection {
            "▸ "
        } else {
            "  "
        };
        let (label, label_color) = match node.kind {
            crate::app::ChangeKind::Added => ("[+]", theme::SUCCESS),
            crate::app::ChangeKind::Modified => ("[~]", theme::WARNING),
            crate::app::ChangeKind::Deleted => ("[-]", theme::ERROR),
            crate::app::ChangeKind::Unchanged => continue,
        };
        if let Some(rect) = visible_content_rows(inner, lines.len(), 1, scroll) {
            app.clickable_regions
                .push(ClickableRegion::ChangeSummaryRow { rect, index: i });
        }
        let label_span = Span::styled(
            format!("{}{} ", marker, label),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD),
        );
        let scenario_span = Span::styled(
            format!(" {}  ", node.scenario_name),
            Style::default().fg(theme::PRIMARY),
        );
        let step_span = Span::raw(node.step_text.clone());
        let mut line = Line::from(vec![label_span, scenario_span, step_span]);
        // Highlight selected row
        if i == app.change_summary_selection {
            line = apply_line_background(line, theme::selected_row(true));
        }
        line = truncate_line_to_cols(line, inner.width);
        line = pad_line_to_width(line, inner.width, Style::default());
        lines.push(line);
    }

    frame.render_widget(
        Paragraph::new(Text::from(theme::popup_rows(lines, inner.width)))
            .scroll((scroll as u16, 0)),
        inner,
    );
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[
            (
                crate::i18n::t("change_summary.jump"),
                Action::ChangeSummaryJump,
            ),
            (crate::i18n::t("dialog.close.esc"), Action::ClearInputState),
        ],
        true,
    );
}

fn apply_line_background(line: Line<'static>, bg: Style) -> Line<'static> {
    let mut out_spans: Vec<Span<'static>> = Vec::new();
    for span in line.spans {
        let text = span.content.to_string();
        let style = span.style.patch(bg);
        out_spans.push(Span::styled(text, style));
    }
    let mut out = Line::from(out_spans);
    out.style = line.style.patch(bg);
    out.alignment = line.alignment;
    out
}

/// Renders the collapsible tree using `tui-tree-widget`.
fn render_tree_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect, focused: bool) {
    if !app.mindmap_within_budget() {
        frame.render_widget(
            Paragraph::new(crate::i18n::t("mindmap.budget"))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .block(theme::panel_block(
                    crate::i18n::t("mindmap.pane.current"),
                    focused,
                )),
            area,
        );
        return;
    }
    let items = &app.mindmap_index.items;

    let highlight_style = theme::selected_row(focused);

    // Build title with indicators when highlights/filter are active
    let mut title_parts: Vec<&str> = vec![if app.lazy_features.is_some() {
        crate::i18n::t("mindmap.pane.current")
    } else {
        crate::i18n::t("mindmap.pane")
    }];
    if app.mindmap_index.has_active_filter() {
        title_parts.push(crate::i18n::t("mindmap.filtered"));
    }
    if app.mindmap_index.has_active_highlights() {
        title_parts.push(crate::i18n::t("mindmap.highlighted"));
    }
    let title = title_parts.join(" ");

    let block = theme::panel_block(title, focused);

    let block_inner = block.inner(area);
    frame.render_widget(block, area);
    if block_inner.width == 0 || block_inner.height == 0 {
        app.tree_panel_rect = Some(block_inner);
        return;
    }
    let (content_inner, action_area) = split_pane_action_row(block_inner);
    let total_rows = app.tree_state.flatten(items).len();
    let has_vbar = total_rows > content_inner.height as usize && content_inner.width >= 3;
    let tree_inner = if has_vbar {
        Rect {
            width: content_inner.width - 1,
            ..content_inner
        }
    } else {
        content_inner
    };
    let tree = Tree::new(items)
        .expect("tree construction should succeed")
        .highlight_style(highlight_style);

    app.tree_panel_rect = Some(tree_inner);
    app.clickable_regions.push(ClickableRegion::Tree);

    frame.render_stateful_widget(tree, tree_inner, &mut app.tree_state);
    if has_vbar {
        let offset = app.tree_state.get_offset();
        let vbar = Rect::new(tree_inner.right(), tree_inner.y, 1, tree_inner.height);
        render_vertical_scrollbar(
            frame,
            app,
            crate::app::VerticalScrollTarget::MindMapTree,
            vbar,
            total_rows,
            offset,
        );
    }
    if let Some(action_area) = action_area {
        let mindmap_actions = [
            ActionItem::new(
                crate::i18n::t("mindmap.btn.toggle.key"),
                crate::i18n::t("mindmap.btn.toggle"),
                Action::TreeToggle,
                theme::ActionTone::Neutral,
            ),
            ActionItem::new(
                crate::i18n::t("mindmap.btn.ai_panel.key"),
                crate::i18n::t("mindmap.btn.ai_panel"),
                Action::ToggleMindMapAiPanel,
                theme::ActionTone::Info,
            ),
            ActionItem::new(
                crate::i18n::t("mindmap.btn.ask_ai.key"),
                crate::i18n::t("mindmap.btn.ask_ai"),
                Action::MindMapSendToAi,
                theme::ActionTone::Primary,
            ),
            ActionItem::new(
                crate::i18n::t("mindmap.btn.locations.key"),
                crate::i18n::t("mindmap.btn.locations"),
                Action::ToggleScenarioDropdown,
                theme::ActionTone::Warning,
            ),
        ];
        render_chip_action_bar(
            frame,
            &mut app.clickable_regions,
            action_area,
            &mindmap_actions,
            false,
        );
    }
}

/// Renders the scenario preview panel in the MindMap tab (top-right).
/// Shows the complete scenario/background text for the selected tree node.
fn render_mindmap_scenario_preview(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    focused: bool,
) {
    app.preview_panel_rect = Some(area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    }));
    app.clickable_regions.push(ClickableRegion::PreviewPanel);
    if area.width < 10 || area.height < 3 {
        return;
    }
    let Some(buffer) = app.preview_buffer.as_ref() else {
        // No preview available — render an empty placeholder panel
        let title = crate::i18n::t("preview.pane");
        let block = theme::panel_block(title, focused);
        frame.render_widget(block, area);
        return;
    };

    let title = if app.preview_title.is_empty() {
        crate::i18n::t("preview.pane")
    } else {
        app.preview_title.as_str()
    };

    let block = theme::panel_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    clear_to_surface(frame, inner);

    // Paint full-width spaces first to avoid stale pixels from previous frames
    let buf = frame.buffer_mut();
    for i in 0..inner.height {
        let y = inner.y.saturating_add(i);
        if y >= inner.bottom() {
            break;
        }
        buf.set_string(
            inner.x,
            y,
            " ".repeat(inner.width as usize),
            Style::default(),
        );
    }

    // ── Scenario location dropdown ───────────────────────────────────
    if app.scenario_dropdown_open
        && let Some(id) = crate::mindmap::selected_node_id(&app.tree_state)
        && let Some(locations) = app.mindmap_index.locations_for(id)
        && !locations.is_empty()
    {
        let locations = locations.to_vec();
        render_scenario_dropdown(frame, app, inner, &locations);
        return; // dropdown replaces preview content
    }

    let cursor_row = app.preview_cursor_row;
    let visible_lines = inner.height as usize;

    // Compute scroll: center the cursor row
    let line_count = buffer.line_count();
    let has_vbar = line_count > visible_lines && inner.width >= 3;
    let text_area = if has_vbar {
        Rect {
            width: inner.width - 1,
            ..inner
        }
    } else {
        inner
    };
    let max_scroll = line_count.saturating_sub(visible_lines);
    let actual_scroll = if cursor_row < visible_lines / 2 || line_count <= visible_lines {
        0
    } else {
        (cursor_row - visible_lines / 2).min(max_scroll)
    };
    app.preview_scroll_row = actual_scroll;

    // Render lines with Gherkin syntax highlighting
    let mut step_state = crate::highlight::StepHighlightState::default();
    let highlight_style = Style::default().bg(theme::CURRENT_LINE_BG).fg(theme::FG);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible_lines);

    for i in 0..visible_lines {
        let buf_row = actual_scroll + i;
        if buf_row >= line_count {
            lines.push(pad_line_to_width(
                Line::raw(String::new()),
                text_area.width,
                Style::default(),
            ));
            continue;
        }
        let line = buffer.line(buf_row);
        let mut styled = highlight_line_with_state(&line, &mut step_state, buffer.language());

        if buf_row == cursor_row {
            styled = apply_line_background(styled, highlight_style);
        }

        styled = truncate_line_to_cols(styled, text_area.width);
        let pad_trail = if buf_row == cursor_row {
            highlight_style
        } else {
            Style::default()
        };
        styled = pad_line_to_width(styled, text_area.width, pad_trail);
        lines.push(styled);
    }

    for (i, line) in lines.iter().enumerate() {
        let y = text_area.y.saturating_add(i as u16);
        if y >= text_area.bottom() {
            break;
        }
        buf.set_line(text_area.x, y, line, text_area.width);
    }
    if has_vbar {
        let vbar = Rect::new(text_area.right(), text_area.y, 1, text_area.height);
        render_vertical_scrollbar(
            frame,
            app,
            crate::app::VerticalScrollTarget::MindMapPreview,
            vbar,
            line_count,
            actual_scroll,
        );
    }
}

/// Label of the viewer's close control on its top border.
const IMAGE_VIEWER_CLOSE: &str = " Close [Esc] ";
/// Close label for terminals too narrow to show [`IMAGE_VIEWER_CLOSE`] whole.
const IMAGE_VIEWER_CLOSE_SHORT: &str = " Esc ";
/// Label of the viewer's hosted Web UI control on its top border.
const IMAGE_VIEWER_WEB: &str = " Web [w] ";
/// Web label for terminals too narrow to show [`IMAGE_VIEWER_WEB`] whole.
const IMAGE_VIEWER_WEB_SHORT: &str = " w ";

/// Close and Web labels that fit `popup_width`, preferring the full wording.
///
/// Close stays whenever any columns remain. Web is omitted when even the short
/// pair would consume the whole border, so the title is not covered.
fn viewer_border_controls(popup_width: u16) -> (&'static str, u16, &'static str, u16) {
    let close_full = IMAGE_VIEWER_CLOSE.width() as u16;
    let web_full = IMAGE_VIEWER_WEB.width() as u16;
    let close_short = IMAGE_VIEWER_CLOSE_SHORT.width() as u16;
    let web_short = IMAGE_VIEWER_WEB_SHORT.width() as u16;
    // One column separates the controls; one stays inside the right border.
    let budget = popup_width.saturating_sub(2);
    if budget >= close_full.saturating_add(web_full).saturating_add(1) {
        (IMAGE_VIEWER_CLOSE, close_full, IMAGE_VIEWER_WEB, web_full)
    } else if budget >= close_short.saturating_add(web_short).saturating_add(1) {
        (
            IMAGE_VIEWER_CLOSE_SHORT,
            close_short,
            IMAGE_VIEWER_WEB_SHORT,
            web_short,
        )
    } else if budget >= close_full {
        (IMAGE_VIEWER_CLOSE, close_full, "", 0)
    } else {
        (IMAGE_VIEWER_CLOSE_SHORT, close_short.min(budget), "", 0)
    }
}

/// Columns kept free between the image viewer and the panel edges.
const VIEWER_GUTTER_COLS: u16 = 3;
/// Rows kept free above and below the image viewer inside the content area.
const VIEWER_GUTTER_ROWS: u16 = 1;

/// The viewer box: `area` shrunk by the gutters, on every terminal size.
///
/// Very small terminals give the gutters up evenly rather than collapsing the
/// box, so the border and its controls stay addressable.
fn image_viewer_rect(area: Rect) -> Rect {
    let cols = VIEWER_GUTTER_COLS.min(area.width.saturating_sub(2) / 2);
    let rows = VIEWER_GUTTER_ROWS.min(area.height.saturating_sub(2) / 2);
    Rect::new(
        area.x + cols,
        area.y + rows,
        area.width.saturating_sub(cols * 2),
        area.height.saturating_sub(rows * 2),
    )
}

/// Renders the image viewer as a floating box inside the content area.
///
/// The tab row and the footer stay visible, so the viewer reads as a panel
/// instead of a mode switch and the surrounding affordances never disappear.
/// The box keeps a gutter on every side; the close control sits on its border,
/// outside the box pixels are drawn into, so a fitted image never hides it.
/// Terminals without a pixel protocol keep pointing at the system viewer
/// instead of drawing half-blocks, which show a UI screenshot as noise.
fn render_screenshot_overlay(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if !app.screenshot.visible || area.is_empty() {
        return;
    }
    clear_to_surface(frame, area);
    let popup = image_viewer_rect(area);
    let standalone = app.screenshot.origin == crate::image_view::ScreenshotOrigin::Image;
    let noun = if standalone { "image" } else { "capture" };
    let total = app.screenshot.refs.len();
    let index = app.screenshot.index.min(total.saturating_sub(1));
    let size = app
        .screenshot
        .rendered
        .as_ref()
        .map(|rendered| {
            let source = format!("  {}×{}", rendered.pixels.0, rendered.pixels.1);
            if app.screenshot.view.is_zoomed() {
                format!("{source} @ {},{}", rendered.region.x, rendered.region.y)
            } else {
                source
            }
        })
        .unwrap_or_default();
    let view = app.screenshot.view.label();
    let progress = if app.screenshot.loading {
        " · Loading"
    } else if app
        .screenshot
        .rendered
        .as_ref()
        .is_some_and(|image| image.is_drawing())
    {
        " · Drawing"
    } else {
        ""
    };
    let title = match app.screenshot.refs.get(index) {
        Some(shot) if standalone => format!("Image{size} · {view}{progress}  {}", shot.file),
        Some(shot) if total > 0 => {
            let step = shot.step_text.trim();
            let step = if step.is_empty() {
                "step capture"
            } else {
                step
            };
            format!(
                "Screenshot {}/{}{} · {}{}  {} · {}",
                index + 1,
                total,
                size,
                view,
                progress,
                shot.scenario,
                step
            )
        }
        _ => "Screenshot".to_string(),
    };
    let (close_label, close_width, web_label, web_width) = viewer_border_controls(popup.width);
    let title = truncate_line_to_cols(
        Line::raw(title),
        popup
            .width
            .saturating_sub(close_width.saturating_add(web_width).saturating_add(4)),
    );
    let block = theme::popup_block(title);
    let warning = app.screenshot.support_detail.contains("Windows Zellij")
        && app.screenshot.support.is_pixels();
    let web_notice = app.web_review.notice().map(str::to_string);
    let footer_rows = 1 + u16::from(warning) + u16::from(web_notice.is_some());
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(footer_rows)])
        .areas(block.inner(popup));
    frame.render_widget(block, popup);
    if close_width > 0 {
        let close = Rect::new(
            popup.right().saturating_sub(close_width + 1),
            popup.y,
            close_width,
            1,
        );
        frame.render_widget(
            Paragraph::new(Line::from(theme::chip(close_label, theme::PRIMARY))),
            close,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: close,
            action: Action::ImageViewerClose,
            modal: true,
        });
    }
    if web_width > 0 {
        let web = Rect::new(
            popup.right().saturating_sub(close_width + web_width + 2),
            popup.y,
            web_width,
            1,
        );
        // Same emphasis as Close. A muted chip reads as disabled, but this
        // control is always actionable: the first activation starts a daemon
        // when none is listening, and a later one reuses it.
        frame.render_widget(
            Paragraph::new(Line::from(theme::chip(web_label, theme::PRIMARY))),
            web,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: web,
            action: Action::OpenReplayWebUi,
            modal: true,
        });
    }
    if body.width == 0 || body.height == 0 {
        return;
    }
    app.screenshot.request_viewport(body);
    if let Some(rendered) = app.screenshot.rendered.as_mut() {
        crate::image_view::render(frame, body, rendered);
    } else {
        let mut lines: Vec<Line<'static>> = Vec::new();
        if let Some(error) = app.screenshot.error.as_ref() {
            lines.push(Line::raw(format!("Cannot show {noun}: {error}")));
            lines.push(Line::raw(
                "Press o to open the original in the system viewer.",
            ));
        } else if app.screenshot.loading {
            lines.push(Line::raw("Preparing image in the background..."));
            lines.push(Line::raw(if standalone {
                "Zoom, or press Esc to close."
            } else {
                "Zoom, switch steps, or press i/Esc to close."
            }));
        } else if app.screenshot.refs.is_empty() {
            lines.push(Line::raw("No captured screenshots for this scenario."));
            lines.push(Line::raw(String::new()));
            let captured: Vec<_> = app
                .capture_index
                .entries
                .iter()
                .filter(|entry| entry.count > 0)
                .collect();
            if captured.is_empty() {
                lines.push(Line::raw("Run a replay for this Feature to capture steps."));
            } else {
                lines.push(Line::styled(
                    format!("Scenarios with captures ({}):", captured.len()),
                    Style::default().fg(theme::MUTED),
                ));
                for entry in captured.iter().take(10) {
                    lines.push(Line::raw(format!(
                        "  {} · {} ({})",
                        entry.feature, entry.scenario, entry.count
                    )));
                }
                if captured.len() > 10 {
                    lines.push(Line::styled(
                        format!("  … {} more", captured.len() - 10),
                        Style::default().fg(theme::MUTED),
                    ));
                }
            }
        } else {
            if !app.screenshot.support.is_pixels() {
                lines.push(Line::raw(format!(
                    "This terminal has no image protocol ({}).",
                    app.screenshot.support.label()
                )));
                if !app.screenshot.support_detail.is_empty() {
                    lines.push(Line::styled(
                        format!("  {}", app.screenshot.support_detail),
                        Style::default().fg(theme::MUTED),
                    ));
                }
                lines.push(Line::raw(String::new()));
                lines.push(Line::raw(
                    "Use a terminal with image support, or open the original externally.",
                ));
                lines.push(Line::raw(format!(
                    "Press o to open the {noun} in the system viewer."
                )));
            }
            if let Some(shot) = app.screenshot.refs.get(index) {
                lines.push(Line::raw(
                    crate::image_view::capture_path(&app.project.root_dir, shot)
                        .display()
                        .to_string(),
                ));
            }
        }
        frame.render_widget(Paragraph::new(lines), body);
    }
    let mut footer_lines = Vec::new();
    if warning {
        footer_lines.push(Line::styled(
            " Windows Zellij may filter images; try direct Windows Terminal or press o.",
            Style::default().fg(theme::MUTED),
        ));
    }
    footer_lines.push(Line::styled(
        if standalone {
            " +/- or wheel zoom · 1 100% · 0 fit · arrows pan · o open · Esc close"
        } else {
            " +/- zoom · 1 100% · 0 fit · arrows pan · j/k steps · o open · i/Esc close"
        },
        Style::default().fg(theme::MUTED),
    ));
    if let Some(notice) = web_notice {
        footer_lines.push(Line::styled(
            format!(" {notice}"),
            Style::default().fg(theme::WARNING),
        ));
    }
    frame.render_widget(Paragraph::new(footer_lines), footer);
}

/// Renders the scenario location dropdown inside the preview panel.
fn render_scenario_dropdown(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    locations: &[crate::mindmap::NodeLocation],
) {
    use crate::mindmap::LocationContext;

    let count = locations.len();
    let selection = app.scenario_dropdown_selection.min(count.saturating_sub(1));
    let max_items = (area.height as usize).saturating_sub(3).min(count);
    let list_height = (max_items + 2).min(area.height.saturating_sub(1) as usize) as u16;

    let dropdown_area = Rect::new(area.x, area.y, area.width, list_height.min(area.height));
    if dropdown_area.height < 3 {
        return;
    }
    app.modal_card_rect = Some(dropdown_area);

    // Build item labels
    let mut items: Vec<String> = Vec::with_capacity(count);
    for loc in locations.iter().take(count) {
        let feature_name = app
            .project
            .features
            .get(loc.feature_idx)
            .and_then(|f| f.file_path.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("feature_{}", loc.feature_idx));

        let label = match loc.context {
            LocationContext::Scenario(sci) => {
                let scenario_name = app
                    .project
                    .features
                    .get(loc.feature_idx)
                    .and_then(|f| f.scenario_at(sci))
                    .map(|s| s.name.as_str())
                    .unwrap_or("?");
                format!("{}: Scenario: {}", feature_name, scenario_name)
            }
            LocationContext::Background => {
                format!("{}: Background", feature_name)
            }
        };
        items.push(label);
    }

    // Render dropdown block
    let block = theme::popup_block("Open Scenario");
    let inner = block.inner(dropdown_area);
    frame.render_widget(Clear, dropdown_area);
    frame.render_widget(block, dropdown_area);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(max_items);
    let visible_items = inner.height as usize;
    let scroll_start = selection.saturating_sub(visible_items.saturating_sub(1) / 2);
    let scroll_end = (scroll_start + visible_items).min(items.len());

    for (i, item) in items[scroll_start..scroll_end].iter().enumerate() {
        let idx = scroll_start + i;
        app.clickable_regions
            .push(ClickableRegion::MindMapLocation {
                rect: Rect::new(inner.x, inner.y + i as u16, inner.width, 1),
                index: idx,
            });
        let is_selected = idx == selection;
        let prefix = if is_selected { "▸ " } else { "  " };
        let style = if is_selected {
            theme::selected_row(true)
        } else {
            Style::default().fg(theme::FG)
        };
        let text = truncate_str(item, inner.width.saturating_sub(2) as usize);
        lines.push(Line::styled(format!("{}{}", prefix, text), style));
    }

    frame.render_widget(
        Paragraph::new(Text::from(theme::popup_rows(lines, inner.width))),
        inner,
    );

    // Hint line at the bottom of the dropdown area
    if dropdown_area.y + dropdown_area.height < area.y + area.height {
        let hint_y = dropdown_area.y + dropdown_area.height;
        render_action_bar(
            frame,
            &mut app.clickable_regions,
            Rect::new(area.x, hint_y, area.width, 1),
            "",
            &[
                (
                    if area.width < 25 {
                        "Open "
                    } else {
                        "Open [Enter] "
                    },
                    Action::ScenarioDropdownSelect,
                ),
                (
                    if area.width < 25 {
                        "Close"
                    } else {
                        crate::i18n::t("dialog.close.esc")
                    },
                    Action::ScenarioDropdownClose,
                ),
            ],
            true,
        );
    }
}

/// Truncate a string to fit within `max_width` columns (Unicode-aware).
fn truncate_str(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let width = s.width();
    if width <= max_width {
        return s.to_string();
    }
    let mut out = String::with_capacity(max_width);
    let mut w = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw + 1 > max_width {
            out.push('…');
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// Renders the AI chat panel in the MindMap tab (bottom-right).
fn render_mindmap_agent_chat(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 10 || area.height < 3 {
        return;
    }
    let focused = app.mindmap_focus == MindMapFocus::AiPanel;
    chat::render_chat_panel(frame, app, area, focused, chat::ChatPlacement::Embedded);
}

/// Renders the editor panel showing the active feature file.
///
/// When `preview` is true (stage 2), the panel is read-only with no cursor. Otherwise (stage 3),
/// it shows the full interactive editor with cursor highlighting.
fn render_editor_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect, preview: bool) {
    let title_base = if preview {
        if app.preview_title.is_empty() {
            "Preview".to_string()
        } else {
            app.preview_title.clone()
        }
    } else {
        app.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Editor".to_string())
    };
    let title_line: Line<'static> = if preview {
        Line::from(format!("{title_base} (preview)"))
    } else {
        let dirty_suffix = if app.dirty { " *" } else { "" };
        let (mode_label, mode_color) = if app.step_input_active {
            (crate::i18n::t("mode.insert"), theme::ACCENT)
        } else {
            (crate::i18n::t("mode.normal").trim(), theme::PRIMARY)
        };
        Line::from(vec![
            Span::raw(format!("{title_base}{dirty_suffix} ")),
            theme::chip(mode_label, mode_color),
        ])
    };
    let editor_block = theme::panel_block(title_line, !preview);
    let editor_inner = editor_block.inner(area);
    frame.render_widget(editor_block, area);
    clear_to_surface(frame, editor_inner);

    let (editor_body, editor_action_area) = if !preview {
        split_pane_action_row(editor_inner)
    } else {
        (editor_inner, None)
    };

    // Keep a small, non-blocking diagnostics pane inside the editor.  The
    // source area retains at least two rows in normal terminal sizes, while
    // very small terminals keep the editor fully usable and omit the pane.
    let (editor_source_area, diagnostics_area) =
        if !preview && !app.buffer_diagnostics.is_empty() && editor_body.height >= 4 {
            let diagnostic_height = (app.buffer_diagnostics.len().min(4) as u16 + 2)
                .min(editor_body.height.saturating_sub(2));
            if diagnostic_height >= 2 {
                let [source_area, diagnostics_area] =
                    Layout::vertical([Constraint::Min(2), Constraint::Length(diagnostic_height)])
                        .areas(editor_body);
                (source_area, diagnostics_area)
            } else {
                (editor_body, Rect::default())
            }
        } else {
            (editor_body, Rect::default())
        };

    let visible_lines = editor_source_area.height as usize;
    let buffer = if preview {
        app.preview_buffer.as_ref().unwrap_or(&app.buffer)
    } else {
        &app.buffer
    };
    let cursor_row = if preview {
        app.preview_cursor_row
    } else {
        app.cursor_row
    };
    let scroll_row = if preview {
        app.preview_scroll_row
    } else {
        app.scroll_row
    };
    let mut visible_rows = if preview {
        (0..buffer.line_count()).collect::<Vec<_>>()
    } else {
        app.visible_editor_rows()
    };
    if visible_rows.is_empty() {
        visible_rows.push(0);
    }
    let has_vbar = !preview && visible_rows.len() > visible_lines && editor_source_area.width >= 3;
    let editor_area = if has_vbar {
        Rect {
            width: editor_source_area.width - 1,
            ..editor_source_area
        }
    } else {
        editor_source_area
    };
    let cursor_idx = visible_rows
        .iter()
        .position(|&row| row == cursor_row)
        .or_else(|| visible_rows.iter().rposition(|&row| row <= cursor_row))
        .unwrap_or(0);
    let mut scroll_idx = visible_rows
        .iter()
        .position(|&row| row == scroll_row)
        .or_else(|| visible_rows.iter().position(|&row| row >= scroll_row))
        .unwrap_or(0);
    if !preview {
        if cursor_idx < scroll_idx {
            scroll_idx = cursor_idx;
        } else if cursor_idx >= scroll_idx.saturating_add(visible_lines) {
            scroll_idx = cursor_idx.saturating_sub(visible_lines.saturating_sub(1));
        }
    } else {
        scroll_idx = cursor_idx.saturating_sub(visible_lines / 2);
    }

    let mut lines = Vec::with_capacity(visible_lines);
    let preview_row_style = Style::default().bg(theme::CURRENT_LINE_BG).fg(theme::FG);
    let mut step_state = StepHighlightState::default();
    for &row in visible_rows.iter().take(scroll_idx) {
        if row >= buffer.line_count() {
            break;
        }
        let line = buffer.line(row);
        let _ = highlight_line_with_state(&line, &mut step_state, buffer.language());
    }
    for visible_idx in scroll_idx..scroll_idx.saturating_add(visible_lines) {
        let Some(&row) = visible_rows.get(visible_idx) else {
            let empty = pad_line_to_width(
                Line::raw(String::new()),
                editor_area.width,
                Style::default(),
            );
            lines.push(empty);
            continue;
        };
        let line = buffer.line(row);
        let mut display_line = line.clone();
        if !preview && let Some(step_count) = app.folded_step_count(row) {
            display_line.push_str(&format!("  [folded: {step_count} steps]"));
        }
        let (padded_display, pad_offset, pad_start) =
            step_line_display(&display_line, step_state.in_doc_string, buffer.language());
        let display_len = if pad_offset > 0 {
            padded_display.chars().count()
        } else {
            display_line.chars().count()
        };
        let mut styled =
            highlight_line_with_state(&display_line, &mut step_state, buffer.language());

        // Mark source rows with errors while preserving the editor's normal
        // highlighting and cursor/selection behavior.
        if !preview
            && app.buffer_diagnostics.iter().any(|diagnostic| {
                diagnostic.line == row.saturating_add(1)
                    && diagnostic.severity == teshi_core::DiagnosticSeverity::Error
            })
        {
            styled = apply_line_background(
                styled,
                Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::UNDERLINED),
            );
        }

        // When a scenario is focused, dim steps in non-focused scenarios
        if !preview && let Some(focus_row) = app.editor_focus_scenario_row {
            let header_row = crate::bdd_nav::scenario_header_for_row(buffer, row);
            // Dim the row if it belongs to a different (non-focused) scenario
            // AND is NOT itself the focused scenario header
            let is_different_scenario =
                header_row.is_some() && header_row != Some(focus_row) && header_row != Some(row);
            let is_not_focused_header = row != focus_row;
            let in_background = row_is_in_background_block(buffer, row);
            if is_different_scenario && is_not_focused_header && !in_background {
                styled = apply_line_background(
                    styled,
                    Style::default()
                        .fg(theme::MUTED)
                        .add_modifier(Modifier::DIM),
                );
            }
        }

        if row == cursor_row && !preview {
            // Use a block-cursor background for edit mode so the cursor is visible
            // even when positioned at the end of the line (on a space character).
            let edit_cursor_style = Style::default()
                .fg(theme::FG)
                .bg(theme::EDITOR_SELECTION_BG);
            let line_len = display_len;
            if app.is_editor_nav_mode() {
                let focus_patch = theme::selected_row(true);
                let hl_range = nav_body_char_range_in_buffer(buffer, row, &line)
                    .or_else(|| structural_prefix_char_range(&line, buffer.language()));
                if let Some(mut r) = hl_range
                    && r.start < r.end
                {
                    if pad_offset > 0 && r.start >= pad_start {
                        r.start += pad_offset;
                        r.end += pad_offset;
                    }
                    styled = apply_patch_to_char_range(styled, r, focus_patch);
                }
            } else if app.step_input_active || app.step_keyword_picker.is_some() {
                let cursor_col = if pad_offset > 0 && app.cursor_col >= pad_start {
                    app.cursor_col + pad_offset
                } else {
                    app.cursor_col
                };
                if line_len == 0 {
                    styled = Line::from(vec![Span::styled(" ", edit_cursor_style)]);
                } else if cursor_col < line_len {
                    styled = apply_patch_to_char_range(
                        styled,
                        cursor_col..cursor_col.saturating_add(1),
                        edit_cursor_style,
                    );
                } else {
                    let mut spans = styled.spans;
                    spans.push(Span::styled(" ", edit_cursor_style));
                    styled = Line::from(spans);
                }
            }
        } else if row == cursor_row && preview {
            // Do not patch syntax-highlight spans: patching by char range can leave columns with
            // default colors between spans, which terminals show as a bright "hole" or bar.
            styled = if display_line.is_empty() {
                Line::from(vec![Span::styled(" ", preview_row_style)])
            } else {
                Line::from(Span::styled(display_line.to_string(), preview_row_style))
            };
        }

        // Apply mouse-drag selection highlight (column-accurate).
        if let (Some(anchor), Some(end)) = (app.selection_anchor, app.selection_end) {
            let sel_lo = anchor.0.min(end.0);
            let sel_hi = anchor.0.max(end.0);
            if !preview && row >= sel_lo && row <= sel_hi {
                let sel_style = Style::default()
                    .bg(theme::EDITOR_SELECTION_BG)
                    .fg(theme::FG);
                let mut range = if sel_lo == sel_hi {
                    // Single row: highlight the exact column range
                    let lo_col = anchor.1.min(end.1);
                    let hi_col = anchor.1.max(end.1).min(display_len);
                    lo_col..hi_col
                } else if row == sel_lo {
                    // First row: from the earlier column to end of line
                    let col = if anchor.0 < end.0 { anchor.1 } else { end.1 };
                    col..display_len
                } else if row == sel_hi {
                    // Last row: from start of line to the later column
                    let col = if anchor.0 > end.0 { anchor.1 } else { end.1 }.min(display_len);
                    0..col
                } else {
                    // Middle rows: full row
                    0..display_len
                };
                // Adjust for step-keyword padding, matching the cursor-row logic
                if pad_offset > 0 && range.start >= pad_start {
                    range.start = range.start.saturating_add(pad_offset);
                }
                if pad_offset > 0 && range.end > pad_start {
                    range.end = range.end.saturating_add(pad_offset);
                }
                if range.start < range.end {
                    styled = apply_patch_to_char_range(styled, range, sel_style);
                }
            }
        }

        let pad_trail = if preview && row == cursor_row && row < buffer.line_count() {
            preview_row_style
        } else {
            Style::default()
        };
        styled = truncate_line_to_cols(styled, editor_area.width);
        styled = pad_line_to_width(styled, editor_area.width, pad_trail);
        lines.push(styled);
    }

    // Stage-2 feature preview (right pane): paint each inner row in two passes. `set_line` alone
    // can leave columns past a truncated long line unchanged in edge cases; an explicit full-width
    // space fill first forces every cell in this frame (helps terminal diff + Windows hosts).
    let buf = frame.buffer_mut();
    if preview {
        for i in 0..visible_lines {
            let y = editor_area.y.saturating_add(i as u16);
            if y >= editor_area.bottom() {
                break;
            }
            let buffer_row = visible_rows
                .get(scroll_idx.saturating_add(i))
                .copied()
                .unwrap_or(usize::MAX);
            let row_fill = if buffer_row == cursor_row && buffer_row < buffer.line_count() {
                preview_row_style
            } else {
                Style::default()
            };
            buf.set_string(
                editor_area.x,
                y,
                " ".repeat(editor_area.width as usize),
                row_fill,
            );
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let y = editor_area.y.saturating_add(i as u16);
        if y >= editor_area.bottom() {
            break;
        }
        buf.set_line(editor_area.x, y, line, editor_area.width);
    }
    if preview {
        app.preview_scroll_row = visible_rows.get(scroll_idx).copied().unwrap_or(0);
    } else {
        app.scroll_row = visible_rows.get(scroll_idx).copied().unwrap_or(0);
        app.editor_panel_rect = Some(editor_area);
        if has_vbar {
            let vbar = Rect::new(editor_area.right(), editor_area.y, 1, editor_area.height);
            render_vertical_scrollbar(
                frame,
                app,
                crate::app::VerticalScrollTarget::ExploreEditor,
                vbar,
                visible_rows.len(),
                scroll_idx,
            );
        }
        render_buffer_diagnostics(frame, app, diagnostics_area);
        if let Some(action_area) = editor_action_area {
            let controls = if app.step_input_active {
                vec![
                    ActionItem::new(
                        crate::i18n::t("editor.btn.commit.key"),
                        crate::i18n::t("editor.btn.commit"),
                        Action::Enter,
                        theme::ActionTone::Success,
                    ),
                    ActionItem::new(
                        crate::i18n::t("editor.btn.normal.key"),
                        crate::i18n::t("editor.btn.normal"),
                        Action::ClearInputState,
                        theme::ActionTone::Warning,
                    ),
                    ActionItem::new(
                        crate::i18n::t("editor.btn.save.key"),
                        crate::i18n::t("editor.btn.save"),
                        Action::Save,
                        theme::ActionTone::Info,
                    ),
                ]
            } else {
                vec![
                    ActionItem::new(
                        crate::i18n::t("editor.btn.back.key"),
                        crate::i18n::t("editor.btn.back"),
                        Action::ClearInputState,
                        theme::ActionTone::Warning,
                    ),
                    ActionItem::new(
                        crate::i18n::t("editor.btn.save.key"),
                        crate::i18n::t("editor.btn.save"),
                        Action::Save,
                        theme::ActionTone::Success,
                    ),
                    ActionItem::new(
                        crate::i18n::t("editor.btn.run.key"),
                        crate::i18n::t("editor.btn.run"),
                        Action::RunBackground,
                        theme::ActionTone::Primary,
                    ),
                    ActionItem::new(
                        crate::i18n::t("editor.btn.fold.key"),
                        crate::i18n::t("editor.btn.fold"),
                        Action::ToggleScenarioFold,
                        theme::ActionTone::Neutral,
                    ),
                ]
            };
            render_chip_action_bar(
                frame,
                &mut app.clickable_regions,
                action_area,
                &controls,
                false,
            );
        }
    }
}

/// Render source-aware diagnostics without replacing the permissive editor
/// buffer.  Each finding is kept on one line so locations and repair hints
/// remain visible even when a source line contains wide Unicode text.
fn render_buffer_diagnostics(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let title = format!("Diagnostics ({})", app.buffer_diagnostics.len());
    let block = theme::panel_block(title, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = app
        .buffer_diagnostics
        .iter()
        .take(inner.height as usize)
        .map(|diagnostic| {
            let severity = match diagnostic.severity {
                teshi_core::DiagnosticSeverity::Error => ("error", theme::ERROR),
                teshi_core::DiagnosticSeverity::Warning => ("warning", theme::WARNING),
                teshi_core::DiagnosticSeverity::Suggestion => ("suggestion", theme::MUTED),
            };
            let suggestion = diagnostic
                .suggestion
                .as_deref()
                .map(|value| format!(" -> {value}"))
                .unwrap_or_default();
            let text = format!(
                " {} {}:{}:{} [{}] {}{}",
                severity.0,
                diagnostic.path,
                diagnostic.line,
                diagnostic.column,
                diagnostic.code,
                diagnostic.message,
                suggestion,
            );
            Line::styled(
                truncate_string_to_cols(&text, inner.width),
                Style::default().fg(severity.1),
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Draws the step-keyword overlay when [`App::step_keyword_picker`] is active.
fn render_step_keyword_picker(frame: &mut Frame<'_>, app: &mut App, editor_area: Rect) {
    let Some(picker) = app.step_keyword_picker else {
        return;
    };

    const TITLE: &str = "Step keyword";
    let all_kw = app.buffer.language().all_step_keywords();
    let keywords: Vec<&str> = all_kw
        .iter()
        .filter(|kw| kw.as_str() != "*")
        .map(|s| s.as_str())
        .collect();
    let max_kw_w = keywords.iter().map(|s| s.width()).max().unwrap_or(5);
    // Use max display width for popup width, but ensure at least title width
    let title_width = TITLE.width();
    let inner_w_ch = max_kw_w.max(title_width).saturating_add(2);
    let list_w = (inner_w_ch as u16).saturating_add(2).max(34);
    let n_items = keywords.len();
    let list_h = (n_items as u16).saturating_add(3);

    let visible_lines = editor_area.height as usize;
    let row_in_view = picker.buffer_row.saturating_sub(app.scroll_row);
    let y_below = if row_in_view < visible_lines {
        editor_area.y + 1 + row_in_view as u16
    } else {
        editor_area
            .y
            .saturating_add(editor_area.height.saturating_sub(list_h))
    };
    let max_y = editor_area.y + editor_area.height;
    let mut y = y_below;
    if y.saturating_add(list_h) > max_y {
        y = max_y.saturating_sub(list_h);
    }
    y = y.max(editor_area.y);

    let h_avail = max_y.saturating_sub(y);
    let h = list_h.min(h_avail).max(3);
    let w = list_w.min(editor_area.width).max(3);
    let area = Rect::new(editor_area.x, y, w, h);
    app.modal_card_rect = Some(area);

    frame.render_widget(Clear, area);

    let block = theme::popup_block(TITLE);
    let [inner, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(block.inner(area));
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let row_width = inner.width as usize;
    let max_rows = inner.height as usize;

    let selected_row_style = theme::selected_row(true);
    let normal = Style::default();

    let mut lines: Vec<Line> = Vec::with_capacity(max_rows.min(n_items));
    let start = picker.selected.saturating_add(1).saturating_sub(max_rows);
    app.clickable_regions.push(ClickableRegion::ModalList {
        rect: inner,
        up: Action::StepKeywordPickerUp,
        down: Action::StepKeywordPickerDown,
    });
    for (i, kw) in keywords.iter().enumerate().skip(start).take(max_rows) {
        app.clickable_regions.push(ClickableRegion::StepKeywordRow {
            rect: Rect::new(inner.x, inner.y + (i - start) as u16, inner.width, 1),
            index: i,
        });
        let style = if i == picker.selected {
            selected_row_style
        } else {
            normal
        };
        let mut text = String::from(" ");
        text.push_str(kw);
        let used = UnicodeWidthStr::width(text.as_str());
        let pad = row_width.saturating_sub(used);
        text.push_str(&" ".repeat(pad));
        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[
            ("Confirm [Enter] ", Action::StepKeywordPickerConfirm),
            (
                crate::i18n::t("dialog.cancel.esc"),
                Action::StepKeywordPickerCancel,
            ),
        ],
        true,
    );
}

/// Renders the Run Details pane beside Steps.
///
/// The header keeps two fixed rows (selected step/status/run history, then
/// inner tabs) so switching families or stepping never moves the content
/// origin. Content scrolls independently of the Steps selection.
fn render_run_details(frame: &mut Frame<'_>, app: &mut App, area: Rect, focused: bool) {
    app.ensure_detail_shots();
    let block = theme::panel_block("Run Details", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if !app.explore_edit_mode && area.width > 0 && area.height > 0 {
        // Broad target registered first; tabs and items layered above it win
        // the reverse-order hit test.
        app.clickable_regions
            .push(ClickableRegion::ExploreDetailPane { rect: area });
    }
    render_explore_details_toggle_chip(frame, app, area, true, !app.explore_edit_mode);

    let detail = app.current_detail().cloned();
    let status = app.detail_selection_status();
    // Target prep replaces the step title briefly while a Feature target is
    // launching or attaching, so progress stays visible without a third row.
    let step_text = app
        .explore_history
        .target_status()
        .map(str::to_string)
        .or_else(|| app.explore_selected_step_text())
        .unwrap_or_else(|| "-".into());
    let tabs = app.detail_tabs();
    let active_tab = app.active_detail_tab();
    // Screenshots own the pane: no bottom hint strip, so the fit preview can
    // use the full remaining height (and reflow when the feature tree hides).
    let (body_area, action_area) =
        if matches!(active_tab, Some(crate::app::RunDetailTab::Screenshots)) {
            (inner, None)
        } else {
            split_pane_action_row(inner)
        };
    if body_area.height == 0 {
        return;
    }
    let item_count = app.detail_item_count();
    if app.explore_details_selection >= item_count {
        app.explore_details_selection = item_count.saturating_sub(1);
    }
    let selection = app.explore_details_selection;

    // Two fixed rows: selected step/status/run history, then inner tabs.
    let header_height = 2.min(body_area.height);
    let header_area = Rect {
        height: header_height,
        ..body_area
    };
    let content_area = Rect {
        x: body_area.x,
        y: body_area.y + header_height,
        width: body_area.width,
        height: body_area.height - header_height,
    };
    let (tabs_line, tab_hits) = detail_tabs_line(app, &tabs, active_tab, header_area.width);
    let run_key = app.current_detail_key();
    let run_selected = app.explore_history.selected(run_key).unwrap_or(0);
    let menu_open = app.explore_history.menu().is_some();
    let run_selector = (!app.explore_edit_mode)
        .then_some(detail.as_ref())
        .flatten()
        .and_then(|detail| detail.started_at.as_deref())
        .filter(|_| header_area.width > 30)
        .map(|started| {
            let time = short_run_time(started);
            let background = if menu_open {
                theme::TAB_ACTIVE_BG
            } else {
                theme::BLOCK_BG
            };
            let mut spans = vec![Span::styled(
                format!(" {time} "),
                Style::default().bg(background).fg(theme::FG),
            )];
            if run_selected == 0 {
                spans.push(theme::chip(" LATEST ", theme::PRIMARY));
            }
            spans.push(Span::styled(
                " ▾ ",
                Style::default().bg(background).fg(theme::MUTED),
            ));
            spans
        });
    let selector_width = run_selector.as_ref().map_or(0, |spans| {
        spans
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum::<usize>() as u16
    });
    let header = vec![
        detail_step_status_line(&step_text, status, run_selector, header_area.width),
        tabs_line,
    ];
    frame.render_widget(
        Paragraph::new(Text::from(truncate_lines(header, header_area.width))),
        header_area,
    );
    if !app.explore_edit_mode && header_height == 2 {
        for hit in tab_hits {
            app.clickable_regions
                .push(ClickableRegion::ExploreDetailTab {
                    tab: hit.tab,
                    rect: Rect::new(
                        header_area.x + hit.x,
                        header_area.y + 1,
                        hit.width.min(header_area.width.saturating_sub(hit.x)),
                        1,
                    ),
                });
        }
        if selector_width > 0 && selector_width < header_area.width {
            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect: Rect::new(
                    header_area.x + header_area.width - selector_width,
                    header_area.y,
                    selector_width,
                    1,
                ),
                action: Action::ExploreRunMenu,
                modal: false,
            });
        }
    }

    let pixels = app.inline_images.has_pixels();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut content = DetailContent::default();
    match active_tab {
        Some(crate::app::RunDetailTab::Screenshots) => {
            let shots = app.detail_screenshots();
            // Fill the pane so hiding the feature tree reflows the fit preview.
            content = push_screenshot_rows(
                &mut lines,
                app,
                &shots,
                selection,
                content_area.width,
                pixels,
                content_area.height.max(1),
            );
        }
        Some(crate::app::RunDetailTab::Api) => {
            let exchanges = app.detail_exchanges();
            content = push_exchange_rows(
                &mut lines,
                app,
                &exchanges,
                selection,
                detail.as_ref(),
                content_area.width,
            );
        }
        Some(crate::app::RunDetailTab::Logs) => {
            if let Some(detail) = detail.as_ref() {
                content = push_log_rows(&mut lines, &detail.logs, selection, content_area.width);
            }
        }
        Some(crate::app::RunDetailTab::Files) => {
            let files = app.detail_files();
            content = push_file_rows(&mut lines, app, &files, selection, content_area.width);
        }
        None => {
            push_empty_detail_rows(&mut lines, detail.as_ref(), status, content_area.width);
        }
    }

    let total_lines = lines.len();
    let max_offset = total_lines.saturating_sub(content_area.height as usize);
    let mut offset = app.explore_details_scroll.min(max_offset);
    if app.explore_details_reveal {
        if let Some(line) = content.selected_line {
            if line < offset {
                offset = line;
            } else if line >= offset + content_area.height as usize {
                offset = line + 1 - content_area.height as usize;
            }
        }
        app.explore_details_reveal = false;
    }
    app.explore_details_scroll = offset;
    let has_vbar = total_lines > content_area.height as usize && content_area.width >= 3;
    let detail_text_area = if has_vbar {
        Rect {
            width: content_area.width - 1,
            ..content_area
        }
    } else {
        content_area
    };
    if !app.explore_edit_mode {
        for (index, rect) in content.item_rects(detail_text_area, offset) {
            app.clickable_regions
                .push(ClickableRegion::ExploreDetailItem { index, rect });
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(truncate_lines(lines, detail_text_area.width)))
            .scroll((offset.min(u16::MAX as usize) as u16, 0)),
        detail_text_area,
    );
    if has_vbar {
        let vbar = Rect::new(
            detail_text_area.right(),
            detail_text_area.y,
            1,
            detail_text_area.height,
        );
        render_vertical_scrollbar(
            frame,
            app,
            crate::app::VerticalScrollTarget::ExploreDetails,
            vbar,
            total_lines,
            offset,
        );
    }

    // Pixels draw over the reserved blank cells after the text frame.
    // ViewState::Fit already keeps scale ≤ 100% and only shrinks when the
    // capture exceeds the pane; placement centers the result horizontally.
    if pixels
        && let Some(preview) = content.preview.as_ref()
        && preview.line >= offset
        && preview.line < offset + content_area.height as usize
    {
        let top = content_area.y + (preview.line - offset) as u16;
        let rows = preview.rows.min(content_area.bottom().saturating_sub(top));
        if rows > 0 && content_area.width > 0 {
            let area = Rect::new(content_area.x, top, content_area.width, rows);
            let _ =
                app.inline_images
                    .place(frame, &preview.path, (area.width, area.height), area, 0);
        }
    }

    if let Some(action_area) = action_area {
        render_detail_actions(frame, app, action_area, active_tab);
    }
    if !app.explore_edit_mode && app.explore_history.menu().is_some() {
        render_run_menu(frame, app, body_area, header_area);
    }
}

/// Anchored run-history selector under the timestamp button.
fn render_run_menu(frame: &mut Frame<'_>, app: &mut App, body: Rect, header: Rect) {
    let Some(menu) = app.explore_history.menu() else {
        return;
    };
    let total = app.explore_history.total(menu.key);
    if total == 0 {
        return;
    }
    let rows: Vec<_> = (0..total)
        .filter_map(|index| {
            app.explore_history
                .row(menu.key, index)
                .map(|row| (index, row))
        })
        .collect();
    if rows.is_empty() {
        return;
    }
    let width = 44u16.min(body.width.saturating_sub(2)).max(28);
    let visible = rows.len().min(8);
    let height = (visible as u16 + 3).min(body.height);
    if height < 3 || width < 20 {
        return;
    }
    let x = body.right().saturating_sub(width + 1);
    let mut y = header.bottom();
    if y.saturating_add(height) > body.bottom() {
        y = body.bottom().saturating_sub(height);
    }
    let area = Rect::new(x, y, width, height);
    frame.render_widget(Clear, area);
    let block = theme::popup_block(" Runs · newest first ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let rows_area = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let footer_area = Rect {
        y: inner.bottom().saturating_sub(1),
        height: 1,
        ..inner
    };
    let visible_rows = rows_area.height as usize;
    if visible_rows == 0 {
        return;
    }
    let start = menu
        .selection
        .saturating_sub(visible_rows.saturating_sub(1));
    let current = app.explore_history.selected(menu.key).unwrap_or(0);
    let mut lines = Vec::new();
    for (offset, (index, row)) in rows.iter().enumerate().skip(start).take(visible_rows) {
        let marker = if *index == current { "●" } else { " " };
        let (label, color) = run_status_label(row.status);
        let line = Line::from(vec![
            Span::styled(format!(" {marker} "), Style::default().fg(theme::PRIMARY)),
            Span::styled(
                short_run_timestamp(&row.started_at),
                Style::default().fg(theme::FG),
            ),
            Span::raw("  "),
            Span::styled(format!("{label:<7}"), Style::default().fg(color)),
            Span::styled(
                format!("{:>6}", format_elapsed(row.duration_ms)),
                Style::default().fg(theme::MUTED),
            ),
        ]);
        let line = if *index == menu.selection {
            select_detail_line(line, true, rows_area.width)
        } else {
            pad_detail_line(line, rows_area.width)
        };
        app.clickable_regions.push(ClickableRegion::ExploreRunRow {
            index: *index,
            rect: Rect::new(
                rows_area.x,
                rows_area.y + (offset - start) as u16,
                rows_area.width,
                1,
            ),
        });
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), rows_area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            " j/k move · Enter · L latest · Esc",
            Style::default().fg(theme::MUTED),
        )),
        footer_area,
    );
}

fn run_status_label(status: RunStatus) -> (&'static str, Color) {
    match status {
        RunStatus::Passed => ("PASSED", theme::SUCCESS),
        RunStatus::Failed => ("FAILED", theme::ERROR),
        RunStatus::Blocked => ("BLOCKED", theme::WARNING),
        RunStatus::Skipped => ("SKIPPED", theme::MUTED),
        RunStatus::Running => ("RUNNING", theme::WARNING),
        RunStatus::Idle => ("UNKNOWN", theme::MUTED),
    }
}

/// Reserved pixel area for one selected screenshot.
struct DetailPreview {
    path: std::path::PathBuf,
    /// Content-relative line where the preview starts.
    line: usize,
    rows: u16,
}

/// Content-relative line span owned by one selectable Run Details row/block.
#[derive(Debug, Clone, Copy)]
struct DetailItemRange {
    index: usize,
    start: usize,
    /// Exclusive end line.
    end: usize,
}

/// Rendered Run Details content plus the geometry needed for mouse hits.
#[derive(Default)]
struct DetailContent {
    selected_line: Option<usize>,
    items: Vec<DetailItemRange>,
    preview: Option<DetailPreview>,
}

impl DetailContent {
    /// Clips item blocks to the visible viewport and returns screen rectangles.
    fn item_rects(&self, area: Rect, offset: usize) -> Vec<(usize, Rect)> {
        if area.height == 0 {
            return Vec::new();
        }
        let top = offset;
        let bottom = offset + area.height as usize;
        self.items
            .iter()
            .filter_map(|item| {
                let start = item.start.max(top);
                let end = item.end.min(bottom);
                (start < end).then(|| {
                    (
                        item.index,
                        Rect::new(
                            area.x,
                            area.y + (start - top) as u16,
                            area.width,
                            (end - start) as u16,
                        ),
                    )
                })
            })
            .collect()
    }
}

/// One inner tab's text offset and clickable width inside the tab row.
struct DetailTabHit {
    tab: crate::app::RunDetailTab,
    x: u16,
    width: u16,
}

/// Compact `MM-DD HH:MM:SS` for a selector row.
fn short_run_timestamp(started_at: &str) -> String {
    let bytes = started_at.as_bytes();
    if started_at.len() >= 19
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && matches!(bytes.get(10).copied(), Some(b'T' | b' '))
    {
        format!(
            "{}-{} {}",
            &started_at[5..7],
            &started_at[8..10],
            &started_at[11..19]
        )
    } else {
        started_at.to_string()
    }
}

/// Compact `MM-DD HH:MM` from an RFC 3339 timestamp; other text is kept as-is.
fn short_run_time(started_at: &str) -> String {
    let bytes = started_at.as_bytes();
    if started_at.len() >= 16
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && matches!(bytes.get(10).copied(), Some(b'T' | b' '))
    {
        format!(
            "{}-{} {}",
            &started_at[5..7],
            &started_at[8..10],
            &started_at[11..16]
        )
    } else {
        started_at.to_string()
    }
}

/// Selected step text, step-level status chip, and the run-history selector.
fn detail_step_status_line(
    step_text: &str,
    status: RunStatus,
    run_selector: Option<Vec<Span<'static>>>,
    width: u16,
) -> Line<'static> {
    let (label, background) = match status {
        RunStatus::Passed => ("PASSED", theme::SUCCESS),
        RunStatus::Failed => ("FAILED", theme::ERROR),
        RunStatus::Blocked => ("BLOCKED", theme::WARNING),
        RunStatus::Running => ("RUNNING", theme::WARNING),
        RunStatus::Skipped => ("SKIPPED", theme::MUTED),
        RunStatus::Idle => ("IDLE", theme::MUTED),
    };
    let selector_width = run_selector.as_ref().map_or(0, |spans| {
        spans
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum::<usize>()
    });
    // Reserve the status chip plus a gap; leave the selector its natural width.
    let reserved = selector_width + 12;
    let spans = vec![
        Span::styled(
            truncate_ellipsis(step_text, width.saturating_sub(reserved as u16) as usize),
            Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        theme::chip(format!(" {label} "), background),
    ];
    detail_line_with_right_spans(spans, run_selector.unwrap_or_default(), width)
}

/// Inner family tabs, showing only families that have content.
///
/// Returns the rendered line and each tab's span so the caller can register
/// click targets without duplicating the width arithmetic. Active and inactive
/// tabs share a solid fill so the row reads as a clickable control strip.
fn detail_tabs_line(
    app: &App,
    tabs: &[crate::app::RunDetailTab],
    active: Option<crate::app::RunDetailTab>,
    width: u16,
) -> (Line<'static>, Vec<DetailTabHit>) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut hits = Vec::new();
    let mut used = 0usize;
    for tab in tabs {
        let count = match tab {
            crate::app::RunDetailTab::Screenshots => app.detail_screenshots().len(),
            crate::app::RunDetailTab::Api => app.detail_exchanges().len(),
            crate::app::RunDetailTab::Logs => {
                app.current_detail().map_or(0, |detail| detail.logs.len())
            }
            crate::app::RunDetailTab::Files => app.detail_files().len(),
        };
        // One filled chip per family: label and count share the same background
        // so inactive tabs still look clickable, not like muted prose.
        let label = format!(" {} {count} ", tab.label());
        let tab_width = UnicodeWidthStr::width(label.as_str());
        if used + tab_width + 1 > width as usize {
            break;
        }
        let selected = active == Some(*tab);
        let style = if selected {
            Style::default()
                .fg(theme::FG)
                .bg(theme::TAB_ACTIVE_BG)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(theme::TAB_INACTIVE).bg(theme::BLOCK_BG)
        };
        hits.push(DetailTabHit {
            tab: *tab,
            x: used as u16,
            width: tab_width as u16,
        });
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
        used += tab_width + 1;
    }
    (Line::from(spans), hits)
}

/// Screenshot rows with an inline preview box reserved for the selected image.
fn push_screenshot_rows(
    lines: &mut Vec<Line<'static>>,
    app: &App,
    shots: &[crate::app::DetailScreenshot],
    selection: usize,
    width: u16,
    pixels: bool,
    preview_rows: u16,
) -> DetailContent {
    let mut content = DetailContent::default();
    if shots.is_empty() {
        lines.push(Line::styled(
            "  No screenshots for this step",
            Style::default().fg(theme::MUTED),
        ));
        return content;
    }
    let selected = selection.min(shots.len().saturating_sub(1));
    let shot = &shots[selected];
    let can_preview = pixels && shot.size.is_some() && shot.path.is_some();
    if can_preview {
        // Image only: filenames appear only when the terminal cannot draw pixels.
        let start = lines.len();
        let rows = preview_rows.max(1);
        for _ in 0..rows {
            lines.push(pad_detail_line(Line::raw(""), width));
        }
        content.selected_line = Some(start);
        let Some(path) = shot.path.clone() else {
            return content;
        };
        content.preview = Some(DetailPreview {
            path,
            line: start,
            rows,
        });
        content.items.push(DetailItemRange {
            index: selected,
            start,
            end: lines.len(),
        });
        return content;
    }

    // Fallback list: keep a path so the step is still actionable without pixels.
    for (index, shot) in shots.iter().enumerate() {
        let start = lines.len();
        let selected = index == selection;
        let path_label = shot.path.as_deref().map_or_else(
            || shot.reported.clone(),
            |path| app.project_display_path(path),
        );
        let mut row = vec![
            Span::styled("▣ ", Style::default().fg(theme::PRIMARY)),
            Span::styled(shot.file.clone(), Style::default().fg(theme::FG)),
        ];
        let size = shot.size.map_or_else(String::new, human_bytes);
        if !size.is_empty() {
            row.push(Span::raw(" "));
        }
        let line = detail_line_with_right(
            row,
            Span::styled(size, Style::default().fg(theme::MUTED)),
            width,
        );
        let line = select_detail_line(line, selected, width);
        if selected {
            content.selected_line = Some(start);
        }
        lines.push(line);
        lines.push(pad_detail_line(
            Line::styled(format!(" {path_label}"), Style::default().fg(theme::MUTED)),
            width,
        ));
        if selected {
            lines.push(pad_detail_line(
                Line::styled(
                    "  preview unavailable — terminal has no graphics protocol",
                    Style::default().fg(theme::MUTED),
                ),
                width,
            ));
            if shot.path.is_none() {
                lines.push(pad_detail_line(
                    Line::styled(
                        "  path is outside .teshi/logs/replay-screenshots",
                        Style::default().fg(theme::WARNING),
                    ),
                    width,
                ));
            }
        }
        content.items.push(DetailItemRange {
            index,
            start,
            end: lines.len(),
        });
    }
    content
}

/// Recordings and unknown artifact kinds; never silently dropped.
fn push_file_rows(
    lines: &mut Vec<Line<'static>>,
    app: &App,
    files: &[crate::app::DetailFile],
    selection: usize,
    width: u16,
) -> DetailContent {
    let mut content = DetailContent::default();
    if files.is_empty() {
        lines.push(Line::styled(
            "  No recordings or attachments for this step",
            Style::default().fg(theme::MUTED),
        ));
        return content;
    }
    for (index, file) in files.iter().enumerate() {
        let start = lines.len();
        let selected = index == selection;
        let video = is_video_path(std::path::Path::new(&file.reported));
        let mut row = vec![Span::styled(
            if video { "▶ " } else { "◦ " },
            Style::default().fg(if video { theme::PRIMARY } else { theme::MUTED }),
        )];
        row.push(Span::styled(
            file.file.clone(),
            Style::default().fg(theme::FG),
        ));
        if !file.kind.is_empty() {
            row.push(Span::styled(
                format!(" · {}", file.kind),
                Style::default().fg(theme::MUTED),
            ));
        }
        let size = file.size.map_or_else(String::new, human_bytes);
        if !size.is_empty() {
            row.push(Span::raw(" "));
        }
        let line = select_detail_line(
            detail_line_with_right(
                row,
                Span::styled(size, Style::default().fg(theme::MUTED)),
                width,
            ),
            selected,
            width,
        );
        if selected {
            content.selected_line = Some(start);
        }
        lines.push(line);
        let path_label = file.path.as_deref().map_or_else(
            || file.reported.clone(),
            |path| app.project_display_path(path),
        );
        lines.push(pad_detail_line(
            Line::styled(format!(" {path_label}"), Style::default().fg(theme::MUTED)),
            width,
        ));
        if selected && video {
            lines.push(pad_detail_line(
                Line::styled(
                    "  Enter opens the system player",
                    Style::default().fg(theme::MUTED),
                ),
                width,
            ));
        }
        content.items.push(DetailItemRange {
            index,
            start,
            end: lines.len(),
        });
    }
    content
}

/// HTTP exchanges with folded headers and on-demand bodies.
fn push_exchange_rows(
    lines: &mut Vec<Line<'static>>,
    app: &App,
    exchanges: &[teshi_core::HttpExchange],
    selection: usize,
    detail: Option<&CaseDetail>,
    width: u16,
) -> DetailContent {
    let mut content = DetailContent::default();
    if let Some(detail) = detail
        && let Some(message) = detail.message.as_deref()
        && !message.trim().is_empty()
    {
        let color = if detail.status == RunStatus::Blocked {
            theme::WARNING
        } else {
            theme::ERROR
        };
        lines.push(pad_detail_line(
            Line::styled(
                format!(
                    " assertion failed  {}",
                    message.lines().next().unwrap_or("")
                ),
                Style::default().fg(color),
            ),
            width,
        ));
        lines.push(pad_detail_line(Line::raw(""), width));
    }
    if exchanges.is_empty() {
        lines.push(Line::styled(
            "  No HTTP exchanges for this step",
            Style::default().fg(theme::MUTED),
        ));
        return content;
    }
    for (index, exchange) in exchanges.iter().enumerate() {
        let start = lines.len();
        let selected = index == selection;
        let status = exchange.status;
        let status_style = match status {
            Some(code) if code >= 400 => Style::default().fg(theme::ERROR),
            Some(_) => Style::default().fg(theme::SUCCESS),
            None => Style::default().fg(theme::MUTED),
        };
        let mut row = vec![
            Span::styled(
                format!("{} ", exchange.method),
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(exchange.url.clone(), Style::default().fg(theme::FG)),
        ];
        let status = status.map_or_else(|| "-".to_string(), |code| code.to_string());
        row.push(Span::raw(" "));
        let line = select_detail_line(
            detail_line_with_right(row, Span::styled(status, status_style), width),
            selected,
            width,
        );
        if selected {
            content.selected_line = Some(start);
        }
        lines.push(line);

        let mut meta = Vec::new();
        if let Some(duration) = exchange.duration_ms {
            meta.push(format!("{duration} ms"));
        }
        let request_bytes = json_bytes(&exchange.request_body);
        let response_bytes = json_bytes(&exchange.response_body);
        meta.push(format!(
            "req {} · res {}",
            human_bytes(request_bytes as u64),
            human_bytes(response_bytes as u64)
        ));
        if let Some(content_type) = content_type(exchange) {
            meta.push(content_type);
        }
        lines.push(pad_detail_line(
            Line::styled(
                format!(" {}", meta.join(" · ")),
                Style::default().fg(theme::MUTED),
            ),
            width,
        ));

        for assertion in exchange
            .asserts
            .iter()
            .filter(|assertion| !assertion.passed)
        {
            lines.push(pad_detail_line(
                Line::styled(
                    format!(" ✗ {}", assertion.name),
                    Style::default().fg(theme::ERROR),
                ),
                width,
            ));
        }

        let expanded = app.explore_details_expanded.contains(&exchange.exchange_id);
        push_fold_row(
            lines,
            expanded,
            "Request headers",
            &format!("{}", json_count(&exchange.request_headers)),
            width,
        );
        push_fold_row(
            lines,
            expanded,
            "Request body",
            &format!("{} · not loaded", human_bytes(request_bytes as u64)),
            width,
        );
        if expanded {
            push_json_lines(lines, &exchange.request_body, width);
        }
        push_fold_row(
            lines,
            expanded,
            "Response headers",
            &format!("{}", json_count(&exchange.response_headers)),
            width,
        );
        push_fold_row(
            lines,
            expanded,
            "Response body",
            &format!("{} · not loaded", human_bytes(response_bytes as u64)),
            width,
        );
        if expanded {
            push_json_lines(lines, &exchange.response_body, width);
            if !exchange.extract.is_null()
                && exchange
                    .extract
                    .as_object()
                    .is_some_and(|map| !map.is_empty())
            {
                lines.push(pad_detail_line(
                    Line::styled(
                        " extract",
                        Style::default()
                            .fg(theme::MUTED)
                            .add_modifier(Modifier::BOLD),
                    ),
                    width,
                ));
                push_json_lines(lines, &exchange.extract, width);
            }
        }
        lines.push(pad_detail_line(
            Line::styled(
                "  Space expands · body renders only on demand",
                Style::default().fg(theme::MUTED),
            ),
            width,
        ));
        lines.push(pad_detail_line(
            Line::styled(" ─────", Style::default().fg(theme::BORDER)),
            width,
        ));
        content.items.push(DetailItemRange {
            index,
            start,
            end: lines.len(),
        });
    }
    content
}

/// Runner logs are case-level; the selected row can be copied.
fn push_log_rows(
    lines: &mut Vec<Line<'static>>,
    logs: &[String],
    selection: usize,
    width: u16,
) -> DetailContent {
    let mut content = DetailContent::default();
    if logs.is_empty() {
        lines.push(Line::styled(
            "  No log lines for this case",
            Style::default().fg(theme::MUTED),
        ));
        return content;
    }
    for (index, log) in logs.iter().enumerate() {
        let start = lines.len();
        let selected = index == selection;
        let line = select_detail_line(
            pad_detail_line(
                Line::styled(format!(" {log}"), Style::default().fg(theme::FG)),
                width,
            ),
            selected,
            width,
        );
        if selected {
            content.selected_line = Some(start);
        }
        lines.push(line);
        content.items.push(DetailItemRange {
            index,
            start,
            end: lines.len(),
        });
    }
    content
}

fn push_empty_detail_rows(
    lines: &mut Vec<Line<'static>>,
    detail: Option<&CaseDetail>,
    status: RunStatus,
    width: u16,
) {
    lines.push(pad_detail_line(Line::raw(""), width));
    if let Some(detail) = detail {
        lines.push(pad_detail_line(
            Line::styled(
                "  No evidence recorded for this step",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::BOLD),
            ),
            width,
        ));
        if detail.historical && detail.steps.is_empty() {
            lines.push(pad_detail_line(
                Line::styled(
                    "  Case-level results only; no step-level screenshots, API or logs.",
                    Style::default().fg(theme::MUTED),
                ),
                width,
            ));
        }
        if let Some(message) = detail.message.as_deref()
            && !message.trim().is_empty()
        {
            let color = if status == RunStatus::Blocked {
                theme::WARNING
            } else {
                theme::ERROR
            };
            for line in message.lines().take(3) {
                lines.push(pad_detail_line(
                    Line::styled(format!("  {line}"), Style::default().fg(color)),
                    width,
                ));
            }
        }
    } else {
        lines.push(pad_detail_line(
            Line::styled(
                "  No run details yet",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::BOLD),
            ),
            width,
        ));
        lines.push(pad_detail_line(Line::raw(""), width));
        lines.push(pad_detail_line(
            Line::styled(
                "  Run [r] to execute scenarios.",
                Style::default().fg(theme::MUTED),
            ),
            width,
        ));
    }
}

fn render_detail_actions(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    active: Option<crate::app::RunDetailTab>,
) {
    let mut controls: Vec<ActionItem<'_>> = match active {
        Some(crate::app::RunDetailTab::Screenshots) => vec![
            ActionItem::new(
                "Fullscreen [Enter]",
                "Fullscreen",
                Action::ExploreDetailEnter,
                theme::ActionTone::Primary,
            ),
            ActionItem::new(
                "Open [o]",
                "Open",
                Action::ExploreOpenExternal,
                theme::ActionTone::Info,
            ),
            ActionItem::new(
                "Copy path [y]",
                "Copy",
                Action::ExploreCopyPath,
                theme::ActionTone::Neutral,
            ),
        ],
        Some(crate::app::RunDetailTab::Api) => vec![
            ActionItem::new(
                "Expand [Space]",
                "Expand",
                Action::ExploreDetailToggle,
                theme::ActionTone::Info,
            ),
            ActionItem::new(
                "Plaintext [p]",
                "Plaintext",
                Action::ToggleInspectorPlaintext,
                theme::ActionTone::Warning,
            ),
            ActionItem::new(
                "Copy URL [y]",
                "Copy",
                Action::ExploreCopyPath,
                theme::ActionTone::Neutral,
            ),
        ],
        Some(crate::app::RunDetailTab::Logs) => vec![ActionItem::new(
            "Copy line [y]",
            "Copy",
            Action::ExploreCopyPath,
            theme::ActionTone::Neutral,
        )],
        Some(crate::app::RunDetailTab::Files) => vec![
            ActionItem::new(
                "Open [Enter]",
                "Open",
                Action::ExploreDetailEnter,
                theme::ActionTone::Primary,
            ),
            ActionItem::new(
                "Copy path [y]",
                "Copy",
                Action::ExploreCopyPath,
                theme::ActionTone::Neutral,
            ),
        ],
        None => Vec::new(),
    };
    // Always first so the hint survives a narrow pane's control clipping.
    controls.insert(
        0,
        ActionItem::new(
            "[ ] switch run",
            "Runs",
            Action::ExploreRunOlder,
            theme::ActionTone::Neutral,
        ),
    );
    render_chip_action_bar(frame, &mut app.clickable_regions, area, &controls, false);
}

/// Join left and right spans on one row, right-aligning `right`.
fn detail_line_with_right(
    left: Vec<Span<'static>>,
    right: Span<'static>,
    width: u16,
) -> Line<'static> {
    detail_line_with_right_spans(left, vec![right], width)
}

fn detail_line_with_right_spans(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: u16,
) -> Line<'static> {
    let left_width: usize = left
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let right_width: usize = right
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let width = width as usize;
    if left_width + right_width < width {
        left.push(Span::raw(" ".repeat(width - left_width - right_width)));
    } else {
        left.push(Span::raw(" "));
    }
    left.extend(right);
    Line::from(left)
}

/// Apply the selection fill to a whole row and its trailing cells.
fn select_detail_line(mut line: Line<'static>, selected: bool, width: u16) -> Line<'static> {
    if selected {
        let style = theme::selected_row(true);
        line.style = style;
        for span in &mut line.spans {
            span.style = span.style.patch(style);
        }
        pad_detail_line(line, width)
    } else {
        pad_detail_line(line, width)
    }
}

fn pad_detail_line(line: Line<'static>, width: u16) -> Line<'static> {
    let style = line.style;
    pad_line_to_width(line, width, style)
}

fn push_fold_row(
    lines: &mut Vec<Line<'static>>,
    expanded: bool,
    label: &str,
    right: &str,
    width: u16,
) {
    let caret = if expanded { "▾ " } else { "▸ " };
    let spans = vec![
        Span::styled(caret, Style::default().fg(theme::MUTED)),
        Span::styled(label.to_string(), Style::default().fg(theme::FG)),
    ];
    lines.push(pad_detail_line(
        detail_line_with_right(
            spans,
            Span::styled(right.to_string(), Style::default().fg(theme::MUTED)),
            width,
        ),
        width,
    ));
}

fn push_json_lines(lines: &mut Vec<Line<'static>>, value: &serde_json::Value, width: u16) {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    for line in text.lines().take(12) {
        lines.push(pad_detail_line(
            Line::styled(format!("   {line}"), Style::default().fg(theme::MUTED)),
            width,
        ));
    }
    if text.lines().count() > 12 {
        lines.push(pad_detail_line(
            Line::styled("   …", Style::default().fg(theme::MUTED)),
            width,
        ));
    }
}

fn truncate_ellipsis(text: &str, max: usize) -> String {
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for character in text.chars() {
        if UnicodeWidthStr::width(out.as_str()) + UnicodeWidthChar::width(character).unwrap_or(0)
            >= max
        {
            break;
        }
        out.push(character);
    }
    out.push('…');
    out
}

fn format_elapsed(duration: Option<u64>) -> String {
    match duration {
        Some(ms) if ms >= 1000 => format!("{:.2}s", ms as f64 / 1000.0),
        Some(ms) => format!("{ms} ms"),
        None => "—".to_string(),
    }
}

fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let bytes = bytes as f64;
    if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn json_bytes(value: &serde_json::Value) -> usize {
    serde_json::to_string(value).map_or(0, |text| text.len())
}

fn json_count(value: &serde_json::Value) -> usize {
    value.as_object().map_or(0, |map| map.len())
}

/// Content type from either header block, lower-cased for the metadata row.
fn content_type(exchange: &teshi_core::HttpExchange) -> Option<String> {
    [&exchange.response_headers, &exchange.request_headers]
        .into_iter()
        .find_map(|headers| {
            headers.as_object().and_then(|headers| {
                headers.iter().find_map(|(name, value)| {
                    name.eq_ignore_ascii_case("content-type")
                        .then(|| value.as_str().unwrap_or_default().to_string())
                })
            })
        })
}

/// Whether the artifact path is a recording the system player should own.
fn is_video_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp4" | "webm" | "mov" | "mkv" | "avi"
            )
        })
}

fn truncate_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let mut out_lines = Vec::with_capacity(lines.len());
    for line in lines {
        out_lines.push(truncate_line_to_cols(line, width));
    }
    out_lines
}

/// Renders the auth management overlay panel over the entire screen.
fn render_auth_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::raw(""));

    match app.status.as_str() {
        s if s.starts_with("Auth: add provider") => {
            lines.push(Line::styled(
                crate::i18n::t("auth.add.title"),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(crate::i18n::t("auth.add.hint")));
        }
        s if s.starts_with("Auth: remove provider") => {
            let provider = s
                .strip_prefix("Auth: remove provider '")
                .and_then(|r| r.strip_suffix('\''))
                .unwrap_or("?");
            lines.push(Line::styled(
                crate::i18n::t("auth.remove.title").replace("{provider}", provider),
                Style::default()
                    .fg(theme::WARNING)
                    .add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(crate::i18n::t("auth.remove.hint")));
        }
        _ => {
            // Default: show shared model-profile overview
            lines.push(Line::styled(
                crate::i18n::t("auth.overview.title"),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));

            if app.model_profiles.is_empty() {
                lines.push(Line::raw(crate::i18n::t("auth.empty")));
                lines.push(Line::raw(crate::i18n::t("auth.empty.hint")));
            } else {
                for profile in &app.model_profiles {
                    let has_key = !profile.api_key.is_empty();
                    let key_status = if has_key { "✓" } else { "✗" };
                    let key_color = if has_key {
                        theme::SUCCESS
                    } else {
                        theme::ERROR
                    };
                    let active = if app.model_active_id.as_deref() == Some(profile.id.as_str()) {
                        " *"
                    } else {
                        ""
                    };
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            format!("{:<14}", profile.name),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{}{}", crate::i18n::t("auth.key"), key_status),
                            Style::default().fg(key_color),
                        ),
                        Span::raw(format!(
                            "  {} / {}{}",
                            profile.provider, profile.model_id, active
                        )),
                    ]));
                }
            }

            if let Some(ref active) = app.model_active_id {
                lines.push(Line::raw(""));
                lines.push(Line::from(vec![
                    Span::raw(crate::i18n::t("auth.active_id")),
                    Span::styled(active.as_str(), Style::default().fg(theme::PRIMARY)),
                ]));
            }
        }
    }

    // The overview grows with the profile list; the card keeps iCode's 85%
    // height cap and leaves the backdrop margin on every screen.
    let content_rows = lines.len() as u16 + 3;
    let panel_area = overlay::OverlaySpec::form(60, 8, 20).rect(area, (0, content_rows));
    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, panel_area);

    let block = theme::popup_block(crate::i18n::t("auth.title"));
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    let [content, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new(theme::popup_rows(lines, content.width)),
        content,
    );
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[(crate::i18n::t("dialog.close.esc"), Action::AuthPanelClose)],
        true,
    );
}

/// Renders the model profile management overlay panel.
fn render_model_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use crate::app::ModelPanelMode;

    // The list follows the profile count and the form its seven field rows,
    // both inside iCode's 85% height cap on a 60-column card. Below 14 rows the
    // share would clip the seventh field, so the card takes the full height.
    let content_rows = match app.model_panel_mode {
        ModelPanelMode::List if app.model_profiles.is_empty() => 8,
        ModelPanelMode::List => app.model_profiles.len() as u16 + 4,
        ModelPanelMode::Adding | ModelPanelMode::Editing => 11,
    };
    let height_share = if area.height < 14 {
        100
    } else {
        overlay::FORM_SHARE
    };
    // The list's five actions need the full 60-column card; spending the 10%
    // backdrop margin below that clips the Close control. Form cards keep the
    // same width so the list-to-form transition does not resize the card.
    let spec = overlay::OverlaySpec {
        width: overlay::Axis::cells(60, 60, 100),
        height: overlay::Axis::content(8, 22, height_share),
    };
    let panel_area = spec.rect(area, (0, content_rows));
    app.modal_card_rect = Some(panel_area);

    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, panel_area);

    match app.model_panel_mode {
        ModelPanelMode::List => render_model_list(frame, app, panel_area),
        ModelPanelMode::Adding => {
            render_model_form(frame, app, crate::i18n::t("model.panel.add"), panel_area)
        }
        ModelPanelMode::Editing => {
            render_model_form(frame, app, crate::i18n::t("model.panel.edit"), panel_area)
        }
    }
}

/// Renders the list of model profiles.
fn render_model_list(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let block = theme::popup_block(crate::i18n::t("model.panel.title"));
    let [inner, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(block.inner(area));
    frame.render_widget(block, area);
    let scroll = app
        .model_panel_selection
        .saturating_add(2)
        .saturating_sub(inner.height as usize);

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::raw(""));

    if app.model_profiles.is_empty() {
        lines.push(Line::from(vec![Span::raw(crate::i18n::t("model.empty"))]));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled(
            crate::i18n::t("model.empty.hint"),
            Style::default().fg(theme::MUTED),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "  ~/.config/teshi/models/",
            Style::default().fg(theme::MUTED),
        )]));
    } else {
        for (i, profile) in app.model_profiles.iter().enumerate() {
            if let Some(rect) = visible_content_rows(inner, lines.len(), 1, scroll) {
                app.clickable_regions
                    .push(ClickableRegion::ModelProfileRow { rect, index: i });
            }
            let is_active = app.model_active_id.as_deref() == Some(&profile.id);
            let prefix = if i == app.model_panel_selection {
                " ▸ "
            } else {
                "   "
            };
            let active_mark = if is_active {
                crate::i18n::t("model.active")
            } else {
                ""
            };

            let style = if i == app.model_panel_selection {
                theme::selected_row(true)
            } else if is_active {
                Style::default().fg(theme::SUCCESS)
            } else {
                Style::default()
            };

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{:<20}", profile.name), style),
                Span::raw("  "),
                Span::styled(
                    format!("{} | {}", profile.provider, profile.model_id),
                    Style::default().fg(theme::PRIMARY),
                ),
                Span::styled(active_mark, Style::default().fg(theme::SUCCESS)),
            ]));
        }
    }

    frame.render_widget(
        Paragraph::new(theme::popup_rows(lines, inner.width)).scroll((scroll as u16, 0)),
        inner,
    );
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[
            (crate::i18n::t("model.add"), Action::ModelPanelAdd),
            (crate::i18n::t("model.edit"), Action::ModelPanelEdit),
            (crate::i18n::t("model.delete"), Action::ModelPanelDelete),
            (crate::i18n::t("model.use"), Action::ModelPanelActivate),
            (crate::i18n::t("dialog.close.esc"), Action::ModelPanelClose),
        ],
        true,
    );
}

/// Renders the model profile form (add or edit).
fn render_model_form(frame: &mut Frame<'_>, app: &mut App, title: &str, area: Rect) {
    let block = theme::popup_block(title);
    let [inner, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(block.inner(area));
    frame.render_widget(block, area);

    // 7 form fields
    let fields: [(&str, &str); 7] = [
        (crate::i18n::t("model.field.name"), &app.model_form_name),
        (
            crate::i18n::t("model.field.provider"),
            &app.model_form_provider,
        ),
        (crate::i18n::t("model.field.model"), &app.model_form_model),
        (
            crate::i18n::t("model.field.base_url"),
            &app.model_form_base_url,
        ),
        (
            crate::i18n::t("model.field.api_key"),
            &app.model_form_api_key,
        ),
        (
            crate::i18n::t("model.field.max_tokens"),
            &app.model_form_max_tokens,
        ),
        (
            crate::i18n::t("model.field.temperature"),
            &app.model_form_temperature,
        ),
    ];

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::raw(""));

    for (i, (label, value)) in fields.iter().enumerate() {
        if let Some(rect) = visible_content_rows(inner, lines.len(), 1, 0) {
            app.clickable_regions
                .push(ClickableRegion::ModelFormField { rect, index: i });
        }
        let focused = i == app.model_form_focus;
        let indicator = if focused { " ▸ " } else { "   " };
        let label_style = if focused {
            theme::selected_row(true)
        } else {
            Style::default().fg(theme::PRIMARY)
        };
        let val_style = if focused {
            theme::selected_row(true)
        } else {
            Style::default().fg(theme::FG)
        };
        let display_val = if focused {
            input_with_cursor(value, app.model_form_cursor, inner.width.saturating_sub(16))
        } else if value.is_empty() {
            if focused {
                crate::i18n::t("model.form.type_here").to_string()
            } else {
                crate::i18n::t("model.form.empty").to_string()
            }
        } else {
            // Mask API key when not focused
            if i == 4 && value.chars().count() > 8 {
                let chars: Vec<_> = value.chars().collect();
                format!(
                    "{}...{}",
                    chars[..4].iter().collect::<String>(),
                    chars[chars.len() - 4..].iter().collect::<String>()
                )
            } else {
                value.to_string()
            }
        };

        lines.push(Line::from(vec![
            Span::styled(indicator, label_style),
            Span::styled(format!("{:<12}", label), label_style),
            Span::raw(" "),
            Span::styled(display_val, val_style),
        ]));
    }

    frame.render_widget(Paragraph::new(theme::popup_rows(lines, inner.width)), inner);
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[
            (
                crate::i18n::t("model.form.save"),
                Action::ModelPanelFormSubmit,
            ),
            (
                crate::i18n::t("dialog.cancel.esc"),
                Action::ModelPanelFormCancel,
            ),
        ],
        true,
    );
}

/// Renders the session browser overlay panel.
fn format_session_size(bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) => format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)),
        None => "—".to_string(),
    }
}

fn session_relative_age(updated_at: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let Ok(updated) = updated_at.parse::<u64>() else {
        return "—".to_string();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(updated);
    let seconds = now.saturating_sub(updated) / 1000;
    let (key, value) = match seconds {
        0..=59 => ("age.just_now", None),
        60..=3599 => ("age.minutes", Some(seconds / 60)),
        3600..=86_399 => ("age.hours", Some(seconds / 3600)),
        _ => ("age.days", Some(seconds / 86_400)),
    };
    match value {
        Some(value) => crate::i18n::t(key).replace("{n}", &value.to_string()),
        None => crate::i18n::t(key).to_string(),
    }
}

pub(super) fn ellipsize_to_cols(text: &str, width: u16) -> String {
    let truncated = truncate_string_to_cols(text, width);
    if truncated.chars().count() < text.chars().count() && width > 1 {
        format!(
            "{}…",
            truncate_string_to_cols(text, width.saturating_sub(1))
        )
    } else {
        truncated
    }
}

/// Truncate long identifiers (such as git branch names) in the middle, keeping both prefix and suffix.
pub(super) fn middle_ellipsize_to_cols(text: &str, width: u16) -> String {
    let max = width as usize;
    if max == 0 {
        return String::new();
    }
    if text.width() <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let avail = max - 1;
    let head_budget = (avail * 3).div_ceil(5).max(1);
    let tail_budget = avail.saturating_sub(head_budget);
    let head = truncate_string_to_cols(text, head_budget as u16);
    let used_head = head.width();
    let tail_cap = avail.saturating_sub(used_head);
    let mut tail_rev = Vec::new();
    let mut tail_w = 0usize;
    for ch in text
        .chars()
        .skip(head.chars().count())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let w = ch.width().unwrap_or(0);
        if tail_w + w > tail_cap {
            break;
        }
        tail_w += w;
        tail_rev.push(ch);
    }
    let tail: String = tail_rev.into_iter().rev().collect();
    if tail.is_empty() && tail_budget > 0 {
        return ellipsize_to_cols(text, width);
    }
    format!("{head}…{tail}")
}

fn session_label(text: &str, width: u16) -> String {
    ellipsize_to_cols(text, width)
}

fn render_session_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let session_count = app.session_list.len();
    let desired_height = (session_count as u16)
        .saturating_mul(2)
        .saturating_add(4)
        .max(10);
    // Two rows per session plus the hint and action rows; the card stops at
    // 40 rows so a long history still shows the backdrop.
    let panel_area = overlay::OverlaySpec::prompt(88, 10, 40).rect(area, (0, desired_height));
    app.modal_card_rect = Some(panel_area);

    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, panel_area);

    let title = if app.session_panel_legacy {
        crate::i18n::t("sessions.title.legacy").replace("{count}", &session_count.to_string())
    } else {
        crate::i18n::fill(
            crate::i18n::t("sessions.title"),
            &[
                (
                    "project",
                    &app.project_switch.active.identity.display_name(),
                ),
                ("count", &session_count.to_string()),
            ],
        )
    };
    let block = theme::popup_block(title.as_str());
    let [inner, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
        .areas(block.inner(panel_area));
    frame.render_widget(block, panel_area);

    if inner.width < 10 || inner.height < 2 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    if app.session_panel_legacy {
        lines.push(Line::raw(crate::i18n::t("sessions.hint.legacy")));
    } else {
        lines.push(Line::raw(crate::i18n::t("sessions.hint.scope")));
    }

    if app.session_list.is_empty() {
        lines.push(Line::raw(crate::i18n::t("sessions.empty")));
        lines.push(Line::raw(""));
        lines.push(Line::raw(crate::i18n::t("sessions.empty.hint")));
    } else {
        let visible_sessions = (inner.height.saturating_sub(1) as usize / 2).max(1);
        let start = app
            .session_panel_selection
            .saturating_sub(visible_sessions.saturating_sub(1));
        for (index, session) in app
            .session_list
            .iter()
            .enumerate()
            .skip(start)
            .take(visible_sessions)
        {
            let is_selected = index == app.session_panel_selection;
            let current = app.agent().current_session_id() == Some(session.id.as_str());
            let row = lines.len();
            if let Some(rect) = visible_content_rows(inner, row, 2, 0) {
                app.clickable_regions
                    .push(ClickableRegion::SessionRow { rect, index });
            }
            let marker = if current {
                "●"
            } else if is_selected {
                "▸"
            } else {
                " "
            };
            let dir = session
                .project_root
                .as_ref()
                .and_then(|root| root.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::i18n::t("sessions.unassigned").to_string());
            let model = session
                .model_label
                .clone()
                .or_else(|| session.acp_agent_id.clone())
                .unwrap_or_else(|| "—".to_string());
            let first = session_label(
                &crate::i18n::fill(
                    crate::i18n::t("sessions.row"),
                    &[
                        ("marker", marker),
                        ("title", &session.display_title()),
                        ("count", &session.message_count.to_string()),
                        ("size", &format_session_size(session.file_size())),
                    ],
                ),
                inner.width,
            );
            let second = session_label(
                &crate::i18n::fill(
                    crate::i18n::t("sessions.row.meta"),
                    &[
                        ("dir", &dir),
                        ("age", &session_relative_age(&session.updated_at)),
                        ("model", &model),
                    ],
                ),
                inner.width,
            );
            let style = if is_selected {
                theme::selected_row(true)
            } else {
                Style::default()
            };
            lines.push(Line::styled(first, style));
            lines.push(Line::styled(
                second,
                style.patch(Style::default().fg(theme::MUTED)),
            ));
        }
    }

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(
        Paragraph::new(Text::from(theme::popup_rows(visible, inner.width))),
        inner,
    );
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[
            (
                crate::i18n::t("sessions.load"),
                Action::SessionPanelActivate,
            ),
            (
                crate::i18n::t("sessions.delete"),
                Action::SessionPanelDelete,
            ),
            (
                crate::i18n::t("dialog.close.esc"),
                Action::SessionPanelClose,
            ),
        ],
        true,
    );
}

/// Ctrl+R prompt-history search overlay.
fn render_history_search_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(search) = app.history_search.as_ref() else {
        return;
    };
    let matches = app.prompt_history.search(&search.query);
    let selection = search.selection.min(matches.len().saturating_sub(1));
    let query = search.query.clone();

    // A command-palette shape: a share of the screen with an absolute cap, so
    // the list stops widening on very large terminals. Percent axes ignore the
    // measurement tuple.
    const HISTORY_SEARCH: overlay::OverlaySpec = overlay::OverlaySpec {
        width: overlay::Axis::percent(70, 36, 100, overlay::DEFAULT_SHARE),
        height: overlay::Axis::percent(55, 10, 24, overlay::DEFAULT_SHARE),
    };
    let popup = HISTORY_SEARCH.rect(area, (0, 0));
    app.modal_card_rect = Some(popup);
    clear_to(frame, area, theme::OVERLAY_BG);
    let block = theme::popup_block(" Prompt history ");
    let [query_row, list, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(block.inner(popup));
    frame.render_widget(block, popup);

    let caret = if query.is_empty() { " " } else { "" };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Filter: ", theme::muted()),
            Span::styled(query.clone(), theme::input_style(true)),
            Span::styled(
                caret,
                Style::default().bg(theme::CURSOR_BG).fg(theme::CURSOR_FG),
            ),
        ])),
        query_row,
    );

    let mut lines: Vec<Line<'static>> = Vec::new();
    if matches.is_empty() {
        lines.push(Line::styled("No matches", theme::muted()));
    } else {
        let visible = list.height as usize;
        let start = selection.saturating_sub(visible.saturating_sub(1));
        for (index, text) in matches.iter().enumerate().skip(start).take(visible) {
            let preview: String = text
                .chars()
                .take(popup.width.saturating_sub(4) as usize)
                .collect();
            let marker = if index == selection { "▸ " } else { "  " };
            let style = if index == selection {
                theme::selected_row(true)
            } else {
                Style::default()
            };
            lines.push(Line::styled(format!("{marker}{preview}"), style));
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(theme::popup_rows(lines, list.width))),
        list,
    );
    frame.render_widget(
        Paragraph::new(theme::footer_line([
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" select  ", theme::muted()),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel", theme::muted()),
        ])),
        footer,
    );
}

/// Fullscreen draft editor opened with Ctrl+O.
fn render_composer_editor_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let Some(editor) = app.composer_editor.as_ref() else {
        return;
    };
    let popup_w = area.width.saturating_sub(4).max(40).min(area.width);
    let popup_h = area.height.saturating_sub(4).max(12).min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(popup_w)) / 2,
        area.y + (area.height.saturating_sub(popup_h)) / 2,
        popup_w,
        popup_h,
    );
    clear_to(frame, area, theme::OVERLAY_BG);
    let block = theme::popup_block(" Draft editor ");
    let [body, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(block.inner(popup));
    frame.render_widget(block, popup);

    let width = body.width.max(1);
    let layout = crate::composer::layout(editor.draft.text(), width);
    let cursor = layout.position_of(editor.draft.cursor());
    let scroll = cursor.0.saturating_sub(body.height.saturating_sub(1));
    let rows: Vec<Line<'static>> = layout
        .rows
        .iter()
        .skip(usize::from(scroll))
        .take(body.height as usize)
        .map(|row| Line::raw(row.clone()))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(rows)), body);
    if let Some(selection) = editor.draft.selection() {
        paint_overlay_composer_selection(frame, body, &layout, selection, scroll);
    }
    let under = editor.draft.chars().nth(editor.draft.cursor());
    let position = (body.x + cursor.1, body.y + cursor.0 - scroll);
    if body.contains(position.into()) {
        paint_overlay_block_cursor(frame, body, position, under);
        frame.set_cursor_position(position);
    }
    frame.render_widget(
        Paragraph::new(theme::footer_line([
            Span::styled(
                "Ctrl+S",
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" apply  ", theme::muted()),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" discard", theme::muted()),
        ])),
        footer,
    );
}

fn paint_overlay_composer_selection(
    frame: &mut Frame<'_>,
    area: Rect,
    layout: &crate::composer::Layout,
    selection: std::ops::Range<usize>,
    scroll: u16,
) {
    let style = Style::default()
        .fg(theme::FG)
        .bg(theme::EDITOR_SELECTION_BG);
    for &(row, column, cells) in layout.cells.get(selection).unwrap_or_default() {
        if cells == 0 || row < scroll {
            continue;
        }
        let y = area.y + row - scroll;
        if y >= area.bottom() {
            continue;
        }
        let start = area.x + column;
        let end = (start + cells).min(area.right());
        for x in start..end {
            frame.buffer_mut()[(x, y)].set_style(style);
        }
    }
}

fn paint_overlay_block_cursor(
    frame: &mut Frame<'_>,
    area: Rect,
    (x, y): (u16, u16),
    under: Option<char>,
) {
    let cells = under
        .filter(|ch| !ch.is_control())
        .and_then(UnicodeWidthChar::width)
        .unwrap_or(1)
        .max(1) as u16;
    let style = Style::default().fg(theme::CURSOR_FG).bg(theme::CURSOR_BG);
    let buffer = frame.buffer_mut();
    for column in x..x.saturating_add(cells).min(area.right()) {
        if area.contains((column, y).into()) {
            buffer[(column, y)].set_style(style);
        }
    }
}

/// Render the quit confirmation popup panel.
fn render_quit_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let message = if app.dirty {
        crate::i18n::t("dialog.quit.body_dirty")
    } else {
        crate::i18n::t("dialog.quit.body")
    };
    // The arrow keys move the selection that Enter activates.
    let rects = Dialog {
        title: crate::i18n::t("dialog.quit.title"),
        warning: app.dirty,
        body: vec![dialog_question(message.to_string())],
        buttons: vec![
            DialogButton {
                label: crate::i18n::t("dialog.quit.yes"),
                tone: ButtonTone::Danger,
                focused: app.quit_panel_selection == 0,
            },
            DialogButton {
                label: crate::i18n::t("dialog.quit.no"),
                tone: ButtonTone::Warning,
                focused: app.quit_panel_selection == 1,
            },
        ],
    }
    .render(frame, area);
    if let [yes, no] = rects[..] {
        app.clickable_regions
            .push(ClickableRegion::QuitConfirmYes { rect: yes });
        app.clickable_regions
            .push(ClickableRegion::QuitConfirmNo { rect: no });
    }
}

/// Render the "interrupt the current run" confirmation popup.
///
/// Mirrors the host agent's own prompt: the turn is only cut when the user
/// confirms, and the ACP session behind it is kept either way.
fn render_interrupt_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let rects = Dialog {
        title: crate::i18n::t("dialog.interrupt.title"),
        warning: false,
        body: vec![
            dialog_question(crate::i18n::t("dialog.interrupt.body").to_string()),
            Line::styled(crate::i18n::t("dialog.interrupt.hint"), theme::muted()),
        ],
        buttons: vec![
            DialogButton {
                label: crate::i18n::t("dialog.interrupt.confirm"),
                tone: ButtonTone::Danger,
                focused: app.ai_interrupt_selection == 0,
            },
            DialogButton {
                label: crate::i18n::t("dialog.interrupt.continue"),
                tone: ButtonTone::Warning,
                focused: app.ai_interrupt_selection == 1,
            },
        ],
    }
    .render(frame, area);
    if let [interrupt, keep] = rects[..] {
        app.clickable_regions
            .push(ClickableRegion::AiInterruptConfirm { rect: interrupt });
        app.clickable_regions
            .push(ClickableRegion::AiInterruptContinue { rect: keep });
    }
}

/// Render the approval mode selection popup panel.
fn render_approval_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use teshi_agent::approval::ApprovalMode;

    let modes = ApprovalMode::ALL;
    let panel_w = area.width.min(56);
    // Layout: top border + 3 items × (title + desc) + separator line + bottom border
    let item_rows: u16 = (modes.len() * 2 + 1) as u16; // +1 for separator
    let panel_h = (item_rows + 4).min(area.height);
    let panel_x = area.x + (area.width.saturating_sub(panel_w)) / 2;
    let panel_y = area.y + (area.height.saturating_sub(panel_h)) / 2;
    let panel_area = Rect::new(panel_x, panel_y, panel_w, panel_h);
    app.modal_card_rect = Some(panel_area);

    frame.render_widget(Clear, panel_area);

    let overridden = app.agent().backend.acp_controls().is_some()
        && std::env::var("TESHI_ACP_PERMISSION_POLICY").as_deref() == Ok("auto");
    let block = theme::popup_block(if overridden {
        crate::i18n::t("approval.title_override")
    } else {
        crate::i18n::t("approval.title")
    });
    let [inner, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
        .areas(block.inner(panel_area));
    frame.render_widget(block, panel_area);

    // Track which rows are rendered so we can register clickable regions
    let mut row_offset = inner.y;
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, mode) in modes.iter().enumerate() {
        let is_current = *mode == app.effective_approval_mode();
        let is_selected = i == app.approval_panel_selection;
        let label = mode.display_name();
        let desc = mode.description();

        // ── Title line ──
        let (main_fg, desc_fg, marker) = if is_selected {
            // Selected item: always yellow with arrow, regardless of current
            (theme::WARNING, theme::MUTED, "\u{25b6}")
        } else if is_current {
            // Current but not selected: dimmed with circle
            (theme::MUTED, theme::MUTED, "\u{25cb}")
        } else {
            (theme::FG, theme::TAB_INACTIVE, "  ")
        };
        let title = format!(" {} {}", marker, label);
        let mut title_style =
            Style::default()
                .fg(main_fg)
                .add_modifier(if is_selected || is_current {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                });
        if is_selected {
            title_style = theme::selected_row(true);
        }
        // Hover effect: reverse video to indicate clickability
        let is_hovered = app
            .mouse_position
            .is_some_and(|(mx, my)| my == row_offset && mx >= inner.x && mx < inner.right());
        if is_hovered {
            title_style = title_style.add_modifier(Modifier::REVERSED);
        }
        lines.push(Line::styled(title, title_style));

        // Register clickable region for this option's title row
        app.clickable_regions.push(ClickableRegion::ApprovalOption {
            option_idx: i,
            row_y: row_offset,
            col_x: inner.x,
            col_right: inner.right(),
        });
        row_offset += 1;

        // ── Description line ──
        let desc_text = format!("   {}", desc);
        lines.push(Line::styled(desc_text, Style::default().fg(desc_fg)));
        row_offset += 1;

        // ── Separator ──
        if i < modes.len() - 1 {
            lines.push(Line::raw(""));
            row_offset += 1;
        }
    }

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(
        Paragraph::new(Text::from(theme::popup_rows(visible, inner.width))),
        inner,
    );
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[(crate::i18n::t("dialog.close.esc"), Action::ClearInputState)],
        true,
    );
}

/// Render the app Settings overlay (prompt injection + UI language).
///
/// The layout mirrors iCode's settings dialog: a quiet card on the dialog
/// surface, a tab-like header, bordered sections, and rows whose hints hang
/// under a `└─` glyph. The bottom border carries the save status and a
/// semantic Close control.
fn render_settings_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let panel_area = SETTINGS_SPEC.rect(area, (0, 0));
    if panel_area.width < 12 || panel_area.height < 5 {
        return;
    }
    app.modal_card_rect = Some(panel_area);
    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, panel_area);

    let status = crate::i18n::t("settings.autosave");
    let close = crate::i18n::t("settings.close_btn");
    let block = theme::dialog_block(crate::i18n::t("settings.title"), false)
        .border_style(Style::default().fg(theme::PRIMARY_MUTED))
        .title_bottom(Line::styled(status, theme::muted()).left_aligned())
        .title_bottom(
            Line::styled(
                close,
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )
            .right_aligned(),
        );
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    if panel_area.width > close.width() as u16 + 1 {
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: Rect::new(
                panel_area.right().saturating_sub(close.width() as u16 + 1),
                panel_area.bottom().saturating_sub(1),
                close.width() as u16,
                1,
            ),
            action: Action::SettingsPanelClose,
            modal: true,
        });
    }

    // Card padding: one row top/bottom, two columns left/right.
    let content = Rect::new(
        inner.x.saturating_add(2),
        inner.y.saturating_add(1),
        inner.width.saturating_sub(4),
        inner.height.saturating_sub(2),
    );
    if content.width < 12 || content.height < 10 {
        return;
    }

    let sections = Layout::vertical([
        Constraint::Length(1), // tab header
        Constraint::Length(1), // rule
        Constraint::Length(1), // gap
        Constraint::Length(1), // subtitle
        Constraint::Length(1), // gap
        Constraint::Length(6), // AI Chat section
        Constraint::Length(1), // gap
        Constraint::Min(5),    // Language section
    ])
    .split(content);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            crate::i18n::t("settings.tab.general"),
            Style::default()
                .fg(theme::PRIMARY)
                .bg(theme::TAB_ACTIVE_BG)
                .add_modifier(Modifier::BOLD),
        ))),
        sections[0],
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            "─".repeat(sections[1].width as usize),
            Style::default().fg(theme::BORDER),
        )),
        sections[1],
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            crate::i18n::t("settings.subtitle"),
            theme::muted(),
        )),
        sections[3],
    );

    render_settings_row(
        frame,
        app,
        sections[5],
        SettingsRow {
            index: 0,
            title: crate::i18n::t("settings.section.ai_chat"),
            label: crate::i18n::t("settings.inject_prompt"),
            hint: crate::i18n::t("settings.inject_prompt.hint"),
            kind: SettingsRowKind::Checkbox(app.inject_prompt_sections),
        },
    );
    render_settings_row(
        frame,
        app,
        sections[7],
        SettingsRow {
            index: 1,
            title: crate::i18n::t("settings.section.language"),
            label: crate::i18n::t("settings.ui_locale"),
            hint: crate::i18n::t("settings.ui_locale.hint"),
            kind: SettingsRowKind::Value(crate::i18n::preference_label(app.ui_locale)),
        },
    );
}

/// One selectable row rendered inside a Settings section card.
struct SettingsRow {
    /// Index into [`App::SETTINGS_ROWS`], shared with activation and selection.
    index: usize,
    /// Bordered card title above the row.
    title: &'static str,
    /// Row label to the right of the marker (or checkbox).
    label: &'static str,
    /// Muted explanation wrapped under the `└─` glyph.
    hint: &'static str,
    /// How the row presents its current value.
    kind: SettingsRowKind,
}

/// How the interactive Settings row presents its current value.
enum SettingsRowKind {
    Checkbox(bool),
    Value(&'static str),
}

/// Render one Settings section card with its selectable row.
///
/// The row registers a modal click target so pointer activation shares the
/// `SettingsPanelActivate(index)` path with Enter/Space.
fn render_settings_row(frame: &mut Frame<'_>, app: &mut App, area: Rect, row: SettingsRow) {
    let SettingsRow {
        index: row_index,
        title,
        label,
        hint,
        kind,
    } = row;
    let section = theme::panel_block(title, false).title_style(
        Style::default()
            .fg(theme::SECONDARY)
            .add_modifier(Modifier::BOLD),
    );
    let section_inner = section.inner(area);
    frame.render_widget(section, area);
    if section_inner.width < 8 || section_inner.height < 3 {
        return;
    }
    let body = Rect::new(
        section_inner.x.saturating_add(2),
        section_inner.y.saturating_add(1),
        section_inner.width.saturating_sub(4),
        section_inner.height.saturating_sub(2),
    );
    if body.width < 8 || body.height == 0 {
        return;
    }

    let selected = app.settings_panel_selection == row_index;
    let marker = if selected { "\u{25b6}" } else { " " };
    let hovered = app
        .mouse_position
        .is_some_and(|(mx, my)| my == body.y && mx >= body.x && mx < body.right());
    let mut label_style = Style::default().fg(theme::FG);
    if selected || hovered {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }

    let mut spans = vec![Span::styled(
        format!(" {marker} "),
        Style::default().fg(theme::PRIMARY),
    )];
    match kind {
        SettingsRowKind::Checkbox(enabled) => {
            let (check, check_style) = if enabled {
                (
                    "[x]",
                    Style::default()
                        .fg(theme::SUCCESS)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("[ ]", Style::default().fg(theme::MUTED))
            };
            spans.push(Span::styled(check, check_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(label, label_style));
        }
        SettingsRowKind::Value(value) => {
            spans.push(Span::styled(label, label_style));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                value,
                Style::default()
                    .fg(theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(body.x, body.y, body.width, 1),
    );
    app.clickable_regions.push(ClickableRegion::ActionButton {
        rect: Rect::new(body.x, body.y, body.width, 1),
        action: Action::SettingsPanelActivate(row_index),
        modal: true,
    });

    let hint_width = body.width.saturating_sub(7) as usize;
    if body.height > 1 && hint_width > 0 {
        let mut rows =
            chat::wrap_line_to_width(Line::from(Span::styled(hint, theme::muted())), hint_width);
        for (index, row) in rows.iter_mut().enumerate() {
            row.spans
                .insert(0, Span::raw(if index == 0 { "    └─ " } else { "       " }));
        }
        rows.truncate(body.height.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(Text::from(rows)),
            Rect::new(body.x, body.y + 1, body.width, body.height - 1),
        );
    }
}

/// Sidebar width for the agent configuration modal (matches AI agent sidebar).
const AGENT_PROFILE_SIDEBAR_WIDTH: u16 = 28;
const AGENT_PROFILE_PANEL_MIN_WIDTH: u16 = 60;

fn render_acp_agent_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let panel = agent_profile_panel_rect(area);
    app.modal_card_rect = Some(panel);
    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, panel);
    let title = match app.agent_panel_mode {
        crate::app::AgentPanelMode::SelectBackend => {
            crate::i18n::t("agent.panel.select").to_string()
        }
        crate::app::AgentPanelMode::InstalledAcp => crate::i18n::t("agent.panel.installed")
            .replace("{count}", &app.installed_acp_agents.len().to_string()),
        crate::app::AgentPanelMode::Registry => crate::i18n::t("agent.panel.registry")
            .replace("{count}", &app.acp_registry_agents.len().to_string()),
        _ => crate::i18n::t("agent.panel.title").to_string(),
    };
    let block = theme::popup_block(title);
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    let [list_area, footer_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .areas(inner);
    let mut controls = vec![(
        if app.agent_panel_mode == crate::app::AgentPanelMode::Registry {
            crate::i18n::t("agent.install")
        } else {
            crate::i18n::t("agent.use")
        },
        Action::AgentPanelSelect,
    )];
    if app.agent_panel_mode == crate::app::AgentPanelMode::InstalledAcp {
        controls.push((
            crate::i18n::t("agent.registry.browse"),
            Action::AgentPanelAdd,
        ));
    }
    controls.push((crate::i18n::t("dialog.close.esc"), Action::AgentPanelClose));
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer_area,
        "",
        &controls,
        true,
    );

    let muted = Style::default().fg(theme::MUTED);
    let id_style = Style::default().fg(theme::PRIMARY);
    let detail_width = list_area.width.saturating_sub(4) as usize;
    let selected_style = theme::selected_row(true);
    let rows = |index| {
        if index == app.agent_profile_panel_selection {
            selected_style
        } else {
            Style::default()
        }
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if app.agent_panel_mode == crate::app::AgentPanelMode::SelectBackend {
        let marker = if app.agent_profile_panel_selection == 0 {
            "▶ "
        } else {
            "  "
        };
        let active = if app.agent().backend_kind() == teshi_agent::backend::AgentBackendKind::Native
        {
            crate::i18n::t("agent.selected")
        } else {
            ""
        };
        lines.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(crate::i18n::t("agent.native"), rows(0)),
            Span::styled(active, Style::default().fg(theme::SUCCESS)),
        ]));
    }
    match app.agent_panel_mode {
        crate::app::AgentPanelMode::SelectBackend | crate::app::AgentPanelMode::InstalledAcp => {
            for (index, agent) in app.installed_acp_agents.iter().enumerate() {
                let row = index
                    + usize::from(
                        app.agent_panel_mode == crate::app::AgentPanelMode::SelectBackend,
                    );
                let marker = if row == app.agent_profile_panel_selection {
                    "▶ "
                } else {
                    "  "
                };
                let active = if app.agent().selected_acp_id.as_deref() == Some(&agent.id) {
                    crate::i18n::t("agent.selected")
                } else {
                    ""
                };
                lines.push(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(agent.name.clone(), rows(row)),
                    Span::styled(format!(" ({}) ", agent.id), id_style),
                    theme::chip(crate::i18n::t("agent.installed"), theme::SUCCESS),
                    Span::styled(active, Style::default().fg(theme::SUCCESS)),
                ]));
            }
        }
        crate::app::AgentPanelMode::Registry => {
            for (index, agent) in app.acp_registry_agents.iter().enumerate() {
                let marker = if index == app.agent_profile_panel_selection {
                    "▶ "
                } else {
                    "  "
                };
                let (badge, badge_style) = if app
                    .installed_acp_agents
                    .iter()
                    .any(|item| item.id == agent.id)
                {
                    (
                        crate::i18n::t("agent.installed"),
                        Style::default().fg(theme::SUCCESS),
                    )
                } else {
                    (crate::i18n::t("agent.available"), muted)
                };
                lines.push(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(agent.name.clone().unwrap_or(agent.id.clone()), rows(index)),
                    Span::styled(format!(" ({}) ", agent.id), id_style),
                    Span::styled(badge, badge_style),
                ]));
                let description = agent
                    .description
                    .as_deref()
                    .unwrap_or(crate::i18n::t("agent.no_description"));
                lines.push(Line::styled(
                    format!("   {}", truncate_str_to_width(description, detail_width)),
                    muted,
                ));
                lines.push(Line::raw(""));
            }
        }
        _ => {}
    }
    if lines.is_empty() {
        let label = match app.agent_panel_mode {
            crate::app::AgentPanelMode::Registry if app.acp_registry_rx.is_some() => {
                crate::i18n::t("agent.registry.loading")
            }
            crate::app::AgentPanelMode::Registry => crate::i18n::t("agent.registry.empty"),
            crate::app::AgentPanelMode::InstalledAcp => crate::i18n::t("agent.installed.empty"),
            _ => crate::i18n::t("agent.empty"),
        };
        frame.render_widget(
            Paragraph::new(Line::styled(label, muted)).alignment(Alignment::Center),
            Rect::new(
                list_area.x,
                list_area.y + list_area.height / 2,
                list_area.width,
                1,
            ),
        );
        return;
    }
    let per_row = if app.agent_panel_mode == crate::app::AgentPanelMode::Registry {
        3
    } else {
        1
    };
    let selected_line = app.agent_profile_panel_selection * per_row;
    let scroll = selected_line
        .saturating_add(per_row)
        .saturating_sub(list_area.height as usize);
    let count = match app.agent_panel_mode {
        crate::app::AgentPanelMode::SelectBackend => app.installed_acp_agents.len() + 1,
        crate::app::AgentPanelMode::InstalledAcp => app.installed_acp_agents.len(),
        crate::app::AgentPanelMode::Registry => app.acp_registry_agents.len(),
        crate::app::AgentPanelMode::List => 0,
    };
    for index in 0..count {
        if let Some(rect) = visible_content_rows(list_area, index * per_row, per_row, scroll) {
            app.clickable_regions
                .push(ClickableRegion::AgentProfileRow { rect, index });
        }
    }
    frame.render_widget(
        Paragraph::new(theme::popup_rows(lines, inner.width)).scroll((scroll as u16, 0)),
        list_area,
    );
}
const AGENT_PROFILE_PANEL_MAX_WIDTH: u16 = 100;
const AGENT_PROFILE_PANEL_MIN_HEIGHT: u16 = 18;
const AGENT_PROFILE_PANEL_MAX_HEIGHT: u16 = 50;

/// Compute centered agent configuration modal bounds (iCode's 95% x 85% with caps).
fn agent_profile_panel_rect(area: Rect) -> Rect {
    AGENT_PROFILE_SPEC.rect(area, (0, 0))
}

/// Render the agent profile configuration overlay (chrys-style layout).
fn render_agent_profile_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let panel_area = agent_profile_panel_rect(area);
    app.modal_card_rect = Some(panel_area);
    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, panel_area);

    let block = theme::popup_block(crate::i18n::t("agent.config.title"));
    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    let [inner, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    render_action_bar(
        frame,
        &mut app.clickable_regions,
        footer,
        "",
        &[
            (crate::i18n::t("agent.apply"), Action::AgentPanelSelect),
            (crate::i18n::t("dialog.close.esc"), Action::AgentPanelClose),
        ],
        true,
    );

    let [sidebar_area, main_area] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(AGENT_PROFILE_SIDEBAR_WIDTH),
            Constraint::Min(10),
        ])
        .areas(inner);

    render_agent_profile_sidebar(frame, app, sidebar_area);
    render_agent_profile_main(frame, app, main_area);
}

/// Left sidebar: profile list with two-line entries (name + description).
fn render_agent_profile_sidebar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme::MUTED));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let scroll = app
        .agent_profile_panel_selection
        .saturating_mul(3)
        .saturating_add(2)
        .saturating_sub(inner.height as usize);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, profile) in app.agent_registry.iter().enumerate() {
        if let Some(rect) = visible_content_rows(inner, lines.len(), 2, scroll) {
            app.clickable_regions
                .push(ClickableRegion::AgentProfileRow { rect, index: i });
        }
        let is_active = app
            .agent()
            .profile_id
            .as_deref()
            .map_or(profile.id == "default", |id| id == profile.id);
        let is_selected = i == app.agent_profile_panel_selection;

        let marker = if is_selected && is_active {
            "▶◦"
        } else if is_selected {
            "▶ "
        } else if is_active {
            " ◦"
        } else {
            "  "
        };
        let main_fg = if is_selected {
            theme::WARNING
        } else if is_active {
            theme::SUCCESS
        } else {
            theme::FG
        };
        let name_style = if is_selected {
            theme::selected_row(true)
        } else {
            Style::default().fg(main_fg).add_modifier(if is_active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            })
        };

        let desc = if profile.description.is_empty() {
            crate::i18n::t("agent.profile.no_desc").to_string()
        } else {
            profile.description.clone()
        };
        let desc_trunc = truncate_str_to_width(&desc, inner.width.saturating_sub(4) as usize);

        lines.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(format!(" {}", profile.name), name_style),
        ]));
        lines.push(Line::styled(
            format!("    {}", desc_trunc),
            Style::default().fg(theme::MUTED),
        ));
        if i + 1 < app.agent_registry.len() {
            lines.push(Line::raw(""));
        }
    }

    if app.agent_registry.is_empty() {
        lines.push(Line::styled(
            crate::i18n::t("agent.profile.none"),
            Style::default().fg(theme::MUTED),
        ));
    }

    frame.render_widget(
        Paragraph::new(Text::from(theme::popup_rows(lines, inner.width)))
            .scroll((scroll as u16, 0)),
        inner,
    );
}

/// Right panel: read-only profile summary.
fn render_agent_profile_main(frame: &mut Frame<'_>, app: &App, area: Rect) {
    render_agent_profile_summary(frame, app, area);
}

/// Read-only profile summary when not in the form editor.
fn render_agent_profile_summary(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    let Some(profile) = app
        .agent_registry
        .get_index(app.agent_profile_panel_selection)
    else {
        lines.extend(hatched_empty_lines(
            area.height as usize,
            crate::i18n::t("agent.profile.empty"),
        ));
        frame.render_widget(Paragraph::new(Text::from(lines)), area);
        return;
    };

    let section = Style::default()
        .fg(theme::PRIMARY)
        .add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(theme::MUTED);
    let body = Style::default().fg(theme::FG);

    lines.push(Line::styled(
        crate::i18n::t("agent.profile.section"),
        section,
    ));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(crate::i18n::t("agent.profile.name"), muted),
        Span::styled(profile.name.clone(), body),
    ]));
    let desc = if profile.description.is_empty() {
        crate::i18n::t("agent.profile.empty_value").to_string()
    } else {
        profile.description.clone()
    };
    lines.push(Line::from(vec![
        Span::styled(crate::i18n::t("agent.profile.description"), muted),
        Span::styled(desc, body),
    ]));

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        crate::i18n::t("agent.prompt.section"),
        section,
    ));
    let instr = &profile.system_prompt;
    let preview_lines = area.height.saturating_sub(14) as usize;
    let mut shown = 0usize;
    for line_text in instr.lines() {
        if shown >= preview_lines.max(3) {
            lines.push(Line::styled("  ...", muted));
            break;
        }
        lines.push(Line::styled(format!("  {}", line_text), body));
        shown += 1;
    }
    if shown == 0 {
        lines.push(Line::styled(
            format!("  {}", crate::i18n::t("agent.profile.empty_value")),
            muted,
        ));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled(
        crate::i18n::t("agent.config.section"),
        section,
    ));
    let tools_summary = match &profile.tools {
        teshi_agent::definition::ToolPermission::All => {
            crate::i18n::t("agent.tools.all").to_string()
        }
        teshi_agent::definition::ToolPermission::None => {
            crate::i18n::t("agent.tools.none").to_string()
        }
        teshi_agent::definition::ToolPermission::Whitelist(list) => {
            crate::i18n::t("agent.tools.enabled").replace("{count}", &list.len().to_string())
        }
    };
    lines.push(Line::from(vec![
        Span::styled(crate::i18n::t("agent.tools.label"), muted),
        Span::styled(tools_summary, body),
    ]));
    let skills_summary = crate::i18n::t("agent.skills.hint").to_string();
    lines.push(Line::from(vec![
        Span::styled(crate::i18n::t("agent.skills.label"), muted),
        Span::styled(skills_summary, body),
    ]));
    let model_summary = if profile.model_ref.is_empty() {
        crate::i18n::t("agent.model.active").to_string()
    } else {
        profile.model_ref.clone()
    };
    lines.push(Line::from(vec![
        Span::styled(crate::i18n::t("agent.model.label"), muted),
        Span::styled(model_summary, body),
    ]));

    lines.push(Line::raw(""));
    lines.push(theme::action_line(crate::i18n::t("agent.apply_hint")));

    let visible: Vec<Line<'static>> = lines.into_iter().take(area.height as usize).collect();
    frame.render_widget(Paragraph::new(Text::from(visible)), area);
}

/// Truncate a string to fit within `max_cols` display columns.
fn truncate_str_to_width(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > max_cols {
            break;
        }
        out.push(ch);
        width += w;
    }
    out
}

/// Simple hatched empty placeholder (chrys HatchedEmptyState inspired).
fn hatched_empty_lines(height: usize, label: &str) -> Vec<Line<'static>> {
    let h = height.max(5);
    let mut lines = Vec::with_capacity(h);
    let mid = h / 2;
    for row in 0..h {
        if row == mid {
            let pad = " ".repeat(4);
            lines.push(Line::styled(
                format!("{pad}{label}{pad}"),
                Style::default().fg(theme::MUTED),
            ));
        } else if row > 1 && row < h.saturating_sub(1) {
            let hatch = " ╲ ".repeat(8);
            lines.push(Line::styled(hatch, Style::default().fg(theme::BORDER)));
        } else {
            lines.push(Line::raw(""));
        }
    }
    lines
}

/// Content viewport of a requirement pane.
///
/// The last interior column stays empty because the vertical scrollbar paints
/// on the right border; without the gap the widest rows run into the bar.
fn requirement_viewport(area: Rect) -> Rect {
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    }
}

/// Requirement tree viewport.
///
/// The tree keeps one interior column to itself for the entry rail, so the panel
/// border stays intact and the rail never lands on the frame.
fn requirement_tree_viewport(area: Rect) -> Rect {
    let viewport = requirement_viewport(area);
    Rect {
        x: viewport.x + 1,
        width: viewport.width.saturating_sub(1),
        ..viewport
    }
}

/// Draw overflow indicators on panel borders, preserving the content viewport.
///
/// Every pane shares the chat scrollbar paint: solid cells instead of glyphs, a
/// lifted thumb under the pointer, and a brighter thumb while it is dragged.
/// The drawn rectangles are recorded so the pointer can grab what it sees.
fn render_requirement_scrollbars(
    frame: &mut Frame<'_>,
    ui: &mut crate::authoring_tab::AuthoringUiState,
    pane: usize,
    area: Rect,
    extent: (usize, usize),
    offset: (usize, usize),
    content_height: u16,
) {
    let viewport = requirement_viewport(area);
    let (width, height) = extent;
    // The action strip takes a row from the pane interior. Limits, the thumb,
    // and the drag math must agree with the rows actually painted, or the last
    // line/item can never be reached.
    let content_height = content_height.min(viewport.height);
    ui.scroll_limits[pane] = [
        width.saturating_sub(viewport.width as usize),
        height.saturating_sub(content_height as usize),
    ];
    ui.pane_bars[pane] = [None; 2];

    let grab = ui.scrollbar_grab;
    let grabbed = |axis: usize| {
        grab.is_some_and(|(dragged, dragged_axis, _)| dragged == pane && dragged_axis == axis)
    };
    if width > viewport.width as usize && viewport.width > 0 {
        let rect = Rect::new(
            viewport.x,
            area.bottom().saturating_sub(1),
            viewport.width,
            1,
        );
        let bar = crate::app::ScrollbarBar {
            rect,
            axis: theme::ScrollAxis::Horizontal,
            total: width,
        };
        let state = crate::app::scrollbar_state(ui.pointer, &bar, grabbed(0));
        frame.render_widget(
            Paragraph::new(theme::scrollbar_lines(
                width,
                viewport.width as usize,
                offset.0,
                theme::ScrollAxis::Horizontal,
                state,
            )),
            rect,
        );
        ui.pane_bars[pane][0] = Some(bar);
    }
    if height > content_height as usize && content_height > 0 {
        let rect = Rect::new(
            area.right().saturating_sub(1),
            viewport.y,
            1,
            content_height,
        );
        let bar = crate::app::ScrollbarBar {
            rect,
            axis: theme::ScrollAxis::Vertical,
            total: height,
        };
        let state = crate::app::scrollbar_state(ui.pointer, &bar, grabbed(1));
        frame.render_widget(
            Paragraph::new(theme::scrollbar_lines(
                height,
                content_height as usize,
                offset.1,
                theme::ScrollAxis::Vertical,
                state,
            )),
            rect,
        );
        ui.pane_bars[pane][1] = Some(bar);
    }
}

fn render_requirements_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    app.authoring_ui.pane_areas = [Rect::default(); 3];
    app.authoring_ui.scroll_limits = [[0; 2]; 3];
    app.authoring_ui.pane_bars = [[None; 2]; 3];
    app.authoring_ui.pointer = app.mouse_position;
    if area.width < 20 || area.height < 3 {
        return;
    }
    let layout = compact_panes(
        area,
        match app.authoring_ui.focus {
            crate::authoring_tab::RequirementsFocus::Tree => 0,
            crate::authoring_tab::RequirementsFocus::Editor => 1,
            crate::authoring_tab::RequirementsFocus::LinkedTestPoints => 2,
        },
    );

    {
        for (index, pane) in layout.iter().enumerate() {
            app.authoring_ui.pane_areas[index] = *pane;
        }
        let ui = &mut app.authoring_ui;
        let images = &mut app.inline_images;
        let regions = &mut app.clickable_regions;
        let row_spans = &mut app.req_tree_row_spans;
        let tree_focused = ui.focus == crate::authoring_tab::RequirementsFocus::Tree;
        render_requirements_tree(frame, ui, regions, row_spans, layout[0], tree_focused);

        if layout.len() == 2 {
            render_requirements_editor(frame, ui, regions, Some(images), layout[1], true);
        } else {
            let editor_focused = ui.focus == crate::authoring_tab::RequirementsFocus::Editor;
            let linked_focused =
                ui.focus == crate::authoring_tab::RequirementsFocus::LinkedTestPoints;
            render_requirements_editor(frame, ui, regions, Some(images), layout[1], editor_focused);
            render_requirements_linked(frame, ui, layout[2], linked_focused);
        }
    }
    if app.authoring_ui.current_document_meta().is_some() && layout[1].width > 2 {
        let pane = layout[1];
        let width = 17.min(pane.width.saturating_sub(2));
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: Rect::new(pane.right().saturating_sub(width + 1), pane.y, width, 1),
            action: Action::ReqEditPriority,
            modal: false,
        });
    }
}

/// Requirements tree title: an active `/` search shows the query with a caret,
/// otherwise the idle hint. Both mirror the Explore Features column.
fn requirement_tree_title(ui: &crate::authoring_tab::AuthoringUiState) -> Line<'static> {
    if ui.search_active || !ui.search_query.text().is_empty() {
        let mut spans = search_query_spans(&ui.search_query, ui.search_active);
        spans.push(Span::raw(format!(" · {}", ui.tree_status_title())));
        Line::from(spans)
    } else {
        Line::from(crate::i18n::fill(
            crate::i18n::t("req.tree.title"),
            &[("status", &ui.tree_status_title())],
        ))
    }
}

fn render_requirements_tree(
    frame: &mut Frame<'_>,
    ui: &mut crate::authoring_tab::AuthoringUiState,
    regions: &mut Vec<ClickableRegion>,
    row_spans: &mut Vec<(u16, u16)>,
    area: Rect,
    focused: bool,
) {
    let title = requirement_tree_title(ui);
    let title_width = title.width() as u16;
    let block = theme::panel_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut title_right_limit = area.right().saturating_sub(1);
    let searching = ui.search_active;
    let search_has_text = !ui.search_query.text().is_empty();
    if (searching || search_has_text) && title_right_limit > area.x + 4 {
        let clear_rect = Rect::new(title_right_limit.saturating_sub(3), area.y, 3, 1);
        regions.push(ClickableRegion::ReqSearchClear { rect: clear_rect });
        let (_, clear_style) = theme::action_chip_styles(theme::ActionTone::Warning);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" ✗ ", clear_style))),
            clear_rect,
        );
        title_right_limit = clear_rect.x.saturating_sub(1);
    }
    if searching && title_right_limit > area.x + 4 {
        let confirm_rect = Rect::new(title_right_limit.saturating_sub(3), area.y, 3, 1);
        regions.push(ClickableRegion::ReqSearchConfirm { rect: confirm_rect });
        let (_, confirm_style) = theme::action_chip_styles(theme::ActionTone::Success);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" ✓ ", confirm_style))),
            confirm_rect,
        );
        title_right_limit = confirm_rect.x.saturating_sub(1);
    }
    let trigger_w = title_width.min(title_right_limit.saturating_sub(area.x + 1));
    if trigger_w > 0 {
        let trigger_rect = Rect::new(area.x + 1, area.y, trigger_w, 1);
        regions.push(ClickableRegion::ReqSearchTrigger { rect: trigger_rect });
    }
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (content_inner, action_area) = split_pane_action_row(inner);
    let tree_actions = [
        ActionItem::new(
            crate::i18n::t("req.filter.key"),
            crate::i18n::t("req.filter"),
            Action::ReqFilterOverlay,
            theme::ActionTone::Warning,
        ),
        ActionItem::new(
            crate::i18n::t("req.new.doc.ctrl"),
            crate::i18n::t("req.new"),
            Action::ReqNewDocument,
            theme::ActionTone::Success,
        ),
        ActionItem::new(
            crate::i18n::t("req.actions.space"),
            crate::i18n::t("req.actions"),
            Action::ReqActionMenu,
            theme::ActionTone::Primary,
        ),
    ];

    if let Some(error) = &ui.project_identity_error {
        frame.render_widget(
            Paragraph::new(crate::i18n::fill(
                crate::i18n::t("req.tree.identity_error"),
                &[("error", error)],
            ))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(Style::default().fg(theme::ERROR)),
            content_inner,
        );
        if let Some(action_area) = action_area {
            render_chip_action_bar(frame, regions, action_area, &tree_actions, false);
        }
        return;
    }
    if !ui.discovered {
        frame.render_widget(
            Paragraph::new(crate::i18n::t("req.tree.none"))
                .style(Style::default().fg(theme::MUTED)),
            content_inner,
        );
        if let Some(action_area) = action_area {
            render_chip_action_bar(frame, regions, action_area, &tree_actions, false);
        }
        return;
    }

    if ui.tree_items.is_empty() {
        ui.scroll_limits[0] = [0, 0];
        let message = if ui.search_active || !ui.search_query.text().is_empty() {
            crate::i18n::fill(
                crate::i18n::t("req.tree.no_match"),
                &[("query", ui.search_query.text())],
            )
        } else {
            crate::i18n::t("req.tree.empty").to_string()
        };
        frame.render_widget(
            Paragraph::new(message)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(Style::default().fg(theme::MUTED)),
            content_inner,
        );
        if let Some(action_area) = action_area {
            render_chip_action_bar(frame, regions, action_area, &tree_actions, false);
        }
        return;
    }
    // Shrink the border-derived viewport by the action strip so scroll math
    // matches the painted content rows.
    let mut viewport = requirement_tree_viewport(area);
    if action_area.is_some() {
        viewport.height = viewport.height.saturating_sub(1);
    }
    let wrap = RequirementTreeWrap {
        labels: &ui.tree_labels,
        selected: ui.tree_state.selected(),
        opened: ui.tree_state.opened(),
        width: viewport.width as usize,
        max_rows: viewport.height as usize,
    };
    let wrapped = wrap.items(&ui.tree_items, &mut Vec::new());
    let heights: Vec<usize> = ui
        .tree_state
        .flatten(&wrapped)
        .iter()
        .map(|node| node.item.height())
        .collect();
    // Tree offsets count items, not rows: the furthest offset is the first item
    // from which every remaining row fits, so the scroll extent is that offset
    // plus one viewport.
    let max_start = last_fitting_start(&heights, viewport.height as usize);
    ui.horizontal_scroll[0] = 0;
    let tree = Tree::new(&wrapped)
        .expect("requirement tree")
        .highlight_style(theme::selected_row(focused));
    frame.render_stateful_widget(tree, viewport, &mut ui.tree_state);
    draw_requirement_entry_bars(frame.buffer_mut(), viewport, &wrapped, &ui.tree_state);
    {
        let offset = ui.tree_state.get_offset();
        let bottom = viewport.y + viewport.height;
        let mut y = viewport.y;
        for &height in heights.iter().skip(offset) {
            if y >= bottom {
                break;
            }
            let drawn = (height as u16).min(bottom.saturating_sub(y));
            if drawn > 0 {
                row_spans.push((y, drawn));
            }
            y = y.saturating_add(drawn);
        }
    }
    render_requirement_scrollbars(
        frame,
        ui,
        0,
        area,
        (
            viewport.width as usize,
            max_start + viewport.height as usize,
        ),
        (0, ui.tree_state.get_offset()),
        viewport.height,
    );

    let diag_lines: Vec<Line> = ui
        .visible_diagnostics()
        .iter()
        .take(3)
        .map(|d| {
            Line::styled(
                format!("! {}", d.message),
                Style::default().fg(theme::ERROR),
            )
        })
        .collect();
    if !diag_lines.is_empty() {
        let diag_height = diag_lines.len() as u16;
        let y = content_inner.y + content_inner.height.saturating_sub(diag_height);
        frame.render_widget(
            Paragraph::new(diag_lines),
            Rect {
                x: viewport.x,
                y,
                width: viewport.width,
                height: diag_height,
            },
        );
    }
    if let Some(action_area) = action_area {
        render_chip_action_bar(frame, regions, action_area, &tree_actions, false);
    }
}

/// Draws the entry rail in the tree's reserved interior column.
///
/// The rail occupies `viewport.x - 1`, the column `requirement_tree_viewport`
/// keeps clear on the left, so it never touches the panel border or the entry
/// text. Only entries that wrap to several rows get a rail: a one-row entry is
/// already unambiguous, and marking it would turn the rail into a bullet column.
///
/// Each rail starts with `╷`, whose stroke begins at the row's middle, and ends
/// with `└`, so neighbouring entries are separated by half a row of blank cells
/// instead of fusing into one line (the A problem: multi-line rows of
/// neighbouring entries used to blur into one block). An entry cut off by the
/// bottom of the viewport keeps `│`: it has not ended, it just continues
/// off-screen. The selected entry's rail is accented to reinforce the
/// whole-entry highlight.
fn draw_requirement_entry_bars(
    buf: &mut Buffer,
    viewport: Rect,
    wrapped: &[TreeItem<'_, String>],
    tree_state: &TreeState<String>,
) {
    let offset = tree_state.get_offset();
    let flattened = tree_state.flatten(wrapped);
    let selected = tree_state.selected();
    let bar_style = Style::default().fg(theme::MUTED);
    let selected_bar_style = Style::default().fg(theme::ACCENT);
    let bottom = viewport.y + viewport.height;
    let mut y = viewport.y;
    for node in flattened.iter().skip(offset) {
        let height = node.item.height();
        if y >= bottom {
            break;
        }
        let entry_bottom = (y + height as u16).min(bottom);
        let drawn = entry_bottom.saturating_sub(y);
        let complete = drawn as usize == height;
        if height > 1 {
            let style = if node.identifier.as_slice() == selected {
                selected_bar_style
            } else {
                bar_style
            };
            for row in 0..drawn {
                let glyph = match (row == 0, complete && row + 1 == drawn) {
                    (true, _) => "╷",
                    (_, true) => "└",
                    _ => "│",
                };
                if let Some(cell) = buf.cell_mut((viewport.x.saturating_sub(1), y + row)) {
                    cell.set_symbol(glyph);
                    cell.set_style(style);
                }
            }
        }
        y = entry_bottom;
    }
}

/// Rows an unselected requirement tree label may wrap to before it is truncated.
const REQUIREMENT_TREE_LABEL_ROWS: usize = 3;

/// Rewraps requirement tree labels to the pane width for one frame.
///
/// The selected row shows its whole label (up to one viewport, so the widget can
/// always keep it in view); every other row stops at
/// [`REQUIREMENT_TREE_LABEL_ROWS`].
struct RequirementTreeWrap<'a> {
    labels: &'a std::collections::HashMap<String, Line<'static>>,
    selected: &'a [String],
    opened: &'a std::collections::HashSet<Vec<String>>,
    /// Viewport columns, including the indent and the folder marker.
    width: usize,
    max_rows: usize,
}

impl RequirementTreeWrap<'_> {
    fn items(
        &self,
        items: &[tui_tree_widget::TreeItem<'static, String>],
        path: &mut Vec<String>,
    ) -> Vec<tui_tree_widget::TreeItem<'static, String>> {
        items
            .iter()
            .map(|item| {
                let id = item.identifier();
                path.push(id.clone());
                // The widget draws two columns of indent per level plus a two-column marker.
                let width = self.width.saturating_sub(path.len() * 2).max(1);
                let rows = if path.as_slice() == self.selected {
                    self.max_rows
                } else {
                    REQUIREMENT_TREE_LABEL_ROWS.min(self.max_rows)
                };
                let label = self
                    .labels
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| Line::from(id.clone()));
                // Rows under a closed node are never drawn, so they keep their unwrapped text.
                let children = if self.opened.contains(path) {
                    self.items(item.children(), path)
                } else {
                    item.children().to_vec()
                };
                path.pop();
                tui_tree_widget::TreeItem::new(
                    id.clone(),
                    wrap_tree_label(label, width, rows),
                    children,
                )
                .unwrap_or_else(|_| item.clone())
            })
            .collect()
    }
}

/// Wraps a tree label into at most `max_rows` rows of `width` columns, ending
/// a truncated label with `…`.
fn wrap_tree_label(label: Line<'static>, width: usize, max_rows: usize) -> Text<'static> {
    let mut rows = chat::wrap_line_to_width(label, width);
    if rows.len() > max_rows.max(1) {
        rows.truncate(max_rows.max(1));
        if let Some(last) = rows.pop() {
            let budget = width.saturating_sub(1).min(u16::MAX as usize) as u16;
            let mut kept = truncate_line_to_cols(trim_line_end(last), budget);
            kept.push_span(Span::raw("…"));
            rows.push(kept);
        }
    }
    Text::from(rows)
}

/// Drops trailing whitespace that wrapping leaves at a break point.
fn trim_line_end(mut line: Line<'static>) -> Line<'static> {
    while let Some(span) = line.spans.last_mut() {
        let trimmed = span.content.trim_end();
        if trimmed.is_empty() {
            line.spans.pop();
            continue;
        }
        if trimmed.len() != span.content.len() {
            span.content = trimmed.to_string().into();
        }
        break;
    }
    line
}

/// First item index from which all remaining item heights fit in `rows`.
fn last_fitting_start(heights: &[usize], rows: usize) -> usize {
    let mut used = 0usize;
    for (index, height) in heights.iter().enumerate().rev() {
        used += height;
        if used > rows {
            // A last item taller than the viewport still scrolls to itself.
            return (index + 1).min(heights.len() - 1);
        }
    }
    0
}

fn render_requirements_editor(
    frame: &mut Frame<'_>,
    ui: &mut crate::authoring_tab::AuthoringUiState,
    regions: &mut Vec<ClickableRegion>,
    images: Option<&mut crate::image_view::InlineImages>,
    area: Rect,
    focused: bool,
) {
    let meta = ui.current_document_meta();
    let priority = meta.and_then(|meta| meta.priority);
    let missing = ui.current_document_missing();
    // The document title is the body's H1, which the editor and preview already
    // show; repeat it only when the body cannot be shown.
    let label = if meta.is_some() {
        crate::i18n::t("req.editor.title")
    } else {
        crate::i18n::t("req.source.title")
    };
    let display_title = match meta {
        Some(meta) if missing => format!("{}{}", meta.title, crate::i18n::t("req.missing_suffix")),
        _ if ui.buffer_dirty => format!("{label} *"),
        _ => label.to_string(),
    };
    let (mode, mode_color) = match ui.editor_mode {
        crate::authoring_tab::RequirementsEditorMode::Preview => {
            (crate::i18n::t("mode.preview"), theme::SUCCESS)
        }
        crate::authoring_tab::RequirementsEditorMode::Browse => {
            (crate::i18n::t("mode.browse"), theme::PRIMARY)
        }
        crate::authoring_tab::RequirementsEditorMode::Insert => {
            (crate::i18n::t("mode.insert"), theme::ACCENT)
        }
        crate::authoring_tab::RequirementsEditorMode::VisualChar => {
            (crate::i18n::t("mode.visual"), theme::WARNING)
        }
        crate::authoring_tab::RequirementsEditorMode::VisualLine => {
            (crate::i18n::t("mode.visual_line"), theme::WARNING)
        }
    };
    let mut heading = vec![Span::raw(format!("{display_title} "))];
    if let Some(priority) = priority {
        let priority_color = match priority {
            teshi_core::authoring::RequirementPriority::P0 => theme::ERROR,
            teshi_core::authoring::RequirementPriority::P1 => theme::WARNING,
            teshi_core::authoring::RequirementPriority::P2 => theme::ACCENT,
            teshi_core::authoring::RequirementPriority::P3 => theme::MUTED,
        };
        heading.push(theme::chip(format!("[{priority}]"), priority_color));
        heading.push(Span::raw(" "));
    }
    if let teshi_core::authoring::RequirementProjectFilter::Project(id) = &ui.project_filter
        && let Some(status) = ui
            .current_document_meta()
            .and_then(|meta| meta.projects.get(id))
    {
        let label = match status {
            teshi_core::authoring::RequirementProjectStatus::Planned => {
                crate::i18n::t("req.status.planned")
            }
            teshi_core::authoring::RequirementProjectStatus::Implemented => {
                crate::i18n::t("req.status.implemented")
            }
            teshi_core::authoring::RequirementProjectStatus::Verified => {
                crate::i18n::t("req.status.verified")
            }
        };
        heading.push(theme::chip(label, theme::MUTED));
        heading.push(Span::raw(" "));
    }
    heading.push(theme::chip(mode, mode_color));
    let priority_title = ui.current_document_meta().map(|_| {
        let label = priority.map_or_else(
            || crate::i18n::t("req.priority.unset").to_string(),
            |value| value.to_string(),
        );
        crate::i18n::fill(crate::i18n::t("req.priority.title"), &[("value", &label)])
    });
    let heading_width = area.width.saturating_sub(2).saturating_sub(
        priority_title
            .as_ref()
            .map_or(0, |title| title.len() as u16),
    );
    let mut block = theme::panel_block(
        truncate_line_to_cols(Line::from(heading), heading_width),
        focused,
    );
    if let Some(title) = priority_title {
        block = block.title_top(Line::from(title).right_aligned());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (content_inner, action_area) = split_pane_action_row(inner);
    let mut editor_actions: Vec<ActionItem<'_>> = Vec::new();
    match ui.editor_mode {
        crate::authoring_tab::RequirementsEditorMode::Preview => {
            editor_actions.push(ActionItem::new(
                crate::i18n::t("req.edit.key"),
                crate::i18n::t("req.edit"),
                Action::ReqEnterEditView,
                theme::ActionTone::Info,
            ));
        }
        crate::authoring_tab::RequirementsEditorMode::Browse => {
            editor_actions.push(ActionItem::new(
                crate::i18n::t("req.preview.esc"),
                crate::i18n::t("req.preview"),
                Action::ReqEnterPreview,
                theme::ActionTone::Info,
            ));
        }
        _ => {}
    }
    if !ui.is_previewing() {
        editor_actions.push(ActionItem::new(
            crate::i18n::t("req.point.key"),
            crate::i18n::t("req.point"),
            Action::ReqNewTestPoint,
            theme::ActionTone::Primary,
        ));
    }
    if ui.buffer_dirty {
        editor_actions.push(ActionItem::new(
            crate::i18n::t("req.save.ctrl"),
            crate::i18n::t("footer.save"),
            Action::Save,
            theme::ActionTone::Success,
        ));
    }
    editor_actions.push(ActionItem::new(
        crate::i18n::t("req.new.doc.ctrl"),
        crate::i18n::t("req.new"),
        Action::ReqNewDocument,
        theme::ActionTone::Success,
    ));
    clear_to_surface(frame, content_inner);

    if missing {
        frame.render_widget(
            Paragraph::new(crate::i18n::t("req.missing_file"))
                .style(Style::default().fg(theme::ERROR)),
            content_inner,
        );
        if let Some(action_area) = action_area {
            render_chip_action_bar(frame, regions, action_area, &editor_actions, false);
        }
        return;
    }

    ui.horizontal_scroll[1] = 0;
    let mut viewport = requirement_viewport(area);
    if action_area.is_some() {
        viewport.height = viewport.height.saturating_sub(1);
    }
    if ui.is_previewing() {
        render_requirement_preview(frame, ui, area, viewport, images);
        if let Some(action_area) = action_area {
            render_chip_action_bar(frame, regions, action_area, &editor_actions, false);
        }
        return;
    }
    let resized = ui.editor_wrap_width != viewport.width as usize;
    ui.wrap_editor_rows(viewport.width as usize);
    let cursor = (ui.cursor_row, ui.cursor_col);
    let cursor_display_row = ui
        .editor_display_rows
        .iter()
        .position(|&(row, start, end)| {
            row == ui.cursor_row && start <= ui.cursor_col && ui.cursor_col < end
        })
        .unwrap_or(0);
    let visible_lines = content_inner.height as usize;
    let line_count = ui.editor_display_rows.len();
    if focused && (resized || ui.rendered_cursor != Some(cursor)) {
        if cursor_display_row < ui.scroll_row {
            ui.scroll_row = cursor_display_row;
        } else if cursor_display_row >= ui.scroll_row + visible_lines {
            ui.scroll_row = cursor_display_row.saturating_sub(visible_lines - 1);
        }
    }
    ui.rendered_cursor = Some(cursor);
    ui.scroll_row = ui.scroll_row.min(line_count.saturating_sub(visible_lines));
    let scroll_row = ui.scroll_row;
    let cursor_row = ui.cursor_row;
    let buffer = &ui.buffer;

    let highlight_ranges = ui
        .highlight_test_point_id
        .as_ref()
        .map(|id| ui.highlight_ranges_for_test_point(id))
        .unwrap_or_default();

    let mut lines = Vec::with_capacity(visible_lines);
    for &(row, start, end) in ui
        .editor_display_rows
        .iter()
        .skip(scroll_row)
        .take(visible_lines)
    {
        let line_text = buffer.line(row);
        let mut styled = Line::from(line_text.clone());

        for range in &highlight_ranges {
            styled = apply_char_range_highlight(
                styled,
                range,
                buffer,
                row,
                Style::default().fg(theme::PRIMARY),
            );
        }

        if let (Some(anchor), Some(end)) = (ui.selection_anchor, ui.selection_end) {
            let sel_lo = anchor.0.min(end.0);
            let sel_hi = anchor.0.max(end.0);
            if row >= sel_lo && row <= sel_hi {
                let line_len = line_text.chars().count();
                let (lo_col, hi_col) =
                    if ui.editor_mode == crate::authoring_tab::RequirementsEditorMode::VisualLine {
                        (0, line_len)
                    } else if sel_lo == sel_hi {
                        (anchor.1.min(end.1), anchor.1.max(end.1).min(line_len))
                    } else if row == sel_lo {
                        let col = if anchor.0 < end.0 { anchor.1 } else { end.1 };
                        (col, line_len)
                    } else if row == sel_hi {
                        let col = if anchor.0 > end.0 { anchor.1 } else { end.1 }.min(line_len);
                        (0, col)
                    } else {
                        (0, line_len)
                    };
                if lo_col < hi_col {
                    styled = apply_col_range_highlight(
                        styled,
                        lo_col,
                        hi_col,
                        Style::default()
                            .bg(theme::EDITOR_SELECTION_BG)
                            .fg(theme::FG),
                    );
                }
            }
        }

        if focused && row == cursor_row {
            let line_len = line_text.chars().count();
            let cursor_col = ui.cursor_col.min(line_len);
            let cursor_style = Style::default()
                .fg(theme::FG)
                .bg(theme::EDITOR_SELECTION_BG);
            if line_len == 0 {
                styled = Line::from(vec![Span::styled(" ", cursor_style)]);
            } else if cursor_col < line_len {
                styled =
                    apply_col_range_highlight(styled, cursor_col, cursor_col + 1, cursor_style);
            } else {
                let mut spans = styled.spans;
                spans.push(Span::styled(" ", cursor_style));
                styled = Line::from(spans);
            }
        }

        // Slice styled source characters using the same wrap map as mouse hit testing.
        let mut col = 0;
        let spans = styled
            .spans
            .into_iter()
            .filter_map(|span| {
                let len = span.content.chars().count();
                let text: String = span
                    .content
                    .chars()
                    .skip(start.saturating_sub(col))
                    .take(end.saturating_sub(col.max(start)))
                    .collect();
                col += len;
                (!text.is_empty()).then(|| Span::styled(text, span.style))
            })
            .collect::<Vec<_>>();
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), viewport);
    render_requirement_scrollbars(
        frame,
        ui,
        1,
        area,
        (viewport.width as usize, line_count),
        (0, scroll_row),
        viewport.height,
    );
    if let Some(action_area) = action_area {
        render_chip_action_bar(frame, regions, action_area, &editor_actions, false);
    }
}

/// Draws the read-only Markdown preview, with images below the lines that reference them.
fn render_requirement_preview(
    frame: &mut Frame<'_>,
    ui: &mut crate::authoring_tab::AuthoringUiState,
    area: Rect,
    viewport: Rect,
    images: Option<&mut crate::image_view::InlineImages>,
) {
    let image_rows = if images.as_ref().is_some_and(|images| images.has_pixels()) {
        REQUIREMENT_IMAGE_ROWS
    } else {
        1
    };
    let total = ui
        .refresh_preview(viewport.width as usize, image_rows)
        .lines
        .len();
    let visible = viewport.height as usize;
    ui.preview_scroll = ui.preview_scroll.min(total.saturating_sub(visible));
    let scroll = ui.preview_scroll;
    let preview = ui.preview();
    let lines: Vec<Line<'static>> = preview
        .lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .map(|(index, line)| match preview.images.get(&index) {
            Some(image) => requirement_image_label(image, viewport.width as usize),
            None => line.clone(),
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), viewport);
    if let Some(images) = images {
        place_requirement_images(frame, &preview.images, total, images, viewport, scroll);
    }
    render_requirement_scrollbars(
        frame,
        ui,
        1,
        area,
        (viewport.width as usize, total),
        (0, scroll),
        viewport.height,
    );
}

/// Rows an image takes below its line in a requirement document.
const REQUIREMENT_IMAGE_ROWS: u16 = 12;

/// Text for an image display row: blank under pixels, otherwise a stand-in.
fn requirement_image_label(
    image: &crate::authoring_tab::EditorImageRow,
    width: usize,
) -> Line<'static> {
    use crate::authoring_tab::EditorImageRow;
    let (text, color) = match image {
        EditorImageRow::Pixels { .. } => return Line::default(),
        EditorImageRow::Label { path, alt } => {
            let name = crate::chat_images::display_name(path);
            let text = if alt.is_empty() {
                format!("▣ {name}")
            } else {
                format!("▣ {alt} · {name}")
            };
            (text, theme::MUTED)
        }
        EditorImageRow::Missing { target } => {
            (format!("▣ Image not found: {target}"), theme::WARNING)
        }
    };
    truncate_line_to_cols(
        Line::styled(text, Style::default().fg(color)),
        width.min(u16::MAX as usize) as u16,
    )
}

/// Places the visible rows of each image box in the requirement preview.
///
/// `rows` maps display rows to image rows; `total` is the preview's row count.
fn place_requirement_images(
    frame: &mut Frame<'_>,
    rows: &std::collections::HashMap<usize, crate::authoring_tab::EditorImageRow>,
    total: usize,
    images: &mut crate::image_view::InlineImages,
    inner: Rect,
    scroll_row: usize,
) {
    use crate::authoring_tab::EditorImageRow;
    let end = scroll_row.saturating_add(inner.height as usize).min(total);
    let mut index = scroll_row;
    while index < end {
        let Some(EditorImageRow::Pixels { path, row, size }) = rows.get(&index) else {
            index += 1;
            continue;
        };
        let first = index;
        // A run is the visible, consecutive rows of one box.
        while index < end
            && matches!(
                rows.get(&index),
                Some(EditorImageRow::Pixels { path: next, row: next_row, .. })
                    if next == path && usize::from(*next_row) == usize::from(*row) + (index - first)
            )
        {
            index += 1;
        }
        let area = Rect::new(
            inner.x,
            inner.y + (first - scroll_row) as u16,
            size.0.min(inner.width),
            (index - first) as u16,
        );
        let note = match images.place(frame, path, *size, area, *row) {
            crate::image_view::InlineState::Shown => None,
            crate::image_view::InlineState::Loading => {
                Some(("Loading image…".to_string(), theme::MUTED))
            }
            crate::image_view::InlineState::Failed(reason) => {
                Some((format!("▣ Cannot show image: {reason}"), theme::WARNING))
            }
            crate::image_view::InlineState::Placeholder => Some((
                format!("▣ {}", crate::chat_images::display_name(path)),
                theme::MUTED,
            )),
        };
        if let Some((text, color)) = note {
            frame.render_widget(
                Paragraph::new(truncate_line_to_cols(
                    Line::styled(text, Style::default().fg(color)),
                    area.width,
                )),
                Rect::new(area.x, area.y, area.width, 1),
            );
        }
    }
}

fn apply_col_range_highlight(
    line: Line<'static>,
    lo_col: usize,
    hi_col: usize,
    patch: Style,
) -> Line<'static> {
    apply_patch_to_char_range(line, lo_col..hi_col, patch)
}

fn apply_char_range_highlight(
    line: Line<'static>,
    range: &teshi_core::authoring::TextRange,
    buffer: &crate::editor_buffer::EditorBuffer,
    row: usize,
    patch: Style,
) -> Line<'static> {
    let body = buffer.as_string();
    let start_lc = teshi_core::authoring::char_position_to_line_col(body.as_str(), range.start);
    let end_lc = teshi_core::authoring::char_position_to_line_col(body.as_str(), range.end);
    if let (Some((start_line, start_col)), Some((end_line, end_col))) = (start_lc, end_lc) {
        if row < start_line || row > end_line {
            return line;
        }
        let lo = if row == start_line { start_col } else { 0 };
        let hi = if row == end_line {
            end_col
        } else {
            buffer.line(row).chars().count()
        };
        if lo < hi {
            return apply_col_range_highlight(line, lo, hi, patch);
        }
    }
    line
}

fn render_requirements_linked(
    frame: &mut Frame<'_>,
    ui: &mut crate::authoring_tab::AuthoringUiState,
    area: Rect,
    focused: bool,
) {
    let block = theme::panel_block(crate::i18n::t("req.linked.title"), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let linked = ui.filtered_linked_test_points();
    let mut lines: Vec<Line> = Vec::new();
    if linked.is_empty() {
        lines.push(Line::styled(
            " (no linked test points)",
            Style::default().fg(theme::MUTED),
        ));
    } else {
        for (i, tp) in linked.iter().enumerate() {
            let badge = crate::authoring_tab::review_state_label(tp.review_state);
            let badge_style = crate::authoring_tab::review_state_style(tp.review_state);
            let label = tp.title.clone();
            let style = if i == ui.selected_linked_index {
                theme::selected_row(focused)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if i == ui.selected_linked_index {
                        "› "
                    } else {
                        "  "
                    },
                    style,
                ),
                theme::chip(
                    format!("[{badge}] "),
                    badge_style.fg.unwrap_or(theme::MUTED),
                ),
                Span::styled(label, style),
            ]));
        }
    }

    let height = lines.len();
    let width = lines.iter().map(Line::width).max().unwrap_or(0);
    ui.linked_scroll = ui
        .linked_scroll
        .min(height.saturating_sub(inner.height as usize));
    if ui.rendered_linked_selection != Some(ui.selected_linked_index) {
        if ui.selected_linked_index < ui.linked_scroll {
            ui.linked_scroll = ui.selected_linked_index;
        } else if ui.selected_linked_index >= ui.linked_scroll + inner.height as usize {
            ui.linked_scroll = ui.selected_linked_index + 1 - inner.height as usize;
        }
    }
    ui.rendered_linked_selection = Some(ui.selected_linked_index);
    let viewport = requirement_viewport(area);
    ui.horizontal_scroll[2] =
        ui.horizontal_scroll[2].min(width.saturating_sub(viewport.width as usize));
    frame.render_widget(
        Paragraph::new(lines).scroll((
            ui.linked_scroll.min(u16::MAX as usize) as u16,
            ui.horizontal_scroll[2].min(u16::MAX as usize) as u16,
        )),
        viewport,
    );
    render_requirement_scrollbars(
        frame,
        ui,
        2,
        area,
        (width, height),
        (ui.horizontal_scroll[2], ui.linked_scroll),
        viewport.height,
    );
}

/// A dialog field editing `input`; only the focused field draws the caret.
fn form_field<'a>(
    label: &'a str,
    input: &'a crate::authoring_tab::LineInput,
    focused: bool,
) -> FormField<'a> {
    let (before, after) = input.split_at_cursor();
    FormField {
        label,
        before,
        after,
        focused,
    }
}

fn render_requirements_overlay(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use crate::project_context::SwitchOverlay;
    if !matches!(app.project_switch.overlay, SwitchOverlay::None) {
        render_project_switch_overlay(frame, app, area);
        return;
    }
    use crate::authoring_tab::RequirementsOverlay;
    match &app.authoring_ui.overlay {
        RequirementsOverlay::None => {}
        RequirementsOverlay::Unsaved { .. } => {
            let title = app
                .authoring_ui
                .current_document_meta()
                .map_or(crate::i18n::t("req.editor.title"), |doc| doc.title.as_str());
            let body = vec![dialog_question(crate::i18n::fill(
                crate::i18n::t("req.overlay.unsaved.body"),
                &[("title", title)],
            ))];
            // Enter has no meaning here, so no button is shown as focused.
            render_confirm_dialog(
                frame,
                app,
                area,
                Dialog {
                    title: crate::i18n::t("req.overlay.unsaved.title"),
                    warning: false,
                    body,
                    buttons: vec![
                        DialogButton {
                            label: crate::i18n::t("req.overlay.save"),
                            tone: ButtonTone::Primary,
                            focused: false,
                        },
                        DialogButton {
                            label: crate::i18n::t("req.overlay.discard"),
                            tone: ButtonTone::Danger,
                            focused: false,
                        },
                        DialogButton {
                            label: crate::i18n::t("req.overlay.cancel"),
                            tone: ButtonTone::Warning,
                            focused: false,
                        },
                    ],
                },
                &[
                    Action::OverlayInsert('s'),
                    Action::OverlayInsert('d'),
                    Action::OverlayCancel,
                ],
            );
        }
        RequirementsOverlay::FilterPicker { selection } => {
            let selection = *selection;
            let rows = app
                .authoring_ui
                .filter_picker_options()
                .iter()
                .enumerate()
                .map(|(index, filter)| {
                    let label = match filter {
                        teshi_core::authoring::RequirementIterationFilter::All => {
                            crate::i18n::t("req.iteration.all").to_string()
                        }
                        teshi_core::authoring::RequirementIterationFilter::Unassigned => {
                            crate::i18n::t("req.iteration.unassigned").to_string()
                        }
                        teshi_core::authoring::RequirementIterationFilter::Named(name) => {
                            name.clone()
                        }
                    };
                    picker_row(format!(" {label}"), index == selection)
                })
                .collect();
            render_picker_dialog(
                frame,
                app,
                area,
                ListDialog {
                    title: crate::i18n::t("req.overlay.filter.title"),
                    rows,
                    selected: selection,
                    notes: Vec::new(),
                    buttons: vec![cancel_button(crate::i18n::t("dialog.cancel.esc"))],
                },
                &[Action::OverlayCancel],
            );
        }
        RequirementsOverlay::ActionMenu { target, selection } => {
            let items = app.authoring_ui.menu_items(target);
            let title = app.authoring_ui.menu_title(target);
            let selection = *selection;
            let label_width = items
                .iter()
                .map(|item| UnicodeWidthStr::width(item.label))
                .max()
                .unwrap_or(0);
            let rows = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let style = picker_row_style(index == selection);
                    let pad = label_width - UnicodeWidthStr::width(item.label);
                    Line::from(vec![
                        Span::styled(format!(" {}{}   ", item.label, " ".repeat(pad)), style),
                        Span::styled(item.hint, style.patch(theme::muted())),
                    ])
                })
                .collect();
            render_picker_dialog(
                frame,
                app,
                area,
                ListDialog {
                    title: &title,
                    rows,
                    selected: selection,
                    notes: Vec::new(),
                    buttons: vec![
                        confirm_button(crate::i18n::t("req.overlay.select")),
                        cancel_button(crate::i18n::t("dialog.close.esc")),
                    ],
                },
                &[Action::OverlayConfirm, Action::OverlayCancel],
            );
        }
        RequirementsOverlay::FolderPicker {
            document_id,
            options,
            checked,
            selection,
        } => {
            let title = app
                .authoring_ui
                .artifacts
                .as_ref()
                .and_then(|a| a.index.documents.iter().find(|d| &d.id == document_id))
                .map_or(document_id.clone(), |d| d.title.clone());
            let rows = options
                .iter()
                .enumerate()
                .map(|(index, (id, path))| {
                    let mark = if checked.contains(id) { "[x]" } else { "[ ]" };
                    picker_row(format!(" {mark} {path}"), index == *selection)
                })
                .collect();
            let mut notes = Vec::new();
            if options.is_empty() {
                notes.push(Line::styled(
                    crate::i18n::t("req.overlay.folders.none"),
                    theme::muted(),
                ));
            }
            notes.push(Line::styled(
                if checked.is_empty() {
                    crate::i18n::t("req.overlay.folders.none_checked")
                } else {
                    crate::i18n::t("req.overlay.folders.checked")
                },
                theme::muted(),
            ));
            let title = crate::i18n::fill(
                crate::i18n::t("req.overlay.folders.title"),
                &[("title", &title)],
            );
            render_picker_dialog(
                frame,
                app,
                area,
                ListDialog {
                    title: &title,
                    rows,
                    selected: *selection,
                    notes,
                    buttons: vec![
                        neutral_button(crate::i18n::t("req.overlay.toggle")),
                        confirm_button(crate::i18n::t("req.overlay.save_enter")),
                        cancel_button(crate::i18n::t("dialog.cancel.esc")),
                    ],
                },
                &[
                    Action::OverlayInsert(' '),
                    Action::OverlayConfirm,
                    Action::OverlayCancel,
                ],
            );
        }
        RequirementsOverlay::TagPicker {
            document_id,
            options,
            checked,
            styles,
            selection,
            new_tag,
            ..
        } => {
            let title = app
                .authoring_ui
                .artifacts
                .as_ref()
                .and_then(|a| a.index.documents.iter().find(|d| &d.id == document_id))
                .map_or(document_id.clone(), |d| d.title.clone());
            let mut rows: Vec<Line> = options
                .iter()
                .enumerate()
                .map(|(index, tag)| {
                    let style = picker_row_style(index == *selection);
                    let mark = if checked.contains(tag) { "[x]" } else { "[ ]" };
                    let tag_style = styles.get(tag).copied().unwrap_or_default();
                    // Preview the tree rendering: a chip before titles, colored text after.
                    let label = format!("#{tag}");
                    let tag_span = match (tag_style.prefix, tag_style.color) {
                        (true, color) => {
                            theme::chip(label, color.map_or(theme::ACCENT, theme::tag_color))
                        }
                        (false, Some(color)) => {
                            Span::styled(label, Style::default().fg(theme::tag_color(color)))
                        }
                        (false, None) => Span::raw(label),
                    };
                    let mut spans = vec![Span::raw(format!(" {mark} ")), tag_span];
                    if !tag_style.is_default() {
                        let color = tag_style
                            .color
                            .map_or(crate::i18n::t("req.overlay.tags.no_color"), |color| {
                                color.as_str()
                            });
                        let position = if tag_style.prefix {
                            crate::i18n::t("req.overlay.tags.before")
                        } else {
                            crate::i18n::t("req.overlay.tags.after")
                        };
                        spans.push(Span::styled(
                            format!("  {color} · {position}"),
                            theme::muted(),
                        ));
                    }
                    Line::from(spans).style(style)
                })
                .collect();
            // The trailing row adds a tag; its index is `options.len()`.
            rows.push(picker_row(
                match new_tag {
                    Some(buffer) => crate::i18n::fill(
                        crate::i18n::t("req.overlay.tags.new_buffer"),
                        &[("buffer", buffer)],
                    ),
                    None => crate::i18n::t("req.overlay.tags.new").to_string(),
                },
                options.len() == *selection,
            ));
            let mut notes = Vec::new();
            if options.is_empty() {
                notes.push(Line::styled(
                    crate::i18n::t("req.overlay.tags.none"),
                    theme::muted(),
                ));
            }
            notes.push(Line::styled(
                crate::i18n::t("req.overlay.tags.note"),
                theme::muted(),
            ));
            let (buttons, actions) = if new_tag.is_some() {
                (
                    vec![
                        confirm_button(crate::i18n::t("req.overlay.add")),
                        cancel_button(crate::i18n::t("dialog.cancel.esc")),
                    ],
                    vec![Action::OverlayConfirm, Action::OverlayCancel],
                )
            } else {
                (
                    vec![
                        neutral_button(crate::i18n::t("req.overlay.toggle")),
                        neutral_button(crate::i18n::t("req.overlay.color")),
                        neutral_button(crate::i18n::t("req.overlay.front")),
                        confirm_button(crate::i18n::t("req.overlay.save_enter")),
                        cancel_button(crate::i18n::t("dialog.cancel.esc")),
                    ],
                    vec![
                        Action::OverlayInsert(' '),
                        Action::OverlayInsert('c'),
                        Action::OverlayInsert('f'),
                        Action::OverlayConfirm,
                        Action::OverlayCancel,
                    ],
                )
            };
            let title = crate::i18n::fill(
                crate::i18n::t("req.overlay.tags.title"),
                &[("title", &title)],
            );
            render_picker_dialog(
                frame,
                app,
                area,
                ListDialog {
                    title: &title,
                    rows,
                    selected: *selection,
                    notes,
                    buttons,
                },
                &actions,
            );
        }
        RequirementsOverlay::OrganizationForm(form) => {
            use crate::authoring_tab::OrganizationForm;
            let (title, fields, notes, query) = match form {
                OrganizationForm::NewFolder {
                    parent_path, name, ..
                } => (
                    parent_path.as_ref().map_or(
                        crate::i18n::t("req.overlay.folder.new").to_string(),
                        |parent| {
                            crate::i18n::fill(
                                crate::i18n::t("req.overlay.folder.new_child"),
                                &[("parent", parent)],
                            )
                        },
                    ),
                    vec![form_field(
                        crate::i18n::t("req.overlay.field.name"),
                        name,
                        true,
                    )],
                    vec![Line::styled(
                        crate::i18n::t("req.overlay.folder.hint"),
                        theme::muted(),
                    )],
                    false,
                ),
                OrganizationForm::Rename { query, name, .. } => (
                    if *query {
                        crate::i18n::t("req.overlay.rename.query").to_string()
                    } else {
                        crate::i18n::t("req.overlay.rename.folder").to_string()
                    },
                    vec![form_field(
                        crate::i18n::t("req.overlay.field.name"),
                        name,
                        true,
                    )],
                    vec![Line::styled(
                        crate::i18n::t("req.overlay.rename.hint"),
                        theme::muted(),
                    )],
                    false,
                ),
                OrganizationForm::Query {
                    query_id,
                    name,
                    condition,
                    condition_focused,
                } => {
                    let mut notes = match app.authoring_ui.query_form_preview() {
                        Some(Ok(count)) => vec![Line::styled(
                            if count == 1 {
                                crate::i18n::t("req.overlay.query.matches_one").to_string()
                            } else {
                                crate::i18n::t("req.overlay.query.matches")
                                    .replace("{count}", &count.to_string())
                            },
                            Style::default().fg(theme::SUCCESS),
                        )],
                        Some(Err(error)) => vec![Line::styled(
                            crate::i18n::fill(
                                crate::i18n::t("req.overlay.query.error"),
                                &[("error", &error.to_string())],
                            ),
                            Style::default().fg(theme::ERROR),
                        )],
                        None => Vec::new(),
                    };
                    notes.extend(
                        [
                            crate::i18n::t("req.overlay.query.help1"),
                            crate::i18n::t("req.overlay.query.help2"),
                            crate::i18n::t("req.overlay.query.help3"),
                        ]
                        .map(|help| Line::styled(help, theme::muted())),
                    );
                    (
                        if query_id.is_some() {
                            crate::i18n::t("req.overlay.query.edit").to_string()
                        } else {
                            crate::i18n::t("req.overlay.query.new").to_string()
                        },
                        vec![
                            form_field(
                                crate::i18n::t("req.overlay.field.name"),
                                name,
                                !condition_focused,
                            ),
                            form_field(
                                crate::i18n::t("req.overlay.field.condition"),
                                condition,
                                *condition_focused,
                            ),
                        ],
                        notes,
                        true,
                    )
                }
            };
            let mut buttons = vec![confirm_button(crate::i18n::t("req.overlay.save_enter"))];
            let mut actions = vec![Action::OverlayConfirm];
            if query {
                buttons.push(neutral_button(crate::i18n::t("req.overlay.next_field")));
                actions.push(Action::OverlayMoveDown);
            }
            buttons.push(cancel_button(crate::i18n::t("dialog.cancel.esc")));
            actions.push(Action::OverlayCancel);
            let rects = FormDialog {
                title: &title,
                fields,
                notes,
                buttons,
            }
            .render(frame, area);
            register_dialog_buttons(app, rects, &actions);
        }
        RequirementsOverlay::RemoveCollection {
            name,
            query,
            inbox_returns,
            ..
        } => {
            let (title, body) = if *query {
                (
                    crate::i18n::t("req.overlay.query.delete.title"),
                    vec![
                        dialog_question(crate::i18n::fill(
                            crate::i18n::t("req.overlay.query.delete.body"),
                            &[("name", name)],
                        )),
                        Line::styled(
                            crate::i18n::t("req.overlay.query.delete.note"),
                            theme::muted(),
                        ),
                    ],
                )
            } else {
                (
                    crate::i18n::t("req.overlay.folder.delete.title"),
                    vec![
                        dialog_question(crate::i18n::fill(
                            crate::i18n::t("req.overlay.folder.delete.body"),
                            &[("name", name)],
                        )),
                        Line::styled(
                            match inbox_returns {
                                0 => crate::i18n::t("req.overlay.folder.delete.none").to_string(),
                                1 => crate::i18n::t("req.overlay.folder.delete.one").to_string(),
                                n => crate::i18n::t("req.overlay.folder.delete.many")
                                    .replace("{n}", &n.to_string()),
                            },
                            theme::muted(),
                        ),
                        Line::styled(
                            crate::i18n::t("req.overlay.folder.delete.note"),
                            theme::muted(),
                        ),
                    ],
                )
            };
            render_confirm_dialog(
                frame,
                app,
                area,
                delete_dialog(title, body, crate::i18n::t("req.overlay.delete")),
                &[Action::OverlayConfirm, Action::OverlayCancel],
            );
        }
        RequirementsOverlay::IterationEdit { buffer } => {
            // The iteration buffer only grows or shrinks at its end.
            let rects = FormDialog {
                title: crate::i18n::t("req.overlay.iteration.title"),
                fields: vec![FormField {
                    label: crate::i18n::t("req.overlay.iteration.field"),
                    before: buffer,
                    after: "",
                    focused: true,
                }],
                notes: vec![Line::styled(
                    crate::i18n::t("req.overlay.iteration.hint"),
                    theme::muted(),
                )],
                buttons: vec![
                    confirm_button(crate::i18n::t("req.overlay.confirm")),
                    cancel_button(crate::i18n::t("dialog.cancel.esc")),
                ],
            }
            .render(frame, area);
            register_dialog_buttons(app, rects, &[Action::OverlayConfirm, Action::OverlayCancel]);
        }
        RequirementsOverlay::PriorityEdit { selection, .. } => {
            let rows = [crate::i18n::t("req.priority.unset"), "P0", "P1", "P2", "P3"]
                .iter()
                .enumerate()
                .map(|(index, label)| picker_row(format!(" {label}"), index == *selection))
                .collect();
            render_picker_dialog(
                frame,
                app,
                area,
                ListDialog {
                    title: crate::i18n::t("req.overlay.priority.title"),
                    rows,
                    selected: *selection,
                    notes: Vec::new(),
                    buttons: vec![cancel_button(crate::i18n::t("dialog.cancel.esc"))],
                },
                &[Action::OverlayCancel],
            );
        }
        RequirementsOverlay::DeleteConfirmation { document_id, title } => {
            let body = vec![
                dialog_question(crate::i18n::fill(
                    crate::i18n::t("req.overlay.requirement.delete.body"),
                    &[("title", title), ("id", document_id)],
                )),
                Line::styled(
                    crate::i18n::t("req.overlay.requirement.delete.note1"),
                    theme::muted(),
                ),
                Line::styled(
                    crate::i18n::t("req.overlay.requirement.delete.note2"),
                    theme::muted(),
                ),
            ];
            render_confirm_dialog(
                frame,
                app,
                area,
                delete_dialog(
                    crate::i18n::t("req.overlay.requirement.delete.title"),
                    body,
                    crate::i18n::t("req.overlay.confirm"),
                ),
                &[Action::OverlayConfirm, Action::OverlayCancel],
            );
        }
    }

    // The iteration picker owns input while open; keep its parent beneath it.
    if let Some(prompt) = &app.generation_scope_prompt
        && !app.authoring_ui.overlay_active()
    {
        let filter = match &prompt.iteration {
            teshi_core::authoring::RequirementIterationFilter::All => {
                crate::i18n::t("req.iteration.all").to_string()
            }
            teshi_core::authoring::RequirementIterationFilter::Unassigned => {
                crate::i18n::t("req.iteration.unassigned").to_string()
            }
            teshi_core::authoring::RequirementIterationFilter::Named(name) => name.clone(),
        };
        let store_id = app
            .authoring_ui
            .artifacts
            .as_ref()
            .and_then(|a| a.index.store_id.as_ref())
            .map(|id| id.to_string())
            .unwrap_or_else(|| crate::i18n::t("req.overlay.scope.uninitialized").into());
        let lines = vec![
            Line::raw(crate::i18n::fill(
                crate::i18n::t("req.overlay.scope.store"),
                &[("path", &app.requirements_root.display().to_string())],
            )),
            Line::raw(crate::i18n::fill(
                crate::i18n::t("req.overlay.scope.store_id"),
                &[("id", &store_id)],
            )),
            Line::raw(crate::i18n::fill(
                crate::i18n::t("req.overlay.scope.iteration"),
                &[("filter", &filter)],
            )),
            Line::raw(crate::i18n::fill(
                crate::i18n::t("req.overlay.scope.project"),
                &[("scope", app.authoring_ui.project_scope_label())],
            )),
            Line::raw(""),
        ];
        // The five info rows plus the action row inside the border.
        let rows = lines.len() as u16 + 3;
        let panel_area = overlay::OverlaySpec::prompt(72, 8, rows).rect(area, (0, rows));
        frame.render_widget(Clear, panel_area);
        let block = theme::popup_block(crate::i18n::t("req.overlay.scope.title"));
        let [content, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
            .areas(block.inner(panel_area));
        frame.render_widget(block, panel_area);
        frame.render_widget(Paragraph::new(lines), content);
        render_action_bar(
            frame,
            &mut app.clickable_regions,
            footer,
            "",
            &[
                (
                    crate::i18n::t("req.overlay.scope.confirm"),
                    Action::GenerationScopeConfirm,
                ),
                (
                    crate::i18n::t("req.overlay.scope.iteration_key"),
                    Action::GenerationScopeChangeIteration,
                ),
                (
                    crate::i18n::t("dialog.cancel.esc"),
                    Action::GenerationScopeCancel,
                ),
            ],
            true,
        );
    }
}

fn render_project_picker(frame: &mut Frame<'_>, app: &mut App, area: Rect, selection: usize) {
    let width = overlay::Axis::cells(100, 20, overlay::DEFAULT_SHARE).resolve(area.width, 0);
    let inner_width = width.saturating_sub(2) as usize;
    if inner_width == 0 || area.height < 4 {
        return;
    }
    let footer = chat::wrap_line_to_width(
        theme::action_line(crate::i18n::t("project.picker.hint")),
        inner_width,
    );
    let mut rows = Vec::new();
    let mut selected_start = 0;
    let mut selected_end = 0;
    for (index, entry) in app.project_switch.recent.iter().enumerate() {
        let label = Line::styled(
            format!(
                "{}{}  {}",
                if index == selection { "> " } else { "  " },
                entry.display_name,
                entry.root
            ),
            if index == selection {
                theme::selected_row(true)
            } else {
                Style::default()
            },
        );
        if index == selection {
            selected_start = rows.len();
        }
        rows.extend(
            chat::wrap_line_to_width(label, inner_width)
                .into_iter()
                .map(|line| (Some(index), line)),
        );
        if index == selection {
            selected_end = rows.len();
        }
    }
    if rows.is_empty() {
        rows.push((None, Line::raw(crate::i18n::t("project.recent.empty"))));
    }
    // Reserve the controls before allocating path rows. Wrapping and hit tests
    // share these physical rows, including continuation lines of long paths.
    let reserved = footer.len() + 3;
    let list_height = rows
        .len()
        .min((area.height as usize).saturating_sub(reserved));
    let start = selected_end
        .saturating_sub(list_height)
        .min(selected_start)
        .min(rows.len().saturating_sub(list_height));
    let height = (list_height + reserved).min(area.height as usize) as u16;
    let panel = overlay::centered(area, width, height);
    app.modal_card_rect = Some(panel);
    let mut lines = Vec::new();
    for (offset, (index, line)) in rows.iter().skip(start).take(list_height).enumerate() {
        lines.push(line.clone());
        if let Some(index) = index {
            app.clickable_regions
                .push(crate::app::ClickableRegion::ProjectRecent {
                    index: *index,
                    rect: Rect::new(
                        panel.x + 1,
                        panel.y + 1 + offset as u16,
                        inner_width as u16,
                        1,
                    ),
                });
        }
    }
    lines.push(Line::raw(""));
    let open_y = panel.y + 2 + list_height as u16;
    lines.extend(footer);
    frame.render_widget(Clear, panel);
    frame.render_widget(
        Paragraph::new(lines).block(theme::popup_block(crate::i18n::t("project.open"))),
        panel,
    );
    if open_y < panel.bottom().saturating_sub(1) {
        app.clickable_regions
            .push(crate::app::ClickableRegion::ProjectOpenPath {
                rect: Rect::new(panel.x + 1, open_y, inner_width as u16, 1),
            });
    }
}

fn render_project_switch_overlay(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    use crate::project_context::SwitchOverlay;
    let overlay = app.project_switch.overlay.clone();
    if matches!(overlay, SwitchOverlay::None) {
        return;
    }
    dialog::dim_backdrop(frame, area);
    let (title, lines, max_width) = match overlay {
        SwitchOverlay::None => return,
        SwitchOverlay::Picker { selection } => {
            render_project_picker(frame, app, area, selection);
            return;
        }
        SwitchOverlay::OpenPath { input } => (
            crate::i18n::t("project.open_path"),
            vec![
                Line::raw(crate::i18n::t("project.open_path.prompt")),
                Line::styled(format!("> {input}"), Style::default().fg(theme::FG)),
                Line::raw(""),
                theme::action_line(crate::i18n::t("project.open_path.hint")),
            ],
            100,
        ),
        SwitchOverlay::Dirty { target } => {
            let mut rows = vec![
                Line::raw(crate::i18n::fill(
                    crate::i18n::t("project.switch.to"),
                    &[("path", &target.root().display().to_string())],
                )),
                Line::raw(crate::i18n::t("project.dirty.body")),
            ];
            if let Some(request) = &app.project_switch.pending {
                rows.extend(request.dirty.iter().map(|item| {
                    Line::styled(
                        format!("  • {}", item.label()),
                        Style::default().fg(theme::WARNING),
                    )
                }));
            }
            rows.push(Line::raw(""));
            rows.push(theme::action_line(crate::i18n::t("project.dirty.hint")));
            (crate::i18n::t("project.dirty.title"), rows, 86)
        }
        SwitchOverlay::Busy { target } => (
            crate::i18n::t("project.busy.title"),
            vec![
                Line::raw(crate::i18n::fill(
                    crate::i18n::t("project.switch.to"),
                    &[("path", &target.root().display().to_string())],
                )),
                Line::raw(crate::i18n::t("project.busy.body")),
                Line::raw(crate::i18n::t("project.busy.hint")),
                Line::raw(crate::i18n::t("project.busy.wait")),
            ],
            86,
        ),
    };
    // Keep a backdrop margin on narrow terminals and size the card from the
    // wrapped lines instead of a fixed guess.
    let width_axis = overlay::Axis::cells(max_width, 20, overlay::DEFAULT_SHARE);
    let width = width_axis.resolve(area.width, 0);
    let body_rows: u32 = lines
        .iter()
        .map(|line| {
            chat::wrap_line_to_width(line.clone(), width.saturating_sub(2) as usize).len() as u32
        })
        .sum();
    let spec = overlay::OverlaySpec {
        width: width_axis,
        height: overlay::Axis::content(4, 20, overlay::DEFAULT_SHARE),
    };
    let panel = spec.rect(area, (0, body_rows.min(20) as u16 + 2));
    frame.render_widget(Clear, panel);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(theme::popup_block(title)),
        panel,
    );
}

fn render_test_points_panel(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 20 || area.height < 3 {
        return;
    }
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::SCREEN_BG)),
        area,
    );
    app.test_points_canvas_rect = None;

    let root_name = app
        .project
        .root_dir
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Test Points")
        .to_string();
    app.refresh_test_points_exec_statuses();
    {
        let artifacts = app.authoring_ui.artifacts.as_ref();
        app.test_points_ui
            .rebuild_canvas_layout(artifacts, &root_name);
    }

    if app.test_points_ui.fullscreen_canvas {
        app.clickable_regions.push(ClickableRegion::TestPointsPane {
            pane: 0,
            rect: area,
        });
        let focused = app.test_points_ui.focus == crate::test_points_tab::TestPointsFocus::Tree;
        render_test_points_canvas(frame, app, area, focused);
        return;
    }

    if area.width < 70 {
        if app.test_points_ui.focus == crate::test_points_tab::TestPointsFocus::Tree {
            app.clickable_regions.push(ClickableRegion::TestPointsPane {
                pane: 0,
                rect: area,
            });
            render_test_points_canvas(frame, app, area, true);
        } else {
            app.clickable_regions.push(ClickableRegion::TestPointsPane {
                pane: 1,
                rect: area,
            });
            render_test_points_inspector(frame, app, area, true);
        }
        return;
    }

    let [canvas_rect, inspector_rect] =
        Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)]).areas(area);
    app.clickable_regions.push(ClickableRegion::TestPointsPane {
        pane: 0,
        rect: canvas_rect,
    });
    app.clickable_regions.push(ClickableRegion::TestPointsPane {
        pane: 1,
        rect: inspector_rect,
    });

    let canvas_focused = app.test_points_ui.focus == crate::test_points_tab::TestPointsFocus::Tree;
    render_test_points_canvas(frame, app, canvas_rect, canvas_focused);
    render_test_points_inspector(frame, app, inspector_rect, !canvas_focused);
}

/// Split a pane interior into a content region and an optional bottom action row.
///
/// A one-row interior becomes the action strip alone so Run/Approve-style chips
/// stay reachable on short panes; taller interiors keep at least one content row.
pub(super) fn split_pane_action_row(inner: Rect) -> (Rect, Option<Rect>) {
    if inner.width == 0 || inner.height == 0 {
        return (inner, None);
    }
    if inner.height == 1 {
        return (Rect { height: 0, ..inner }, Some(inner));
    }
    let content = Rect {
        height: inner.height - 1,
        ..inner
    };
    let actions = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
    (content, Some(actions))
}

/// A clickable pill button rendered inside a pane action strip.
pub(super) struct ActionItem<'a> {
    pub label: &'a str,
    pub compact: &'a str,
    pub action: Action,
    pub tone: theme::ActionTone,
}

impl<'a> ActionItem<'a> {
    pub(super) const fn new(
        label: &'a str,
        compact: &'a str,
        action: Action,
        tone: theme::ActionTone,
    ) -> Self {
        Self {
            label,
            compact,
            action,
            tone,
        }
    }
}

/// Renders one styled pill label with bracketed shortcut hints highlighted in
/// the button's semantic accent color.
fn action_chip_line(text: &str, tone: theme::ActionTone) -> Line<'static> {
    let (label_style, key_style) = theme::action_chip_styles(tone);
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']').map(|end| start + end + 1) else {
            break;
        };
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_owned(), label_style));
        }
        spans.push(Span::styled(rest[start..end].to_owned(), key_style));
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_owned(), label_style));
    }
    Line::from(spans).style(label_style)
}

/// Renders a row of distinct, color-tinted clickable pill buttons inside a pane
/// action strip, automatically falling back to compact labels when narrow.
pub(super) fn render_chip_action_bar(
    frame: &mut Frame<'_>,
    regions: &mut Vec<ClickableRegion>,
    area: Rect,
    controls: &[ActionItem<'_>],
    modal: bool,
) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(theme::FOOTER_BG)),
        area,
    );
    if controls.is_empty() {
        return;
    }
    let full_total: u16 = controls
        .iter()
        .enumerate()
        .map(|(i, item)| (item.label.trim().width() as u16).saturating_add(2) + u16::from(i > 0))
        .sum();
    let use_compact = full_total > area.width;
    let mut x = area.x;
    for item in controls {
        let raw = if use_compact {
            item.compact.trim()
        } else {
            item.label.trim()
        };
        let padded = format!(" {raw} ");
        let width = padded.width() as u16;
        if x.saturating_add(width) > area.right() {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        frame.render_widget(Paragraph::new(action_chip_line(&padded, item.tone)), rect);
        regions.push(ClickableRegion::ActionButton {
            rect,
            action: item.action.clone(),
            modal,
        });
        x = x.saturating_add(width + 1);
    }
}

/// Paints a 1-column solid vertical scrollbar into an explicit scrollbar list
/// (used when callers hold split borrows of `App`).
#[allow(clippy::too_many_arguments)]
fn render_vertical_scrollbar_into(
    frame: &mut Frame<'_>,
    scrollbars: &mut Vec<(crate::app::VerticalScrollTarget, crate::app::ScrollbarBar)>,
    mouse_position: Option<(u16, u16)>,
    grab: Option<(crate::app::VerticalScrollTarget, u16)>,
    target: crate::app::VerticalScrollTarget,
    bar_rect: Rect,
    total: usize,
    start: usize,
) {
    if bar_rect.is_empty() || total <= bar_rect.height as usize {
        return;
    }
    let bar = crate::app::ScrollbarBar {
        rect: bar_rect,
        axis: theme::ScrollAxis::Vertical,
        total,
    };
    let grabbed = grab.is_some_and(|(active, _)| active == target);
    let state = crate::app::scrollbar_state(mouse_position, &bar, grabbed);
    let lines = theme::scrollbar_lines(
        total,
        bar_rect.height as usize,
        start,
        theme::ScrollAxis::Vertical,
        state,
    );
    frame.render_widget(Paragraph::new(lines), bar_rect);
    scrollbars.push((target, bar));
}

/// Paints a 1-column solid vertical scrollbar on `bar_rect` and registers it
/// for hover, drag, and track-paging pointer interaction.
pub(super) fn render_vertical_scrollbar(
    frame: &mut Frame<'_>,
    app: &mut App,
    target: crate::app::VerticalScrollTarget,
    bar_rect: Rect,
    total: usize,
    start: usize,
) {
    render_vertical_scrollbar_into(
        frame,
        &mut app.vertical_scrollbars,
        app.mouse_position,
        app.vertical_scrollbar_grab,
        target,
        bar_rect,
        total,
        start,
    );
}

fn render_action_bar(
    frame: &mut Frame<'_>,
    regions: &mut Vec<ClickableRegion>,
    area: Rect,
    prefix: &str,
    controls: &[(&str, Action)],
    modal: bool,
) {
    let mut spans = vec![Span::raw(prefix.to_string())];
    let mut used = prefix.width() as u16;
    for (label, action) in controls {
        let width = label.width() as u16;
        // A clipped label is not an actionable control. The compact variants
        // fit the supported narrow width without invisible hit targets.
        if used + width > area.width {
            break;
        }
        regions.push(ClickableRegion::ActionButton {
            rect: Rect::new(area.x + used, area.y, width, 1),
            action: action.clone(),
            modal,
        });
        spans.push(Span::styled(
            label.to_string(),
            Style::default()
                .fg(theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));
        used += width;
    }
    frame.render_widget(Paragraph::new(theme::footer_line(spans)), area);
}

fn visible_content_rows(inner: Rect, row: usize, count: usize, offset: usize) -> Option<Rect> {
    let start = row.max(offset);
    let end = row
        .saturating_add(count)
        .min(offset + inner.height as usize);
    (start < end).then(|| {
        Rect::new(
            inner.x,
            inner.y + (start - offset) as u16,
            inner.width,
            (end - start) as u16,
        )
    })
}

fn render_test_points_canvas(frame: &mut Frame<'_>, app: &mut App, area: Rect, focused: bool) {
    let filter_label = match app.test_points_ui.review_filter {
        crate::test_points_tab::StatusFilter::All => crate::i18n::t("tp.filter.all"),
        crate::test_points_tab::StatusFilter::Uncovered => crate::i18n::t("tp.filter.uncovered"),
        crate::test_points_tab::StatusFilter::Linked => crate::i18n::t("tp.filter.linked"),
        crate::test_points_tab::StatusFilter::Passed => crate::i18n::t("tp.filter.passed"),
        crate::test_points_tab::StatusFilter::Failed => crate::i18n::t("tp.filter.failed"),
    };
    let mut title = crate::i18n::fill(
        crate::i18n::t("tp.canvas.title"),
        &[("filter", filter_label)],
    );
    if app.test_points_ui.fullscreen_canvas {
        title.push_str(" [ZOOM]");
    }
    if app.test_points_ui.inline_edit.is_some() {
        title.push_str(" [EDIT]");
    }
    let block = theme::panel_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (content_inner, action_area) = split_pane_action_row(inner);
    let tp_actions: Vec<ActionItem<'_>> = if app.test_points_ui.editing {
        vec![
            ActionItem::new(
                crate::i18n::t("editor.btn.save.key"),
                crate::i18n::t("tp.insert.save"),
                Action::Save,
                theme::ActionTone::Success,
            ),
            ActionItem::new(
                crate::i18n::t("editor.btn.normal.key"),
                crate::i18n::t("tp.insert.normal"),
                Action::TpExitInsert,
                theme::ActionTone::Warning,
            ),
        ]
    } else {
        vec![
            ActionItem::new(
                crate::i18n::t("tp.btn.add_child.key"),
                crate::i18n::t("tp.btn.add_child"),
                Action::TpAddChild,
                theme::ActionTone::Primary,
            ),
            ActionItem::new(
                crate::i18n::t("tp.btn.add_sibling.key"),
                crate::i18n::t("tp.btn.add_sibling"),
                Action::TpAddSibling,
                theme::ActionTone::Info,
            ),
            ActionItem::new(
                crate::i18n::t("tp.btn.edit.key"),
                crate::i18n::t("tp.btn.edit"),
                Action::TpInlineEdit,
                theme::ActionTone::Neutral,
            ),
            ActionItem::new(
                crate::i18n::t("tp.btn.delete.key"),
                crate::i18n::t("tp.btn.delete"),
                Action::TpDeleteNode,
                theme::ActionTone::Danger,
            ),
            ActionItem::new(
                crate::i18n::t("tp.btn.gen.key"),
                crate::i18n::t("tp.btn.gen"),
                Action::TpGenerateFeature,
                theme::ActionTone::Success,
            ),
            ActionItem::new(
                crate::i18n::t("tp.btn.run.key"),
                crate::i18n::t("tp.btn.run"),
                Action::TpRunTestPoint,
                theme::ActionTone::Warning,
            ),
            ActionItem::new(
                crate::i18n::t("tp.btn.sync.key"),
                crate::i18n::t("tp.btn.sync"),
                Action::TpSyncFromFeatures,
                theme::ActionTone::Info,
            ),
            ActionItem::new(
                crate::i18n::t("tp.filter.key"),
                crate::i18n::t("tp.filter"),
                Action::TpReviewFilter,
                theme::ActionTone::Neutral,
            ),
            ActionItem::new(
                crate::i18n::t("tp.btn.zoom.key"),
                crate::i18n::t("tp.btn.zoom"),
                Action::TpToggleFullscreen,
                theme::ActionTone::Neutral,
            ),
        ]
    };

    if content_inner.width > 0 && content_inner.height > 0 {
        let ui = &mut app.test_points_ui;
        let has_vbar = ui.canvas_height > content_inner.height && content_inner.width >= 4;
        let canvas_area = if has_vbar {
            Rect {
                width: content_inner.width - 1,
                ..content_inner
            }
        } else {
            content_inner
        };
        app.test_points_canvas_rect = Some(canvas_area);
        ui.canvas_viewport = (canvas_area.width, canvas_area.height);
        ui.canvas_scroll_x = ui
            .canvas_scroll_x
            .min(ui.canvas_width.saturating_sub(canvas_area.width));
        ui.canvas_scroll_y = ui
            .canvas_scroll_y
            .min(ui.canvas_height.saturating_sub(canvas_area.height));

        let canvas = ui.build_canvas(theme::SCREEN_BG);
        let lines = canvas.to_ratatui_lines(
            ui.canvas_scroll_x,
            ui.canvas_scroll_y,
            canvas_area.width,
            canvas_area.height,
            theme::SCREEN_BG,
        );
        frame.render_widget(Paragraph::new(lines), canvas_area);

        // Register clickable hit targets for every visible MindMap node box.
        let vx0 = ui.canvas_scroll_x;
        let vy0 = ui.canvas_scroll_y;
        let vx1 = vx0.saturating_add(canvas_area.width);
        let vy1 = vy0.saturating_add(canvas_area.height);
        for node in &ui.placed_nodes {
            let nx0 = node.x.max(vx0);
            let ny0 = node.y.max(vy0);
            let nx1 = node.x.saturating_add(node.width).min(vx1);
            let ny1 = node.y.saturating_add(node.height).min(vy1);
            if nx0 < nx1 && ny0 < ny1 {
                app.clickable_regions
                    .push(ClickableRegion::TestPointCanvasNode {
                        rect: Rect::new(
                            canvas_area.x + (nx0 - vx0),
                            canvas_area.y + (ny0 - vy0),
                            nx1 - nx0,
                            ny1 - ny0,
                        ),
                        node_id: node.id.clone(),
                    });
            }
        }

        if has_vbar {
            let vbar = Rect::new(canvas_area.right(), canvas_area.y, 1, canvas_area.height);
            render_vertical_scrollbar_into(
                frame,
                &mut app.vertical_scrollbars,
                app.mouse_position,
                app.vertical_scrollbar_grab,
                crate::app::VerticalScrollTarget::TestPointsPane(0),
                vbar,
                ui.canvas_height as usize,
                ui.canvas_scroll_y as usize,
            );
        }
    }

    if let Some(action_area) = action_area {
        render_chip_action_bar(
            frame,
            &mut app.clickable_regions,
            action_area,
            &tp_actions,
            false,
        );
    }
}

fn inspector_step_line(keyword: &str, text: &str, max_width: u16) -> Line<'static> {
    let kw_trim = keyword.trim();
    let kw_color = if matches!(kw_trim, "Given" | "假如" | "假设") {
        theme::INFO
    } else if matches!(kw_trim, "When" | "当") {
        theme::ACCENT
    } else if matches!(kw_trim, "Then" | "那么") {
        theme::SUCCESS
    } else {
        theme::MUTED
    };
    let full = format!("    {kw_trim} {text}");
    let clipped = truncate_string_to_cols(&full, max_width);
    let prefix_len = format!("    {kw_trim} ").len().min(clipped.len());
    if prefix_len == clipped.len() {
        Line::from(Span::styled(
            clipped,
            Style::default().fg(kw_color).bg(theme::SCREEN_BG),
        ))
    } else {
        let (left, right) = clipped.split_at(prefix_len);
        Line::from(vec![
            Span::styled(
                left.to_owned(),
                Style::default()
                    .fg(kw_color)
                    .bg(theme::SCREEN_BG)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                right.to_owned(),
                Style::default().fg(theme::FG).bg(theme::SCREEN_BG),
            ),
        ])
    }
}

fn render_test_points_inspector(frame: &mut Frame<'_>, app: &mut App, area: Rect, focused: bool) {
    let ui = &mut app.test_points_ui;
    let artifacts = app.authoring_ui.artifacts.as_ref();
    let title = if ui.editing && ui.focus != crate::test_points_tab::TestPointsFocus::Tree {
        crate::i18n::t("tp.details.title.edit")
    } else {
        crate::i18n::t("tp.details.title")
    };
    let block = theme::panel_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let Some(node) = ui.selected_node().cloned() else {
        frame.render_widget(
            Paragraph::new(crate::i18n::t("tp.details.empty"))
                .style(Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG)),
            inner,
        );
        return;
    };

    let Some(tp_id) = ui.selected_test_point_id.clone() else {
        // Branch or Root summary in Inspector
        let st = node.exec_status;
        let summary_lines = vec![
            Line::from(vec![
                theme::chip(
                    format!(" {} {} ", st.glyph(), st.label()),
                    st.style().fg.unwrap_or(theme::MUTED),
                ),
                Span::styled(
                    format!("  {} scenarios", node.scenario_count),
                    Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
                ),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::styled(
                    "Node: ",
                    Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
                ),
                Span::styled(
                    truncate_string_to_cols(&node.label, inner.width.saturating_sub(6)),
                    Style::default()
                        .fg(theme::FG)
                        .bg(theme::SCREEN_BG)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                format!("Direct children: {}", node.children_ids.len()),
                Style::default().fg(theme::FG).bg(theme::SCREEN_BG),
            )),
            Line::raw(""),
            Line::from(Span::styled(
                "[a] +Child  [n] +Sibling  [i] Rename",
                Style::default().fg(theme::INFO).bg(theme::SCREEN_BG),
            )),
            Line::from(Span::styled(
                "[Space] Fold  [r] Run subtree  [d] Delete",
                Style::default().fg(theme::INFO).bg(theme::SCREEN_BG),
            )),
        ];
        frame.render_widget(
            Paragraph::new(theme::popup_rows(summary_lines, inner.width)),
            inner,
        );
        return;
    };

    let Some(tp) =
        artifacts.and_then(|a| a.test_points.test_points.iter().find(|tp| tp.id == tp_id))
    else {
        frame.render_widget(
            Paragraph::new(crate::i18n::t("tp.details.unavailable"))
                .style(Style::default().fg(theme::ERROR).bg(theme::SCREEN_BG)),
            inner,
        );
        return;
    };

    let status = ui.exec_status_for(tp);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        theme::chip(
            format!(" {} {} ", status.glyph(), status.label()),
            status.style().fg.unwrap_or(theme::MUTED),
        ),
        Span::styled(
            format!("  id: {}", tp.id),
            Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
        ),
    ]));

    let mut fields = vec![
        (
            crate::test_points_tab::DetailField::Title,
            crate::i18n::t("tp.field.title"),
            if ui.detail_field == crate::test_points_tab::DetailField::Title && ui.field_dirty {
                ui.field_buffer.clone()
            } else {
                tp.title.clone()
            },
        ),
        (
            crate::test_points_tab::DetailField::Hierarchy,
            crate::i18n::t("tp.field.hierarchy"),
            if ui.detail_field == crate::test_points_tab::DetailField::Hierarchy && ui.field_dirty {
                ui.field_buffer.clone()
            } else {
                tp.hierarchy_path.segments().join(" / ")
            },
        ),
        (
            crate::test_points_tab::DetailField::Objective,
            crate::i18n::t("tp.field.note"),
            if ui.detail_field == crate::test_points_tab::DetailField::Objective && ui.field_dirty {
                ui.field_buffer.clone()
            } else {
                tp.objective.clone()
            },
        ),
    ];
    if tp.preconditions.is_some()
        || ui.detail_field == crate::test_points_tab::DetailField::Preconditions
    {
        fields.push((
            crate::test_points_tab::DetailField::Preconditions,
            crate::i18n::t("tp.field.preconditions"),
            if ui.detail_field == crate::test_points_tab::DetailField::Preconditions
                && ui.field_dirty
            {
                ui.field_buffer.clone()
            } else {
                tp.preconditions.clone().unwrap_or_default()
            },
        ));
    }
    if tp.expected_outcomes.is_some()
        || ui.detail_field == crate::test_points_tab::DetailField::ExpectedOutcomes
    {
        fields.push((
            crate::test_points_tab::DetailField::ExpectedOutcomes,
            crate::i18n::t("tp.field.expected"),
            if ui.detail_field == crate::test_points_tab::DetailField::ExpectedOutcomes
                && ui.field_dirty
            {
                ui.field_buffer.clone()
            } else {
                tp.expected_outcomes.clone().unwrap_or_default()
            },
        ));
    }

    let details_focused = focused && ui.focus != crate::test_points_tab::TestPointsFocus::Excerpts;
    let excerpts_focused = focused && ui.focus == crate::test_points_tab::TestPointsFocus::Excerpts;

    let mut field_rows = Vec::new();
    for (field, label, value) in fields {
        field_rows.push((lines.len(), field));
        let active = ui.detail_field == field;
        let label_style = if active && details_focused {
            theme::selected_row(true)
        } else if active {
            Style::default().fg(theme::PRIMARY).bg(theme::SCREEN_BG)
        } else {
            Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG)
        };
        let value_style = if active && details_focused {
            theme::selected_row(true)
        } else {
            Style::default().fg(theme::FG).bg(theme::SCREEN_BG)
        };
        let avail = inner.width.saturating_sub(label.width() as u16 + 3);
        let display = if active && details_focused && ui.editing {
            input_with_cursor(&value, ui.field_cursor, avail)
        } else {
            truncate_string_to_cols(&value, avail)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{label}: "), label_style),
            Span::styled(display, value_style),
        ]));
    }

    // Live Realized Scenarios + Gherkin Step Preview
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        crate::i18n::t("tp.realized"),
        Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
    ));
    let mut scenario_rows = Vec::new();
    let mut matched_live = Vec::new();
    for (fi, feature) in app.project.features.iter().enumerate() {
        let rel = feature
            .file_path
            .strip_prefix(&app.project.root_dir)
            .unwrap_or(&feature.file_path)
            .to_string_lossy()
            .replace('\\', "/");
        for (si, sc) in feature.all_scenarios().into_iter().enumerate() {
            if teshi_core::authoring::parse_teshi_tp_tags(&sc.tags)
                .iter()
                .any(|id| id == &tp.id)
            {
                matched_live.push((fi, si, rel.clone(), sc));
            }
        }
    }

    if !matched_live.is_empty() {
        for (idx, (fi, si, rel, sc)) in matched_live.iter().enumerate() {
            scenario_rows.push((lines.len(), idx));
            let st_glyph = match app.explore_case_status.get(&(*fi, *si)) {
                Some(crate::app::RunStatus::Passed) => "✔",
                Some(crate::app::RunStatus::Failed) => "✖",
                Some(crate::app::RunStatus::Running) => "⠋",
                _ => "◐",
            };
            let selected = idx == ui.selected_scenario_ref_index;
            let row_style = if selected && details_focused {
                theme::selected_row(true)
            } else {
                Style::default().fg(theme::FG).bg(theme::SCREEN_BG)
            };
            let header = truncate_string_to_cols(
                &format!("  {st_glyph} {} ({rel})", sc.name),
                inner.width.saturating_sub(1),
            );
            lines.push(Line::from(Span::styled(header, row_style)));
            for step in sc.steps.iter().take(5) {
                lines.push(inspector_step_line(
                    &step.keyword,
                    &step.text,
                    inner.width.saturating_sub(1),
                ));
            }
            if sc.steps.len() > 5 {
                lines.push(Line::from(Span::styled(
                    format!("    … +{} steps", sc.steps.len() - 5),
                    Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
                )));
            }
        }
    } else if !tp.scenario_refs.is_empty() {
        for (idx, sc_ref) in tp.scenario_refs.iter().enumerate() {
            scenario_rows.push((lines.len(), idx));
            let label = sc_ref.scenario_name.clone().unwrap_or_else(|| {
                crate::i18n::t("tp.line")
                    .replace("{n}", &sc_ref.scenario_line.unwrap_or(0).to_string())
            });
            let selected = idx == ui.selected_scenario_ref_index;
            let row_style = if selected && details_focused {
                theme::selected_row(true)
            } else {
                Style::default().fg(theme::FG).bg(theme::SCREEN_BG)
            };
            lines.push(Line::from(Span::styled(
                truncate_string_to_cols(
                    &format!("  ◐ {} — {}", label, sc_ref.feature_path),
                    inner.width.saturating_sub(1),
                ),
                row_style,
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            truncate_string_to_cols(
                crate::i18n::t("tp.no_scenario_hint"),
                inner.width.saturating_sub(1),
            ),
            Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
        )));
    }

    // Linked Requirement Documents
    let excerpts = artifacts
        .map(|a| crate::test_points_tab::excerpts_for_test_point(tp, a))
        .unwrap_or_default();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        crate::i18n::t("tp.excerpts.title"),
        Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
    ));
    let mut excerpt_rows = Vec::new();
    if excerpts.is_empty() {
        lines.push(Line::styled(
            crate::i18n::t("tp.excerpts.empty"),
            Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
        ));
    } else {
        for (i, ex) in excerpts.iter().enumerate() {
            excerpt_rows.push((lines.len(), i));
            let tag = if ex.resolved {
                crate::i18n::t("tp.excerpt.ok")
            } else {
                crate::i18n::t("tp.excerpt.stale")
            };
            let tag_color = if ex.resolved {
                theme::SUCCESS
            } else {
                theme::WARNING
            };
            let row_style = if i == ui.selected_excerpt_index && excerpts_focused {
                theme::selected_row(true)
            } else {
                Style::default().fg(theme::FG).bg(theme::SCREEN_BG)
            };
            let doc = truncate_string_to_cols(&ex.document_title, inner.width.saturating_sub(8));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("[{tag}] "),
                    Style::default().fg(tag_color).bg(theme::SCREEN_BG),
                ),
                Span::styled(doc, row_style),
            ]));
            if !ex.quote.is_empty() {
                let quote = truncate_string_to_cols(&ex.quote, inner.width.saturating_sub(4));
                lines.push(Line::from(Span::styled(
                    format!("  \"{quote}\""),
                    Style::default().fg(theme::MUTED).bg(theme::SCREEN_BG),
                )));
            }
        }
    }

    let total_rows = lines.len();
    let has_vbar = total_rows > inner.height as usize && inner.width >= 3;
    let body_area = if has_vbar {
        Rect {
            width: inner.width - 1,
            ..inner
        }
    } else {
        inner
    };
    let rows = theme::popup_rows(lines, body_area.width);
    let max_scroll = rows.len().saturating_sub(body_area.height as usize);
    ui.scroll_offsets[1] = ui.scroll_offsets[1].min(max_scroll);
    for (row, field) in field_rows {
        if let Some(rect) = visible_content_rows(body_area, row, 1, ui.scroll_offsets[1]) {
            app.clickable_regions
                .push(ClickableRegion::TestPointField { rect, field });
        }
    }
    for (row, index) in scenario_rows {
        if let Some(rect) = visible_content_rows(body_area, row, 1, ui.scroll_offsets[1]) {
            app.clickable_regions
                .push(ClickableRegion::TestPointScenario { rect, index });
        }
    }
    for (row, index) in excerpt_rows {
        if let Some(rect) = visible_content_rows(body_area, row, 1, ui.scroll_offsets[1]) {
            app.clickable_regions
                .push(ClickableRegion::TestPointExcerpt { rect, index });
        }
    }
    frame.render_widget(
        Paragraph::new(rows).scroll((ui.scroll_offsets[1] as u16, 0)),
        body_area,
    );
    if has_vbar {
        let vbar = Rect::new(body_area.right(), body_area.y, 1, body_area.height);
        render_vertical_scrollbar_into(
            frame,
            &mut app.vertical_scrollbars,
            app.mouse_position,
            app.vertical_scrollbar_grab,
            crate::app::VerticalScrollTarget::TestPointsPane(1),
            vbar,
            total_rows,
            ui.scroll_offsets[1],
        );
    }
}

#[cfg(test)]
mod truncate_tests {
    use std::path::PathBuf;

    use super::{
        Line, Span, explore_panes, explore_panes_with_details, explore_run_summary_label,
        explore_scenarios_title, push_exchange_rows, render, truncate_line_to_cols,
    };
    use crate::app::{App, CaseDetail, ClickableRegion, ColumnFocus, RunStatus};
    use crate::keymap::Action;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};
    use teshi_core::gherkin;

    #[test]
    fn explore_scenarios_take_only_their_rows_while_steps_and_details_share_the_rest() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);

        let [_, scenario, steps, details] = explore_panes(area, 0, 1);
        assert_eq!(
            scenario.height, 4,
            "one scenario plus borders and the action strip"
        );
        assert_eq!(steps.height, 26);
        assert_eq!(details.height, 26);
        assert_eq!(steps.width, 38, "Steps keeps its documented 38-column band");
        assert_eq!(details.x, steps.right());
        assert_eq!(details.right(), area.right());
        assert_eq!(scenario.y, area.y, "Scenarios sit above the Steps list");
        assert_eq!(steps.bottom(), area.bottom());

        let [_, scenario, steps, details] = explore_panes(area, 0, 0);
        assert_eq!(
            scenario.height, 4,
            "an empty list still has borders and the action strip"
        );
        assert_eq!(steps.height, 26);
        assert_eq!(details.height, 26);

        let [_, scenario, steps, details] = explore_panes(area, 0, 30);
        assert_eq!(scenario.height, 12, "the scenario pane caps at 40%");
        assert_eq!(steps.height, 18);
        assert_eq!(details.height, 18);

        let [left, _, collapsed_steps, collapsed_details] =
            explore_panes_with_details(area, 0, 1, true);
        assert_eq!(collapsed_details, ratatui::layout::Rect::default());
        assert_eq!(collapsed_steps.x, left.right());
        assert_eq!(collapsed_steps.right(), area.right());
    }

    #[test]
    fn explore_narrow_or_short_layout_shows_only_the_focused_pane() {
        let area = ratatui::layout::Rect::new(0, 0, 60, 20);
        let panes = explore_panes(area, 2, 5);
        assert_eq!(panes[0], ratatui::layout::Rect::default());
        assert_eq!(panes[1], ratatui::layout::Rect::default());
        assert_eq!(panes[2], area);
        assert_eq!(panes[3], ratatui::layout::Rect::default());

        let panes = explore_panes(area, 3, 5);
        assert_eq!(panes[3], area, "Details joins the focus rotation");
        assert_eq!(panes[0], ratatui::layout::Rect::default());
    }

    #[test]
    fn workspace_meta_pins_the_version_to_the_tab_row() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.version = "0.7.10-local.20261014.1700+0800 (1234567)".into();

        // A wide row seats the project label left of the version.
        let mut wide = Terminal::new(TestBackend::new(200, 24)).unwrap();
        wide.draw(|frame| render(frame, &mut app)).unwrap();
        let row: String = (0..200)
            .map(|x| wide.backend().buffer()[(x, 0)].symbol())
            .collect();
        assert!(row.contains("Test Points [5]"), "tabs keep their row");
        assert!(row.contains("Project:"), "{row}");
        assert!(row.ends_with("(1234567)"), "{row}");
        assert_eq!(
            wide.backend().buffer()[(199, 0)].fg,
            crate::theme::MUTED,
            "the version sits in the corner"
        );

        // A narrow row keeps the project control and drops the version.
        let mut narrow = Terminal::new(TestBackend::new(120, 24)).unwrap();
        narrow.draw(|frame| render(frame, &mut app)).unwrap();
        let row: String = (0..120)
            .map(|x| narrow.backend().buffer()[(x, 0)].symbol())
            .collect();
        assert!(row.contains("Project: "), "{row}");
        assert!(!row.contains("v0.7.10-local"), "{row}");
        assert!(
            app.clickable_regions
                .iter()
                .any(|region| matches!(region, ClickableRegion::ProjectControl { .. })),
            "the project label stays clickable when the version yields"
        );
    }

    #[test]
    fn screen_surface_covers_every_tab() {
        // iCode paints `MainScreen` with the theme background; teshi does the
        // same so no cell keeps the terminal profile's own colors. A cleared
        // region that forgets `clear_to` shows up here as a `Reset` cell.
        for tab in [
            crate::app::MainTab::Explore,
            crate::app::MainTab::MindMap,
            crate::app::MainTab::Ai,
            crate::app::MainTab::Requirements,
            crate::app::MainTab::TestPoints,
        ] {
            let mut app = App::from_args().expect("app init");
            app.set_ui_locale(teshi_engine::UiLocalePreference::En);
            app.active_tab = tab;
            app.status_message = None;
            let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let width = buffer.area.width as usize;
            // A default background is the terminal's own; no leading cell may
            // keep it, or the terminal profile would show through the surface.
            let terminal_owned: ratatui::style::Color = Default::default();
            // Ratatui resets the hidden continuation cell of a wide glyph, so
            // only a leading cell can expose the terminal background.
            let transparent: Vec<(u16, u16)> = buffer
                .content
                .iter()
                .enumerate()
                .filter(|(_, cell)| cell.bg == terminal_owned)
                .filter(|(index, _)| {
                    *index % width == 0
                        || unicode_width::UnicodeWidthStr::width(buffer.content[index - 1].symbol())
                            < 2
                })
                .map(|(index, _)| ((index % width) as u16, (index / width) as u16))
                .collect();
            assert!(
                transparent.is_empty(),
                "{tab:?} leaves {} cells on the terminal background: {:?}",
                transparent.len(),
                &transparent[..transparent.len().min(12)]
            );
            assert!(
                buffer
                    .content
                    .iter()
                    .any(|cell| cell.bg == crate::theme::SCREEN_BG),
                "{tab:?} paints the screen surface"
            );
        }
    }

    /// Bounds of the rounded card whose top border carries `title`.
    fn card_bounds(buffer: &ratatui::buffer::Buffer, title: &str) -> Option<(u16, u16, u16, u16)> {
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            let Some(byte) = row.find(title) else {
                continue;
            };
            let title_x = unicode_width::UnicodeWidthStr::width(&row[..byte]) as u16;
            let left = (0..title_x)
                .rev()
                .find(|x| buffer[(*x, y)].symbol() == "╭")?;
            let right = ((title_x + title.len() as u16)..buffer.area.width)
                .find(|x| buffer[(*x, y)].symbol() == "╮")?;
            let bottom =
                ((y + 1)..buffer.area.height).find(|row| buffer[(left, *row)].symbol() == "╰")?;
            return Some((left, y, right, bottom));
        }
        None
    }

    #[test]
    fn history_search_caps_its_width_and_height_on_a_large_terminal() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.active_tab = crate::app::MainTab::Ai;
        app.ai_input_focused = true;
        app.handle_action(Action::AiHistorySearchOpen).unwrap();
        assert!(app.history_search.is_some(), "history search opened");

        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (left, top, right, bottom) =
            card_bounds(buffer, " Prompt history ").expect("history search renders");
        let width = right - left + 1;
        let height = bottom - top + 1;
        let right_margin = 200 - right - 1;
        assert_eq!(width, 100, "70% width capped at 100 columns");
        assert_eq!(height, 24, "55% height capped at 24 rows");
        assert!(left >= 10 && right_margin >= 10, "10% backdrop margin");

        let mut narrow = Terminal::new(TestBackend::new(40, 20)).unwrap();
        narrow.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = narrow.backend().buffer();
        let (left, _top, right, _bottom) =
            card_bounds(buffer, " Prompt history ").expect("history search renders narrow");
        assert_eq!(
            right - left + 1,
            36,
            "minimum width survives a narrow screen"
        );
    }

    #[test]
    fn auth_overview_hugs_its_lines_on_a_large_terminal() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.auth_panel_active = true;
        app.status = "Auth: add provider".into();

        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (left, top, right, bottom) =
            card_bounds(buffer, " Auth Management ").expect("auth card renders");
        let width = right - left + 1;
        let height = bottom - top + 1;
        assert_eq!(width, 60, "fixed 60-column form card");
        assert_eq!(height, 8, "three lines plus chrome, floored at 8 rows");
    }

    #[test]
    fn model_list_hugs_its_profiles_on_a_large_terminal() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.model_panel_active = true;
        app.model_panel_mode = crate::app::ModelPanelMode::List;
        app.model_profiles.clear();

        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (left, top, right, bottom) =
            card_bounds(buffer, " Model Profiles ").expect("model card renders");
        let width = right - left + 1;
        let height = bottom - top + 1;
        assert_eq!(width, 60, "fixed 60-column form card");
        assert_eq!(height, 8, "the empty state plus chrome");
    }

    fn diff_lines(count: usize, text: impl Fn(usize) -> String) -> Vec<teshi_core::diff::DiffLine> {
        (0..count)
            .map(|index| teshi_core::diff::DiffLine {
                line_number_1based: index,
                kind: teshi_core::diff::ChangeKind::Added,
                text: text(index),
                old_text: None,
            })
            .collect()
    }

    fn pending_change_app(description: &str, diff: Vec<teshi_core::diff::DiffLine>) -> App {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.active_tab = crate::app::MainTab::Ai;
        app.status_message = None;
        app.pending_agent_changes
            .push(crate::app::AgentPendingChange {
                description: description.into(),
                file_path: "features/sample.feature".into(),
                mutation: crate::app::AgentMutation::ReplaceLine {
                    row_0based: 0,
                    new_text: "Scenario: sample".into(),
                },
                scenario_name: "sample".into(),
                test_point_ids: Vec::new(),
                tool_call_id: "call-1".into(),
                old_buffer_snapshot: String::new(),
                agent_idx: 0,
            });
        app.pending_change_diffs.push(diff);
        app
    }

    /// Screen text and card bounds for one pending-change render.
    fn pending_change_frame(
        app: &mut App,
        width: u16,
        height: u16,
    ) -> (String, (u16, u16, u16, u16)) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let bounds =
            card_bounds(buffer, " Pending Agent Change ").expect("pending change card renders");
        let screen: String = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<String>>()
            .join("\n");
        (screen, bounds)
    }

    #[test]
    fn pending_change_prompt_caps_its_diff_and_reports_hidden_rows() {
        let mut app = pending_change_app(
            "Update the sample scenario",
            diff_lines(40, |index| format!("step {index}")),
        );
        let (screen, (left, top, right, bottom)) = pending_change_frame(&mut app, 200, 60);
        let width = right - left + 1;
        let height = bottom - top + 1;
        assert_eq!(width, 60, "content width with a floor for the decision row");
        assert_eq!(height, 22, "40 diff rows stop at the 22-row cap");
        assert_eq!(left, (200 - width) / 2, "centered with a backdrop margin");
        assert!(
            screen.contains("+23 more lines"),
            "hidden diff lines are reported:\n{screen}"
        );
    }

    #[test]
    fn pending_change_prompt_shows_a_diff_that_exactly_fits_without_a_hint() {
        // The floor card has body 5: one description row + four diff rows.
        let mut app = pending_change_app(
            "Update the sample scenario",
            diff_lines(4, |index| format!("step {index}")),
        );
        let (screen, (_, top, _, bottom)) = pending_change_frame(&mut app, 200, 60);
        assert_eq!(bottom - top + 1, 8, "content fits the floor card");
        assert!(
            screen.contains("step 3"),
            "every diff line is drawn:\n{screen}"
        );
        assert!(
            !screen.contains("more lines"),
            "a fitting diff needs no hint:\n{screen}"
        );
    }

    #[test]
    fn pending_change_prompt_counts_wrapped_diff_rows() {
        // 200-cell lines wrap to three rows each at the 100-column cap; the
        // height budget must count those rows, not the eight source lines.
        let mut app = pending_change_app(
            "Update the sample scenario",
            diff_lines(8, |index| format!("{index:02}{}", "x".repeat(198))),
        );
        let (screen, (_, top, _, bottom)) = pending_change_frame(&mut app, 200, 60);
        assert_eq!(
            bottom - top + 1,
            22,
            "wrapped rows fill the 22-row cap, not a 12-row guess"
        );
        assert!(screen.contains("xxx"), "diff content is visible:\n{screen}");
        assert!(
            screen.contains("more lines"),
            "the hint stays inside the card:\n{screen}"
        );
    }

    #[test]
    fn pending_change_prompt_without_a_diff_stays_compact() {
        let mut app = pending_change_app("Update the sample scenario", Vec::new());
        let (screen, (_, top, _, bottom)) = pending_change_frame(&mut app, 200, 60);
        assert_eq!(bottom - top + 1, 8, "description plus chrome");
        assert!(screen.contains("Update the sample scenario"));
        assert!(!screen.contains("more lines"));
    }

    #[test]
    fn model_form_uses_its_eleven_content_rows() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.model_panel_active = true;
        app.model_panel_mode = crate::app::ModelPanelMode::Adding;

        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (left, top, right, bottom) =
            card_bounds(buffer, " Add Model Profile ").expect("model form renders");
        let width = right - left + 1;
        let height = bottom - top + 1;
        assert_eq!(width, 60);
        assert_eq!(height, 11, "blank row + seven fields + chrome");
    }

    #[test]
    fn model_form_keeps_all_fields_on_a_short_terminal() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.model_panel_active = true;
        app.model_panel_mode = crate::app::ModelPanelMode::Adding;

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (_, top, _, bottom) =
            card_bounds(buffer, " Add Model Profile ").expect("model form renders");
        assert_eq!(
            bottom - top + 1,
            11,
            "the full-content card, without clipping"
        );
        let screen: String = (0..12)
            .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<String>>()
            .join("\n");
        assert!(
            screen.contains("Temperature"),
            "the last field stays visible:\n{screen}"
        );
    }

    #[test]
    fn acp_permission_card_sizes_to_its_options() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.active_tab = crate::app::MainTab::Ai;
        app.status_message = None;
        app.agent_mut().pending_acp_permission = Some(crate::app::AcpPermissionPrompt {
            request_id: 1,
            options: vec![
                teshi_agent::backend::PermissionChoice {
                    id: "allow_once".into(),
                    label: Some("Allow once".into()),
                    kind: None,
                },
                teshi_agent::backend::PermissionChoice {
                    id: "always".into(),
                    label: Some("Always allow".into()),
                    kind: None,
                },
                teshi_agent::backend::PermissionChoice {
                    id: "reject".into(),
                    label: Some("Reject".into()),
                    kind: None,
                },
            ],
            selected: 0,
        });

        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (left, top, right, bottom) =
            card_bounds(buffer, " ACP permission requested ").expect("ACP card renders");
        assert_eq!(
            right - left + 1,
            60,
            "option width with the 60-column floor"
        );
        assert_eq!(bottom - top + 1, 8, "three option rows plus chrome");
        let choices = app
            .clickable_regions
            .iter()
            .filter(|region| {
                matches!(
                    region,
                    ClickableRegion::ActionButton {
                        action: Action::AcpPermissionChoose(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(choices, 3, "every visible option is clickable");
    }

    #[test]
    fn migrated_viewer_specs_stay_inside_every_viewport() {
        for (width, height) in [(200u16, 60u16), (120, 36), (80, 24), (40, 20)] {
            for spec in [
                super::CHANGE_SUMMARY_SPEC,
                super::SETTINGS_SPEC,
                super::AGENT_PROFILE_SPEC,
            ] {
                let placed = spec.rect(ratatui::layout::Rect::new(0, 0, width, height), (0, 0));
                assert!(
                    placed.width <= width && placed.height <= height,
                    "{spec:?} overflows {width}x{height}: {placed:?}"
                );
                assert!(placed.right() <= width && placed.bottom() <= height);
            }
        }

        let area = ratatui::layout::Rect::new(0, 0, 200, 60);
        let summary = super::CHANGE_SUMMARY_SPEC.rect(area, (0, 0));
        assert_eq!((summary.width, summary.height), (110, 33));
        let settings = super::SETTINGS_SPEC.rect(area, (0, 0));
        assert_eq!((settings.width, settings.height), (80, 22));
        let agent = super::AGENT_PROFILE_SPEC.rect(area, (0, 0));
        assert_eq!((agent.width, agent.height), (100, 50));
    }

    #[test]
    fn global_footer_lists_semantic_actions_on_every_tab() {
        // Contextual workflows live in pane strips; the global footer keeps
        // Settings (with its F10 chord) and Exit (with Ctrl+Q) only.
        for tab in [
            crate::app::MainTab::Explore,
            crate::app::MainTab::MindMap,
            crate::app::MainTab::Ai,
            crate::app::MainTab::Requirements,
            crate::app::MainTab::TestPoints,
        ] {
            // Labels follow the UI locale catalog; pin English so this test
            // asserts the same strings on a zh-CN host as in CI.
            let mut app = App::from_args().expect("app init");
            app.set_ui_locale(teshi_engine::UiLocalePreference::En);
            app.active_tab = tab;
            app.status_message = None;
            let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..160)
                .map(|x| buffer[(x, 23)].symbol())
                .collect::<String>();
            assert!(
                !text.contains("Navigate")
                    && !text.contains("Sessions")
                    && !text.contains("Agents")
                    && !text.contains("Models")
                    && !text.contains("Save"),
                "footer left keeps Settings only on {tab:?}: {text}"
            );
            assert!(
                text.contains("Settings [F10]"),
                "the Settings control stays on {tab:?}: {text}"
            );
            assert!(
                text.contains("Exit [Ctrl+Q]"),
                "Exit advertises Ctrl+Q on {tab:?}: {text}"
            );
            // footer_line paints bracketed chords in PRIMARY; the action name
            // stays FOOTER_DESC so the key tip is the discoverable accent.
            let byte = text.find("[F10]").unwrap();
            let offset = unicode_width::UnicodeWidthStr::width(&text[..byte]) as u16;
            assert_eq!(
                buffer[(offset, 23)].fg,
                crate::theme::PRIMARY,
                "{tab:?} Settings advertises F10: {text}"
            );
            let exit_byte = text.find("[Ctrl+Q]").unwrap();
            let exit_offset = unicode_width::UnicodeWidthStr::width(&text[..exit_byte]) as u16;
            assert_eq!(
                buffer[(exit_offset, 23)].fg,
                crate::theme::PRIMARY,
                "{tab:?} Exit advertises Ctrl+Q: {text}"
            );
            assert!(
                app.clickable_regions.iter().any(|region| matches!(
                    region,
                    ClickableRegion::ActionButton {
                        action: Action::SettingsPanelOpen,
                        ..
                    }
                )),
                "Settings is clickable"
            );
            let exit = app
                .clickable_regions
                .iter()
                .find_map(|region| match region {
                    ClickableRegion::ActionButton {
                        rect,
                        action: Action::Quit,
                        ..
                    } => Some(*rect),
                    _ => None,
                })
                .expect("bottom-right exit control");
            assert_eq!(exit.y, 23);
            assert_eq!(exit.right(), 160);
        }
    }

    #[test]
    fn exit_control_is_pinned_to_the_bottom_right_in_every_tab() {
        use crate::app::MainTab;
        for (index, tab) in [
            MainTab::Explore,
            MainTab::MindMap,
            MainTab::Ai,
            MainTab::Requirements,
            MainTab::TestPoints,
        ]
        .into_iter()
        .enumerate()
        {
            let mut app = App::from_args().expect("app init");
            app.set_ui_locale(teshi_engine::UiLocalePreference::ZhCn);
            app.active_tab = tab;
            app.status_message = None;
            let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let exit = app
                .clickable_regions
                .iter()
                .find_map(|region| match region {
                    ClickableRegion::ActionButton {
                        rect,
                        action: Action::Quit,
                        ..
                    } => Some(*rect),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("tab {index} has no clickable exit control"));
            assert_eq!(exit.y, 23, "tab {index}");
            assert_eq!(exit.right(), 100, "tab {index}");
            let label = (exit.x..exit.right())
                .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
                .collect::<String>();
            assert!(label.contains('退'), "tab {index}: {label}");
            assert!(
                label.contains("Ctrl+Q"),
                "tab {index} advertises the quit chord: {label}"
            );
        }
    }

    #[test]
    fn explore_scenarios_pane_exposes_run_edit_detail_actions() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.active_tab = crate::app::MainTab::Explore;
        app.status_message = None;
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        for action in [
            Action::RunScenario,
            Action::FocusRunDetails,
            Action::EnterEdit,
        ] {
            assert!(
                app.clickable_regions.iter().any(|region| matches!(
                    region,
                    ClickableRegion::ActionButton {
                        action: found,
                        ..
                    } if found == &action
                )),
                "Explore scenarios pane missing {action:?}"
            );
        }
        let footer = (0..160)
            .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
            .collect::<String>();
        assert!(
            !footer.contains("Run") && !footer.contains("Edit"),
            "Explore actions left the global footer: {footer}"
        );
    }

    #[test]
    fn settings_panel_shows_the_prompt_section_switch_and_click_target() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.settings_panel_active = true;
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let screen: String = (0..28)
            .map(|y| {
                (0..100)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Inject Teshi prompt context"), "{screen}");
        assert!(screen.contains("UI Language"), "{screen}");
        for label in [
            " General ",
            " AI Chat ",
            " Language ",
            "Control what Teshi adds to every AI conversation.",
            "└─",
        ] {
            assert!(screen.contains(label), "{label}\n{screen}");
        }
        assert!(
            screen.contains("[x]"),
            "the default-on switch is checked\n{screen}"
        );
        assert!(
            screen.contains("Changes are saved as you make them"),
            "the bottom border states the save semantics\n{screen}"
        );
        assert!(
            screen.contains(" Close "),
            "the bottom border offers a semantic close control\n{screen}"
        );
        assert!(
            !screen.contains("Close [Esc]"),
            "key-hint footer is gone\n{screen}"
        );
        assert!(app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ActionButton {
                action: Action::SettingsPanelActivate(0),
                ..
            }
        )));
        assert!(app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ActionButton {
                action: Action::SettingsPanelActivate(1),
                ..
            }
        )));
        assert!(
            app.clickable_regions.iter().any(|region| matches!(
                region,
                ClickableRegion::ActionButton {
                    action: Action::SettingsPanelClose,
                    ..
                }
            )),
            "Close is clickable"
        );
        app.handle_action(Action::SettingsPanelClose).unwrap();
        assert!(!app.settings_panel_active, "the Close control dismisses");
    }

    #[test]
    fn narrow_footer_keeps_tab_actions_without_the_exit_control() {
        let mut app = App::from_args().expect("app init");
        app.status_message = None;
        let mut terminal = Terminal::new(TestBackend::new(50, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert!(!app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ActionButton {
                action: Action::Quit,
                ..
            }
        )));
    }

    #[test]
    fn command_line_renders_in_the_footer_and_keeps_the_exit_control() {
        let mut app = App::from_args().expect("app init");
        app.status_message = None;
        app.command_line = Some(crate::authoring_tab::LineInput::new("wq"));
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let row = (0..100)
            .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
            .collect::<String>();
        assert!(row.starts_with(":wq"), "{row}");
        assert!(row.contains("Exit [Ctrl+Q]"), "{row}");
        assert!(
            !app.clickable_regions.iter().any(|region| matches!(
                region,
                ClickableRegion::ActionButton {
                    action: Action::Save,
                    ..
                }
            )),
            "the prompt replaces the tab action bar"
        );
    }

    #[test]
    fn truncate_line_to_cols_limits_display_width() {
        let line = Line::from(Span::raw("a".repeat(100)));
        assert!(line.width() > 68);
        let out = truncate_line_to_cols(line, 68);
        assert!(out.width() <= 68);
    }

    #[test]
    fn test_explore_scenarios_title_shows_zero_when_no_feature_selected() {
        let app = App::from_args().expect("app init should work");
        assert_eq!(explore_scenarios_title(&app), "Scenarios (0)");
    }

    fn detail_render_app(root: &std::path::Path) -> App {
        let content = "Feature: Login\n  Scenario: Successful login\n    Given the API is running\n    Then status is 200\n";
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.project.root_dir = root.to_path_buf();
        app.project.features = vec![gherkin::parse_feature(content, root.join("login.feature"))];
        app.active_tab = crate::app::MainTab::Explore;
        app.explore_selected_feature = 0;
        app.explore_selected_scenario = 0;
        app.explore_selected_step = 1;
        app
    }

    fn draw_detail_screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| render(frame, app))
            .expect("render details");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn clickable_detail_app(root: &std::path::Path) -> App {
        let mut app = detail_render_app(root);
        let exchange = |id: &str, status: u16| {
            teshi_core::HttpExchange::from_value(&serde_json::json!({
                "type": "http_exchange",
                "exchange_id": id,
                "case_id": "f0:s1",
                "step_id": "f0:s1:L4",
                "template": "login.json.j2",
                "method": "POST",
                "url": "/api/login",
                "request_headers": {"Accept": "application/json"},
                "request_body": {"password": "***", "name": "Ada"},
                "status": status,
                "response_headers": {"content-type": "application/json"},
                "response_body": {"session": "8f2c…a91"},
                "duration_ms": 128,
                "extract": {},
                "asserts": [{"name": "status_ok", "passed": true}],
                "redacted": true
            }))
            .expect("exchange")
        };
        let detail = CaseDetail {
            case_id: "f0:s1".into(),
            status: RunStatus::Passed,
            duration_ms: Some(1240),
            message: None,
            stack: None,
            attachments: vec![
                crate::runner::RunAttachment {
                    kind: "screenshot".into(),
                    path: root
                        .join(".teshi/logs/replay-screenshots/run/after-login.png")
                        .display()
                        .to_string(),
                    step_id: Some("f0:s1:L4".into()),
                },
                crate::runner::RunAttachment {
                    kind: "recording".into(),
                    path: root
                        .join(".teshi/logs/replay-screenshots/run/run.mp4")
                        .display()
                        .to_string(),
                    step_id: None,
                },
            ],
            logs: (0..3).map(|index| format!("log line {index}")).collect(),
            exchanges: vec![exchange("e1", 200), exchange("e2", 201)],
            steps: vec![crate::app::StepRunRecord {
                step_id: "f0:s1:L4".into(),
                text: "status is 200".into(),
                status: Some("passed".into()),
                message: None,
            }],
            inspector_plaintext: false,
            started_at: None,
            screenshots: Vec::new(),
            historical: false,
        };
        app.explore_case_details.insert((0, 0), detail);
        app.explore_details_tab = crate::app::RunDetailTab::Screenshots;
        app
    }

    fn detail_region_rect(
        app: &App,
        matches: impl Fn(&ClickableRegion) -> bool,
    ) -> ratatui::layout::Rect {
        app.clickable_regions
            .iter()
            .find_map(|region| {
                if !matches(region) {
                    return None;
                }
                match region {
                    ClickableRegion::ExploreDetailPane { rect }
                    | ClickableRegion::ExploreDetailTab { rect, .. }
                    | ClickableRegion::ExploreDetailItem { rect, .. } => Some(*rect),
                    _ => None,
                }
            })
            .expect("requested Run Details region")
    }

    #[test]
    fn run_details_tabs_rows_and_pane_are_clickable() {
        let root = std::env::temp_dir().join("teshi-run-details-click");
        let mut app = clickable_detail_app(&root);
        app.explore_focus = ColumnFocus::Step;
        let mut terminal = Terminal::new(TestBackend::new(160, 32)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");

        let api_tab = detail_region_rect(&app, |region| {
            matches!(
                region,
                ClickableRegion::ExploreDetailTab {
                    tab: crate::app::RunDetailTab::Api,
                    ..
                }
            )
        });
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            api_tab.x + 1,
            api_tab.y,
            KeyModifiers::NONE,
        )
        .expect("click API tab");
        assert_eq!(app.explore_focus, ColumnFocus::Details);
        assert_eq!(app.explore_details_tab, crate::app::RunDetailTab::Api);
        assert!(app.explore_details_tab_pinned);
        assert_eq!(app.explore_details_selection, 0);

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render API");
        let second_exchange = detail_region_rect(&app, |region| {
            matches!(region, ClickableRegion::ExploreDetailItem { index: 1, .. })
        });
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            second_exchange.x + 1,
            second_exchange.y,
            KeyModifiers::NONE,
        )
        .expect("click second exchange");
        assert_eq!(app.explore_focus, ColumnFocus::Details);
        assert_eq!(app.explore_details_selection, 1);

        app.explore_focus = ColumnFocus::Step;
        let pane = detail_region_rect(&app, |region| {
            matches!(region, ClickableRegion::ExploreDetailPane { .. })
        });
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            pane.x,
            pane.y,
            KeyModifiers::NONE,
        )
        .expect("click pane chrome");
        assert_eq!(app.explore_focus, ColumnFocus::Details);
        assert_eq!(
            app.explore_details_selection, 1,
            "clicking pane chrome keeps the selected row"
        );
    }

    #[test]
    fn run_details_wheel_scrolls_content_without_changing_steps() {
        let root = std::env::temp_dir().join("teshi-run-details-wheel");
        let mut app = clickable_detail_app(&root);
        let mut detail = app.explore_case_details.remove(&(0, 0)).expect("detail");
        detail.logs = (0..40).map(|index| format!("log line {index}")).collect();
        app.explore_case_details.insert((0, 0), detail);
        app.explore_details_tab = crate::app::RunDetailTab::Logs;
        app.explore_focus = ColumnFocus::Details;
        let mut terminal = Terminal::new(TestBackend::new(160, 32)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let pane = detail_region_rect(&app, |region| {
            matches!(region, ClickableRegion::ExploreDetailPane { .. })
        });
        let step = app.explore_selected_step;

        app.handle_mouse_event(
            MouseEventKind::ScrollDown,
            pane.x + 2,
            pane.y + 5,
            KeyModifiers::NONE,
        )
        .expect("wheel down");
        let expected = crate::app::content_wheel_step(usize::from(pane.height.saturating_sub(2)));
        assert_eq!(app.explore_details_scroll, expected);
        assert!(!app.explore_details_reveal);
        assert_eq!(app.explore_selected_step, step);

        app.handle_mouse_event(
            MouseEventKind::ScrollUp,
            pane.x + 2,
            pane.y + 5,
            KeyModifiers::NONE,
        )
        .expect("wheel up");
        assert_eq!(app.explore_details_scroll, 0);
    }

    #[test]
    fn run_details_mouse_targets_work_in_a_narrow_single_pane() {
        let root = std::env::temp_dir().join("teshi-run-details-narrow");
        let mut app = clickable_detail_app(&root);
        app.explore_focus = ColumnFocus::Details;
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render narrow");

        let api_tab = detail_region_rect(&app, |region| {
            matches!(
                region,
                ClickableRegion::ExploreDetailTab {
                    tab: crate::app::RunDetailTab::Api,
                    ..
                }
            )
        });
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            api_tab.x + 1,
            api_tab.y,
            KeyModifiers::NONE,
        )
        .expect("click narrow API tab");
        assert_eq!(app.explore_details_tab, crate::app::RunDetailTab::Api);
        assert_eq!(app.explore_focus, ColumnFocus::Details);
    }

    #[test]
    fn run_details_action_strip_click_expands_the_selected_exchange() {
        let root = std::env::temp_dir().join("teshi-run-details-actions");
        let mut app = clickable_detail_app(&root);
        app.explore_details_tab = crate::app::RunDetailTab::Api;
        app.explore_focus = ColumnFocus::Details;
        let mut terminal = Terminal::new(TestBackend::new(160, 32)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let button = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::ActionButton {
                    action: Action::ExploreDetailToggle,
                    rect,
                    ..
                } => Some(*rect),
                _ => None,
            })
            .expect("Space expand button");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            button.x + 1,
            button.y,
            KeyModifiers::NONE,
        )
        .expect("click Space expand");
        assert!(app.explore_details_expanded.contains("e1"));
    }

    #[test]
    fn run_details_edit_mode_registers_no_mouse_targets() {
        let root = std::env::temp_dir().join("teshi-run-details-edit");
        let mut app = clickable_detail_app(&root);
        app.explore_edit_mode = true;
        let mut terminal = Terminal::new(TestBackend::new(160, 32)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render edit mode");
        assert!(!app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ExploreDetailPane { .. }
                | ClickableRegion::ExploreDetailTab { .. }
                | ClickableRegion::ExploreDetailItem { .. }
        )));
    }

    #[test]
    fn explore_run_details_toggle_chip_works_in_browse_and_edit_50_50() {
        let root = std::env::temp_dir().join("teshi-run-details-toggle-chip");
        let mut app = clickable_detail_app(&root);
        app.explore_focus = ColumnFocus::Details;
        let mut terminal = Terminal::new(TestBackend::new(160, 32)).expect("terminal");
        let toggle_chip_rect = |app: &App| {
            app.clickable_regions
                .iter()
                .find_map(|region| match region {
                    ClickableRegion::ActionButton {
                        action: Action::ExploreToggleDetails,
                        rect,
                        ..
                    } => Some(*rect),
                    _ => None,
                })
                .expect("ExploreToggleDetails chip")
        };

        // Browse expanded: Hide [d] chip collapses Run Details and returns focus to Step.
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render browse expanded");
        let hide_chip = toggle_chip_rect(&app);
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            hide_chip.x + 1,
            hide_chip.y,
            KeyModifiers::NONE,
        )
        .expect("click Hide [d]");
        assert!(app.explore_details_collapsed);
        assert_eq!(app.explore_focus, ColumnFocus::Step);

        // Browse collapsed: no Run Details pane targets remain; Details [d] chip re-expands.
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render browse collapsed");
        assert!(
            !app.clickable_regions
                .iter()
                .any(|region| matches!(region, ClickableRegion::ExploreDetailPane { .. }))
        );
        let show_chip = toggle_chip_rect(&app);
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            show_chip.x + 1,
            show_chip.y,
            KeyModifiers::NONE,
        )
        .expect("click Details [d]");
        assert!(!app.explore_details_collapsed);

        // Edit mode: 50%/50% split when expanded, 100% editor width when collapsed.
        app.explore_edit_mode = true;
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render edit 50/50");
        let split_editor_rect = app.editor_panel_rect.expect("editor rect in 50/50");
        assert!(
            (74..=82).contains(&split_editor_rect.width),
            "expected ~50% editor width on 160-col terminal, got {}",
            split_editor_rect.width
        );

        let edit_hide_chip = toggle_chip_rect(&app);
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            edit_hide_chip.x + 1,
            edit_hide_chip.y,
            KeyModifiers::NONE,
        )
        .expect("click edit Hide");
        assert!(app.explore_details_collapsed);

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render edit collapsed");
        let full_editor_rect = app.editor_panel_rect.expect("editor rect when collapsed");
        assert!(
            full_editor_rect.width >= 150,
            "expected full-width editor when collapsed, got {}",
            full_editor_rect.width
        );

        let edit_show_chip = toggle_chip_rect(&app);
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            edit_show_chip.x + 1,
            edit_show_chip.y,
            KeyModifiers::NONE,
        )
        .expect("click edit Details");
        assert!(!app.explore_details_collapsed);
    }

    #[test]
    fn run_details_render_header_screenshot_and_no_protocol_fallback() {
        let root = std::env::temp_dir().join("teshi-run-details-ui");
        let mut app = detail_render_app(&root);
        let detail = CaseDetail {
            case_id: "f0:s1".into(),
            status: RunStatus::Passed,
            duration_ms: Some(1240),
            message: None,
            stack: None,
            attachments: vec![crate::runner::RunAttachment {
                kind: "screenshot".into(),
                path: root
                    .join(".teshi/logs/replay-screenshots/run/after-login.png")
                    .display()
                    .to_string(),
                step_id: Some("f0:s1".into()),
            }],
            logs: Vec::new(),
            exchanges: Vec::new(),
            steps: vec![crate::app::StepRunRecord {
                step_id: "f0:s1".into(),
                text: "status is 200".into(),
                status: Some("passed".into()),
                message: None,
            }],
            inspector_plaintext: false,
            started_at: None,
            screenshots: Vec::new(),
            historical: false,
        };
        app.explore_case_details.insert((0, 0), detail);
        app.explore_details_tab = crate::app::RunDetailTab::Screenshots;

        let screen = draw_detail_screen(&mut app, 160, 32);
        for expected in [
            "Run Details",
            "status is 200",
            "PASSED",
            "Screenshots 1",
            "after-login.png",
            "preview unavailable",
            ".teshi/logs/replay-screenshots/run/after-login.png",
        ] {
            assert!(screen.contains(expected), "missing {expected:?}: {screen}");
        }
        // Scenario name may still appear in the Scenarios pane; the step index
        // and case duration left the Run Details header.
        assert!(
            !screen.contains("#2"),
            "step index left the header: {screen}"
        );
        assert!(
            !screen.contains("1.24s"),
            "case duration left the header: {screen}"
        );
        assert!(!screen.contains("Failure Details"), "{screen}");
    }

    #[test]
    fn run_details_failed_http_rows_are_red_and_expand_on_space() {
        let root = std::env::temp_dir().join("teshi-run-details-api");
        let mut app = detail_render_app(&root);
        let exchange = teshi_core::HttpExchange::from_value(&serde_json::json!({
            "type": "http_exchange",
            "exchange_id": "e1",
            "case_id": "f0:s1",
            "step_id": "f0:s1:L4",
            "template": "login.json.j2",
            "method": "POST",
            "url": "/api/login",
            "request_headers": {"Accept": "application/json"},
            "request_body": {"password": "***", "name": "Ada"},
            "status": 401,
            "response_headers": {"content-type": "application/json"},
            "response_body": {"error": "bad credentials"},
            "duration_ms": 128,
            "extract": {},
            "asserts": [{"name": "status_ok", "passed": false}],
            "redacted": true
        }))
        .expect("exchange");
        let detail = CaseDetail {
            case_id: "f0:s1".into(),
            status: RunStatus::Failed,
            duration_ms: Some(410),
            message: Some("expected 200, got 401".into()),
            stack: None,
            attachments: Vec::new(),
            logs: Vec::new(),
            exchanges: vec![exchange],
            steps: vec![crate::app::StepRunRecord {
                step_id: "f0:s1:L4".into(),
                text: "status is 200".into(),
                status: Some("failed".into()),
                message: Some("expected 200, got 401".into()),
            }],
            inspector_plaintext: false,
            started_at: None,
            screenshots: Vec::new(),
            historical: false,
        };
        app.explore_case_details.insert((0, 0), detail);
        app.explore_details_tab = crate::app::RunDetailTab::Screenshots;

        let mut terminal = Terminal::new(TestBackend::new(160, 32)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for expected in ["FAILED", "API", "401", "assertion failed", "not loaded"] {
            assert!(screen.contains(expected), "missing {expected:?}: {screen}");
        }
        assert!(
            !screen.contains("Ada"),
            "HTTP body stayed lazy before Space: {screen}"
        );
        let (x, y) = cell_at_text(&terminal, "assertion failed").expect("assertion row");
        assert_eq!(
            terminal.backend().buffer()[(x, y)].fg,
            crate::theme::ERROR,
            "the assertion row is red"
        );

        app.explore_details_selection = 0;
        app.explore_detail_toggle();
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render expanded");
        let expanded: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(expanded.contains("Ada"), "{expanded}");
        assert!(expanded.contains("***"), "{expanded}");
    }

    #[test]
    fn run_detail_http_rows_keep_redacted_values_and_lazy_bodies() {
        let exchange = teshi_core::HttpExchange::from_value(&serde_json::json!({
            "type": "http_exchange",
            "exchange_id": "e1",
            "template": "create_user.json.j2",
            "method": "POST",
            "url": "https://example.test/users",
            "request_headers": {"Authorization": "***"},
            "request_body": {"password": "***", "name": "Ada"},
            "status": 201,
            "response_headers": {},
            "response_body": {"id": "42"},
            "duration_ms": 8,
            "extract": {},
            "asserts": [{"name": "status_ok", "passed": true}],
            "redacted": true
        }))
        .expect("parse exchange");
        let detail = CaseDetail {
            case_id: "f0:s0".into(),
            status: RunStatus::Passed,
            duration_ms: Some(8),
            message: None,
            stack: None,
            attachments: Vec::new(),
            logs: Vec::new(),
            exchanges: vec![exchange.clone()],
            steps: Vec::new(),
            inspector_plaintext: false,
            started_at: None,
            screenshots: Vec::new(),
            historical: false,
        };
        let mut app = App::from_args().expect("app init");
        let mut lines = Vec::new();
        push_exchange_rows(
            &mut lines,
            &app,
            std::slice::from_ref(&exchange),
            0,
            Some(&detail),
            80,
        );
        let collapsed = lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(collapsed.contains("not loaded"), "{collapsed}");
        assert!(!collapsed.contains("Ada"), "body stayed lazy: {collapsed}");

        app.explore_details_expanded.insert("e1".into());
        let mut expanded = Vec::new();
        push_exchange_rows(
            &mut expanded,
            &app,
            std::slice::from_ref(&exchange),
            0,
            Some(&detail),
            80,
        );
        let expanded = expanded
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(expanded.contains("***"));
        assert!(!expanded.to_lowercase().contains("bearer"));
        assert!(expanded.contains("Ada"), "{expanded}");
    }

    #[test]
    fn explore_background_label_matches_feature_language() {
        use teshi_core::gherkin_lang::{GherkinLanguages, StructuralType};

        let zh = GherkinLanguages::global().get("zh-CN");
        assert_eq!(zh.primary_structural(StructuralType::Background), "背景:");
        let en = GherkinLanguages::global().get("en");
        assert_eq!(
            en.primary_structural(StructuralType::Background),
            "Background:"
        );
    }

    #[test]
    fn test_explore_scenarios_title_shows_selected_feature_scenario_count() {
        let mut app = App::from_args().expect("app init should work");
        let feature = gherkin::parse_feature(
            "Feature: A\n  Scenario: S1\n    Given a\n  Scenario: S2\n    Given b\n",
            PathBuf::from("a.feature"),
        );
        app.project.features = vec![feature];
        app.explore_selected_feature = 0;
        assert_eq!(explore_scenarios_title(&app), "Scenarios (2)");
    }

    #[test]
    fn explore_scenarios_title_carries_the_last_run_summary() {
        let mut app = App::from_args().expect("app init should work");
        let feature = gherkin::parse_feature(
            "Feature: A\n  Scenario: S1\n    Given a\n  Scenario: S2\n    Given b\n",
            PathBuf::from("a.feature"),
        );
        app.project.features = vec![feature];
        app.explore_selected_feature = 0;
        assert_eq!(explore_run_summary_label(&app), "0/2 passed");
        app.explore_run_summary = Some(crate::app::RunSummary {
            total: 2,
            passed: 1,
            failed: 1,
            ..Default::default()
        });
        assert_eq!(explore_run_summary_label(&app), "1/2 passed, 1 failed");
        app.explore_run_summary = Some(crate::app::RunSummary {
            total: 2,
            passed: 1,
            failed: 1,
            blocked: 1,
            ..Default::default()
        });
        assert_eq!(
            explore_run_summary_label(&app),
            "1/2 passed, 1 failed, 1 blocked"
        );
    }

    #[test]
    fn test_explore_scenarios_title_counts_rule_nested_scenarios() {
        let mut app = App::from_args().expect("app init should work");
        let feature = gherkin::parse_feature(
            "\
Feature: Rule only
  Rule: R1
    Scenario: Nested
      Given a
",
            PathBuf::from("rule.feature"),
        );
        assert!(feature.scenarios.is_empty());
        assert_eq!(feature.scenario_count(), 1);
        app.project.features = vec![feature];
        app.explore_selected_feature = 0;
        assert_eq!(explore_scenarios_title(&app), "Scenarios (1)");
    }

    #[test]
    fn explore_large_tree_renders_only_visible_rows_and_correct_hit_targets() {
        let mut app = App::from_args().unwrap();
        app.project.features = (0..10_000)
            .map(|i| {
                gherkin::parse_feature(
                    "Feature: Example",
                    PathBuf::from(format!("file{i:05}.feature")),
                )
            })
            .collect();
        app.git_status = Default::default();
        app.explore_selected_feature = 9_999;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal
            .draw(|frame| super::render_explore_panel(frame, &mut app, frame.area()))
            .unwrap();
        let view = &app.explore_viewports[crate::app::ColumnFocus::Feature.index()];
        let offset = view.offset;
        assert!(offset > 0);
        assert_eq!(view.max_vertical, 10_000 - view.rect.height as usize);
        let targets: Vec<_> = app
            .clickable_regions
            .iter()
            .filter_map(|r| match r {
                ClickableRegion::ExploreFeature {
                    feature_idx, row_y, ..
                } => Some((*feature_idx, *row_y)),
                _ => None,
            })
            .collect();
        assert_eq!(targets.len(), view.rect.height as usize);
        assert_eq!(targets[0], (offset, view.rect.y));
        assert_eq!(targets.last().unwrap().0, 9_999);
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("file09999"));
        assert!(!screen.contains("file00000"));
    }

    #[test]
    fn explore_feature_list_renders_modified_git_marker() {
        let old = "Feature: Backup\n  Scenario: Restore\n    Given a backup exists\n";
        let new = "Feature: Backup\n  Scenario: Restore\n    Given a snapshot exists\n";
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("backup.feature");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Feature;

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("M  backup"), "screen was: {screen}");
        assert!(!screen.contains("backup.feature"), "screen was: {screen}");
    }

    #[test]
    fn pointer_hover_keeps_the_selected_feature_row_readable() {
        let mut app = App::from_args().expect("app init");
        app.active_tab = crate::app::MainTab::Explore;
        app.explore_focus = crate::app::ColumnFocus::Feature;
        app.project.features = vec![
            gherkin::parse_feature(
                "Feature: Alpha\n  Scenario: A\n    Given a step\n",
                PathBuf::from("alpha.feature"),
            ),
            gherkin::parse_feature(
                "Feature: Beta\n  Scenario: B\n    Given a step\n",
                PathBuf::from("beta.feature"),
            ),
        ];

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let rows = app
            .clickable_regions
            .iter()
            .filter_map(|region| match region {
                ClickableRegion::ExploreFeature {
                    feature_idx,
                    row_y,
                    col_x,
                    col_right,
                } => Some((*feature_idx, *row_y, *col_x, *col_right)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let (_, selected_y, selected_x, _) = rows[0];
        let (_, other_y, other_x, _) = rows[1];

        app.mouse_position = Some((selected_x + 1, selected_y));
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let (x, y) = cell_at_text(&terminal, "alpha").expect("selected row");
        assert_eq!(
            terminal.backend().buffer()[(x, y)].bg,
            crate::theme::hover_fill(crate::theme::CURSOR_BG)
        );
        assert_ne!(
            terminal.backend().buffer()[(x, y)].bg,
            crate::theme::CURSOR_BG
        );
        assert_eq!(
            terminal.backend().buffer()[(x, y)].fg,
            crate::theme::CURSOR_FG
        );

        app.mouse_position = Some((other_x + 1, other_y));
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let (x, y) = cell_at_text(&terminal, "beta").expect("hovered row");
        assert_eq!(
            terminal.backend().buffer()[(x, y)].bg,
            crate::theme::HOVER_BG
        );

        app.mouse_position = None;
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let (x, y) = cell_at_text(&terminal, "alpha").expect("selected row");
        assert_eq!(
            terminal.backend().buffer()[(x, y)].bg,
            crate::theme::CURSOR_BG
        );
    }

    #[test]
    fn explore_feature_list_marks_rows_whose_ast_is_not_loaded() {
        let root = tempfile::tempdir().expect("tempdir");
        let loaded = root.path().join("loaded.feature");
        let other = root.path().join("other.feature");
        let source = "Feature: Loaded\n  Scenario: S\n    Given a step\n";
        std::fs::write(&loaded, source).expect("write Feature");
        std::fs::write(&other, "Feature: Other\n  Scenario: S\n    Given a step\n")
            .expect("write Feature");
        let mut app = App::from_args().expect("app init");
        app.active_tab = crate::app::MainTab::Explore;
        app.explore_focus = crate::app::ColumnFocus::Feature;
        app.project.root_dir = root.path().to_path_buf();
        app.project.features = vec![gherkin::parse_feature(source, loaded.clone())];
        let mut lazy = crate::lazy_features::LazyFeatures::new(root.path().to_path_buf(), true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !lazy.ready {
            assert!(std::time::Instant::now() < deadline, "catalog scan stalled");
            lazy.poll_catalog();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        lazy.selected = Some(loaded);
        app.lazy_features = Some(lazy);
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            repository_root: Some(root.path().to_path_buf()),
            statuses: std::collections::HashMap::from([(
                "other.feature".into(),
                teshi_core::git::FileGitStatus::Modified,
            )]),
            ..Default::default()
        };

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("M  other"), "screen was: {screen}");
        assert!(screen.contains("  loaded"), "screen was: {screen}");
    }

    #[test]
    fn requirements_editor_scrollbars_reveal_overflow_and_disappear_when_content_fits() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.editor_mode = crate::authoring_tab::RequirementsEditorMode::Browse;
        ui.buffer = crate::editor_buffer::EditorBuffer::from_string(
            (0..20)
                .map(|_| "abcdefghijklmnopqrstuvwxyz")
                .collect::<Vec<_>>()
                .join("\n"),
        );
        ui.horizontal_scroll[1] = 10;
        ui.cursor_row = 19;
        let mut terminal = Terminal::new(TestBackend::new(12, 6)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        // The in-pane action strip reserves one content row.
        assert_eq!(ui.scroll_limits[1], [0, 57]);
        assert_eq!(ui.scroll_row, 55);
        assert_eq!(terminal.backend().buffer()[(1, 1)].symbol(), "j");
        assert!(
            (1..5).any(|y| terminal.backend().buffer()[(11, y)].bg == crate::theme::SCROLL_THUMB),
            "the vertical bar paints a solid thumb"
        );
        ui.buffer = crate::editor_buffer::EditorBuffer::from_string("short".into());
        ui.cursor_row = 0;
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert_eq!(ui.scroll_limits[1], [0, 0]);
        assert_eq!(ui.horizontal_scroll[1], 0);
        assert_eq!(ui.scroll_row, 0);
        assert_eq!(
            ui.pane_bars[1],
            [None, None],
            "a pane that fits leaves no stale hitbox"
        );
    }

    #[test]
    fn requirements_preview_renders_markdown_and_source_view_shows_raw_text() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.buffer = crate::editor_buffer::EditorBuffer::from_string(
            "# Title\n\nSome **bold** text\n\n![shot](missing.png)".into(),
        );
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
        let mut screen = |ui: &mut crate::authoring_tab::AuthoringUiState| {
            terminal
                .draw(|frame| {
                    super::render_requirements_editor(
                        frame,
                        ui,
                        &mut Vec::new(),
                        None,
                        frame.area(),
                        true,
                    )
                })
                .expect("render");
            let buffer = terminal.backend().buffer();
            (0..12)
                .map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let preview = screen(&mut ui);
        assert!(preview.contains("[PREVIEW]"), "{preview}");
        assert!(
            preview.contains("Title") && !preview.contains("# Title"),
            "{preview}"
        );
        assert!(preview.contains("Some bold text"), "{preview}");
        assert!(!preview.contains("**"), "{preview}");
        assert!(
            preview.contains("Image not found: missing.png"),
            "{preview}"
        );

        ui.editor_mode = crate::authoring_tab::RequirementsEditorMode::Browse;
        let source = screen(&mut ui);
        assert!(source.contains("[BROWSE]"), "{source}");
        assert!(source.contains("# Title"), "{source}");
        assert!(source.contains("Some **bold** text"), "{source}");
        assert!(
            !source.contains("Image not found"),
            "the source view does not render images: {source}"
        );
    }

    #[test]
    fn test_requirements_editor_shows_document_title_only_in_body() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.create_document("req.md", "Login flow").unwrap();
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).expect("terminal");
        for mode in [
            crate::authoring_tab::RequirementsEditorMode::Insert,
            crate::authoring_tab::RequirementsEditorMode::Preview,
        ] {
            ui.editor_mode = mode;
            terminal
                .draw(|frame| {
                    super::render_requirements_editor(
                        frame,
                        &mut ui,
                        &mut Vec::new(),
                        None,
                        frame.area(),
                        true,
                    )
                })
                .expect("render");
            let buffer = terminal.backend().buffer();
            let rows = (0..8)
                .map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>();
            let screen = rows.join("\n");
            assert!(rows[0].contains("Requirement *"), "{screen}");
            assert_eq!(
                screen.matches("Login flow").count(),
                1,
                "the H1 title must not be repeated in the panel border: {screen}"
            );
        }
    }

    #[test]
    fn requirement_bars_highlight_on_hover_and_while_grabbed() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.buffer = crate::editor_buffer::EditorBuffer::from_string(
            (0..40)
                .map(|_| "abcdefghijklmnopqrstuvwxyz")
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let mut terminal = Terminal::new(TestBackend::new(12, 6)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        let bar = ui.pane_bars[1][1].expect("overflowing editor draws a bar");
        let cell = (bar.rect.x, bar.rect.y);
        assert_eq!(
            terminal.backend().buffer()[cell].bg,
            crate::theme::SCROLL_THUMB
        );

        ui.pointer = Some(cell);
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert_eq!(
            terminal.backend().buffer()[cell].bg,
            crate::theme::SCROLL_THUMB_HOVER,
            "hovering the bar lifts its thumb"
        );

        ui.scrollbar_grab = Some((1, 1, 0));
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert_eq!(
            terminal.backend().buffer()[cell].bg,
            crate::theme::SCROLL_THUMB_ACTIVE,
            "a grabbed thumb is brighter than a hovered one"
        );
    }

    #[test]
    fn requirements_editor_wraps_unicode_and_tracks_cursor_after_resize() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.editor_mode = crate::authoring_tab::RequirementsEditorMode::Browse;
        let original = "ab中文cdefghijklmnop";
        ui.buffer = crate::editor_buffer::EditorBuffer::from_string(original.into());
        ui.cursor_col = original.chars().count();
        // Six content columns: the interior keeps one column clear of the scrollbar.
        let mut terminal = Terminal::new(TestBackend::new(9, 4)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert_eq!(ui.scroll_limits[1], [0, 3]);
        assert_eq!(ui.scroll_row, 3);
        ui.set_pane_scroll(1, 1, 0);
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert_eq!(ui.scroll_row, 0);
        assert_eq!(ui.cursor_col, original.chars().count());
        assert_eq!(ui.editor_position_at(0, 1), Some((0, 4)));
        assert_eq!(ui.editor_position_at(3, 0), Some((0, 2)));
        assert_eq!(ui.editor_position_at(4, 0), Some((0, 3)));
        // With the action strip, row 1 is the only content row above it.
        let first_row: String = (1..8)
            .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
            .collect();
        // Wide glyphs paint a spacer cell, so compare the leading glyphs only.
        assert!(
            first_row.starts_with("ab中") && first_row.contains('文'),
            "{first_row:?}"
        );
        let mut narrow = Terminal::new(TestBackend::new(6, 4)).expect("terminal");
        narrow
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert!(ui.scroll_row > 0);
        assert_eq!(ui.scroll_limits[1][0], 0);
        assert_eq!(ui.buffer.as_string(), original);
    }

    #[test]
    fn requirements_linked_scrollbars_follow_selection_and_reveal_long_titles() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.create_document("req.md", "A very long requirement title")
            .unwrap();
        ui.artifacts.as_mut().unwrap().index.store_id =
            Some(teshi_core::authoring::RequirementStoreId::parse("reqstore-test").unwrap());
        ui.selection_anchor = Some((0, 0));
        ui.selection_end = Some((0, 25));
        for _ in 0..10 {
            ui.create_test_point_from_selection().expect("test point");
        }
        ui.selection_anchor = None;
        ui.selection_end = None;
        ui.horizontal_scroll[2] = 5;
        let mut terminal = Terminal::new(TestBackend::new(12, 6)).expect("terminal");
        terminal
            .draw(|frame| super::render_requirements_linked(frame, &mut ui, frame.area(), true))
            .expect("render");
        assert!(ui.scroll_limits[2][0] > 0);
        assert_eq!(ui.scroll_limits[2][1], 6);
        assert_eq!(ui.linked_scroll, 6);
        ui.selected_linked_index = 0;
        terminal
            .draw(|frame| super::render_requirements_linked(frame, &mut ui, frame.area(), true))
            .expect("render");
        assert_eq!(ui.linked_scroll, 0);
    }

    fn long_label_tree(ui: &mut crate::authoring_tab::AuthoringUiState, count: usize) {
        ui.discovered = true;
        ui.tree_items.clear();
        ui.tree_labels.clear();
        for i in 0..count {
            let id = format!("req-iter:long-iteration-{i}");
            let label = format!("long-iteration-{i}");
            ui.tree_labels.insert(id.clone(), Line::from(label.clone()));
            ui.tree_items
                .push(tui_tree_widget::TreeItem::new_leaf(id, label));
        }
    }

    fn tree_screen_rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn requirements_tree_wraps_labels_instead_of_scrolling_horizontally() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        long_label_tree(&mut ui, 10);
        ui.horizontal_scroll[0] = 5;
        // Eight content columns leave six for the label after the folder marker.
        let mut terminal = Terminal::new(TestBackend::new(12, 6)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_tree(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert_eq!(ui.scroll_limits[0][0], 0, "wrapped labels never overflow");
        assert_eq!(ui.horizontal_scroll[0], 0);
        assert!(ui.pane_bars[0][0].is_none());
        // Ten three-row (capped) items: the action strip shortens the viewport,
        // and the furthest start still admits the final item.
        assert_eq!(ui.scroll_limits[0][1], 9);
        let rows = tree_screen_rows(&terminal);
        assert!(rows[1].contains("long-i"), "{rows:#?}");
        assert!(rows[2].contains("terati"), "{rows:#?}");
        ui.tree_items.clear();
        terminal
            .draw(|frame| {
                super::render_requirements_tree(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        assert_eq!(ui.scroll_limits[0], [0, 0]);
    }

    #[test]
    fn requirements_tree_selected_row_shows_whole_label() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        long_label_tree(&mut ui, 2);
        ui.tree_state
            .select(vec!["req-iter:long-iteration-0".to_string()]);
        // Extra row keeps the action strip from covering the second tree item.
        let mut terminal = Terminal::new(TestBackend::new(12, 9)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_tree(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        let rows = tree_screen_rows(&terminal);
        assert!(rows[1].contains("long-i"), "{rows:#?}");
        assert!(rows[2].contains("terati"), "{rows:#?}");
        assert!(rows[3].contains("on-0"), "{rows:#?}");
        assert!(rows[4].contains("long-i"), "next item follows: {rows:#?}");
        assert!(!rows.concat().contains("eration-0…"));
        // A click on a continuation row selects the wrapped item it belongs to.
        assert_eq!(
            ui.tree_state
                .rendered_at(ratatui::layout::Position::new(4, 3))
                .map(<[String]>::to_vec),
            Some(vec!["req-iter:long-iteration-0".to_string()])
        );
    }

    #[test]
    fn requirements_tree_keeps_the_status_badge_with_the_title() {
        // doc-101: a CJK title carries no interior whitespace, so the wrapper
        // broke right after `✓ ` and drew the badge as a row of its own.
        let title = "创建新的requirement应该自动聚焦到右侧编辑窗口，且是Insert状态";
        let label = Line::from(vec![
            Span::styled("✓ ", ratatui::style::Style::default()),
            Span::raw(title),
        ]);
        let rows: Vec<String> = super::wrap_tree_label(label, 30, 3)
            .lines
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(
            rows[0].starts_with("✓ 创建"),
            "the badge stays on the title row: {rows:#?}"
        );
        assert!(
            rows.iter().all(|row| row.trim() != "✓"),
            "no badge-only row: {rows:#?}"
        );
    }

    #[test]
    fn requirements_tree_marks_each_entry_with_a_left_rail() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        // One single-row entry, then a wrapped one, then another single-row one.
        ui.discovered = true;
        for (id, label) in [
            ("req-iter:a", "short"),
            ("req-iter:b", "long-iteration-that-wraps-over-three-rows"),
            ("req-iter:c", "short"),
        ] {
            ui.tree_labels.insert(id.to_string(), Line::from(label));
            ui.tree_items
                .push(tui_tree_widget::TreeItem::new_leaf(id.to_string(), label));
        }
        let mut terminal = Terminal::new(TestBackend::new(16, 10)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_tree(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        let buf = terminal.backend().buffer();
        // The panel border keeps its own column; the rail sits one column inside it.
        assert_eq!(buf[(0, 0)].symbol(), "╭");
        assert_eq!(buf[(0, 1)].symbol(), "│");
        let interior = 1..buf.area.height.saturating_sub(1);
        let gutter: String = interior.map(|y| buf[(1, y)].symbol()).collect();
        let rows: Vec<String> = (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        // Single-row entries leave the gutter empty; the wrapped entry owns a run
        // that opens with the half stroke and closes with the corner, so the two
        // neighbours are separated by blank cells above and below.
        assert!(
            gutter.starts_with(' '),
            "single-row entries mark nothing: {rows:#?}"
        );
        assert_eq!(
            gutter.trim_end(),
            " ╷│└",
            "the wrapped entry spans three rows: {rows:#?}"
        );
        // Entries do not fuse: the closing corner is followed by a blank cell.
        let after_close: Vec<char> = gutter.chars().skip_while(|c| *c != '└').skip(1).collect();
        assert_eq!(
            after_close.first(),
            Some(&' '),
            "the next entry must not continue the previous rail: {rows:#?}"
        );
    }

    #[test]
    fn requirements_tree_rail_never_closes_an_entry_the_viewport_cuts() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.discovered = true;
        for (id, label) in [
            ("req-iter:a", "short"),
            ("req-iter:b", "wraps-over-three-rows-because-it-is-long"),
            ("req-iter:c", "short"),
        ] {
            ui.tree_labels.insert(id.to_string(), Line::from(label));
            ui.tree_items
                .push(tui_tree_widget::TreeItem::new_leaf(id.to_string(), label));
        }
        // Five interior rows: the second entry survives only as a sliver.
        let mut terminal = Terminal::new(TestBackend::new(16, 5)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_tree(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        let buf = terminal.backend().buffer();
        let interior = 1..buf.area.height.saturating_sub(1);
        let gutter: String = interior.map(|y| buf[(1, y)].symbol()).collect();
        assert!(
            !gutter.contains('└'),
            "a clipped entry keeps its rail open: {gutter:?}"
        );
    }

    #[test]
    fn wrap_tree_label_hangs_cjk_and_truncates_with_ellipsis() {
        let text = super::wrap_tree_label(Line::from("登录 失败 后锁定账户并提示用户"), 6, 2);
        let rows: Vec<String> = text.lines.iter().map(|line| line.to_string()).collect();
        assert_eq!(rows, ["登录 ", "失败…"]);
        let whole = super::wrap_tree_label(Line::from("short"), 6, 2);
        assert_eq!(whole.lines.len(), 1);
    }

    #[test]
    fn last_fitting_start_counts_item_heights() {
        let cases: [(&[usize], usize, usize); 4] = [
            (&[], 4, 0),
            (&[1, 1, 1], 4, 0),
            (&[2, 2, 2, 2], 4, 2),
            (&[1, 1, 5], 4, 2),
        ];
        for (heights, rows, expected) in cases {
            assert_eq!(
                super::last_fitting_start(heights, rows),
                expected,
                "heights {heights:?} in {rows} rows"
            );
        }
    }

    #[test]
    fn requirement_tree_keeps_a_column_clear_of_the_vertical_scrollbar() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.discovered = true;
        ui.tree_items = (0..12)
            .map(|i| {
                tui_tree_widget::TreeItem::new_leaf(format!("req-iter:row-{i}"), "x".repeat(40))
            })
            .collect();
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_tree(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        let bar = ui.pane_bars[0][1].expect("overflowing tree draws a vertical bar");
        for y in bar.rect.y..bar.rect.y + bar.rect.height {
            assert_eq!(
                terminal.backend().buffer()[(bar.rect.x - 1, y)].symbol(),
                " ",
                "row {y} runs into the scrollbar"
            );
        }
    }

    #[test]
    fn requirement_search_title_paints_a_block_caret() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.discovered = true;
        ui.search_active = true;
        ui.search_query = crate::authoring_tab::LineInput::new("lo");
        let mut terminal = Terminal::new(TestBackend::new(80, 8)).expect("terminal");
        terminal
            .draw(|frame| {
                super::render_requirements_tree(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    frame.area(),
                    true,
                )
            })
            .expect("render");
        let buffer = terminal.backend().buffer();
        let caret = (0..80)
            .find(|x| buffer[(*x, 0)].bg == crate::theme::CURSOR_BG)
            .expect("an active requirement search draws a caret");
        assert_eq!(
            buffer[(caret, 0)].symbol(),
            " ",
            "the caret trails the query"
        );
    }

    #[test]
    fn explore_search_title_paints_a_block_caret() {
        let mut app = App::from_args().expect("app init");
        app.feature_browser.searching = true;
        app.feature_browser.query = crate::authoring_tab::LineInput::new("lo");
        let mut terminal = Terminal::new(TestBackend::new(80, 8)).expect("terminal");
        terminal
            .draw(|frame| super::render_explore_features(frame, &mut app, frame.area()))
            .expect("render");
        let buffer = terminal.backend().buffer();
        let caret = (0..80)
            .find(|x| buffer[(*x, 0)].bg == crate::theme::CURSOR_BG)
            .expect("an active Explore search draws a caret");
        assert_eq!(
            buffer[(caret, 0)].symbol(),
            " ",
            "the caret trails the query"
        );
    }

    #[test]
    fn explore_feature_overflow_gets_thin_horizontal_scrollbar() {
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("a-very-long-feature-file-name-for-horizontal-scrolling.feature");
        app.project.features = vec![gherkin::parse_feature(
            "Feature: Long name\n  Scenario: Short\n    Given a step\n",
            path,
        )];
        app.explore_horizontal_scroll[crate::app::ColumnFocus::Feature.index()] = usize::MAX;

        let mut terminal = Terminal::new(TestBackend::new(80, 18)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");

        let scrollbar = app
            .clickable_regions
            .iter()
            .find_map(|region| match region {
                ClickableRegion::ExploreHorizontalScrollbar {
                    column,
                    rect,
                    content_width,
                    viewport_width,
                } if *column == crate::app::ColumnFocus::Feature => {
                    Some((*rect, *content_width, *viewport_width))
                }
                _ => None,
            });
        let (rect, content_width, viewport_width) = scrollbar.expect("feature scrollbar");
        assert_eq!(rect.height, 1);
        assert_eq!(
            terminal.backend().buffer()[(rect.x, rect.y)].symbol(),
            "▄",
            "the horizontal bar is a thin half-block row"
        );
        assert!(content_width > viewport_width);
        assert_eq!(
            app.explore_horizontal_scroll[crate::app::ColumnFocus::Feature.index()],
            content_width - viewport_width
        );
    }

    #[test]
    fn deleted_feature_is_selectable_and_renders_git_diff() {
        let old = "Feature: Deleted\n  Scenario: Removed\n    Given old content\n";
        let path = PathBuf::from("deleted.feature");
        let mut app = App::from_args().expect("app init");
        app.project.features.clear();
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            deleted: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Deleted,
                teshi_core::git::GitFileDiff::from_contents(Some(old), None),
            )],
            ..Default::default()
        };
        app.explore_selected_deleted_feature = Some(0);
        app.explore_focus = crate::app::ColumnFocus::Step;

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(screen.contains("D  deleted"), "screen was: {screen}");
        assert!(!screen.contains("deleted.feature"), "screen was: {screen}");
        assert!(
            screen.contains("- Feature: Deleted"),
            "screen was: {screen}"
        );
        assert!(app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ExploreDeletedFeature { deleted_idx: 0, .. }
        )));
    }

    #[test]
    fn current_feature_renders_head_deleted_scenarios_when_all_are_removed() {
        let old = "Feature: Current\n  Scenario: Removed\n    Given old content\n";
        let new = "Feature: Current\n";
        let path = PathBuf::from("current.feature");
        let mut app = App::from_args().expect("app init");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(screen.contains("Scenario: Removed"), "screen was: {screen}");
        assert!(!screen.contains("(no scenarios)"), "screen was: {screen}");
        assert!(app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ExploreScenario {
                scenario_idx: 0,
                ..
            }
        )));
    }

    #[test]
    fn current_feature_head_deleted_scenario_uses_after_current_row_index() {
        let old = "Feature: Current\n  Scenario: Keep\n    Given keep\n  Scenario: Removed\n    Given old content\n";
        let new = "Feature: Current\n  Scenario: Keep\n    Given keep\n";
        let path = PathBuf::from("current.feature");
        let mut app = App::from_args().expect("app init");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_selected_scenario = 1;
        app.explore_focus = crate::app::ColumnFocus::Scenario;

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");

        assert!(app.clickable_regions.iter().any(|region| matches!(
            region,
            ClickableRegion::ExploreScenario {
                scenario_idx: 1,
                ..
            }
        )));
    }

    /// Renders the Explore pane for one Feature and returns the app plus terminal.
    fn explore_screen(feature_text: &str) -> (App, Terminal<TestBackend>) {
        let mut app = App::from_args().expect("app init");
        app.project.features = vec![gherkin::parse_feature(
            feature_text,
            PathBuf::from("table.feature"),
        )];
        app.explore_focus = crate::app::ColumnFocus::Step;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        (app, terminal)
    }

    fn rendered_rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn explore_steps_render_step_data_tables() {
        let (_, terminal) = explore_screen(
            "Feature: Tables\n  Scenario: Send\n    Given a request\n      | field | value |\n      | name  | Ada   |\n    Then it works\n",
        );
        let rows = rendered_rows(&terminal);
        assert!(
            rows.iter().any(|row| row.contains("| field | value |")),
            "table header missing: {rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("| name  | Ada   |")),
            "table body missing: {rows:#?}"
        );
        let (body_x, step_y) = cell_at_text(&terminal, "a request").expect("step body");
        let (table_x, table_y) =
            cell_at_text(&terminal, "| field | value |").expect("table header");
        assert_eq!(table_y, step_y + 1, "the table starts under its step");
        assert_eq!(table_x, body_x, "table cells start at the step body column");
        assert_eq!(
            cell_at_text(&terminal, "Then it works").map(|(_, y)| y),
            Some(step_y + 3),
            "the next step follows the whole table"
        );
    }

    #[test]
    fn explore_steps_data_tables_align_wide_cells_by_display_width() {
        let (_, terminal) = explore_screen(
            "Feature: Tables\n  Scenario: Send\n    Given rows\n      | 名称 | kind |\n      | a    | b    |\n    Then ok\n",
        );
        let rows = rendered_rows(&terminal);
        let header_y = rows
            .iter()
            .position(|row| row.contains('名'))
            .unwrap_or_else(|| panic!("{rows:#?}")) as u16;
        let body_y = rows
            .iter()
            .position(|row| row.contains("| a "))
            .unwrap_or_else(|| panic!("{rows:#?}")) as u16;
        // Measure cell columns directly: a wide glyph occupies two buffer cells,
        // so string widths would count it twice.
        let bar_columns = |y: u16| {
            let buffer = terminal.backend().buffer();
            (0..buffer.area.width)
                .filter(|x| buffer[(*x, y)].symbol() == "|")
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bar_columns(header_y),
            bar_columns(body_y),
            "wide cells must not shift later columns:\n{}\n{}",
            rows[header_y as usize],
            rows[body_y as usize]
        );
    }

    #[test]
    fn explore_steps_data_table_rows_select_their_step() {
        let (app, terminal) = explore_screen(
            "Feature: Tables\n  Scenario: Send\n    Given a request\n      | field | value |\n    Then it works\n",
        );
        let (_, header_y) = cell_at_text(&terminal, "| field | value |").expect("table header");
        let (_, then_y) = cell_at_text(&terminal, "Then it works").expect("second step");
        let selects = |step_idx: usize, row_y: u16| {
            app.clickable_regions.iter().any(|region| {
                matches!(
                    region,
                    ClickableRegion::ExploreStep { step_idx: s, row_y: y, .. }
                        if *s == step_idx && *y == row_y
                )
            })
        };
        assert!(
            selects(0, header_y),
            "a table row selects the step above it"
        );
        assert!(
            !selects(1, header_y),
            "a table row does not select the following step"
        );
        assert!(selects(1, then_y), "the next step keeps its own hit target");
    }

    #[test]
    fn explore_steps_render_background_step_data_tables() {
        let (_, terminal) = explore_screen(
            "Feature: Tables\n  Background:\n    Given accounts\n      | id | name |\n      | 1  | Ada  |\n  Scenario: Send\n    Then it works\n",
        );
        let rows = rendered_rows(&terminal);
        assert!(
            rows.iter().any(|row| row.contains("| id | name |")),
            "background table missing: {rows:#?}"
        );
    }

    #[test]
    fn explore_steps_render_outline_step_tables_before_examples() {
        let (_, terminal) = explore_screen(
            "Feature: Tables\n  Scenario Outline: Send\n    Given a user <name>\n      | role |\n      | admin |\n    Then ok\n\n    Examples:\n      | name |\n      | Ada  |\n",
        );
        let rows = rendered_rows(&terminal);
        let position = |needle: &str| {
            rows.iter()
                .position(|row| row.contains(needle))
                .unwrap_or_else(|| panic!("missing {needle}: {rows:#?}"))
        };
        assert!(
            position("| role") < position("Examples:"),
            "the step table stays with the step, above Examples: {rows:#?}"
        );
    }

    #[test]
    fn explore_steps_render_deleted_and_added_git_lines() {
        let old =
            "Feature: Backup\n  Scenario: Restore\n    Given a backup\n    And app is closed\n";
        let new = "Feature: Backup\n  Scenario: Restore\n    Given a backup\n    And app runs\n";
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("backup.feature");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Step;

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("-     And app is closed"));
        assert!(screen.contains("+     And app runs"));
    }

    /// Coordinates of the first cell whose row contains `needle` (ASCII only).
    fn cell_at_text(terminal: &Terminal<TestBackend>, needle: &str) -> Option<(u16, u16)> {
        let buffer = terminal.backend().buffer();
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            if let Some(index) = row.find(needle) {
                // Border glyphs are multi-byte, so convert to a cell index.
                return Some((row[..index].chars().count() as u16, y));
            }
        }
        None
    }

    #[test]
    fn changed_steps_keep_gherkin_colors() {
        let old =
            "Feature: Backup\n  Scenario: Restore\n    Given a backup\n    And app is closed\n";
        let new = "Feature: Backup\n  Scenario: Restore\n    Given a backup\n    And app runs\n    And cache is warm\n";
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("backup.feature");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Step;
        // Keep the selection off the rows under test: it repaints the row.
        app.explore_selected_step = app
            .explore_changed_steps()
            .iter()
            .position(|change| change.text.contains("cache"))
            .expect("third changed step");

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");

        // The added Step keeps a Gherkin keyword colour and inherits `Given`,
        // exactly like the browse view.
        let (x, y) = cell_at_text(&terminal, "And app runs").expect("added row");
        let cell = &terminal.backend().buffer()[(x, y)];
        assert_eq!(cell.fg, crate::theme::PRIMARY);

        // Removed Steps have no AST counterpart, so they fall back to the plain
        // continuation colour.
        let (x, y) = cell_at_text(&terminal, "And app is closed").expect("removed row");
        let cell = &terminal.backend().buffer()[(x, y)];
        assert_eq!(cell.fg, crate::theme::TAB_INACTIVE);
    }

    #[test]
    fn diff_rows_paint_no_background() {
        let row = super::decorate_diff_row(
            vec![Span::raw("Given a user exists")],
            teshi_core::git::DiffStatus::Added,
        );
        assert!(row.style.bg.is_none());
        assert!(row.spans.iter().all(|span| span.style.bg.is_none()));
    }

    #[test]
    fn selected_changed_step_uses_the_selection_fill() {
        let old = "Feature: Backup\n  Scenario: Restore\n    Given a backup exists\n";
        let new = "Feature: Backup\n  Scenario: Restore\n    Given a snapshot exists\n";
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("backup.feature");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Step;
        app.explore_selected_step = app
            .explore_changed_steps()
            .iter()
            .position(|change| change.text.contains("snapshot"))
            .expect("added step");

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");

        let (x, y) = cell_at_text(&terminal, "Given a snapshot exists").expect("selected row");
        let cell = &terminal.backend().buffer()[(x, y)];
        assert_eq!(cell.fg, crate::theme::CURSOR_FG);
        assert_eq!(cell.bg, crate::theme::CURSOR_BG);
    }

    #[test]
    fn explore_steps_render_feature_level_diff_without_scenarios() {
        let old = "Feature: Old title\n  Old description\n";
        let new = "Feature: New title\n  New description\n";
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("empty.feature");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Step;

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("Feature (Git diff)"));
        assert!(screen.contains("- Feature: Old title"));
        assert!(screen.contains("+ Feature: New title"));
    }

    #[test]
    fn explore_steps_render_feature_and_scenario_changes_together() {
        let old = "Feature: Old title\n  Scenario: Restore\n    Given a backup exists\n";
        let new = "Feature: New title\n  Scenario: Restore\n    Given a snapshot exists\n";
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("backup.feature");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Step;

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("Steps (Git diff)"));
        assert!(screen.contains("- Feature: Old title"));
        assert!(screen.contains("+ Feature: New title"));
        assert!(screen.contains("-     Given a backup exists"));
        assert!(screen.contains("+     Given a snapshot exists"));
    }

    #[test]
    fn changed_step_scroll_and_pointer_rows_include_feature_header_offset() {
        let old = format!(
            "Feature: Old title\n  Scenario: Steps\n{}",
            (0..20)
                .map(|n| format!("    Given old step {n}\n"))
                .collect::<String>()
        );
        let new = format!(
            "Feature: New title\n  Scenario: Steps\n{}",
            (0..20)
                .map(|n| format!("    Given new step {n}\n"))
                .collect::<String>()
        );
        let path = PathBuf::from("steps.feature");
        let mut app = App::from_args().unwrap();
        app.project.features = vec![gherkin::parse_feature(&new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(&old), Some(&new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Step;
        app.explore_selected_step = app.explore_changed_steps().len() - 1;
        let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let viewport = &app.explore_viewports[crate::app::ColumnFocus::Step.index()];
        assert!(viewport.offset > 0);
        let mut selected_visible = false;
        for region in &app.clickable_regions {
            if let ClickableRegion::ExploreStep {
                step_idx, row_y, ..
            } = region
            {
                assert!(*row_y >= viewport.rect.y && *row_y < viewport.rect.bottom());
                let row: String = (viewport.rect.x..viewport.rect.right())
                    .map(|x| terminal.backend().buffer()[(x, *row_y)].symbol())
                    .collect();
                assert!(
                    row.contains(app.explore_changed_steps()[*step_idx].text.trim()),
                    "{row}"
                );
                selected_visible |= *step_idx == app.explore_selected_step;
            }
        }
        assert!(selected_visible);
    }

    #[test]
    fn explore_steps_render_deleted_scenario_git_lines() {
        let old = "Feature: Backup\n  Scenario: Keep\n    Given a backup exists\n  Scenario: Removed\n    Given the old flow\n";
        let new = "Feature: Backup\n  Scenario: Keep\n    Given a backup exists\n";
        let mut app = App::from_args().expect("app init");
        let path = PathBuf::from("backup.feature");
        app.project.features = vec![gherkin::parse_feature(new, path.clone())];
        app.git_status = teshi_core::git::FeatureGitStatusModel {
            available: true,
            current: vec![teshi_core::git::map_feature_git_diff(
                path,
                teshi_core::git::FileGitStatus::Modified,
                teshi_core::git::GitFileDiff::from_contents(Some(old), Some(new)),
            )],
            ..Default::default()
        };
        app.explore_focus = crate::app::ColumnFocus::Step;

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render");
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("- Scenario: Removed"));
        assert!(screen.contains("-     Given the old flow"));
    }

    #[test]
    fn test_step_line_display_chinese_keyword_alignment() {
        use crate::highlight::{STEP_KEYWORD_COL_WIDTH, leading_whitespace_chars};
        use teshi_core::gherkin_lang::GherkinLanguages;
        use unicode_width::UnicodeWidthChar;

        let zh = GherkinLanguages::global().get("zh-CN");

        // At the same indent, keyword right edges and step body text must align.
        let cases = [
            ("  当 用户登录", "当"),
            ("  那么 显示结果", "那么"),
            ("  并且 其他操作", "并且"),
            ("  而且 其他操作", "而且"),
            ("  但是 退出", "但是"),
            ("  假如 用户存在", "假如"),
        ];

        let mut prev_kw_end: Option<usize> = None;
        let mut prev_body_start: Option<usize> = None;
        for (input, expected_kw) in &cases {
            let (output, pad, pad_start) = super::step_line_display(input, false, zh);
            let leading = leading_whitespace_chars(input);
            let kw_end = leading + STEP_KEYWORD_COL_WIDTH;
            let kw_char_len = expected_kw.chars().count();
            let body_start = leading
                + STEP_KEYWORD_COL_WIDTH
                + output
                    .chars()
                    .skip(pad_start + pad + kw_char_len)
                    .take_while(|c| c.is_whitespace())
                    .map(|c| c.width().unwrap_or(0))
                    .sum::<usize>();
            assert_eq!(
                pad_start, leading,
                "pad_start should be at indent for '{}'",
                expected_kw
            );
            if let Some(prev) = prev_kw_end {
                assert_eq!(
                    kw_end, prev,
                    "Keyword '{}' right edge should be at column {}, got {}; output='{}'",
                    expected_kw, prev, kw_end, output
                );
            }
            if let Some(prev) = prev_body_start {
                assert_eq!(
                    body_start, prev,
                    "Body after '{}' should start at column {}, got {}; output='{}'",
                    expected_kw, prev, body_start, output
                );
            }
            prev_kw_end = Some(kw_end);
            prev_body_start = Some(body_start);
        }
    }

    #[test]
    fn test_highlight_chinese_keyword_alignment() {
        use crate::highlight::{
            STEP_KEYWORD_COL_WIDTH, StepHighlightState, highlight_line_with_state,
            leading_whitespace_chars,
        };
        use teshi_core::gherkin_lang::GherkinLanguages;
        let zh = GherkinLanguages::global().get("zh-CN");
        let lines = [
            "    当 我按下 `e` 进入编辑器",
            "    那么 该行应进入步骤输入模式",
            "    并且 其他操作",
            "    而且 其他操作",
        ];

        let mut prev_body_start: Option<usize> = None;
        for input in &lines {
            let leading = leading_whitespace_chars(input);
            let trimmed = input.trim_start();
            let (kw, _) = zh.match_step_prefix(trimmed).expect("step line");
            let kw_char_len = kw.chars().count();
            let (display, pad, pad_start) = super::step_line_display(input, false, zh);
            let body_start = leading
                + STEP_KEYWORD_COL_WIDTH
                + display
                    .chars()
                    .skip(pad_start + pad + kw_char_len)
                    .take_while(|c| c.is_whitespace())
                    .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
                    .sum::<usize>();

            let mut state = StepHighlightState::default();
            let styled = highlight_line_with_state(input, &mut state, zh);
            let mut col = 0usize;
            let mut measured = None;
            let mut past_gutter = false;
            for span in &styled.spans {
                if span.style.fg.is_some() {
                    past_gutter = true;
                    col += span.width();
                    continue;
                }
                if past_gutter && measured.is_none() && !span.content.trim_start().is_empty() {
                    measured = Some(col);
                    break;
                }
                col += span.width();
            }
            let measured = measured.expect("body span");
            // Highlight reports the rest span start (includes the space before body).
            assert!(
                measured >= body_start.saturating_sub(1) && measured <= body_start,
                "highlight body column mismatch for '{input}': expected ~{body_start}, got {measured}"
            );
            if let Some(prev) = prev_body_start {
                assert_eq!(
                    body_start, prev,
                    "Body column mismatch for '{input}': expected {prev}, got {body_start}"
                );
            } else {
                prev_body_start = Some(body_start);
            }
        }
    }
    #[test]
    fn requirements_visual_line_highlights_every_selected_source_line() {
        let mut ui = crate::authoring_tab::AuthoringUiState::empty();
        ui.buffer = crate::editor_buffer::EditorBuffer::from_string("alpha\nbeta\ngamma".into());
        ui.cursor_row = 1;
        ui.editor_mode = crate::authoring_tab::RequirementsEditorMode::VisualLine;
        ui.selection_anchor = Some((0, 0));
        ui.selection_end = Some((1, 0));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(50, 12)).unwrap();
        terminal
            .draw(|frame| {
                super::render_requirements_editor(
                    frame,
                    &mut ui,
                    &mut Vec::new(),
                    None,
                    frame.area(),
                    true,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        for (text, selected) in [("alpha", true), ("beta", true), ("gamma", false)] {
            let (x, y) = (0..12)
                .find_map(|y| {
                    let row = (0..50).map(|x| buffer[(x, y)].symbol()).collect::<String>();
                    row.find(text)
                        .map(|offset| (row[..offset].chars().count() as u16, y))
                })
                .expect("source line must render");
            for offset in 0..text.len() as u16 {
                assert_eq!(
                    buffer[(x + offset, y)].bg == super::theme::EDITOR_SELECTION_BG,
                    selected,
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn session_panel_shows_titles_metadata_and_scope() {
        let mut app = App::from_args().expect("app init");
        app.session_panel_active = true;
        app.active_tab = crate::app::MainTab::Ai;
        let message = crate::app::AiChatMessage {
            role: crate::app::AiRole::User,
            content: "Fix login flow".into(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            source: None,
            activity: None,
        };
        let mut session =
            crate::session::Session::from_messages(vec![message], Some("gpt-test".into()));
        session.project_root = Some(PathBuf::from("demo-project"));
        app.session_list = vec![session];
        app.session_panel_selection = 0;
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Sessions ·"), "{screen}");
        assert!(screen.contains("Fix login flow"), "{screen}");
        assert!(screen.contains("1 msgs"), "{screen}");
        assert!(screen.contains("demo-project"), "{screen}");
        assert!(screen.contains("gpt-test"), "{screen}");
    }
}

#[cfg(test)]
mod left_pane_width_tests {
    use super::{ai_sidebar_width, explore_panes, left_pane_width};

    #[test]
    fn test_left_pane_follows_shared_band() {
        // 30% of the row, clamped to 42..=60.
        assert_eq!(left_pane_width(91), 42);
        assert_eq!(left_pane_width(140), 42);
        assert_eq!(left_pane_width(200), 60);
        assert_eq!(left_pane_width(240), 60);
    }

    #[test]
    fn test_ai_sidebar_hides_when_chat_would_drop_below_min() {
        assert_eq!(ai_sidebar_width(91), None);
        assert_eq!(ai_sidebar_width(92), Some(42));
        assert_eq!(ai_sidebar_width(140), Some(42));
        assert_eq!(ai_sidebar_width(200), Some(60));
        assert_eq!(ai_sidebar_width(240), Some(60));
    }

    #[test]
    fn test_explore_left_column_matches_shared_band() {
        let area = ratatui::layout::Rect::new(0, 0, 140, 30);
        let [left, _, _, _] = explore_panes(area, 0, 1);
        assert_eq!(left.width, left_pane_width(area.width));
        assert_eq!(
            left.width,
            ai_sidebar_width(area.width).expect("AI sidebar fits")
        );
    }
}

#[cfg(test)]
mod gui_chrome_tests {
    use super::render;
    use crate::app::{App, ClickableRegion, MainTab, VerticalScrollTarget};
    use crate::keymap::Action;
    use crate::theme;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    fn row_text(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }
    use std::path::PathBuf;
    use teshi_core::gherkin;

    #[test]
    fn test_top_bar_paints_active_tab_and_project_hover() {
        for tab in [
            MainTab::Explore,
            MainTab::MindMap,
            MainTab::Ai,
            MainTab::Requirements,
            MainTab::TestPoints,
        ] {
            let mut app = App::from_args().expect("app init");
            app.set_ui_locale(teshi_engine::UiLocalePreference::En);
            app.active_tab = tab;
            let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let tab_rect = app
                .clickable_regions
                .iter()
                .find_map(|r| match r {
                    ClickableRegion::Tab { tab: t, rect } if *t == tab => Some(*rect),
                    _ => None,
                })
                .expect("active tab region");
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(tab_rect.x, tab_rect.y)].fg, theme::tab_accent(tab));
            assert_eq!(buffer[(tab_rect.x, tab_rect.y)].bg, theme::TAB_ACTIVE_BG);
            assert_eq!(buffer[(0, 1)].symbol(), "╭");

            let project_rect = app
                .clickable_regions
                .iter()
                .find_map(|r| match r {
                    ClickableRegion::ProjectControl { rect } => Some(*rect),
                    _ => None,
                })
                .expect("project control region");
            let resting_bg = buffer[(project_rect.x, project_rect.y)].bg;
            app.mouse_position = Some((project_rect.x, project_rect.y));
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let hovered_bg = terminal.backend().buffer()[(project_rect.x, project_rect.y)].bg;
            assert_ne!(
                hovered_bg, resting_bg,
                "hovering the project control lifts its chip fill"
            );
        }
    }

    #[test]
    fn test_footer_displays_mode_badge_and_breadcrumb_across_tabs() {
        for (tab, expected_crumb) in [
            (MainTab::Explore, "Explore ›"),
            (MainTab::MindMap, "MindMap ›"),
            (MainTab::Ai, "AI ›"),
            (MainTab::Requirements, "Requirements ›"),
            (MainTab::TestPoints, "Test Points ›"),
        ] {
            let mut app = App::from_args().expect("app init");
            app.set_ui_locale(teshi_engine::UiLocalePreference::En);
            app.active_tab = tab;
            app.status_message = None;
            let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let row: String = (0..140)
                .map(|x| terminal.backend().buffer()[(x, 23)].symbol())
                .collect();
            assert!(
                row.contains("NORMAL") || row.contains("INPUT") || row.contains("PREVIEW"),
                "{tab:?} footer missing mode badge: {row}"
            );
            assert!(
                row.contains(expected_crumb),
                "{tab:?} footer missing breadcrumb {expected_crumb:?}: {row}"
            );
        }
    }

    #[test]
    fn test_mindmap_and_editor_expose_clickable_chip_bars() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.active_tab = MainTab::MindMap;
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        for expected in [
            Action::TreeToggle,
            Action::ToggleMindMapAiPanel,
            Action::MindMapSendToAi,
            Action::ToggleScenarioDropdown,
        ] {
            assert!(
                app.clickable_regions.iter().any(|r| matches!(
                    r,
                    ClickableRegion::ActionButton { action, .. } if *action == expected
                )),
                "MindMap missing chip button {expected:?}"
            );
        }

        app.active_tab = MainTab::Explore;
        app.explore_edit_mode = true;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        for expected in [
            Action::ClearInputState,
            Action::Save,
            Action::RunBackground,
            Action::ToggleScenarioFold,
        ] {
            assert!(
                app.clickable_regions.iter().any(|r| matches!(
                    r,
                    ClickableRegion::ActionButton { action, .. } if *action == expected
                )),
                "Explore editor missing chip button {expected:?}"
            );
        }
    }

    #[test]
    fn test_explore_vertical_scrollbar_renders_and_pages_on_click() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.project.features = (0..40)
            .map(|i| {
                gherkin::parse_feature(
                    &format!("Feature: Feature {i}\n  Scenario: S{i}\n    Given step {i}\n"),
                    PathBuf::from(format!("f{i}.feature")),
                )
            })
            .collect();
        app.active_tab = MainTab::Explore;
        let mut terminal = Terminal::new(TestBackend::new(120, 16)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let bar = app
            .vertical_scrollbars
            .iter()
            .find_map(|(target, bar)| {
                matches!(target, VerticalScrollTarget::ExploreColumn(_)).then_some(*bar)
            })
            .expect("Explore Features column should register a vertical scrollbar");
        assert_eq!(app.explore_viewports[0].offset, 0);
        // Click near the bottom of the track to page forward.
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            bar.rect.x,
            bar.rect.bottom().saturating_sub(1),
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(
            app.explore_viewports[0].offset > 0,
            "clicking below the thumb pages the Explore column down"
        );
    }

    #[test]
    fn test_explore_directory_click_toggles_expansion_and_search_title_is_clickable() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.project.root_dir = PathBuf::from("/proj");
        app.project.features = vec![gherkin::parse_feature(
            "Feature: Login\n  Scenario: Ok\n    Given ready\n",
            PathBuf::from("/proj/auth/login.feature"),
        )];
        app.active_tab = MainTab::Explore;
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert!(app.feature_directory_collapsed("auth"));

        // Single-click the directory row to expand it immediately.
        let (dir_x, dir_y) = app
            .clickable_regions
            .iter()
            .find_map(|r| match r {
                ClickableRegion::ExploreFeature {
                    feature_idx: 0,
                    row_y,
                    col_x,
                    ..
                } => Some((*col_x, *row_y)),
                _ => None,
            })
            .expect("directory row target");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            dir_x,
            dir_y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(
            !app.feature_directory_collapsed("auth"),
            "single-clicking a Feature directory expands it"
        );

        // Clicking the Features title starts inline search; clicking clear resets it.
        let trigger = app
            .clickable_regions
            .iter()
            .find_map(|r| match r {
                ClickableRegion::ExploreSearchTrigger { rect } => Some(*rect),
                _ => None,
            })
            .expect("Explore search trigger");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            trigger.x,
            trigger.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(app.feature_browser.searching);

        app.feature_browser.query = crate::authoring_tab::LineInput::new("login");
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let confirm = app
            .clickable_regions
            .iter()
            .find_map(|r| match r {
                ClickableRegion::ExploreSearchConfirm { rect } => Some(*rect),
                _ => None,
            })
            .expect("Explore search confirm button while searching");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            confirm.x,
            confirm.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(
            !app.feature_browser.searching,
            "clicking confirm commits search and exits search input mode"
        );
        assert_eq!(app.feature_browser.query.text(), "login");

        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let clear = app
            .clickable_regions
            .iter()
            .find_map(|r| match r {
                ClickableRegion::ExploreSearchClear { rect } => Some(*rect),
                _ => None,
            })
            .expect("Explore search clear button");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            clear.x,
            clear.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(app.feature_browser.query.text().is_empty());
        assert!(!app.feature_browser.searching);
    }

    #[test]
    fn test_explore_scenario_double_click_opens_editor() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.project.features = vec![gherkin::parse_feature(
            "Feature: Checkout\n  Scenario: Pay\n    Given a cart\n",
            PathBuf::from("checkout.feature"),
        )];
        app.active_tab = MainTab::Explore;
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let (sx, sy) = app
            .clickable_regions
            .iter()
            .find_map(|r| match r {
                ClickableRegion::ExploreScenario {
                    scenario_idx: 0,
                    row_y,
                    col_x,
                    ..
                } => Some((*col_x, *row_y)),
                _ => None,
            })
            .expect("scenario row target");
        let left = MouseEventKind::Down(MouseButton::Left);
        app.handle_mouse_event(left, sx, sy, KeyModifiers::NONE)
            .unwrap();
        assert!(!app.explore_edit_mode, "first click selects only");
        app.handle_mouse_event(left, sx, sy, KeyModifiers::NONE)
            .unwrap();
        assert!(app.explore_edit_mode, "second click opens editor");
    }

    #[test]
    fn test_chat_input_click_positions_caret_and_modal_backdrop_dismisses() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.active_tab = MainTab::Ai;
        app.agent_mut().input.set_text("hello world".to_string());
        assert_eq!(app.agent().input.cursor(), 11);
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let (text_rect, _) = app
            .chat_input_text_rect
            .expect("chat input text rect recorded");
        // Click column 2 ('l') inside "hello world".
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            text_rect.x + 2,
            text_rect.y,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert_eq!(
            app.agent().input.cursor(),
            2,
            "clicking inside ChatInput places caret at clicked character"
        );

        // Open Settings and verify clicking the dimmed backdrop closes it.
        app.settings_panel_active = true;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let card = app.modal_card_rect.expect("settings modal card rect");
        assert!(card.x > 0, "backdrop left margin exists");
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            0,
            0,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(
            !app.settings_panel_active,
            "clicking outside Settings card dismisses it"
        );
    }

    #[test]
    fn test_help_guide_modal_topics_language_scroll_and_dismiss() {
        use crossterm::event::{KeyCode, KeyEvent};

        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        app.active_tab = MainTab::MindMap;

        let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let footer = row_text(terminal.backend().buffer(), 31);
        assert!(
            footer.contains("Help [?]"),
            "footer advertises Help [?], got: {footer}"
        );

        // Open via F1 (matches active tab = MindMap -> topic 2).
        assert!(app.handle_help_guide_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
        assert!(app.help_guide.active);
        assert_eq!(app.help_guide.selected_topic, 2);

        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let top = row_text(terminal.backend().buffer(), 2);
        assert!(
            top.contains("Teshi User Guide")
                || row_text(terminal.backend().buffer(), 1).contains("Teshi User Guide"),
            "guide title renders on modal top border"
        );

        // Space switches doc language independently without mutating global locale.
        let before_global = app.ui_locale;
        let before_doc = app.help_guide.doc_locale;
        assert!(app.handle_help_guide_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)));
        assert_ne!(app.help_guide.doc_locale, before_doc);
        assert_eq!(app.ui_locale, before_global);

        // Move topic down with j, then focus doc with l and scroll with Ctrl+D.
        assert!(app.handle_help_guide_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));
        assert_eq!(app.help_guide.selected_topic, 3);
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert!(app.handle_help_guide_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)));
        assert!(app.help_guide.focus_doc);
        assert!(
            app.handle_help_guide_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        );
        assert!(
            app.help_guide.doc_scroll > 0 || app.help_guide.doc_max_scroll == 0,
            "Ctrl+D scrolls guide body when overflow exists"
        );

        // Mouse wheel over doc pane scrolls up back to 0.
        if let Some(doc_rect) = app.help_guide.doc_rect {
            app.handle_mouse_event(
                MouseEventKind::ScrollUp,
                doc_rect.x + 2,
                doc_rect.y + 1,
                KeyModifiers::NONE,
            )
            .unwrap();
        }

        // Clicking dimmed backdrop dismisses the User Guide.
        app.handle_mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            0,
            0,
            KeyModifiers::NONE,
        )
        .unwrap();
        assert!(!app.help_guide.active, "backdrop click closes User Guide");
    }

    #[test]
    fn test_phase3_vim_tab_cycle_half_page_and_ex_commands() {
        let mut app = App::from_args().expect("app init");
        app.set_ui_locale(teshi_engine::UiLocalePreference::En);
        assert_eq!(app.active_tab, MainTab::Explore);

        // gt / gT cycle across all 5 tabs.
        app.handle_action(Action::NextTab).unwrap();
        assert_eq!(app.active_tab, MainTab::MindMap);
        app.handle_action(Action::NextTab).unwrap();
        assert_eq!(app.active_tab, MainTab::Ai);
        app.handle_action(Action::PrevTab).unwrap();
        assert_eq!(app.active_tab, MainTab::MindMap);

        // `:` command line Tab autocomplete + `:help` execution.
        app.handle_action(Action::CommandLineOpen).unwrap();
        for ch in ['h', 'e'] {
            app.handle_action(Action::CommandLineInsert(ch)).unwrap();
        }
        app.handle_action(Action::CommandLineAutocomplete).unwrap();
        assert_eq!(
            app.command_line.as_ref().map(|i| i.text()),
            Some("help"),
            "Tab autocompletes :he -> :help"
        );
        app.handle_action(Action::CommandLineConfirm).unwrap();
        assert!(app.help_guide.active, ":help opens the User Guide");
        app.close_help_guide();

        // `:4` jumps directly to Requirements tab.
        app.handle_action(Action::CommandLineOpen).unwrap();
        app.handle_action(Action::CommandLineInsert('4')).unwrap();
        app.handle_action(Action::CommandLineConfirm).unwrap();
        assert_eq!(app.active_tab, MainTab::Requirements);

        // Half-page actions execute cleanly without panicking.
        app.handle_action(Action::HalfPageDown).unwrap();
        app.handle_action(Action::HalfPageUp).unwrap();
    }
}
