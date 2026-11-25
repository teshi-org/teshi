//! Pointer controls for AI conversations and human review decisions.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::{
    AiStatus, App, ChangeKind, ClickableRegion, SidebarSearchHit, VerticalScrollTarget,
};
use crate::keymap::Action;
use crate::theme;
use unicode_width::UnicodeWidthStr;

/// Maximum Working cards rendered simultaneously before the Working sub-list scrolls internally.
const MAX_WORKING_VISIBLE_CARDS: usize = 4;
/// Maximum Settled cards rendered simultaneously when the Settled drawer is expanded.
const MAX_SETTLED_VISIBLE_CARDS: usize = 3;

#[derive(Clone, Copy)]
enum ShelfKind {
    Working,
    Settled,
}

impl ShelfKind {
    fn toggle_action(self) -> Action {
        match self {
            Self::Working => Action::AiSidebarToggleWorking,
            Self::Settled => Action::AiSidebarToggleSettled,
        }
    }
}

#[derive(Clone)]
enum SidebarRow {
    Spacer,
    CardTop {
        target: CardTarget,
        selected: bool,
        settled: bool,
    },
    CardBottom {
        target: CardTarget,
        selected: bool,
        settled: bool,
    },
    EmptyNotice(&'static str),
}

#[derive(Clone)]
enum CardTarget {
    OpenAgent(usize),
    GlobalSession(Box<crate::session::Session>),
}

impl CardTarget {
    fn key_id(&self) -> String {
        match self {
            Self::OpenAgent(idx) => format!("agent:{idx}"),
            Self::GlobalSession(session) => format!("session:{}", session.id),
        }
    }

    fn click_action(&self) -> Action {
        match self {
            Self::OpenAgent(idx) => Action::AgentSelect(*idx),
            Self::GlobalSession(session) => Action::AiSidebarOpenGlobalSession(session.id.clone()),
        }
    }
}

impl SidebarRow {
    fn card_target(&self) -> Option<&CardTarget> {
        match self {
            Self::CardTop { target, .. } | Self::CardBottom { target, .. } => Some(target),
            Self::Spacer | Self::EmptyNotice(_) => None,
        }
    }
}

fn format_compact_age(at: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(at)
        .unwrap_or_default()
        .as_secs();
    if secs < 60 {
        "<1m".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

fn parse_millis_timestamp(raw: &str) -> SystemTime {
    raw.trim()
        .parse::<u64>()
        .ok()
        .map(|millis| std::time::UNIX_EPOCH + std::time::Duration::from_millis(millis))
        .unwrap_or_else(SystemTime::now)
}

fn build_section_card_rows(
    indices: &[usize],
    selected_agent: usize,
    settled: bool,
) -> (Vec<SidebarRow>, Option<usize>) {
    let mut rows = Vec::new();
    let mut selected_start = None;
    for (pos, &agent_idx) in indices.iter().enumerate() {
        if pos > 0 {
            rows.push(SidebarRow::Spacer);
        }
        let card_start = rows.len();
        let selected = agent_idx == selected_agent;
        if selected {
            selected_start = Some(card_start);
        }
        let target = CardTarget::OpenAgent(agent_idx);
        rows.push(SidebarRow::CardTop {
            target: target.clone(),
            selected,
            settled,
        });
        rows.push(SidebarRow::CardBottom {
            target,
            selected,
            settled,
        });
    }
    (rows, selected_start)
}

fn build_search_sidebar_rows(app: &App) -> (Vec<SidebarRow>, Option<usize>) {
    let hits = app.ai_sidebar_search_matches();
    if hits.is_empty() {
        return (
            vec![SidebarRow::EmptyNotice(crate::i18n::t(
                "chat.sidebar.no_matches",
            ))],
            None,
        );
    }
    let selected_pos = app
        .ai_sidebar_search_selection
        .min(hits.len().saturating_sub(1));
    let mut rows = Vec::new();
    let mut selected_start = None;
    for (pos, hit) in hits.into_iter().enumerate() {
        if pos > 0 {
            rows.push(SidebarRow::Spacer);
        }
        let card_start = rows.len();
        let selected = if app.ai_sidebar_searching {
            pos == selected_pos
        } else {
            matches!(hit, SidebarSearchHit::OpenThread { agent_idx, .. } if agent_idx == app.selected_agent)
        };
        if selected {
            selected_start = Some(card_start);
        }
        let (target, settled) = match hit {
            SidebarSearchHit::OpenThread { agent_idx, .. } => {
                let settled = app
                    .agents
                    .get(agent_idx)
                    .is_some_and(|agent| !agent.is_working() && agent.settled);
                (CardTarget::OpenAgent(agent_idx), settled)
            }
            SidebarSearchHit::GlobalSession { session, .. } => {
                let settled = session.settled;
                (CardTarget::GlobalSession(session), settled)
            }
        };
        rows.push(SidebarRow::CardTop {
            target: target.clone(),
            selected,
            settled,
        });
        rows.push(SidebarRow::CardBottom {
            target,
            selected,
            settled,
        });
    }
    (rows, selected_start)
}

fn render_shelf_header_row(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    label: &'static str,
    count: usize,
    expanded: bool,
    kind: ShelfKind,
) {
    if area.is_empty() {
        return;
    }
    let line = shelf_header_line(label, count, expanded, area.width);
    frame.render_widget(Paragraph::new(line), area);
    app.clickable_regions.push(ClickableRegion::ActionButton {
        rect: area,
        action: kind.toggle_action(),
        modal: false,
    });
}

fn shelf_header_line(
    label: &'static str,
    count: usize,
    expanded: bool,
    width: u16,
) -> Line<'static> {
    let chevron = if expanded { "⌄" } else { ">" };
    let right = format!("{count} {chevron}");
    let content_w = width.saturating_sub(2) as usize;
    let label_w = label.width();
    let right_w = right.width();
    let gap = content_w.saturating_sub(label_w + right_w);
    let label_style = Style::default()
        .fg(theme::MUTED)
        .add_modifier(Modifier::BOLD);
    let right_style = Style::default().fg(theme::MUTED);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(label, label_style),
        Span::raw(" ".repeat(gap)),
        Span::styled(right, right_style),
        Span::raw(" "),
    ])
}

fn sidebar_card_bg(app: &App, selected: bool) -> Style {
    if !selected {
        return Style::default();
    }
    let bg = if app.ai_sidebar_focused {
        theme::TAB_ACTIVE_BG
    } else {
        theme::BLOCK_BG
    };
    Style::default().bg(bg)
}

#[allow(clippy::too_many_arguments)]
fn render_top_line_parts(
    row_bg: Style,
    title: &str,
    badge: String,
    badge_style: Style,
    settled: bool,
    selected: bool,
    sidebar_focused: bool,
    width: u16,
) -> Line<'static> {
    let content_w = width.saturating_sub(2) as usize;
    if content_w == 0 {
        return Line::raw("").style(row_bg);
    }
    let right_w = badge.width();
    let title_budget = content_w.saturating_sub(right_w + 1);
    let title_text = super::ellipsize_to_cols(title, title_budget as u16);
    let gap = content_w.saturating_sub(title_text.width() + right_w);
    let (prefix, prefix_style) = if selected && sidebar_focused {
        ("▸", row_bg.fg(theme::PRIMARY).add_modifier(Modifier::BOLD))
    } else if selected {
        ("│", row_bg.fg(theme::MUTED))
    } else {
        (" ", row_bg)
    };
    let title_style = if settled && !selected {
        row_bg.fg(theme::MUTED)
    } else if selected && !sidebar_focused {
        row_bg.fg(theme::TAB_INACTIVE)
    } else {
        row_bg.fg(theme::FG).add_modifier(Modifier::BOLD)
    };

    Line::from(vec![
        Span::styled(prefix, prefix_style),
        Span::styled(title_text, title_style),
        Span::styled(" ".repeat(gap), row_bg),
        Span::styled(badge, badge_style),
        Span::styled(" ", row_bg),
    ])
    .style(row_bg)
}

fn sidebar_card_top_line(
    app: &App,
    target: &CardTarget,
    selected: bool,
    settled: bool,
    width: u16,
) -> Line<'static> {
    let row_bg = sidebar_card_bg(app, selected);
    let sidebar_focused = app.ai_sidebar_focused;
    match target {
        CardTarget::OpenAgent(agent_idx) => {
            let Some(agent) = app.agents.get(*agent_idx) else {
                return Line::raw("").style(row_bg);
            };
            let (badge, badge_style) = if agent.is_working() {
                const WORKING_FRAMES: [&str; 10] =
                    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let spinner = WORKING_FRAMES[super::chat::frame_index(100, WORKING_FRAMES.len())];
                let secs = agent
                    .task_started
                    .map_or(agent.task_elapsed.as_secs(), |start| {
                        start.elapsed().as_secs()
                    });
                let elapsed = if secs >= 60 {
                    format!("{}m{}s", secs / 60, secs % 60)
                } else {
                    format!("{secs}s")
                };
                let label = if width >= 28 {
                    format!(
                        "{spinner} {} {elapsed}",
                        crate::i18n::t("chat.sidebar.working")
                    )
                } else {
                    format!("{spinner} {elapsed}")
                };
                (label, row_bg.fg(theme::INFO).add_modifier(Modifier::BOLD))
            } else if agent.unseen_completion {
                (
                    crate::i18n::t("chat.sidebar.done").to_string(),
                    row_bg.fg(theme::SUCCESS).add_modifier(Modifier::BOLD),
                )
            } else {
                let fg = if agent.status == AiStatus::Error {
                    theme::ERROR
                } else {
                    theme::MUTED
                };
                (format_compact_age(agent.last_activity_at), row_bg.fg(fg))
            };
            render_top_line_parts(
                row_bg,
                &agent.title,
                badge,
                badge_style,
                settled,
                selected,
                sidebar_focused,
                width,
            )
        }
        CardTarget::GlobalSession(session) => {
            let badge = format_compact_age(parse_millis_timestamp(&session.updated_at));
            let badge_style = row_bg.fg(theme::MUTED);
            render_top_line_parts(
                row_bg,
                &session.display_title(),
                badge,
                badge_style,
                settled,
                selected,
                sidebar_focused,
                width,
            )
        }
    }
}

fn global_session_location_label(app: &mut App, session: &crate::session::Session) -> String {
    let project_name = session
        .project_root
        .as_ref()
        .and_then(|root| root.file_name())
        .map(|name| name.to_string_lossy().into_owned());
    let location = if let Some(dir) = session.member_dir.as_ref() {
        if let Some(root) = session.project_root.as_ref()
            && let Ok(rel) = dir.strip_prefix(root)
            && !rel.as_os_str().is_empty()
        {
            Some(
                rel.components()
                    .filter_map(|comp| match comp {
                        std::path::Component::Normal(part) => {
                            Some(part.to_string_lossy().into_owned())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("/"),
            )
        } else {
            app.cached_git_branch(dir).or_else(|| {
                dir.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        }
    } else if let Some(root) = session.project_root.as_ref() {
        app.cached_git_branch(root)
    } else {
        None
    };

    match (project_name, location) {
        (Some(proj), Some(loc)) if !loc.is_empty() => format!("{proj} · {loc}"),
        (Some(proj), _) => proj,
        (None, Some(loc)) => loc,
        (None, None) => crate::i18n::t("sessions.unassigned").to_string(),
    }
}

fn sidebar_card_bottom_line(
    app: &mut App,
    target: &CardTarget,
    selected: bool,
    settled: bool,
    width: u16,
) -> Line<'static> {
    let row_bg = sidebar_card_bg(app, selected);
    let content_w = width.saturating_sub(2) as usize;
    if content_w == 0 {
        return Line::raw("").style(row_bg);
    }

    let (left_raw, right_raw, is_acp) = match target {
        CardTarget::OpenAgent(agent_idx) => {
            let left = app
                .agent_sidebar_location_label(*agent_idx)
                .unwrap_or_default();
            let right = app
                .agent_sidebar_backend_label(*agent_idx)
                .unwrap_or_default();
            let is_acp = app.agents.get(*agent_idx).is_some_and(|agent| {
                agent.backend_kind() == teshi_agent::backend::AgentBackendKind::Acp
                    || agent.selected_acp_id.is_some()
            });
            (left, right, is_acp)
        }
        CardTarget::GlobalSession(session) => {
            let left = global_session_location_label(app, session);
            let right = app.global_session_backend_label(session);
            let is_acp = session.backend_kind == Some(teshi_agent::backend::AgentBackendKind::Acp)
                || session.acp_agent_id.is_some();
            (left, right, is_acp)
        }
    };

    let (left, right) = if left_raw.is_empty() && right_raw.is_empty() {
        (String::new(), String::new())
    } else if left_raw.is_empty() {
        (
            String::new(),
            super::ellipsize_to_cols(&right_raw, content_w as u16),
        )
    } else if right_raw.is_empty() {
        (
            super::middle_ellipsize_to_cols(&left_raw, content_w as u16),
            String::new(),
        )
    } else if left_raw.width() + 1 + right_raw.width() <= content_w {
        (left_raw, right_raw)
    } else {
        let right_cap = (content_w / 2).max(8).min(content_w.saturating_sub(2));
        let right = super::ellipsize_to_cols(&right_raw, right_cap as u16);
        let left_budget = content_w.saturating_sub(right.width() + 1);
        let left = super::middle_ellipsize_to_cols(&left_raw, left_budget as u16);
        (left, right)
    };

    let gap = content_w.saturating_sub(left.width() + right.width());
    let left_style = row_bg.fg(theme::MUTED);
    let right_style = if settled && !selected {
        row_bg.fg(theme::MUTED)
    } else if is_acp {
        row_bg.fg(theme::INFO)
    } else {
        row_bg.fg(theme::MUTED)
    };

    Line::from(vec![
        Span::styled(" ", row_bg),
        Span::styled(left, left_style),
        Span::styled(" ".repeat(gap), row_bg),
        Span::styled(right, right_style),
        Span::styled(" ", row_bg),
    ])
    .style(row_bg)
}

fn render_sidebar_search_bar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let active = app.ai_sidebar_searching || !app.ai_sidebar_query.is_empty();
    let bar_bg = if app.ai_sidebar_searching {
        theme::CHAT_CHIP_BG
    } else {
        theme::BLOCK_BG
    };
    let bar_style = Style::default().bg(bar_bg);
    let clear_width = if active && area.width >= 8 {
        3u16
    } else {
        0u16
    };
    let input_width = area.width.saturating_sub(clear_width);

    if input_width > 0 {
        let input_rect = Rect::new(area.x, area.y, input_width, 1);
        let content_w = input_width.saturating_sub(2) as usize;
        let line = if active {
            let prefix = "⌕ ";
            let cursor = if app.ai_sidebar_searching { "█" } else { "" };
            let query_budget = content_w.saturating_sub(prefix.width() + cursor.width());
            let query_text = super::ellipsize_to_cols(&app.ai_sidebar_query, query_budget as u16);
            let used = prefix.width() + query_text.width() + cursor.width();
            let pad = content_w.saturating_sub(used);
            Line::from(vec![
                Span::styled(" ", bar_style),
                Span::styled(prefix, bar_style.fg(theme::INFO)),
                Span::styled(query_text, bar_style.fg(theme::FG)),
                Span::styled(cursor, bar_style.fg(theme::PRIMARY)),
                Span::styled(" ".repeat(pad + 1), bar_style),
            ])
            .style(bar_style)
        } else {
            let label = format!("⌕ {}", crate::i18n::t("chat.sidebar.search"));
            let hint = "/";
            let label_w = label.width();
            let hint_w = hint.width();
            let show_hint = content_w > label_w + hint_w;
            let gap = if show_hint {
                content_w.saturating_sub(label_w + hint_w)
            } else {
                content_w.saturating_sub(label_w)
            };
            let mut spans = vec![
                Span::styled(" ", bar_style),
                Span::styled(
                    super::ellipsize_to_cols(&label, content_w as u16),
                    bar_style.fg(theme::MUTED),
                ),
                Span::styled(" ".repeat(gap), bar_style),
            ];
            if show_hint {
                spans.push(Span::styled(hint, bar_style.fg(theme::BORDER)));
            }
            spans.push(Span::styled(" ", bar_style));
            Line::from(spans).style(bar_style)
        };
        frame.render_widget(Paragraph::new(line), input_rect);
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: input_rect,
            action: Action::AiSidebarStartSearch,
            modal: false,
        });
    }

    if clear_width > 0 {
        let clear_rect = Rect::new(area.x + input_width, area.y, clear_width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" ✗ ", bar_style.fg(theme::MUTED)))),
            clear_rect,
        );
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: clear_rect,
            action: Action::AiSidebarClearSearch,
            modal: false,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn render_card_list_region(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    rows: &[SidebarRow],
    selected_start: Option<usize>,
    scroll_target: VerticalScrollTarget,
    scroll_offset: usize,
    follow_selection: bool,
) -> usize {
    if area.is_empty() || rows.is_empty() {
        return 0;
    }
    let view_h = area.height as usize;
    let need_scrollbar = rows.len() > view_h && area.width >= 8;
    let (list_area, bar_area) = if need_scrollbar {
        (
            Rect::new(area.x, area.y, area.width - 1, area.height),
            Some(Rect::new(area.right() - 1, area.y, 1, area.height)),
        )
    } else {
        (area, None)
    };

    let max_scroll = rows.len().saturating_sub(view_h);
    let mut scroll = scroll_offset.min(max_scroll);
    if follow_selection && let Some(card_start) = selected_start {
        let card_end = (card_start + 2).min(rows.len());
        if card_start < scroll {
            scroll = card_start;
        } else if card_end > scroll + view_h {
            scroll = card_end.saturating_sub(view_h);
        }
        scroll = scroll.min(max_scroll);
    }

    let visible_end = (scroll + view_h).min(rows.len());
    let visible_rows = &rows[scroll..visible_end];
    let mut lines = Vec::with_capacity(visible_rows.len());
    for row in visible_rows {
        let line = match row {
            SidebarRow::Spacer => Line::raw(""),
            SidebarRow::CardTop {
                target,
                selected,
                settled,
            } => sidebar_card_top_line(app, target, *selected, *settled, list_area.width),
            SidebarRow::CardBottom {
                target,
                selected,
                settled,
            } => sidebar_card_bottom_line(app, target, *selected, *settled, list_area.width),
            SidebarRow::EmptyNotice(msg) => Line::styled(format!(" {msg}"), theme::muted()),
        };
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), list_area);

    let mut idx = 0usize;
    while idx < visible_rows.len() {
        if let Some(target) = visible_rows[idx].card_target() {
            let key = target.key_id();
            let mut height = 1u16;
            while idx + (height as usize) < visible_rows.len()
                && visible_rows[idx + (height as usize)]
                    .card_target()
                    .is_some_and(|next| next.key_id() == key)
            {
                height += 1;
            }
            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect: Rect::new(
                    list_area.x,
                    list_area.y + idx as u16,
                    list_area.width,
                    height,
                ),
                action: target.click_action(),
                modal: false,
            });
            idx += height as usize;
        } else {
            idx += 1;
        }
    }

    if let Some(bar_rect) = bar_area {
        super::render_vertical_scrollbar(frame, app, scroll_target, bar_rect, rows.len(), scroll);
    }

    scroll
}

fn max_cards_height(card_count: usize, max_cards: usize) -> u16 {
    let shown = card_count.min(max_cards);
    if shown == 0 {
        0
    } else {
        (shown * 2 + shown - 1) as u16
    }
}

/// Draw top search bar, upper Active history, bottom-docked Working (max 4) + Settled drawer, and bottom controls.
pub(super) fn render_agent_sidebar(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 5 || area.height < 3 {
        return;
    }
    let raw_title = crate::i18n::t("chat.chats.title");
    let block = if app.ai_sidebar_focused {
        theme::panel_block(format!(" ● {} ", raw_title.trim()), true)
    } else {
        theme::panel_block(raw_title, false).title_style(theme::muted())
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let (main_area, action_area) = super::split_pane_action_row(inner);
    if !main_area.is_empty() {
        let side_inset = u16::from(main_area.width >= 14);
        let padded_w = main_area.width.saturating_sub(side_inset * 2);
        let (search_area, body_area) = if main_area.height >= 4 {
            let search = Rect::new(main_area.x + side_inset, main_area.y, padded_w, 1);
            let body = Rect::new(
                main_area.x + side_inset,
                main_area.y + 2,
                padded_w,
                main_area.height - 2,
            );
            (Some(search), body)
        } else {
            (
                None,
                Rect::new(
                    main_area.x + side_inset,
                    main_area.y,
                    padded_w,
                    main_area.height,
                ),
            )
        };

        if let Some(search) = search_area {
            render_sidebar_search_bar(frame, app, search);
        }

        if !body_area.is_empty() {
            if !app.ai_sidebar_query.trim().is_empty() {
                app.ai_sidebar_rect = Some(body_area);
                let (rows, selected_start) = build_search_sidebar_rows(app);
                app.ai_sidebar_scroll = render_card_list_region(
                    frame,
                    app,
                    body_area,
                    &rows,
                    selected_start,
                    VerticalScrollTarget::AiSidebarActive,
                    app.ai_sidebar_scroll,
                    true,
                );
            } else {
                let sections = app.agent_sidebar_sections();
                let total_h = body_area.height;
                let min_active_h = if sections.active.is_empty() {
                    0u16
                } else {
                    2u16
                };

                // 1. Settled drawer is always anchored at the very bottom of body_area.
                let settled_cards_desired =
                    if app.ai_sidebar_settled_expanded && !sections.settled.is_empty() {
                        max_cards_height(sections.settled.len(), MAX_SETTLED_VISIBLE_CARDS)
                    } else {
                        0
                    };
                let settled_header_h = 1u16;
                let max_for_dock = total_h.saturating_sub(min_active_h);
                let settled_cards_h =
                    settled_cards_desired.min(max_for_dock.saturating_sub(settled_header_h));
                let settled_total_h = (settled_header_h + settled_cards_h).min(total_h);

                // 2. Working shelf is anchored immediately above Settled when non-empty.
                let has_working = !sections.working.is_empty();
                let working_gap_h = u16::from(has_working && total_h > settled_total_h + 1);
                let avail_for_working = total_h
                    .saturating_sub(settled_total_h + working_gap_h)
                    .saturating_sub(min_active_h);
                let working_header_h = if has_working && avail_for_working >= 1 {
                    1u16
                } else {
                    0u16
                };
                let working_cards_desired = if has_working && app.ai_sidebar_working_expanded {
                    max_cards_height(sections.working.len(), MAX_WORKING_VISIBLE_CARDS)
                } else {
                    0
                };
                let working_cards_h =
                    working_cards_desired.min(avail_for_working.saturating_sub(working_header_h));
                let working_total_h = working_header_h + working_cards_h;

                let dock_total_h = settled_total_h + working_gap_h + working_total_h;
                let raw_active_h = total_h.saturating_sub(dock_total_h);
                let divider_h = u16::from(raw_active_h >= 2 && dock_total_h > 0);
                let active_h = raw_active_h.saturating_sub(divider_h);

                // Top: Active / historical completed sessions.
                if active_h > 0 {
                    let active_area =
                        Rect::new(body_area.x, body_area.y, body_area.width, active_h);
                    app.ai_sidebar_rect = Some(active_area);
                    let (active_rows, active_selected) =
                        build_section_card_rows(&sections.active, app.selected_agent, false);
                    app.ai_sidebar_scroll = render_card_list_region(
                        frame,
                        app,
                        active_area,
                        &active_rows,
                        active_selected,
                        VerticalScrollTarget::AiSidebarActive,
                        app.ai_sidebar_scroll,
                        app.ai_sidebar_follow_selection,
                    );
                }

                // Hairline separator between upper Active list and bottom dock.
                let mut cursor_y = body_area.y + active_h;
                if divider_h > 0 {
                    let rule_rect = Rect::new(body_area.x, cursor_y, body_area.width, 1);
                    let rule = "─".repeat(body_area.width as usize);
                    frame.render_widget(
                        Paragraph::new(Line::styled(rule, Style::default().fg(theme::BORDER))),
                        rule_rect,
                    );
                    cursor_y += 1;
                }

                // Middle-bottom: Working shelf (max 4 visible cards + internal scrollbar).
                if working_total_h > 0 {
                    let working_header_rect = Rect::new(body_area.x, cursor_y, body_area.width, 1);
                    render_shelf_header_row(
                        frame,
                        app,
                        working_header_rect,
                        crate::i18n::t("chat.sidebar.working"),
                        sections.working.len(),
                        app.ai_sidebar_working_expanded,
                        ShelfKind::Working,
                    );
                    cursor_y += 1;
                    if working_cards_h > 0 {
                        let working_cards_rect =
                            Rect::new(body_area.x, cursor_y, body_area.width, working_cards_h);
                        app.ai_sidebar_working_rect = Some(working_cards_rect);
                        let (working_rows, working_selected) =
                            build_section_card_rows(&sections.working, app.selected_agent, false);
                        app.ai_sidebar_working_scroll = render_card_list_region(
                            frame,
                            app,
                            working_cards_rect,
                            &working_rows,
                            working_selected,
                            VerticalScrollTarget::AiSidebarWorking,
                            app.ai_sidebar_working_scroll,
                            app.ai_sidebar_follow_selection,
                        );
                        cursor_y += working_cards_h;
                    }
                    cursor_y += working_gap_h;
                }

                // Very bottom: Settled drawer (collapsed by default).
                if settled_total_h > 0 {
                    let settled_header_rect = Rect::new(body_area.x, cursor_y, body_area.width, 1);
                    render_shelf_header_row(
                        frame,
                        app,
                        settled_header_rect,
                        crate::i18n::t("chat.sidebar.settled"),
                        sections.settled.len(),
                        app.ai_sidebar_settled_expanded,
                        ShelfKind::Settled,
                    );
                    cursor_y += 1;
                    if settled_cards_h > 0 {
                        let settled_cards_rect =
                            Rect::new(body_area.x, cursor_y, body_area.width, settled_cards_h);
                        app.ai_sidebar_settled_rect = Some(settled_cards_rect);
                        let (settled_rows, settled_selected) =
                            build_section_card_rows(&sections.settled, app.selected_agent, true);
                        app.ai_sidebar_settled_scroll = render_card_list_region(
                            frame,
                            app,
                            settled_cards_rect,
                            &settled_rows,
                            settled_selected,
                            VerticalScrollTarget::AiSidebarSettled,
                            app.ai_sidebar_settled_scroll,
                            app.ai_sidebar_follow_selection,
                        );
                    }
                }
            }
        }
    }

    if let Some(action_area) = action_area {
        let (settle_label, settle_compact) = if app.agent().settled {
            (
                crate::i18n::t("chat.reopen.key"),
                crate::i18n::t("chat.reopen"),
            )
        } else {
            (
                crate::i18n::t("chat.settle.key"),
                crate::i18n::t("chat.settle"),
            )
        };
        super::render_chip_action_bar(
            frame,
            &mut app.clickable_regions,
            action_area,
            &[
                super::ActionItem::new(
                    crate::i18n::t("chat.new.key"),
                    crate::i18n::t("chat.new"),
                    Action::AgentNew,
                    theme::ActionTone::Success,
                ),
                super::ActionItem::new(
                    crate::i18n::t("chat.close.key"),
                    crate::i18n::t("chat.close"),
                    Action::AgentClose,
                    theme::ActionTone::Danger,
                ),
                super::ActionItem::new(
                    settle_label,
                    settle_compact,
                    Action::AgentToggleSettle,
                    theme::ActionTone::Neutral,
                ),
            ],
            false,
        );
    }
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

/// Renders the `@` mention context popup directly above the composer meta strip.
///
/// Clicking any row immediately mounts the candidate (or drills into a source
/// prefix such as `@issue:`). Uses only semantic theme tokens.
pub(super) fn render_mention_popup(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    items: &[teshi_core::mention::MentionItem],
) {
    use teshi_core::mention::{MentionItem, MentionSource};
    use unicode_width::UnicodeWidthStr;

    if area.is_empty() {
        return;
    }

    let source_color = |source: MentionSource| match source {
        MentionSource::LastFail => theme::ERROR,
        MentionSource::Issue => theme::INFO,
        MentionSource::TestPoint => theme::PRIMARY,
        MentionSource::Feature => theme::SUCCESS,
    };

    let rows = area.height.saturating_sub(1) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", crate::i18n::t("chat.mention.no_matches")),
            Style::default().fg(theme::MUTED),
        )));
    } else {
        let selected = app
            .mention_popup
            .selection()
            .min(items.len().saturating_sub(1));
        let start = selected.saturating_sub(rows.saturating_sub(1));
        let max_width = area.width as usize;

        for (index, item) in items.iter().enumerate().skip(start).take(rows) {
            let is_selected = index == selected;
            let row_rect = Rect::new(area.x, area.y + (index - start) as u16, area.width, 1);
            let hovered = app
                .mouse_position
                .is_some_and(|(mx, my)| row_rect.contains(ratatui::layout::Position::new(mx, my)));
            let row_bg = if is_selected {
                Some(theme::TAB_ACTIVE_BG)
            } else if hovered {
                Some(theme::HOVER_BG)
            } else {
                None
            };
            let paint = |mut style: Style| {
                if let Some(bg) = row_bg {
                    style = style.bg(bg);
                }
                style
            };

            let (prefix, prefix_color, main_text, detail_text) = match item {
                MentionItem::Category {
                    source,
                    count,
                    preview,
                } => {
                    let count_str = count.to_string();
                    let label = match source {
                        MentionSource::LastFail => {
                            crate::i18n::t("chat.mention.last_fail_empty").to_string()
                        }
                        MentionSource::Issue => crate::i18n::fill(
                            crate::i18n::t("chat.mention.category.issue"),
                            &[("count", count_str.as_str())],
                        ),
                        MentionSource::TestPoint => crate::i18n::fill(
                            crate::i18n::t("chat.mention.category.tp"),
                            &[("count", count_str.as_str())],
                        ),
                        MentionSource::Feature => crate::i18n::fill(
                            crate::i18n::t("chat.mention.category.feature"),
                            &[("count", count_str.as_str())],
                        ),
                    };
                    (
                        source.prefix().to_string(),
                        source_color(*source),
                        label,
                        preview.clone(),
                    )
                }
                MentionItem::Candidate(candidate) => (
                    candidate.source.prefix().to_string(),
                    source_color(candidate.source),
                    candidate.label.clone(),
                    candidate.detail.clone(),
                ),
            };

            let prefix_col = format!(" {prefix:<10} ");
            let used = prefix_col.width();
            let remaining = max_width.saturating_sub(used);
            let mut spans = vec![Span::styled(
                prefix_col,
                paint(
                    Style::default()
                        .fg(prefix_color)
                        .add_modifier(Modifier::BOLD),
                ),
            )];

            let detail_clean = detail_text
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty());
            let (label_text, right_detail) = if let Some(detail) = detail_clean {
                let detail_budget = (remaining / 3).clamp(10, 32);
                let label_budget = remaining.saturating_sub(detail_budget + 2);
                if label_budget >= 8 {
                    (
                        super::truncate_str(&main_text, label_budget),
                        Some(super::truncate_str(detail, detail_budget)),
                    )
                } else {
                    (super::truncate_str(&main_text, remaining), None)
                }
            } else {
                (super::truncate_str(&main_text, remaining), None)
            };

            let main_fg = if is_selected {
                theme::FG
            } else if matches!(
                item,
                MentionItem::Category {
                    source: MentionSource::LastFail,
                    count: 0,
                    ..
                }
            ) {
                theme::MUTED
            } else {
                theme::FG
            };
            spans.push(Span::styled(
                label_text,
                paint(Style::default().fg(main_fg)),
            ));
            if let Some(detail) = right_detail {
                spans.push(Span::styled(
                    format!("  {detail}"),
                    paint(Style::default().fg(theme::MUTED)),
                ));
            }

            let mut line = Line::from(spans);
            if let Some(bg) = row_bg {
                line = line.style(Style::default().bg(bg));
            }
            lines.push(line);

            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect: row_rect,
                action: Action::AiMentionChoose(index),
                modal: false,
            });
        }
    }

    if rows > 0 {
        frame.render_widget(
            Paragraph::new(Text::from(lines)),
            Rect::new(area.x, area.y, area.width, rows as u16),
        );
    }
    super::render_action_bar(
        frame,
        &mut app.clickable_regions,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        "",
        &[
            (crate::i18n::t("action.select"), Action::AiMentionSelect),
            (crate::i18n::t("action.dismiss"), Action::AiMentionDismiss),
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
        || app.help_guide.active
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
