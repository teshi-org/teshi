//! Modal User Guide & Shortcut Reference modeled on iCode's `GuideDialog`.
//!
//! Layout mirrors iCode's two-pane documentation reader:
//! - Outer modal card: 94% width (min 60, max 220), 90% height, dimmed backdrop.
//! - Top border: bold title on the left, clickable language switch on the right
//!   (`Space` cycles the document body language without changing app settings).
//! - Left pane (30% width, clamped 26..=38): clickable topic list with a solid
//!   vertical divider on its right edge.
//! - Right pane (remaining width): Markdown body rendered through
//!   [`crate::markdown::render_markdown_width`] with a draggable vertical
//!   scrollbar on the right edge.
//! - Bottom border: navigation hints on the left, clickable `Close [Esc]` on the
//!   right.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, Paragraph};
use teshi_engine::EffectiveUiLocale;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ClickableRegion, MainTab, VerticalScrollTarget};
use crate::keymap::Action;
use crate::theme;

use super::{dialog, overlay};

/// Number of guide topics in the left navigation tree.
pub const TOPIC_COUNT: usize = 8;

/// Transient state for the modal User Guide (`?` / `F1` / `:help` / `/help`).
#[derive(Debug, Clone)]
pub struct HelpGuideState {
    pub active: bool,
    pub selected_topic: usize,
    /// `false` when the left topic list owns `j`/`k`, `true` when the right
    /// Markdown reader owns `j`/`k`.
    pub focus_doc: bool,
    pub doc_scroll: usize,
    pub doc_max_scroll: usize,
    /// Document body language (cycled independently via `Space`).
    pub doc_locale: EffectiveUiLocale,
    pub tree_rect: Option<Rect>,
    pub doc_rect: Option<Rect>,
}

impl Default for HelpGuideState {
    fn default() -> Self {
        Self {
            active: false,
            selected_topic: 0,
            focus_doc: false,
            doc_scroll: 0,
            doc_max_scroll: 0,
            doc_locale: EffectiveUiLocale::En,
            tree_rect: None,
            doc_rect: None,
        }
    }
}

impl HelpGuideState {
    /// Opens the guide positioned on the topic matching `active_tab`.
    pub fn open_for_tab(&mut self, active_tab: MainTab) {
        self.active = true;
        self.selected_topic = match active_tab {
            MainTab::Explore => 1,
            MainTab::MindMap => 2,
            MainTab::Ai => 3,
            MainTab::Requirements => 4,
            MainTab::TestPoints => 5,
        };
        self.focus_doc = false;
        self.doc_scroll = 0;
        self.doc_locale = crate::i18n::effective_locale();
    }

    pub fn close(&mut self) {
        self.active = false;
    }

    pub fn select_topic(&mut self, index: usize) {
        let clamped = index.min(TOPIC_COUNT - 1);
        if self.selected_topic != clamped {
            self.selected_topic = clamped;
            self.doc_scroll = 0;
        }
    }

    /// Toggles the guide document body language without affecting global settings.
    pub fn cycle_language(&mut self) {
        self.doc_locale = match self.doc_locale {
            EffectiveUiLocale::En => EffectiveUiLocale::ZhCn,
            EffectiveUiLocale::ZhCn => EffectiveUiLocale::En,
        };
    }

    pub fn scroll_doc(&mut self, delta: isize) {
        if delta < 0 {
            self.doc_scroll = self.doc_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.doc_scroll = self
                .doc_scroll
                .saturating_add(delta as usize)
                .min(self.doc_max_scroll);
        }
    }
}

/// Renders the two-pane User Guide modal when `app.help_guide.active` is true.
pub(super) fn render_help_guide(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if !app.help_guide.active {
        return;
    }
    const GUIDE_SPEC: overlay::OverlaySpec = overlay::OverlaySpec::viewer(94, 60, 220, 90, 12, 60);
    let panel = GUIDE_SPEC.rect(area, (0, 0));
    if panel.width < 24 || panel.height < 8 {
        return;
    }
    app.modal_card_rect = Some(panel);
    app.help_guide.tree_rect = None;
    app.help_guide.doc_rect = None;

    dialog::dim_backdrop(frame, area);
    frame.render_widget(Clear, panel);

    let ui_locale = crate::i18n::effective_locale();
    let title = match ui_locale {
        EffectiveUiLocale::ZhCn => " Teshi 用户指南 ",
        EffectiveUiLocale::En => " Teshi User Guide ",
    };
    let doc_lang_name = match app.help_guide.doc_locale {
        EffectiveUiLocale::ZhCn => "简体中文",
        EffectiveUiLocale::En => "English",
    };
    let lang_subtitle = match ui_locale {
        EffectiveUiLocale::ZhCn => format!(" 语言: {doc_lang_name} (按空格切换) "),
        EffectiveUiLocale::En => format!(" Language: {doc_lang_name} (Space to switch) "),
    };

    let block = theme::dialog_block(
        Line::styled(
            title,
            Style::default()
                .fg(theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        false,
    )
    .border_style(Style::default().fg(theme::BORDER))
    .title_bottom(
        Line::styled(lang_subtitle.clone(), Style::default().fg(theme::PRIMARY)).right_aligned(),
    );
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    // Register clickable bottom-right Language subtitle (matches iCode's Space toggle).
    let lang_w = lang_subtitle.width() as u16;
    if panel.width > lang_w + 2 {
        app.clickable_regions.push(ClickableRegion::ActionButton {
            rect: Rect::new(
                panel.right().saturating_sub(lang_w + 1),
                panel.bottom().saturating_sub(1),
                lang_w,
                1,
            ),
            action: Action::HelpCycleLanguage,
            modal: true,
        });
    }

    if inner.width < 12 || inner.height < 3 {
        return;
    }

    // Two-pane split matching iCode's GuideDialog: left topic tree (30%,
    // clamped 28..=40 on wide terminals), 1-col divider, right Markdown body.
    let tree_w = if inner.width >= 64 {
        (inner.width * 3 / 10).clamp(28, 40)
    } else {
        (inner.width / 3).max(12)
    };
    let [tree_area, divider_area, doc_outer] = Layout::horizontal([
        Constraint::Length(tree_w),
        Constraint::Length(1),
        Constraint::Min(10),
    ])
    .areas(inner);

    app.help_guide.tree_rect = Some(tree_area);
    app.help_guide.doc_rect = Some(doc_outer);

    render_topic_tree(frame, app, tree_area, ui_locale);

    // Subtle vertical divider matching iCode's `#gd-tree { border-right: solid ... }`.
    let div_lines: Vec<Line<'static>> = (0..divider_area.height)
        .map(|_| Line::styled("│", Style::default().fg(theme::BORDER).bg(theme::DIALOG_BG)))
        .collect();
    frame.render_widget(Paragraph::new(div_lines), divider_area);

    render_topic_doc(frame, app, doc_outer);
}

struct GuideGroup {
    zh: &'static str,
    en: &'static str,
    topics: &'static [usize],
}

const GUIDE_GROUPS: &[GuideGroup] = &[
    GuideGroup {
        zh: "快速开始",
        en: "Quick Start",
        topics: &[0],
    },
    GuideGroup {
        zh: "工作区",
        en: "Workspaces",
        topics: &[1, 2, 3, 4, 5],
    },
    GuideGroup {
        zh: "参考",
        en: "Reference",
        topics: &[6, 7],
    },
];

fn render_topic_tree(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    ui_locale: EffectiveUiLocale,
) {
    let focused = !app.help_guide.focus_doc;
    let mouse = app.mouse_position;
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(TOPIC_COUNT + GUIDE_GROUPS.len());
    let mut row_offset = 0u16;

    for group in GUIDE_GROUPS {
        let row_y = area.y + row_offset;
        if row_y >= area.bottom() {
            break;
        }
        let group_title = match ui_locale {
            EffectiveUiLocale::ZhCn => group.zh,
            EffectiveUiLocale::En => group.en,
        };
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled("▼", Style::default().fg(theme::FG)),
            Span::styled(
                group_title.to_string(),
                Style::default().fg(theme::FG).add_modifier(Modifier::BOLD),
            ),
        ]));
        row_offset += 1;

        for (pos, &topic_idx) in group.topics.iter().enumerate() {
            let row_y = area.y + row_offset;
            if row_y >= area.bottom() {
                break;
            }
            let is_last = pos + 1 == group.topics.len();
            let selected = topic_idx == app.help_guide.selected_topic;
            let row_rect = Rect::new(area.x, row_y, area.width, 1);
            let hovered = mouse
                .is_some_and(|(mx, my)| row_rect.contains(ratatui::layout::Position::new(mx, my)));
            app.clickable_regions.push(ClickableRegion::ActionButton {
                rect: row_rect,
                action: Action::HelpSelectTopic(topic_idx),
                modal: true,
            });

            let branch = if is_last {
                " └── "
            } else {
                " ├── "
            };
            let label = topic_label(topic_idx, ui_locale);
            let label_style = if selected {
                if focused {
                    Style::default()
                        .fg(theme::CHIP_FG)
                        .bg(theme::PRIMARY)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme::FG)
                        .bg(theme::SELECTION_INACTIVE_BG)
                        .add_modifier(Modifier::BOLD)
                }
            } else if hovered {
                Style::default().fg(theme::FG).bg(theme::HOVER_BG)
            } else {
                Style::default().fg(theme::FG)
            };

            lines.push(Line::from(vec![
                Span::styled(branch, Style::default().fg(theme::BORDER)),
                Span::styled(label.to_string(), label_style),
            ]));
            row_offset += 1;
        }
    }

    frame.render_widget(Paragraph::new(theme::popup_rows(lines, area.width)), area);
}

fn render_topic_doc(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    // 2-cell left padding like iCode's `#gd-doc { padding: 0 0 0 2; }`.
    let pad_left = 2.min(area.width.saturating_sub(2));
    let body_outer = Rect::new(
        area.x + pad_left,
        area.y,
        area.width.saturating_sub(pad_left),
        area.height,
    );
    let md = topic_markdown(app.help_guide.selected_topic, app.help_guide.doc_locale);

    // Reserve 1 column on the right for the vertical scrollbar when needed.
    let preliminary =
        crate::markdown::render_markdown_width(md, body_outer.width.saturating_sub(1).max(1));
    let total_rows = preliminary.len();
    let has_vbar = total_rows > body_outer.height as usize && body_outer.width >= 3;
    let doc_area = if has_vbar {
        Rect {
            width: body_outer.width - 1,
            ..body_outer
        }
    } else {
        body_outer
    };
    let mut rendered = if has_vbar {
        preliminary
    } else {
        crate::markdown::render_markdown_width(md, doc_area.width)
    };
    // Center the leading H1 heading to match iCode's GuideDialog document header.
    if let Some(first) = rendered.first_mut() {
        *first = first.clone().centered();
    }

    let max_scroll = rendered.len().saturating_sub(doc_area.height as usize);
    app.help_guide.doc_max_scroll = max_scroll;
    app.help_guide.doc_scroll = app.help_guide.doc_scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(Text::from(rendered))
            .scroll((app.help_guide.doc_scroll.min(u16::MAX as usize) as u16, 0)),
        doc_area,
    );

    if has_vbar {
        let vbar = Rect::new(doc_area.right(), doc_area.y, 1, doc_area.height);
        super::render_vertical_scrollbar(
            frame,
            app,
            VerticalScrollTarget::HelpGuideDoc,
            vbar,
            total_rows,
            app.help_guide.doc_scroll,
        );
    }
}

fn topic_label(index: usize, locale: EffectiveUiLocale) -> &'static str {
    match (index, locale) {
        (0, EffectiveUiLocale::ZhCn) => "概览与模式",
        (0, EffectiveUiLocale::En) => "Overview & Modes",
        (1, EffectiveUiLocale::ZhCn) => "探索 (Explore)",
        (1, EffectiveUiLocale::En) => "Explore",
        (2, EffectiveUiLocale::ZhCn) => "思维导图 (MindMap)",
        (2, EffectiveUiLocale::En) => "MindMap",
        (3, EffectiveUiLocale::ZhCn) => "AI 对话",
        (3, EffectiveUiLocale::En) => "AI Chat",
        (4, EffectiveUiLocale::ZhCn) => "需求文档",
        (4, EffectiveUiLocale::En) => "Requirements",
        (5, EffectiveUiLocale::ZhCn) => "测试点",
        (5, EffectiveUiLocale::En) => "Test Points",
        (6, EffectiveUiLocale::ZhCn) => "Vim 与命令行",
        (6, EffectiveUiLocale::En) => "Vim & Commands",
        (7, EffectiveUiLocale::ZhCn) => "鼠标与 GUI 操作",
        (_, _) => "Mouse & GUI",
    }
}

fn topic_markdown(index: usize, locale: EffectiveUiLocale) -> &'static str {
    match (index, locale) {
        (0, EffectiveUiLocale::ZhCn) => {
            r#"# 概览与模式

Teshi 终端界面融合了 **GUI 直观点击** 与 **Vim 高效键位**。底部状态栏实时显示当前模式徽章与聚焦面板路径。

## 全局快捷键

| 快捷键 | 作用 |
|---|---|
| `1` – `5` | 直接切换到 探索 / 思维导图 / AI / 需求 / 测试点 |
| `gt` / `gT` | 顺序切换到下一个 / 上一个标签页 |
| `Ctrl+W` + `h/j/k/l` 或方向键 | 在当前页可见面板间切换焦点 |
| `Ctrl+U` / `Ctrl+D` | 向上 / 向下半页滚动（全标签页通用） |
| `Ctrl+B` / `Ctrl+F` 或 `PgUp/PgDn` | 向上 / 向下整页滚动 |
| `gg` / `G` 或 `Home` / `End` | 跳到列表或文档首项 / 末项 |
| `:` | 打开底部 Ex 命令行（支持 `Tab` 自动补全） |
| `?` 或 `F1` | 打开 / 关闭本使用指南 |
| `F10` 或 `Ctrl+Shift+P` | 打开应用设置（语言、AI 提示词开关） |
| `Ctrl+S` / `s` | 保存当前正在编辑的缓冲区 |
| `Ctrl+Q` 或 `:q` | 安全退出（有未保存改动时会确认） |

## 交互模式

- **NORMAL / 浏览**：按键执行导航与命令，所有操作按钮均可用鼠标点击。
- **INSERT / 输入**：全部可打印字符直接输入，`Esc` 返回普通模式。
- **SEARCH / 搜索**：按 `/` 或点击标题栏搜索词进入实时过滤，`Enter` / 点 `✓` 确认，`Esc` / 点 `✗` 清空。
"#
        }
        (0, EffectiveUiLocale::En) => {
            r#"# Overview & Modes

Teshi combines **intuitive GUI mouse controls** with **fast Vim navigation**. The bottom status bar always shows your active mode chip and focused pane breadcrumb.

## Global Shortcuts

| Key | Action |
|---|---|
| `1` – `5` | Jump directly to Explore / MindMap / AI / Requirements / Test Points |
| `gt` / `gT` | Cycle to next / previous tab |
| `Ctrl+W` + `h/j/k/l` or Arrows | Move focus across visible panes |
| `Ctrl+U` / `Ctrl+D` | Scroll half-page up / down across all tabs |
| `Ctrl+B` / `Ctrl+F` or `PgUp/PgDn` | Page up / down |
| `gg` / `G` or `Home` / `End` | Jump to top / bottom |
| `:` | Open Ex command line (supports `Tab` autocomplete) |
| `?` or `F1` | Open / close this User Guide |
| `F10` or `Ctrl+Shift+P` | Open App Settings |
| `Ctrl+S` / `s` | Save the active editable buffer |
| `Ctrl+Q` or `:q` | Guarded quit |

## Modes

- **NORMAL / BROWSE**: Navigation and command mode; pane chip buttons are always clickable.
- **INSERT / INPUT**: Printable characters type text; `Esc` returns to Normal.
- **SEARCH**: Press `/` or click the pane search title; press `Enter` or click `✓` to keep filter, `Esc` or `✗` to clear.
"#
        }
        (1, EffectiveUiLocale::ZhCn) => {
            r#"# 探索 (Explore)

四栏联动浏览与执行 BDD Feature：左侧 **功能树**，右上 **场景列表**，右下分为 **步骤** 与 **运行详情 (Run Details)**。

## 浏览模式快捷键

| 快捷键 | 作用 |
|---|---|
| `j/k` 或 `↑/↓` | 上下选择功能、场景、步骤或运行详情项 |
| `h/l` 或 `←/→` | 展开/折叠目录，或在各栏之间移动焦点 |
| `/` | 搜索功能名称与路径（支持点 `✓` 确认 / `✗` 清除） |
| `r` | 运行当前选中场景（也可点击 `▸ 运行 [r]`） |
| `e` | 进入当前 Feature 源码编辑器（也可双击场景行） |
| `Enter` | 切换目录 / 运行详情主操作（全屏看图、展开接口） |
| `Space` | 展开/收起 HTTP 请求体与响应体 |
| `p` / `o` / `y` | 切换明文 / 系统外部打开 / 复制路径或 URL |
| `F6` | 手动重新扫描 Feature 文件 |

## 源码编辑模式

| 快捷键 | 作用 |
|---|---|
| `i` / `Enter` | 编辑当前步骤或标题；光标在关键字时打开关键字选择器 |
| `o` / `O` | 在下方 / 上方插入新步骤 |
| `dd` / `yy` / `p` | 删除 / 复制 / 粘贴步骤 |
| `Ctrl+J` / `Ctrl+K` | 向下 / 向上移动步骤块 |
| `Space` / `Ctrl+Space` | 折叠当前场景 / 全部场景 |
| `u` / `Ctrl+R` | 撤销 / 重做 |
| `F5` / `Ctrl+S` / `Esc` | 后台运行 / 保存 / 返回浏览模式 |
"#
        }
        (1, EffectiveUiLocale::En) => {
            r#"# Explore

Four-pane BDD Feature browser and runner: **Features** on the left, **Scenarios** top-right, and **Steps** + **Run Details** bottom-right.

## Browse Mode

| Key | Action |
|---|---|
| `j/k` or `↑/↓` | Navigate features, scenarios, steps, or detail items |
| `h/l` or `←/→` | Expand/collapse directories or move focus across panes |
| `/` | Search features by title or path (click `✓` to commit, `✗` to clear) |
| `r` | Run selected scenario (or click `Run [r]`) |
| `e` | Open Feature source editor (or double-click a scenario) |
| `Enter` | Toggle directory / Run Details primary action (fullscreen shot, open artifact) |
| `Space` | Expand/collapse HTTP exchange body |
| `p` / `o` / `y` | Toggle plaintext / open externally / copy path or URL |
| `F6` | Rescan Feature files |

## Feature Source Editor

| Key | Action |
|---|---|
| `i` / `Enter` | Edit selected step/title; opens keyword picker when focused on keyword |
| `o` / `O` | Insert step below / above |
| `dd` / `yy` / `p` | Delete / yank / paste step |
| `Ctrl+J` / `Ctrl+K` | Move step block down / up |
| `Space` / `Ctrl+Space` | Fold current scenario / fold all |
| `u` / `Ctrl+R` | Undo / redo |
| `F5` / `Ctrl+S` / `Esc` | Run in background / save / return to browse |
"#
        }
        (2, EffectiveUiLocale::ZhCn) => {
            r#"# 思维导图 (MindMap)

以公共步骤前缀树可视化当前项目或 Feature 的场景结构，右侧支持场景源码预览与内置 AI 对话面板。

## 常用快捷键

| 快捷键 | 作用 |
|---|---|
| `j/k` 或 `↑/↓` | 移动到上一个 / 下一个树节点 |
| `h/l` 或 `←/→` | 折叠 / 展开节点 |
| `Space` | 切换当前节点展开状态 |
| `J/K` 或 `Shift+↑/↓` | 跳转到上一个 / 下一个同级节点 |
| `[` / `]` | 切换当前节点的上一个 / 下一个关联场景位置 |
| `o` | 打开场景位置下拉列表（也可双击右侧预览区） |
| `p` | 显示 / 隐藏右侧预览与 AI 面板 |
| `a` | 将当前选中节点上下文发送到 AI 对话 |
| `Enter` | 将焦点切换到右侧 AI 面板 |
"#
        }
        (2, EffectiveUiLocale::En) => {
            r#"# MindMap

Visualizes shared step prefixes as a collapsible trie, with a live scenario preview and embedded AI chat panel on the right.

## Shortcuts

| Key | Action |
|---|---|
| `j/k` or `↑/↓` | Next / previous tree node |
| `h/l` or `←/→` | Collapse / expand node |
| `Space` | Toggle node expansion |
| `J/K` or `Shift+↑/↓` | Previous / next sibling node |
| `[` / `]` | Previous / next associated scenario location |
| `o` | Open scenario location dropdown (or double-click Preview) |
| `p` | Show / hide right preview & AI panel |
| `a` | Send selected node context to AI |
| `Enter` | Focus the visible AI panel |
"#
        }
        (3, EffectiveUiLocale::ZhCn) => {
            r#"# AI 对话

多会话 AI 助手工作台，支持原生模型与 ACP Agent、思考与工具过程折叠、排队追问及图片附件。

## 浏览模式 (NORMAL)

| 快捷键 | 作用 |
|---|---|
| `i` / `Enter` | 聚焦输入框开始输入 |
| `j/k` / `↑/↓` | 上下滚动对话记录 |
| `Ctrl+U` / `Ctrl+D` | 半页向上 / 向下滚动 |
| `h` / `Ctrl+W h` | 焦点移至左侧会话列表 |
| `Ctrl+G` | 折叠 / 展开左侧会话列表 |
| `[` / `]` | 切换上一个 / 下一个会话 |
| `a` / `x` | 新建会话 / 关闭当前会话 |
| `m` | 打开模型配置面板 |

## 输入模式 (INPUT)

| 快捷键 | 作用 |
|---|---|
| `Enter` | 发送消息（AI 运行中时加入排队） |
| `Shift+Enter` / `Ctrl+J` | 插入换行 |
| `Ctrl+V` / `Alt+V` | 粘贴文本或剪贴板截图 |
| `Ctrl+R` | 搜索历史提示词 |
| `Ctrl+O` | 打开全屏草稿编辑器 |
| `Esc` | 返回浏览模式 |

## 常用斜杠命令

`/help` `/new` `/resume` `/sessions` `/models` `/agent` `/settings` `/preferences` `/project` `/generate` `/continue` `/details` `/copy`
"#
        }
        (3, EffectiveUiLocale::En) => {
            r#"# AI Chat

Multi-conversation AI workspace supporting Native models and ACP agents, collapsible thinking/tool blocks, follow-up queuing, and image attachments.

## Normal Mode

| Key | Action |
|---|---|
| `i` / `Enter` | Focus the composer input |
| `j/k` / `↑/↓` | Scroll transcript vertically |
| `Ctrl+U` / `Ctrl+D` | Half-page scroll up / down |
| `h` / `Ctrl+W h` | Move focus to the Chats sidebar |
| `Ctrl+G` | Show / hide Chats sidebar |
| `[` / `]` | Previous / next conversation |
| `a` / `x` | New conversation / close current |
| `m` | Open Model Profiles |

## Input Mode

| Key | Action |
|---|---|
| `Enter` | Send message (or queue while agent is running) |
| `Shift+Enter` / `Ctrl+J` | Insert newline |
| `Ctrl+V` / `Alt+V` | Paste text or attach clipboard image |
| `Ctrl+R` | Search persisted prompt history |
| `Ctrl+O` | Open fullscreen draft editor |
| `Esc` | Return to Normal mode |

## Popular Slash Commands

`/help` `/new` `/resume` `/sessions` `/models` `/agent` `/settings` `/preferences` `/project` `/generate` `/continue` `/details` `/copy`
"#
        }
        (4, EffectiveUiLocale::ZhCn) => {
            r#"# 需求文档 (Requirements)

三栏式需求管理与 Markdown 编辑：左侧 **目录树**、右上 **文档预览/源码**、右下 **关联测试点**。

## 目录树快捷键

| 快捷键 | 作用 |
|---|---|
| `j/k` / `↑/↓` | 上下选择目录或需求文档 |
| `h/l` / `←/→` | 折叠/展开目录；在文档行按 `l` 聚焦文档区 |
| `/` | 搜索需求标题、路径与 ID（支持点击 `✓` / `✗`） |
| `Space` | 打开当前行的操作菜单（目录、标签、筛选等） |
| `Ctrl+N` | 新建需求文档 |
| `i` / `o` / `g` / `H` | 迭代筛选 / 项目范围 / 视图切换 / 隐藏已完成 |
| `I` / `p` / `d` | 编辑迭代 / 编辑优先级 / 删除文档 |

## 预览与 Vim 源码编辑

| 快捷键 | 作用 |
|---|---|
| `e` / `i` | 从预览进入源码浏览（Browse）/ 直接插入（Insert） |
| `hjkl`、`w/b/e`、`0/^/$`、`gg/G` | Browse 模式光标移动 |
| `v` / `V` | 字符可视 / 行可视选中，按 `n` 从选中内容创建测试点 |
| `o/O`、`dd`、`yy`、`p/P`、`u`/`Ctrl+R` | 开行 / 删除 / 复制 / 粘贴 / 撤销重做 |
| `Ctrl+S` / `Esc` | 保存 / 逐级退回（Insert → Browse → Preview） |
"#
        }
        (4, EffectiveUiLocale::En) => {
            r#"# Requirements

Three-pane requirement library and Markdown editor: **Tree** on the left, **Preview / Source** top-right, and **Linked Test Points** bottom-right.

## Tree Shortcuts

| Key | Action |
|---|---|
| `j/k` / `↑/↓` | Navigate folders and documents |
| `h/l` / `←/→` | Collapse/expand folders; `l` on a document focuses the doc pane |
| `/` | Search titles, relative paths, and IDs (click `✓` / `✗` to confirm/clear) |
| `Space` | Open action menu (folders, tags, queries, membership) |
| `Ctrl+N` | New requirement document |
| `i` / `o` / `g` / `H` | Iteration filter / project scope / grouping view / toggle completed |
| `I` / `p` / `d` | Edit iteration / priority / delete document |

## Preview & Vim Markdown Editor

| Key | Action |
|---|---|
| `e` / `i` | Enter Browse source view / start Insert at cursor |
| `hjkl`, `w/b/e`, `0/^/$`, `gg/G` | Cursor motion in Browse mode |
| `v` / `V` | Character / line visual selection; press `n` to create a test point |
| `o/O`, `dd`, `yy`, `p/P`, `u`/`Ctrl+R` | Open line / delete / yank / paste / undo & redo |
| `Ctrl+S` / `Esc` | Save / step back (`Insert → Browse → Preview`) |
"#
        }
        (5, EffectiveUiLocale::ZhCn) => {
            r#"# 测试点思维导图 (Test Points)

左侧 **72% 2D 思维导图画布** 呈现测试点层级、业务覆盖与实时运行状态（`○ 未覆盖` / `◐ 已关联` / `⠋ 运行中` / `✔ 通过` / `✖ 失败`）；右侧 **28% 联动详情栏** 实时展示关联的 Gherkin 场景、需求来源、运行结果与精简备注。

## 常用快捷键

| 快捷键 | 作用 |
|---|---|
| `h/j/k/l` 或 `←↓↑→` | 画布内按空间位置在父/子/兄弟节点间导航；在右侧详情区移动字段/条目 |
| `Tab` / `Shift+Tab` / `Ctrl+W h/l` | 在 2D 导图画布、右侧字段与需求来源链接区之间切换焦点 |
| `Space` | 折叠 / 展开当前分支子树 |
| `a` | 新增子测试点并原地编辑（输入含 `/` 时自动拆分为多级分支） |
| `n` / `O` | 新增同级测试点并原地编辑 |
| `i` / `e` / `Enter` | 原地编辑当前节点标题、分类分支名或右侧激活字段；在需求链接区跳转到需求 |
| `d` / `Delete` | 删除选中的测试点或整棵分类子树 |
| `u` / `Ctrl+R` | 撤销 / 重做（最多 50 步历史） |
| `g` | 为当前测试点生成/绑定带 `@teshi-tp:<id>` 的 Gherkin 场景并打开编辑器 |
| `S` | 从项目内未绑定的 `.feature` 场景反向提取并同步为测试点 |
| `r` | 直接运行选中节点或其子树关联的所有 Scenario 并刷新状态 |
| `o` | 跳转到 Explore 视图对应 Gherkin 场景（也可双击场景行） |
| `f` | 循环切换状态筛选器（全部 / 未覆盖 / 已关联 / 通过 / 失败） |
| `z` | 切换 2D 思维导图全屏视图 |
| `s` / `Ctrl+S` | 保存 `testpoints/testpoints.json` |
| 拖拽 / 滚轮 / Shift+滚轮 | 上下与水平平移画布；单击选中节点，双击折叠分支或编辑叶子 |
"#
        }
        (5, EffectiveUiLocale::En) => {
            r#"# Test Points MindMap

Left **72% 2D MindMap canvas** displays test-point hierarchy alongside live coverage & execution status (`○ Uncovered` / `◐ Linked` / `⠋ Running` / `✔ Passed` / `✖ Failed`); right **28% Linked Inspector** syncs associated Gherkin scenarios, requirement sources, run results, and concise notes.

## Shortcuts

| Key | Action |
|---|---|
| `h/j/k/l` or Arrows | Navigate spatially across canvas nodes (parent, child, siblings) or Inspector items |
| `Tab` / `Shift+Tab` / `Ctrl+W h/l` | Cycle focus between Canvas, Inspector Fields, and Requirement Links |
| `Space` | Collapse or expand the selected branch |
| `a` | Add child test point and edit inline (using `/` auto-splits nested branches) |
| `n` / `O` | Add sibling test point and edit inline |
| `i` / `e` / `Enter` | Edit node title or branch inline (or active Inspector field; follow link in Requirements) |
| `d` / `Delete` | Delete the selected test point or branch subtree |
| `u` / `Ctrl+R` | Undo / redo Test Points edits (up to 50 steps) |
| `g` | Scaffold and link a `@teshi-tp:<id>` Scenario in a matching `.feature` file |
| `S` | Reverse-extract unlinked `.feature` Scenarios into the MindMap |
| `r` | Run all Scenarios linked to the selected node or subtree |
| `o` | Open linked Gherkin Scenario in Explore (or double-click scenario row) |
| `f` | Cycle live status filter (All / Uncovered / Linked / Passed / Failed) |
| `z` | Toggle fullscreen MindMap canvas |
| `s` / `Ctrl+S` | Save `testpoints/testpoints.json` |
| Drag / Wheel / Shift+Wheel | Pan the 2D canvas vertically & horizontally; click selects, double-click folds branch or edits leaf |
"#
        }
        (6, EffectiveUiLocale::ZhCn) => {
            r#"# Vim 导航与 Ex 命令行

## 全标签页统一的 Vim 动作

| 快捷键 | 作用 |
|---|---|
| `hjkl` | 左 / 下 / 上 / 右导航 |
| `Ctrl+W` + `h/j/k/l` | 向相邻可见面板移动焦点 |
| `Ctrl+U` / `Ctrl+D` | 半页向上 / 向下滚动 |
| `Ctrl+B` / `Ctrl+F` | 整页向上 / 向下翻动 |
| `gg` / `G` | 跳到首项 / 末项 |
| `gt` / `gT` | 下一个 / 上一个标签页 |
| `?` / `F1` | 打开本帮助指南 |

## `:` Ex 命令行（支持 `Tab` 补全）

在任意普通模式下按 `:` 即可在底部输入命令，按 `Tab` 自动补全：

| 命令 | 作用 |
|---|---|
| `:q` / `:quit` | 安全退出 Teshi |
| `:w` / `:write` | 保存当前编辑器内容 |
| `:wq` / `:x` | 保存后退出 |
| `:h` / `:help` | 打开使用指南 |
| `:set` / `:settings` / `:pref` | 打开应用设置 |
| `:models` / `:sessions` / `:agent` | 打开模型 / 会话 / Agent 管理面板 |
| `:project` | 打开项目切换器 |
| `:1`–`:5` 或 `:explore`/`:map`/`:ai`/`:req`/`:tp` | 快速切换对应标签页 |
"#
        }
        (6, EffectiveUiLocale::En) => {
            r#"# Vim Navigation & Ex Command Line

## Unified Vim Motions Across All Tabs

| Key | Action |
|---|---|
| `hjkl` | Left / down / up / right navigation |
| `Ctrl+W` + `h/j/k/l` | Move focus to adjacent visible pane |
| `Ctrl+U` / `Ctrl+D` | Scroll half-page up / down |
| `Ctrl+B` / `Ctrl+F` | Scroll full-page up / down |
| `gg` / `G` | Jump to first / last item |
| `gt` / `gT` | Next / previous tab |
| `?` / `F1` | Open this User Guide |

## `:` Ex Command Line (with `Tab` Autocomplete)

Press `:` in any Normal/Browse mode to enter an Ex command, and press `Tab` to autocomplete:

| Command | Action |
|---|---|
| `:q` / `:quit` | Guarded quit |
| `:w` / `:write` | Save active buffer |
| `:wq` / `:x` | Save and quit |
| `:h` / `:help` | Open User Guide |
| `:set` / `:settings` / `:pref` | Open App Settings |
| `:models` / `:sessions` / `:agent` | Open Models / Sessions / Agent panel |
| `:project` | Open Project Picker |
| `:1`–`:5` or `:explore`/`:map`/`:ai`/`:req`/`:tp` | Switch active tab |
"#
        }
        (7, EffectiveUiLocale::ZhCn) => {
            r#"# 鼠标与 GUI 操作

Teshi 所有核心工作流均可完全通过鼠标完成，悬停任意可交互控件时会自动提亮反馈。

## 点击与双击

- **顶栏标签与项目芯片**：点击标签切换视图；点击右上 `Project:` 芯片切换项目。
- **目录树**：在 Explore 和 Requirements 中，**单击目录即可展开/折叠**。
- **双击快捷操作**：
  - 双击 Explore 场景行 → 进入源码编辑
  - 双击 Run Details 项 → 全屏查看截图 / 展开接口体 / 打开文件
  - 双击 MindMap 预览区 → 打开场景位置下拉框
  - 双击 Test Points 画布节点 → 折叠/展开分支或原地编辑叶子；双击需求链接或关联场景 → 直接跳转对应需求或 Gherkin
  - 双击弹窗列表项（会话 / 模型 / Agent / 关键字 / 变更摘要）→ 直接加载或应用
- **输入框光标定位**：点击 AI 输入框、Test Points 字段或模型表单字段，可将光标精准放到点击位置；再次点击或双击 Test Points 字段直接进入编辑。
- **标题栏搜索**：点击标题栏即可开始搜索，右侧提供绿色 **`✓`** 确认与橙色 **`✗`** 清空按钮。

## 滚动条与弹窗

- **全页面纵向滚动条**：滚轮按鼠标所在面板独立滚动；可直接拖拽任意面板右侧滑块，或点击轨道翻页。
- **弹窗遮罩关闭**：点击设置、选择器、帮助指南等非破坏性弹窗外部的暗色背景即可关闭。
"#
        }
        (_, _) => {
            r#"# Mouse & GUI Controls

Every major Teshi workflow works smoothly with the mouse, with live hover tinting on all interactive controls.

## Click & Double-Click

- **Top bar**: Click tabs to switch views; click the right-hand `Project:` chip to switch projects.
- **Folder trees**: Single-clicking a folder in both Explore and Requirements toggles expand/collapse immediately.
- **Double-click actions**:
  - Double-click an Explore scenario → open Feature source editor
  - Double-click a Run Details item → fullscreen screenshot / expand exchange / open artifact
  - Double-click the MindMap preview → open scenario location dropdown
  - Double-click a Test Points canvas node → fold/expand branch or edit leaf inline; double-click a link or scenario → jump directly to Requirements or Gherkin
  - Double-click modal list rows (Sessions / Models / Agents / Keywords / Change Summary) → load or apply
- **Direct input carets**: Click inside AI Chat input, Test Points fields, or Model form fields to place the caret at the clicked character; click a selected Test Points field again (or double-click) to start typing.
- **Title-bar search**: Click the search title to start typing, then use the green **`✓`** button to confirm or amber **`✗`** button to clear.

## Scrollbars & Modals

- **Scrollbars everywhere**: Mouse wheel targets the pane under the pointer; drag any vertical or horizontal scrollbar thumb or click its track to page.
- **Backdrop dismiss**: Click the dimmed backdrop outside any non-destructive picker, settings card, or this User Guide to close it.
"#
        }
    }
}
