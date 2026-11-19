//! Pointer controls for AI conversations and human review decisions.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::{AiStatus, App, ChangeKind, ClickableRegion};
use crate::keymap::Action;
use crate::theme;
use unicode_width::UnicodeWidthStr;

/// Draw a scroll-following thread list and persistent New and Close controls.
pub(super) fn render_agent_sidebar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 5 || area.height < 3 {
        return;
    }
    let block = theme::panel_block(crate::i18n::t("chat.chats.title"), app.ai_sidebar_focused)
        .title_style(theme::muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let rows = inner.height.saturating_sub(1) as usize;
    let start = app.selected_agent.saturating_sub(rows.saturating_sub(1));
    let mut lines = Vec::new();
    for (index, agent) in app.agents.iter().enumerate().skip(start).take(rows) {
        // A finished answer nobody has read yet outranks the idle dot: it is
        // the only cue that a conversation other than the visible one is done.
        let (dot, color) = if agent.unseen_completion {
            ("●", theme::SUCCESS)
        } else {
            match agent.status {
                AiStatus::Waiting => ("●", theme::WARNING),
                AiStatus::AwaitingApproval => ("◆", theme::PRIMARY),
                AiStatus::Error => ("●", theme::ERROR),
                AiStatus::Idle => ("○", theme::MUTED),
            }
        };
        let mut spans = vec![
            Span::styled(format!(" {dot} "), Style::default().fg(color)),
            Span::raw(agent.title.clone()),
        ];
        // Agents in another worktree are told apart by their directory name;
        // the full `member @ branch` label is on the composer border.
        if let Some(name) = agent.member_dir.as_deref().and_then(|dir| dir.file_name()) {
            spans.push(Span::styled(
                format!(" {}", name.to_string_lossy()),
                theme::muted(),
            ));
        }
        let line = Line::from(spans);
        lines.push(if index == app.selected_agent {
            line.style(Style::default().bg(theme::CHAT_CHIP_BG).fg(theme::FG))
        } else {
            line
        });
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: Rect::new(inner.x, inner.y + (index - start) as u16, inner.width, 1),
            action: Action::AgentSelect(index),
            modal: false,
        });
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect::new(inner.x, inner.y, inner.width, rows as u16),
    );
    super::render_action_bar(
        frame,
        &mut app.clickable_regions,
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        "",
        &[
            (crate::i18n::t("chat.new"), Action::AgentNew),
            (crate::i18n::t("chat.close"), Action::AgentClose),
        ],
        false,
    );
}

/// Render visible slash rows with a separate explicit command confirmation.
pub(super) fn render_slash_suggestions(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    commands: &[&(&str, &str)],
) {
    if area.is_empty() {
        return;
    }
    let selected = app
        .slash_suggestion_selection
        .min(commands.len().saturating_sub(1));
    let rows = area.height.saturating_sub(1) as usize;
    let start = selected.saturating_sub(rows.saturating_sub(1));
    let mut lines = Vec::new();
    for (index, (name, description)) in commands.iter().enumerate().skip(start).take(rows) {
        let description = crate::i18n::slash_description(name).unwrap_or(description);
        let line = Line::from(format!(" /{name}  {description}"));
        lines.push(if index == selected {
            line.style(theme::selected_row(true))
        } else {
            line
        });
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: Rect::new(area.x, area.y + (index - start) as u16, area.width, 1),
            action: Action::AiSlashChoose(index),
            modal: false,
        });
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect::new(area.x, area.y, area.width, rows as u16),
    );
    super::render_action_bar(
        frame,
        &mut app.clickable_regions,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        "",
        &[
            (crate::i18n::t("action.select"), Action::AiSlashSelect),
            (crate::i18n::t("action.dismiss"), Action::AiSlashDismiss),
        ],
        false,
    );
}

/// Keep every review decision visible and isolate its pointer ownership.
/// Draw the AI footer using the same rectangles for text and pointer actions.
///
/// Labels are action names only. Model stays available from the AI status row
/// and `/models`; the global footer keeps Settings only.
pub(super) fn render_pending_review(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if !app.has_agent_change_prompt() && !app.has_acp_permission_prompt() {
        return;
    }
    // The full-screen image viewer owns the screen; a decision that arrives
    // while it is open waits for it to close.
    if app.screenshot.visible
        || app.change_summary_visible
        || app.auth_panel_active
        || app.model_panel_active
        || app.session_panel_active
        || app.quit_pending_confirm
        || app.ai_interrupt_confirm
        || app.approval_panel_active
        || app.settings_panel_active
        || app.agent_profile_panel_active
        || app.authoring_ui.overlay_active()
        || app.generation_scope_prompt.is_some()
        || app.has_external_change_prompt()
        || app.step_keyword_picker.is_some()
        || app.scenario_dropdown_open
        || !matches!(
            app.project_switch.overlay,
            crate::project_context::SwitchOverlay::None
        )
    {
        return;
    }
    if area.width < 20 || area.height < 5 {
        return;
    }
    if app.has_agent_change_prompt()
        && app.active_tab == crate::app::MainTab::Explore
        && app.explore_diff_lines.is_some()
    {
        app.clickable_regions.clear();
        super::render_action_bar(
            frame,
            &mut app.clickable_regions,
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
            "",
            &[
                (
                    crate::i18n::t("chat.change.accept"),
                    Action::AgentChangeAccept,
                ),
                (
                    crate::i18n::t("chat.change.reject"),
                    Action::AgentChangeReject,
                ),
                (crate::i18n::t("chat.change.diff"), Action::AgentChangeDiff),
                (
                    crate::i18n::t("chat.change.summary"),
                    Action::ToggleChangeSummary,
                ),
            ],
            true,
        );
        return;
    }
    let changes = app.has_agent_change_prompt();
    // iCode's review card: content-driven width up to 100 columns (never below
    // 60, so the decision row keeps every action) and a height that hugs the
    // wrapped diff. One body row is reserved for the overflow hint only when
    // some diff lines do not fit.
    let spec = super::overlay::OverlaySpec::content(60, 100, 8, 22);
    let (width, height) = if changes {
        let change = &app.pending_agent_changes[0];
        let diff = app
            .pending_change_diffs
            .first()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let diff_w = diff
            .iter()
            .map(|line| line.text.width() as u16 + 2)
            .max()
            .unwrap_or(0);
        let title_w = crate::i18n::t("chat.pending.title").width() as u16;
        let desired_w = (change.description.width() as u16).max(diff_w).max(title_w) + 2;
        let width = spec.width.resolve(area.width, desired_w);
        let body_cap = spec.height.resolve(area.height, u16::MAX).saturating_sub(3) as usize;
        let rows = pending_body_rows(
            change_description(change),
            diff,
            width.saturating_sub(2) as usize,
            body_cap,
        );
        (width, spec.height.resolve(area.height, rows as u16 + 3))
    } else {
        let prompt = app.agent().pending_acp_permission.as_ref().unwrap();
        let label_w = prompt
            .options
            .iter()
            .map(|option| option.display_label().width() as u16 + 3)
            .max()
            .unwrap_or(0);
        let title_w = crate::i18n::t("chat.permission.title").width() as u16;
        let width = spec.width.resolve(area.width, label_w.max(title_w) + 2);
        let rows = prompt.options.len() as u16 + 3;
        (width, spec.height.resolve(area.height, rows))
    };
    let panel = super::overlay::centered(area, width, height);
    app.clickable_regions.clear();
    super::clear_to(frame, panel, theme::DIALOG_BG);
    let title = if changes {
        crate::i18n::t("chat.pending.title")
    } else {
        crate::i18n::t("chat.permission.title")
    };
    let block = theme::panel_block(title, true);
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    if changes {
        let change = &app.pending_agent_changes[0];
        let diff = app
            .pending_change_diffs
            .first()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let (lines, _hidden) = pending_body(
            change_description(change),
            diff,
            body.width as usize,
            body.height as usize,
        );
        frame.render_widget(Paragraph::new(Text::from(lines)), body);
        let compact = inner.width < 70;
        super::render_action_bar(
            frame,
            &mut app.clickable_regions,
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            "",
            &[
                (
                    if compact {
                        crate::i18n::t("chat.change.accept")
                    } else {
                        " Accept [y]"
                    },
                    Action::AgentChangeAccept,
                ),
                (
                    if compact {
                        crate::i18n::t("chat.change.reject")
                    } else {
                        " Reject [n]"
                    },
                    Action::AgentChangeReject,
                ),
                (
                    if compact {
                        crate::i18n::t("chat.change.diff")
                    } else {
                        " Diff [d]"
                    },
                    Action::AgentChangeDiff,
                ),
                (
                    if compact {
                        crate::i18n::t("chat.change.summary")
                    } else {
                        " Summary [s]"
                    },
                    Action::ToggleChangeSummary,
                ),
            ],
            true,
        );
    } else {
        let prompt = app.agent().pending_acp_permission.as_ref().unwrap();
        let rows = body.height as usize;
        let start = prompt.selected.saturating_sub(rows.saturating_sub(1));
        let mut lines = Vec::new();
        let selected = prompt.selected;
        let labels: Vec<_> = prompt
            .options
            .iter()
            .map(|option| option.display_label().to_owned())
            .collect();
        for (index, label) in labels.iter().enumerate().skip(start).take(rows) {
            let line = Line::from(format!(
                " {} {label}",
                if index == selected { "▶" } else { " " }
            ));
            lines.push(if index == selected {
                line.style(theme::selected_row(true))
            } else {
                line
            });
            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect: Rect::new(body.x, body.y + (index - start) as u16, body.width, 1),
                action: Action::AcpPermissionChoose(index),
                modal: true,
            });
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), body);
        super::render_action_bar(
            frame,
            &mut app.clickable_regions,
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            "",
            &[
                (
                    crate::i18n::t("chat.permission.allow"),
                    Action::AcpPermissionConfirm,
                ),
                (
                    crate::i18n::t("chat.permission.reject"),
                    Action::AcpPermissionReject,
                ),
            ],
            true,
        );
    }
}

/// The styled description row of the pending-change card.
fn change_description(change: &crate::app::AgentPendingChange) -> Line<'static> {
    Line::styled(
        change.description.clone(),
        Style::default()
            .fg(theme::WARNING)
            .add_modifier(Modifier::BOLD),
    )
}

/// One diff row, with the gutter prefix and colour the card has always used.
fn change_diff_line(line: &teshi_core::diff::DiffLine) -> Line<'static> {
    let (prefix, color) = match line.kind {
        ChangeKind::Added => ("+", theme::SUCCESS),
        ChangeKind::Deleted => ("-", theme::ERROR),
        ChangeKind::Modified => ("~", theme::WARNING),
        ChangeKind::Unchanged => (" ", theme::MUTED),
    };
    Line::styled(
        format!("{prefix} {}", line.text),
        Style::default().fg(color),
    )
}

/// Body rows the pending-change card wants, bounded by `body_cap`.
///
/// Counts wrapped rows, not source lines, so a long step or table row reserves
/// the rows it actually occupies. Adds one row when a hint is needed.
fn pending_body_rows(
    description: Line<'static>,
    diff: &[teshi_core::diff::DiffLine],
    width: usize,
    body_cap: usize,
) -> usize {
    let description_rows = super::chat::wrap_line_to_width(description, width).len();
    let (diff_rows, shown) = diff_fit(diff, width, body_cap.saturating_sub(description_rows));
    description_rows + diff_rows + usize::from(shown < diff.len())
}

/// Wrapped rows and source lines that fit in `rows`.
fn diff_fit(diff: &[teshi_core::diff::DiffLine], width: usize, rows: usize) -> (usize, usize) {
    let mut used = 0usize;
    let mut shown = 0usize;
    for line in diff {
        let wrapped = super::chat::wrap_line_to_width(change_diff_line(line), width);
        if used + wrapped.len() > rows {
            break;
        }
        used += wrapped.len();
        shown += 1;
    }
    (used, shown)
}

/// Wrapped body rows, plus the number of diff lines the body cannot show.
///
/// Fills without a hint first; when the diff does not fit, the last row is
/// reserved for the overflow hint and the diff is filled again to that budget.
fn pending_body(
    description: Line<'static>,
    diff: &[teshi_core::diff::DiffLine],
    width: usize,
    rows: usize,
) -> (Vec<Line<'static>>, usize) {
    let description_lines = super::chat::wrap_line_to_width(description, width);
    let (lines, _used, shown) = fill_pending_body(description_lines.clone(), diff, width, rows);
    if shown == diff.len() {
        return (lines, 0);
    }
    let (mut lines, used, shown) =
        fill_pending_body(description_lines, diff, width, rows.saturating_sub(1));
    let hidden = diff.len() - shown;
    if hidden > 0 && used < rows {
        lines.push(Line::styled(
            crate::i18n::fill(
                crate::i18n::t("chat.change.more_lines"),
                &[("n", &hidden.to_string())],
            ),
            Style::default().fg(theme::MUTED),
        ));
    }
    (lines, hidden)
}

/// Fills `rows` with the wrapped description and as many diff lines as fit.
fn fill_pending_body(
    mut description: Vec<Line<'static>>,
    diff: &[teshi_core::diff::DiffLine],
    width: usize,
    rows: usize,
) -> (Vec<Line<'static>>, usize, usize) {
    if description.len() > rows {
        description.truncate(rows);
    }
    let mut used = description.len();
    let mut shown = 0usize;
    let mut lines = description;
    for line in diff {
        let wrapped = super::chat::wrap_line_to_width(change_diff_line(line), width);
        if used + wrapped.len() > rows {
            break;
        }
        used += wrapped.len();
        shown += 1;
        lines.extend(wrapped);
    }
    (lines, used, shown)
}
