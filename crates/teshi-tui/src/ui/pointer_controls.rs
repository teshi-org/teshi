//! Pointer controls for AI conversations and human review decisions.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{AiStatus, App, ChangeKind, ClickableRegion};
use crate::keymap::Action;
use crate::theme;

/// Draw the AI footer using the same rectangles for text and pointer actions.
pub(super) fn render_ai_footer(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let compact = area.width < 85;
    let first = if app.ai_input_focused {
        (
            if compact { " Send" } else { " Send [Enter]" },
            Action::AiSendMessage,
        )
    } else {
        (
            if compact { " Focus" } else { " Focus [Enter]" },
            Action::AiFocusInput,
        )
    };
    let mut controls = vec![
        first,
        (
            if compact { " Blur" } else { " Blur [Esc]" },
            Action::AiBlurInput,
        ),
        (
            if compact { " Model" } else { " Model [m]" },
            Action::ModelPanelOpen,
        ),
    ];
    if !app.ai_input_focused {
        controls.push((if compact { " Quit" } else { " Quit [q]" }, Action::Quit));
    }
    super::render_action_bar(frame, app, area, "", &controls, false);
    let used = controls
        .iter()
        .map(|(label, _)| label.width() as u16)
        .sum::<u16>();
    if used + 2 < area.width
        && let Some(status) = app.agent().tool_status.as_ref()
    {
        frame.render_widget(
            Paragraph::new(Line::styled(
                status.clone(),
                Style::default().fg(theme::WARNING),
            )),
            Rect::new(area.x + used + 2, area.y, area.width - used - 2, 1),
        );
    }
}

/// Draw a scroll-following thread list and persistent New and Close controls.
pub(super) fn render_agent_sidebar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 5 || area.height < 3 {
        return;
    }
    let block = theme::panel_block(" Agents ", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let rows = inner.height.saturating_sub(1) as usize;
    let start = app.selected_agent.saturating_sub(rows.saturating_sub(1));
    let mut lines = Vec::new();
    for (index, agent) in app.agents.iter().enumerate().skip(start).take(rows) {
        let (dot, color) = match agent.status {
            AiStatus::Waiting => ("●", theme::WARNING),
            AiStatus::AwaitingApproval => ("◆", theme::PRIMARY),
            AiStatus::Error => ("●", theme::ERROR),
            AiStatus::Idle => ("○", theme::SUCCESS),
        };
        let line = Line::from(vec![
            Span::styled(format!(" {dot} "), Style::default().fg(color)),
            Span::raw(agent.title.clone()),
        ]);
        lines.push(if index == app.selected_agent {
            line.style(theme::selected_row(true))
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
        app,
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        "",
        &[
            (" New [a]", Action::AgentNew),
            (" Close [x]", Action::AgentClose),
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
        app,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        "",
        &[
            (" Select", Action::AiSlashSelect),
            (" Dismiss", Action::AiSlashDismiss),
        ],
        false,
    );
}

/// Keep every review decision visible and isolate its pointer ownership.
pub(super) fn render_pending_review(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if !app.has_agent_change_prompt() && !app.has_acp_permission_prompt() {
        return;
    }
    if app.change_summary_visible
        || app.auth_panel_active
        || app.model_panel_active
        || app.session_panel_active
        || app.quit_pending_confirm
        || app.approval_panel_active
        || app.agent_profile_panel_active
        || app.authoring_ui.overlay_active()
        || app.generation_scope_prompt.is_some()
        || app.has_external_change_prompt()
        || app.step_keyword_picker.is_some()
        || app.scenario_dropdown_open
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
            app,
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
            "",
            &[
                (" Accept", Action::AgentChangeAccept),
                (" Reject", Action::AgentChangeReject),
                (" Diff", Action::AgentChangeDiff),
                (" Summary", Action::ToggleChangeSummary),
            ],
            true,
        );
        return;
    }
    let width = area.width.saturating_sub(4).min(100);
    let height = area.height.saturating_sub(4).min(22);
    let panel = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    app.clickable_regions.clear();
    frame.render_widget(Clear, panel);
    let changes = app.has_agent_change_prompt();
    let title = if changes {
        " Pending Agent Change "
    } else {
        " ACP permission requested "
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
        let mut lines = vec![Line::styled(
            change.description.clone(),
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
        )];
        if let Some(diff) = app.pending_change_diffs.first() {
            for line in diff.iter().take(body.height.saturating_sub(1) as usize) {
                let (prefix, color) = match line.kind {
                    ChangeKind::Added => ("+", theme::SUCCESS),
                    ChangeKind::Deleted => ("-", theme::ERROR),
                    ChangeKind::Modified => ("~", theme::WARNING),
                    ChangeKind::Unchanged => (" ", theme::MUTED),
                };
                lines.push(Line::styled(
                    format!("{prefix} {}", line.text),
                    Style::default().fg(color),
                ));
            }
        }
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            body,
        );
        let compact = inner.width < 70;
        super::render_action_bar(
            frame,
            app,
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            "",
            &[
                (
                    if compact { " Accept" } else { " Accept [y]" },
                    Action::AgentChangeAccept,
                ),
                (
                    if compact { " Reject" } else { " Reject [n]" },
                    Action::AgentChangeReject,
                ),
                (
                    if compact { " Diff" } else { " Diff [d]" },
                    Action::AgentChangeDiff,
                ),
                (
                    if compact { " Summary" } else { " Summary [s]" },
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
            app,
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            "",
            &[
                (" Allow [Enter]", Action::AcpPermissionConfirm),
                (" Reject [Esc]", Action::AcpPermissionReject),
            ],
            true,
        );
    }
}
