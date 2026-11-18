//! Lightweight TUI string catalogs keyed by the effective UI locale.
//!
//! English strings live in source as the fallback. The Chinese catalog is a
//! static table consulted only when the effective locale is `zh-CN`. Keys that
//! are missing from a catalog fall back to English rather than failing render.

use std::cell::Cell;

use teshi_engine::{EffectiveUiLocale, UiLocalePreference};

thread_local! {
    /// Effective locale for the thread that paints the TUI.
    ///
    /// A process-wide value would leak between parallel tests and between
    /// independent hosts: `set_ui_locale` runs on the event-loop thread, which
    /// is also the thread that calls [`t`] while rendering. Threads that never
    /// install a locale read English.
    static EFFECTIVE: Cell<EffectiveUiLocale> = const { Cell::new(EffectiveUiLocale::En) };
}

/// Install the effective locale used by subsequent [`t`] lookups on this thread.
pub fn set_effective_locale(locale: EffectiveUiLocale) {
    EFFECTIVE.with(|cell| cell.set(locale));
}

/// Current effective locale for this thread (English until installed).
pub fn effective_locale() -> EffectiveUiLocale {
    EFFECTIVE.with(Cell::get)
}

/// Resolve `preference` against the host OS and install it for [`t`].
pub fn apply_preference(preference: UiLocalePreference) {
    let system = sys_locale::get_locale();
    set_effective_locale(teshi_engine::resolve_effective_ui_locale(
        preference,
        system.as_deref(),
    ));
}

/// Look up a UI string. Unknown keys return `key` so typos stay visible.
pub fn t(key: &str) -> &str {
    let locale = effective_locale();
    match locale {
        EffectiveUiLocale::ZhCn => zh_cn(key).unwrap_or_else(|| en(key).unwrap_or(key)),
        EffectiveUiLocale::En => en(key).unwrap_or(key),
    }
}

fn en(key: &str) -> Option<&'static str> {
    Some(match key {
        "tab.explore" => " Explore [1] ",
        "tab.mindmap" => " MindMap [2] ",
        "tab.ai" => " AI [3] ",
        "tab.requirements" => " Requirements [4] ",
        "tab.test_points" => " Test Points [5] ",
        "tab.explore.compact" => "Explore [1]",
        "tab.mindmap.compact" => "Map [2]",
        "tab.ai.compact" => "AI [3]",
        "tab.requirements.compact" => "Req [4]",
        "tab.test_points.compact" => "Points [5]",
        "settings.title" => " Settings ",
        "settings.tab.general" => " General ",
        "settings.subtitle" => "Control what Teshi adds to every AI conversation.",
        "settings.section.ai_chat" => " AI Chat ",
        "settings.section.language" => " Language ",
        "settings.inject_prompt" => "Inject Teshi prompt context",
        "settings.inject_prompt.hint" => {
            "Adds project context, the authoring pipeline, generation status, \
             exploration, and ACP tool guidance to every AI conversation. Off sends \
             the agent profile prompt only."
        }
        "settings.ui_locale" => "UI Language",
        "settings.ui_locale.hint" => {
            "TUI labels and the default features/<locale>/ directory. Switching \
             language retargets that directory when the project uses locale \
             folders. Does not rewrite Gherkin keywords inside .feature files."
        }
        "settings.locale.system" => "System",
        "settings.locale.en" => "English",
        "settings.locale.zh_cn" => "简体中文",
        "settings.status.open" => "Settings: ↑↓ move · Enter/Space toggle or change · Esc close",
        "settings.status.inject_on" => "System-prompt sections: On",
        "settings.status.inject_off" => "System-prompt sections: Off (agent profile prompt only)",
        "settings.status.locale" => "UI language: {value}",
        "settings.status.locale_dir_switched" => "Feature directory: {path}",
        "settings.status.locale_dir_missing" => "No Feature directory matches the UI language",
        "settings.status.locale_dir_dirty" => {
            "UI language saved; save or discard Feature edits to switch directories"
        }
        "settings.close_btn" => " Close ",
        "settings.autosave" => " Changes are saved as you make them ",
        "footer.exit" => "Exit",
        "footer.settings" => " Settings",
        "footer.sessions" => " Sessions",
        "footer.agents" => " Agents",
        "footer.models" => " Models m",
        "footer.save" => " Save",
        "footer.save_key" => " Save [s]",
        "footer.project" => " Project: {name} ",
        "footer.explore.focus" => " Focus [Tab/←→] ",
        "footer.explore.navigate" => " Navigate [↑↓] ",
        "footer.explore.edit" => " Edit [e/→] ",
        "footer.explore.detail" => " Detail [Enter] ",
        "footer.explore.run" => " Run [r] ",
        "footer.explore.ai" => " AI [a] ",
        "footer.mindmap.navigate" => " Navigate [↑↓] ",
        "footer.mindmap.toggle" => " Toggle [Space] ",
        "footer.mindmap.expand" => " Expand [→] ",
        "footer.mindmap.collapse" => " Collapse [←] ",
        "footer.mindmap.focus_panel" => " Focus Panel [Enter] ",
        "footer.mindmap.toggle_panel" => " Toggle Panel [p] ",
        "footer.mindmap.show_panel" => " Show Panel [p] ",
        "footer.ai.input" => "[INPUT] Enter send · Shift+Enter newline · Esc normal · Ctrl+v paste",
        "footer.ai.normal_sidebar" => {
            "[NORMAL] i/Enter input · h chats · j/k/l/Arrows scroll · [/] agent · a new · x close"
        }
        "footer.ai.normal" => {
            "[NORMAL] i/Enter input · hjkl/Arrows scroll · [/] agent · a new · x close"
        }
        // ── Mode badges ─────────────────────────────────────
        "mode.preview" => "[PREVIEW]",
        "mode.browse" => "[BROWSE]",
        "mode.insert" => "[INSERT]",
        "mode.insert.bar" => "[INSERT] ",
        "mode.visual" => "[VISUAL]",
        "mode.visual_line" => "[V-LINE]",
        "mode.normal" => "[NORMAL] ",
        "mode.normal.edit" => "[NORMAL] i edit ",
        "mode.input" => "[INPUT] ",
        // ── Shared action-bar labels ────────────────────────
        "action.model" => " Model",
        "action.select" => " Select",
        "action.dismiss" => " Dismiss",
        // ── Test Points action bar ──────────────────────────
        "tp.insert.save" => "Save [Ctrl+s] ",
        "tp.insert.normal" => "Normal [Esc]",
        "tp.approve" => " Approve",
        "tp.approve.key" => " Approve [a]",
        "tp.reject" => " Reject",
        "tp.reject.key" => " Reject [r]",
        "tp.batch" => " Batch",
        "tp.batch.key" => " Batch [A]",
        "tp.continue" => " Continue",
        "tp.continue.key" => " Continue [c]",
        "tp.filter" => " Filter",
        "tp.filter.key" => " Filter [f]",
        // ── Explore footer ──────────────────────────────────
        "explore.run" => " Run",
        "explore.run.key" => " Run [r]",
        "explore.detail" => " Detail",
        "explore.detail.key" => " Detail [Enter]",
        "explore.edit" => " Edit",
        "explore.edit.key" => " Edit [e]",
        "explore.summary" => "{passed}/{total} passed, {failed} failed",
        "explore.summary.blocked" => "{passed}/{total} passed, {failed} failed, {blocked} blocked",
        "explore.summary.idle" => "0/{total} passed",
        // ── Editor and MindMap AI footer hints ──────────────
        "editor.hint.insert" => "[INSERT] Enter commit · Esc normal · Ctrl+s save · Arrows move",
        "editor.hint.edit" => "[NORMAL] Edit [i/Enter] ",
        "editor.hint.keyword" => " Given/Then/And [Ctrl+g/t/a] ",
        "editor.hint.step" => " Step [o/O] ",
        "editor.hint.delete" => " Delete [dd] ",
        "editor.hint.copy_paste" => " Copy/Paste [yy/p] ",
        "editor.hint.move_step" => " MoveStep [Ctrl+j/k] ",
        "editor.hint.fold" => " Fold [Space] ",
        "editor.hint.fold_all" => " FoldAll [Ctrl+Space] ",
        "editor.hint.save" => " Save [Ctrl+s] ",
        "mindmap.ai.hint.send" => "[INPUT] Send [Enter] ",
        "mindmap.ai.hint.back" => " Back/Browse [Esc] ",
        "mindmap.ai.hint.newline" => " Newline [Shift+Enter] ",
        "mindmap.ai.hint.return_tree" => "[NORMAL] Return Tree [Esc] ",
        "mindmap.ai.hint.input" => " Input [i/Enter] ",
        "mindmap.ai.hint.scroll" => " Scroll [hjkl/Arrows] ",
        // ── Confirmation dialogs ────────────────────────────
        "dialog.close.title" => "Confirm close",
        "dialog.close.verb" => "Close",
        "dialog.close.body" => "Its history stays available in the session list.",
        "dialog.close.confirm" => "Close [Enter/y]",
        "dialog.delete.title" => "Confirm deletion",
        "dialog.delete.verb" => "Delete",
        "dialog.delete.body" => "This cannot be undone.",
        "dialog.delete.confirm" => "Delete [Enter/y]",
        "dialog.cancel" => "Cancel [Esc/n]",
        "dialog.cancel.esc" => "Cancel [Esc]",
        "dialog.close.esc" => "Close [Esc]",
        "dialog.error" => "Could not {action}: {error}",
        "dialog.quit.title" => "Quit Teshi?",
        "dialog.quit.body" => "Quit Teshi and return to the shell?",
        "dialog.quit.body_dirty" => "You have unsaved changes, quit anyway?",
        "dialog.quit.yes" => "[Yes]",
        "dialog.quit.no" => "[No]",
        "dialog.interrupt.title" => "Interrupt the current run?",
        "dialog.interrupt.body" => "The turn ends; the session and its answer are kept.",
        "dialog.interrupt.hint" => "Enter/y interrupt · Esc/n keep running",
        "dialog.interrupt.confirm" => "[Interrupt]",
        "dialog.interrupt.continue" => "[Continue]",
        "approval.title" => " Approval Mode ",
        "approval.title_override" => " Approvals (environment override: Auto) ",
        // ── External change and change summary overlays ─────
        "external_change.title" => "Feature changed on disk",
        "external_change.body" => "Detected external updates for {file}.",
        "external_change.reload" => "Reload [Enter] ",
        "external_change.keep" => "Keep [Esc]",
        "change_summary.title" => "Change Summary",
        "change_summary.jump" => "Jump [Enter] ",
        // ── Requirements action bar ─────────────────────────
        "req.edit" => " Edit",
        "req.edit.key" => " Edit [e]",
        "req.preview" => " Preview",
        "req.preview.esc" => " Preview [Esc]",
        "req.actions" => " Actions",
        "req.actions.space" => " Actions [Space]",
        "req.new" => " New",
        "req.new.doc.key" => " New doc [n]",
        "req.new.doc.ctrl" => " New doc [Ctrl+n]",
        "req.point" => " Point",
        "req.point.key" => " Point [n]",
        "req.filter" => " Filter",
        "req.filter.key" => " Filter [i]",
        "req.iteration" => " Iteration",
        "req.iteration.key" => " Iteration [I]",
        "req.priority" => " Priority",
        "req.priority.key" => " Priority [Ctrl+p]",
        "req.scope" => " Scope",
        "req.scope.key" => " Scope [o]",
        "req.view" => " View",
        "req.view.key" => " View [g]",
        "req.completed" => " Completed",
        "req.completed.key" => " Completed [H]",
        "req.reload" => " Reload",
        "req.reload.key" => " Reload [R]",
        "req.save.ctrl" => " Save [Ctrl+s]",
        // ── Slash command descriptions ──────────────────────
        "command.new" => "Start a new session",
        "command.exit" => "Exit Teshi",
        "command.resume" => "Resume the most recent session",
        "command.retry" => "Continue an interrupted ACP task",
        "command.copy" => "Copy last N assistant responses to clipboard",
        "command.details" => "Expand or collapse thinking and tool details",
        "command.unqueue" => "Restore the latest queued message for editing",
        "command.rename" => "Rename this conversation (/rename --clear to reset)",
        "command.models" => "Open model settings",
        "command.sessions" => "Browse saved sessions",
        "command.project" => "Open or switch the active project",
        "command.worktree" => "Create a git worktree and work in it from this chat",
        "command.approval" => "Agent approval settings (/approval local for Teshi policy)",
        "command.agent" => "Select session Agent (Native or installed ACP)",
        "command.acp" => "Manage installed ACP Agents",
        "command.settings" => "Select ACP model, mode, thinking and other settings",
        "command.preferences" => "Teshi settings: UI language and AI prompt sections",
        "command.agent-profile" => "Select Native agent profile",
        "command.generate" => "Start requirements → test points → scenarios generation",
        "command.continue" => "Continue generation after approving test points",
        // ── Explore and MindMap content panes ───────────────
        "features.title.search" => "Features [/] Search",
        "features.title.count" => "Features ({count}) [/] File search",
        "features.title.prefix" => "Features ",
        "features.loading" => "Loading...",
        "features.empty.scanning" => "Scanning Feature files...",
        "features.empty.none" => "(no features)",
        "features.empty.indexing" => "Indexing tags...",
        "features.empty.tags_search" => "No files match both tags and search",
        "features.empty.tags" => "No files match the selected tags",
        "features.empty.search" => "No files match the search",
        "scenarios.title" => "Scenarios ({count})",
        "scenarios.empty.select" => "Select a Feature; content loads on demand",
        "scenarios.deleted" => " (deleted Feature; see Git diff)",
        "pane.feature_content" => "Feature content",
        "steps.pane" => "Steps",
        "steps.empty" => " (no steps)",
        "explore.feature" => " Feature",
        "explore.background" => " Background",
        "explore.background_label" => " Background:",
        "explore.examples" => " Examples:",
        "mindmap.pane.current" => "MindMap [current Feature]",
        "mindmap.pane" => "MindMap",
        "mindmap.filtered" => "[filtered]",
        "mindmap.highlighted" => "[highlighted]",
        "mindmap.budget" => {
            "This Feature exceeds the interactive MindMap budget. Its scenarios \
             and steps remain available in Explore."
        }
        "preview.pane" => "Preview",
        // ── Session browser ─────────────────────────────────
        "sessions.title" => " Sessions · {project} · {count} ",
        "sessions.title.legacy" => " Legacy / Unassigned · {count} ",
        "sessions.hint.legacy" => " Loading keeps history unassigned.",
        "sessions.hint.scope" => " Active project only. /sessions legacy for old history.",
        "sessions.empty" => "  No saved sessions.",
        "sessions.empty.hint" => "  Type /new to start a session that will be saved.",
        "sessions.row" => " {marker} {title}  ·  {count} msgs  ·  {size}",
        "sessions.row.meta" => "     {dir}  ·  {age}  ·  {model}",
        "sessions.unassigned" => "unassigned",
        "sessions.load" => "Load [Enter] ",
        "sessions.delete" => "Delete [dd/Del] ",
        "age.just_now" => "just now",
        "age.minutes" => "{n}m ago",
        "age.hours" => "{n}h ago",
        "age.days" => "{n}d ago",
        // ── Test Points tab ─────────────────────────────────
        "tp.tree.title" => "Test points ({filter})",
        "tp.filter.all" => "all",
        "tp.filter.proposed" => "proposed",
        "tp.filter.approved" => "approved",
        "tp.filter.rejected" => "rejected",
        "tp.filter.needs_review" => "needs review",
        "tp.tree.empty" => "No test points yet. Create links from Requirements [4].",
        "tp.review.proposed" => "Proposed",
        "tp.review.approved" => "Approved",
        "tp.review.rejected" => "Rejected",
        "tp.review.needs_review" => "Needs review",
        "tp.details.title.edit" => "Intent & review [INSERT]",
        "tp.details.title" => "Intent & review [NORMAL]",
        "tp.details.empty" => "Select a test point in the tree.",
        "tp.details.unavailable" => "Selected test point is unavailable.",
        "tp.field.title" => "Title",
        "tp.field.objective" => "Objective",
        "tp.field.preconditions" => "Preconditions",
        "tp.field.expected" => "Expected",
        "tp.field.hierarchy" => "Hierarchy",
        "tp.realized" => "Realized scenarios [o]:",
        "tp.line" => "line {n}",
        "tp.excerpts.title" => "Requirement excerpts",
        "tp.excerpts.empty" => " (no linked excerpts)",
        "tp.excerpt.ok" => "ok",
        "tp.excerpt.stale" => "stale",
        // ── Model profiles ──────────────────────────────────
        "model.panel.title" => " Model Profiles ",
        "model.panel.add" => " Add Model Profile ",
        "model.panel.edit" => " Edit Model Profile ",
        "model.empty" => "  No model profiles found.",
        "model.empty.hint" => "  Press 'a' to add one, or place a TOML file in",
        "model.active" => "  [ACTIVE]",
        "model.add" => "Add [a] ",
        "model.edit" => "Edit [e] ",
        "model.delete" => "Del [dd/Del] ",
        "model.use" => "Use [Enter] ",
        "model.form.save" => "Save [Enter] ",
        "model.form.type_here" => "(type here...)",
        "model.form.empty" => "(empty)",
        "model.field.name" => "Name",
        "model.field.provider" => "Provider",
        "model.field.model" => "Model",
        "model.field.base_url" => "Base URL",
        "model.field.api_key" => "API Key",
        "model.field.max_tokens" => "Max Tokens",
        "model.field.temperature" => "Temperature",
        // ── Auth overlay ────────────────────────────────────
        "auth.title" => " Auth Management ",
        "auth.add.title" => "Add Provider",
        "auth.add.hint" => "Use 'teshi auth login' from the CLI to add a provider API key.",
        "auth.remove.title" => "Remove Provider: {provider}",
        "auth.remove.hint" => "Use 'teshi auth remove <provider>' from the CLI to remove.",
        "auth.overview.title" => "Model Profiles (shared store)",
        "auth.empty" => "  No model profiles configured.",
        "auth.empty.hint" => "  Use 'teshi auth login' or press m to add one.",
        "auth.active_id" => "  Active id: ",
        "auth.key" => " key: ",
        // ── Project picker and switch overlays ──────────────
        "project.recent.empty" => "No recent projects",
        "project.picker.hint" => "[O] Open path · ↑/↓ select · Enter open · Esc cancel",
        "project.open" => " Open project ",
        "project.open_path" => " Open project path ",
        "project.open_path.prompt" => "Type a project directory and press Enter:",
        "project.open_path.hint" => "Enter open · Backspace edit · Esc cancel",
        "project.dirty.title" => " Resolve project state ",
        "project.dirty.body" => "Unsaved project state must be resolved first.",
        "project.dirty.hint" => "[S] Save and switch   [D] Discard and switch   [Esc] Cancel",
        "project.busy.title" => " Stop project tasks ",
        "project.busy.body" => "A project-owned task is still running.",
        "project.busy.hint" => "[S] Stop tasks and switch   [Esc] Cancel",
        "project.busy.wait" => "Wait for owned tasks to exit before switching.",
        "project.switch.to" => "Switch to {path}",
        // ── Requirements tree and editor ────────────────────
        "req.scope.all" => "All projects",
        "req.scope.none" => "No project",
        "req.scope.current" => "Current project",
        "req.view.iteration" => "Iteration",
        "req.view.folders" => "Folders",
        "req.view.path" => "Path",
        "req.iteration.all" => "All",
        "req.iteration.unassigned" => "Unassigned",
        "req.states.all" => " · All states",
        "req.states.pending" => " · Pending only",
        "req.tree.title" => "{status} · [/] Search",
        "req.tree.status" => "{scope} · {view} · Iteration: {iteration}",
        "req.tree.identity_error" => "Project identity: {error}",
        "req.tree.none" => {
            "No requirement artifacts. Ctrl+n to create. Scope [o]: all / no project."
        }
        "req.tree.no_match" => {
            "No requirements match /{query}\nEsc: clear search   Enter: keep filter"
        }
        "req.tree.empty" => {
            "No requirements in this scope.\no: change scope   b: link selected\n\
             Ctrl+n: create for current project"
        }
        "req.editor.title" => "Requirement",
        "req.source.title" => "Source",
        "req.missing_suffix" => " (missing file)",
        "req.missing_file" => "Indexed document file is missing on disk.",
        "req.priority.title" => " Priority: {value} ",
        "req.priority.unset" => "Unset",
        "req.linked.title" => "Linked test points",
        "req.status.planned" => "[planned]",
        "req.status.implemented" => "[implemented]",
        "req.status.verified" => "[verified]",
        // ── Agent panels ────────────────────────────────────
        "agent.panel.select" => " Select Agent for this session ",
        "agent.panel.installed" => " Installed ACP Agents ({count}) ",
        "agent.panel.registry" => " Official ACP Registry ({count} agents) ",
        "agent.panel.title" => " Agents ",
        "agent.install" => "Install [Enter] ",
        "agent.use" => "Use [Enter] ",
        "agent.registry.browse" => "Registry [i] ",
        "agent.native" => "Teshi Native",
        "agent.selected" => " [selected]",
        "agent.installed" => "[installed]",
        "agent.available" => "[available]",
        "agent.no_description" => "No description",
        "agent.registry.loading" => "Loading official ACP Registry...",
        "agent.registry.empty" => "No agents published in the registry",
        "agent.installed.empty" => "No ACP agents installed · press i to browse the Registry",
        "agent.empty" => "No agents available",
        "agent.config.title" => " Agent Configuration ",
        "agent.apply" => "Apply [Enter] ",
        "agent.profile.no_desc" => "(no description)",
        "agent.profile.none" => "  No profiles",
        "agent.profile.empty" => "No profile selected",
        "agent.profile.section" => "Profile",
        "agent.profile.name" => "  Name: ",
        "agent.profile.description" => "  Description: ",
        "agent.profile.empty_value" => "(empty)",
        "agent.prompt.section" => "System Prompt (preview)",
        "agent.config.section" => "Configuration",
        "agent.tools.all" => "all tools",
        "agent.tools.none" => "none",
        "agent.tools.enabled" => "{count} enabled",
        "agent.tools.label" => "  Tools: ",
        "agent.skills.hint" => "(configured in agent.yaml)",
        "agent.skills.label" => "  Skills: ",
        "agent.model.active" => "(active model profile)",
        "agent.model.label" => "  Model: ",
        "agent.apply_hint" => "Enter apply · Esc close",
        // ── Chat panel and composer ─────────────────────────
        "chat.panel.title" => " AI Chat ",
        "chat.session.prefix" => "· Session: ",
        "chat.agent.suffix" => "· Agent: {name} ",
        "chat.chats_hidden" => "· chats hidden [Ctrl+g] ",
        "chat.greeting" => "Welcome to AI Chat! Type a message below and press Enter.",
        "chat.greeting.embedded" => "Welcome to AI Chat!",
        "chat.hint.preselected" => {
            "Agent preselected; not connected. Focus the input to start, or use /agent."
        }
        "chat.hint.no_agent" => {
            "No agent selected. Use /agent to choose, or focus the input to use Teshi Native."
        }
        "chat.hint.auth" => {
            "Note: Run 'teshi auth login' or set TESHI_LLM_API_KEY to enable AI responses."
        }
        "chat.ready" => "Ready",
        "chat.agent_native" => " Agent Native ▾ ",
        "chat.agent" => " Agent ▾ ",
        "chat.model" => " Model {model} ▾ ",
        "chat.model.select" => "Select",
        "chat.waiting" => "Waiting for agent…",
        "chat.activity" => "Activity",
        "chat.role.you" => "You",
        "chat.role.teshi" => "Teshi",
        "chat.queued" => "Queued {count}: ",
        "chat.queued.tail" => " · /unqueue to edit · {state}",
        "chat.queue.more" => "Enter queues another",
        "chat.queue.resume" => "send a message to resume",
        "chat.image.loading" => "Loading image…",
        "chat.image.error" => "Cannot show image: {reason}",
        "chat.image.scroll" => "Scroll to show the image",
        "chat.chats.title" => " Chats [Ctrl+g] ",
        "chat.new" => " New [a]",
        "chat.close" => " Close [x]",
        "chat.send" => " Send",
        "chat.send.key" => " Send [Enter]",
        "chat.focus" => " Focus",
        "chat.focus.key" => " Focus [i/Enter]",
        "chat.blur" => " Blur",
        "chat.blur.key" => " Blur [Esc]",
        "chat.model_chip" => " Model",
        "chat.model_chip.key" => " Model [m]",
        "chat.chats_chip" => " Chats",
        "chat.chats_chip.key" => " Chats [Ctrl+g]",
        "mode.chats" => "[CHATS] ",
        "chat.hint.chats_normal" => "Esc then j/k scroll",
        "chat.hint.chats_browse" => "j/k chat · Enter open · l back",
        "chat.hint.normal_sidebar" => {
            "h chats · j/k/l/Arrows scroll · g/G ends · Ctrl+D/U half · [/] chat"
        }
        "chat.hint.normal" => "hjkl/Arrows scroll · g/G ends · Ctrl+D/U half · [/] chat",
        "chat.pending.title" => " Pending Agent Change ",
        "chat.permission.title" => " ACP permission requested ",
        "chat.change.accept" => " Accept",
        "chat.change.reject" => " Reject",
        "chat.change.diff" => " Diff",
        "chat.change.summary" => " Summary",
        "chat.permission.allow" => " Allow [Enter]",
        "chat.permission.reject" => " Reject [Esc]",
        // ── Requirement overlays ────────────────────────────
        "req.overlay.unsaved.title" => "Unsaved requirement",
        "req.overlay.unsaved.body" => "{title} has unsaved changes.",
        "req.overlay.save" => "[S] Save",
        "req.overlay.discard" => "[D] Discard",
        "req.overlay.cancel" => "[Esc] Cancel",
        "req.overlay.select" => "Select [Enter]",
        "req.overlay.toggle" => "Toggle [Space]",
        "req.overlay.save_enter" => "Save [Enter]",
        "req.overlay.add" => "Add [Enter]",
        "req.overlay.confirm" => "Confirm [Enter]",
        "req.overlay.delete" => "Delete [Enter]",
        "req.overlay.color" => "Color [c]",
        "req.overlay.front" => "Front [f]",
        "req.overlay.next_field" => "Next field [Tab]",
        "req.overlay.field.name" => "Name",
        "req.overlay.field.condition" => "Condition",
        "req.overlay.filter.title" => "Filter iteration",
        "req.overlay.folders.title" => "Folders of {title}",
        "req.overlay.folders.none" => {
            "No folders yet. Close, select Inbox, and choose New folder…."
        }
        "req.overlay.folders.none_checked" => {
            "No folder checked: the requirement is listed in Inbox."
        }
        "req.overlay.folders.checked" => "The requirement is listed in every checked folder.",
        "req.overlay.tags.title" => "Tags of {title}",
        "req.overlay.tags.new" => " ＋ New tag…",
        "req.overlay.tags.new_buffer" => " New tag: {buffer}",
        "req.overlay.tags.none" => "No tags yet.",
        "req.overlay.tags.note" => {
            "Tags belong to the document, so every project sees the same set."
        }
        "req.overlay.tags.no_color" => "no color",
        "req.overlay.tags.before" => "before title",
        "req.overlay.tags.after" => "after title",
        "req.overlay.folder.new" => "New folder",
        "req.overlay.folder.new_child" => "New subfolder in {parent}",
        "req.overlay.folder.hint" => "Use / to create nested folders, e.g. Chat/Archive.",
        "req.overlay.rename.query" => "Rename saved query",
        "req.overlay.rename.folder" => "Rename folder",
        "req.overlay.rename.hint" => "Requirements and queries keep their references.",
        "req.overlay.query.matches_one" => "Matches 1 requirement",
        "req.overlay.query.matches" => "Matches {count} requirements",
        "req.overlay.query.error" => "! {error}",
        "req.overlay.query.help1" => "Fields: status, priority, iteration, title, folder",
        "req.overlay.query.help2" => "& = and, | = or, ! = not; & binds tighter than |",
        "req.overlay.query.help3" => "Example: status=planned & priority=P1 | folder=AI/Chat",
        "req.overlay.query.edit" => "Edit saved query",
        "req.overlay.query.new" => "New saved query",
        "req.overlay.query.delete.title" => "Delete saved query",
        "req.overlay.query.delete.body" => "Delete saved query '{name}'?",
        "req.overlay.query.delete.note" => "Requirements are not changed.",
        "req.overlay.folder.delete.title" => "Delete folder",
        "req.overlay.folder.delete.body" => "Delete folder '{name}' and its subfolders?",
        "req.overlay.folder.delete.none" => "No requirement returns to Inbox.",
        "req.overlay.folder.delete.one" => "1 requirement returns to Inbox.",
        "req.overlay.folder.delete.many" => "{n} requirements return to Inbox.",
        "req.overlay.folder.delete.note" => "Requirement documents are never deleted.",
        "req.overlay.requirement.delete.title" => "Delete requirement",
        "req.overlay.requirement.delete.body" => "Delete requirement '{title}' ({id})?",
        "req.overlay.requirement.delete.note1" => {
            "This removes it from the global requirement library."
        }
        "req.overlay.requirement.delete.note2" => {
            "Linked test points stay and become missing-source links."
        }
        "req.overlay.iteration.title" => "Set iteration",
        "req.overlay.iteration.field" => "Iteration",
        "req.overlay.iteration.hint" => "Leave it empty to mark the requirement Unassigned.",
        "req.overlay.priority.title" => "Requirement priority",
        "req.overlay.scope.title" => " Confirm generation source scope ",
        "req.overlay.scope.store" => "Requirement store: {path}",
        "req.overlay.scope.store_id" => "Store ID: {id}",
        "req.overlay.scope.iteration" => "Iteration scope: {filter}",
        "req.overlay.scope.project" => "Project scope: {scope}",
        "req.overlay.scope.confirm" => "Confirm [Enter] ",
        "req.overlay.scope.iteration_key" => "Iteration [i] ",
        "req.overlay.scope.uninitialized" => "(uninitialized)",
        _ => return None,
    })
}

fn zh_cn(key: &str) -> Option<&'static str> {
    Some(match key {
        "tab.explore" => " 探索 [1] ",
        "tab.mindmap" => " 思维导图 [2] ",
        "tab.ai" => " AI [3] ",
        "tab.requirements" => " 需求 [4] ",
        "tab.test_points" => " 测试点 [5] ",
        "tab.explore.compact" => "探索 [1]",
        "tab.mindmap.compact" => "导图 [2]",
        "tab.ai.compact" => "AI [3]",
        "tab.requirements.compact" => "需求 [4]",
        "tab.test_points.compact" => "测试点 [5]",
        "settings.title" => " 设置 ",
        "settings.tab.general" => " 通用 ",
        "settings.subtitle" => "控制 Teshi 向每次 AI 对话附加的内容。",
        "settings.section.ai_chat" => " AI 对话 ",
        "settings.section.language" => " 语言 ",
        "settings.inject_prompt" => "注入 Teshi 提示上下文",
        "settings.inject_prompt.hint" => {
            "向每次 AI 对话附加项目上下文、编写流水线、生成状态、探索说明与 ACP 工具指引。关闭后仅发送 Agent 配置提示词。"
        }
        "settings.ui_locale" => "界面语言",
        "settings.ui_locale.hint" => {
            "影响 TUI 文案，以及存在 features/<locale>/ 时默认打开的目录。切换语言会立即改换该目录。不会改写 .feature 文件中的 Gherkin 关键字。"
        }
        "settings.locale.system" => "跟随系统",
        "settings.locale.en" => "English",
        "settings.locale.zh_cn" => "简体中文",
        "settings.status.open" => "设置：↑↓ 移动 · Enter/Space 切换或更改 · Esc 关闭",
        "settings.status.inject_on" => "系统提示附加段：开",
        "settings.status.inject_off" => "系统提示附加段：关（仅 Agent 配置提示词）",
        "settings.status.locale" => "界面语言：{value}",
        "settings.status.locale_dir_switched" => "Feature 目录：{path}",
        "settings.status.locale_dir_missing" => "没有与界面语言匹配的 Feature 目录",
        "settings.status.locale_dir_dirty" => {
            "界面语言已保存；请先保存或放弃 Feature 编辑后再切换目录"
        }
        "settings.close_btn" => " 关闭 ",
        "settings.autosave" => " 更改会立即保存 ",
        "footer.exit" => "退出",
        "footer.settings" => " 设置",
        "footer.sessions" => " 会话",
        "footer.agents" => " Agents",
        "footer.models" => " 模型 m",
        "footer.save" => " 保存",
        "footer.save_key" => " 保存 [s]",
        "footer.project" => " 项目：{name} ",
        "footer.explore.focus" => " 焦点 [Tab/←→] ",
        "footer.explore.navigate" => " 导航 [↑↓] ",
        "footer.explore.edit" => " 编辑 [e/→] ",
        "footer.explore.detail" => " 详情 [Enter] ",
        "footer.explore.run" => " 运行 [r] ",
        "footer.explore.ai" => " AI [a] ",
        "footer.mindmap.navigate" => " 导航 [↑↓] ",
        "footer.mindmap.toggle" => " 展开/折叠 [Space] ",
        "footer.mindmap.expand" => " 展开 [→] ",
        "footer.mindmap.collapse" => " 折叠 [←] ",
        "footer.mindmap.focus_panel" => " 聚焦面板 [Enter] ",
        "footer.mindmap.toggle_panel" => " 切换面板 [p] ",
        "footer.mindmap.show_panel" => " 显示面板 [p] ",
        "footer.ai.input" => "[输入] Enter 发送 · Shift+Enter 换行 · Esc 普通模式 · Ctrl+v 粘贴",
        "footer.ai.normal_sidebar" => {
            "[普通] i/Enter 输入 · h 会话 · j/k/l/方向键 滚动 · [/] Agent · a 新建 · x 关闭"
        }
        "footer.ai.normal" => {
            "[普通] i/Enter 输入 · hjkl/方向键 滚动 · [/] Agent · a 新建 · x 关闭"
        }
        // ── Mode badges ─────────────────────────────────────
        "mode.preview" => "[预览]",
        "mode.browse" => "[浏览]",
        "mode.insert" => "[插入]",
        "mode.insert.bar" => "[插入] ",
        "mode.visual" => "[可视]",
        "mode.visual_line" => "[可视行]",
        "mode.normal" => "[普通] ",
        "mode.normal.edit" => "[普通] i 编辑 ",
        "mode.input" => "[输入] ",
        // ── Shared action-bar labels ────────────────────────
        "action.model" => " 模型",
        "action.select" => " 选择",
        "action.dismiss" => " 忽略",
        // ── Test Points action bar ──────────────────────────
        "tp.insert.save" => "保存 [Ctrl+s] ",
        "tp.insert.normal" => "普通 [Esc]",
        "tp.approve" => " 批准",
        "tp.approve.key" => " 批准 [a]",
        "tp.reject" => " 驳回",
        "tp.reject.key" => " 驳回 [r]",
        "tp.batch" => " 批量",
        "tp.batch.key" => " 批量 [A]",
        "tp.continue" => " 继续",
        "tp.continue.key" => " 继续 [c]",
        "tp.filter" => " 筛选",
        "tp.filter.key" => " 筛选 [f]",
        // ── Explore footer ──────────────────────────────────
        "explore.run" => " 运行",
        "explore.run.key" => " 运行 [r]",
        "explore.detail" => " 详情",
        "explore.detail.key" => " 详情 [Enter]",
        "explore.edit" => " 编辑",
        "explore.edit.key" => " 编辑 [e]",
        "explore.summary" => "{passed}/{total} 通过，{failed} 失败",
        "explore.summary.blocked" => "{passed}/{total} 通过，{failed} 失败，{blocked} 阻塞",
        "explore.summary.idle" => "0/{total} 通过",
        // ── Editor and MindMap AI footer hints ──────────────
        "editor.hint.insert" => "[插入] Enter 提交 · Esc 普通 · Ctrl+s 保存 · 方向键移动",
        "editor.hint.edit" => "[普通] 编辑 [i/Enter] ",
        "editor.hint.keyword" => " 关键字 [Ctrl+g/t/a] ",
        "editor.hint.step" => " 步骤 [o/O] ",
        "editor.hint.delete" => " 删除 [dd] ",
        "editor.hint.copy_paste" => " 复制/粘贴 [yy/p] ",
        "editor.hint.move_step" => " 移动步骤 [Ctrl+j/k] ",
        "editor.hint.fold" => " 折叠 [Space] ",
        "editor.hint.fold_all" => " 全部折叠 [Ctrl+Space] ",
        "editor.hint.save" => " 保存 [Ctrl+s] ",
        "mindmap.ai.hint.send" => "[输入] 发送 [Enter] ",
        "mindmap.ai.hint.back" => " 返回/浏览 [Esc] ",
        "mindmap.ai.hint.newline" => " 换行 [Shift+Enter] ",
        "mindmap.ai.hint.return_tree" => "[普通] 返回树 [Esc] ",
        "mindmap.ai.hint.input" => " 输入 [i/Enter] ",
        "mindmap.ai.hint.scroll" => " 滚动 [hjkl/方向键] ",
        // ── Confirmation dialogs ────────────────────────────
        "dialog.close.title" => "确认关闭",
        "dialog.close.verb" => "关闭",
        "dialog.close.body" => "历史仍可在会话列表中恢复。",
        "dialog.close.confirm" => "关闭 [Enter/y]",
        "dialog.delete.title" => "确认删除",
        "dialog.delete.verb" => "删除",
        "dialog.delete.body" => "此操作无法撤销。",
        "dialog.delete.confirm" => "删除 [Enter/y]",
        "dialog.cancel" => "取消 [Esc/n]",
        "dialog.cancel.esc" => "取消 [Esc]",
        "dialog.close.esc" => "关闭 [Esc]",
        "dialog.error" => "无法{action}：{error}",
        "dialog.quit.title" => "退出 Teshi？",
        "dialog.quit.body" => "退出 Teshi 并返回终端？",
        "dialog.quit.body_dirty" => "有未保存的修改，仍要退出吗？",
        "dialog.quit.yes" => "[是]",
        "dialog.quit.no" => "[否]",
        "dialog.interrupt.title" => "中断当前运行？",
        "dialog.interrupt.body" => "本轮会结束；会话与其回答会保留。",
        "dialog.interrupt.hint" => "Enter/y 中断 · Esc/n 继续运行",
        "dialog.interrupt.confirm" => "[中断]",
        "dialog.interrupt.continue" => "[继续]",
        "approval.title" => " 审批模式 ",
        "approval.title_override" => " 审批（环境变量覆盖：自动） ",
        // ── External change and change summary overlays ─────
        "external_change.title" => "磁盘上的 Feature 已改变",
        "external_change.body" => "检测到 {file} 的外部更新。",
        "external_change.reload" => "重新加载 [Enter] ",
        "external_change.keep" => "保留本地 [Esc]",
        "change_summary.title" => "变更摘要",
        "change_summary.jump" => "跳转 [Enter] ",
        // ── Requirements action bar ─────────────────────────
        "req.edit" => " 编辑",
        "req.edit.key" => " 编辑 [e]",
        "req.preview" => " 预览",
        "req.preview.esc" => " 预览 [Esc]",
        "req.actions" => " 操作",
        "req.actions.space" => " 操作 [Space]",
        "req.new" => " 新建",
        "req.new.doc.key" => " 新建文档 [n]",
        "req.new.doc.ctrl" => " 新建文档 [Ctrl+n]",
        "req.point" => " 测试点",
        "req.point.key" => " 测试点 [n]",
        "req.filter" => " 筛选",
        "req.filter.key" => " 筛选 [i]",
        "req.iteration" => " 迭代",
        "req.iteration.key" => " 迭代 [I]",
        "req.priority" => " 优先级",
        "req.priority.key" => " 优先级 [Ctrl+p]",
        "req.scope" => " 范围",
        "req.scope.key" => " 范围 [o]",
        "req.view" => " 视图",
        "req.view.key" => " 视图 [g]",
        "req.completed" => " 已完成",
        "req.completed.key" => " 已完成 [H]",
        "req.reload" => " 重新加载",
        "req.reload.key" => " 重新加载 [R]",
        "req.save.ctrl" => " 保存 [Ctrl+s]",
        // ── Slash command descriptions ──────────────────────
        "command.new" => "新建会话",
        "command.exit" => "退出 Teshi",
        "command.resume" => "恢复最近的会话",
        "command.retry" => "继续被中断的 ACP 任务",
        "command.copy" => "复制最近 N 条助手回答到剪贴板",
        "command.details" => "展开或折叠思考与工具详情",
        "command.unqueue" => "取回最近一条排队消息继续编辑",
        "command.rename" => "重命名此会话（/rename --clear 可重置）",
        "command.models" => "打开模型设置",
        "command.sessions" => "浏览已保存的会话",
        "command.project" => "打开或切换当前项目",
        "command.worktree" => "创建 git worktree 并在其中对话",
        "command.approval" => "Agent 审批设置（/approval local 使用 Teshi 策略）",
        "command.agent" => "选择会话 Agent（Native 或已安装的 ACP）",
        "command.acp" => "管理已安装的 ACP Agent",
        "command.settings" => "选择 ACP 模型、模式、思考等设置",
        "command.preferences" => "Teshi 设置：界面语言与 AI 提示附加段",
        "command.agent-profile" => "选择 Native Agent 配置",
        "command.generate" => "启动 需求 → 测试点 → 场景 生成",
        "command.continue" => "批准测试点后继续生成",
        // ── Explore and MindMap content panes ───────────────
        "features.title.search" => "功能 [/] 搜索",
        "features.title.count" => "功能（{count}）[/] 文件搜索",
        "features.title.prefix" => "功能 ",
        "features.loading" => "加载中...",
        "features.empty.scanning" => "正在扫描 Feature 文件...",
        "features.empty.none" => "(没有功能文件)",
        "features.empty.indexing" => "正在索引标签...",
        "features.empty.tags_search" => "没有文件同时匹配标签与搜索",
        "features.empty.tags" => "没有文件匹配所选标签",
        "features.empty.search" => "没有文件匹配搜索",
        "scenarios.title" => "场景（{count}）",
        "scenarios.empty.select" => "选择一个功能；内容按需加载",
        "scenarios.deleted" => " （已删除的功能；见 Git 差异）",
        "pane.feature_content" => "功能内容",
        "steps.pane" => "步骤",
        "steps.empty" => " (无步骤)",
        "explore.feature" => " 功能",
        "explore.background" => " 背景",
        "explore.background_label" => " 背景：",
        "explore.examples" => " 示例：",
        "mindmap.pane.current" => "思维导图 [当前功能]",
        "mindmap.pane" => "思维导图",
        "mindmap.filtered" => "[已筛选]",
        "mindmap.highlighted" => "[已高亮]",
        "mindmap.budget" => "该功能超出交互式思维导图预算，其场景与步骤仍可在“探索”中查看。",
        "preview.pane" => "预览",
        // ── Session browser ─────────────────────────────────
        "sessions.title" => " 会话 · {project} · {count} ",
        "sessions.title.legacy" => " 旧版 / 未分配 · {count} ",
        "sessions.hint.legacy" => " 加载后历史仍保持未分配。",
        "sessions.hint.scope" => " 仅当前项目。旧历史请用 /sessions legacy。",
        "sessions.empty" => "  没有已保存的会话。",
        "sessions.empty.hint" => "  输入 /new 开始一个将被保存的会话。",
        "sessions.row" => " {marker} {title}  ·  {count} 条消息  ·  {size}",
        "sessions.row.meta" => "     {dir}  ·  {age}  ·  {model}",
        "sessions.unassigned" => "未分配",
        "sessions.load" => "加载 [Enter] ",
        "sessions.delete" => "删除 [dd/Del] ",
        "age.just_now" => "刚刚",
        "age.minutes" => "{n} 分钟前",
        "age.hours" => "{n} 小时前",
        "age.days" => "{n} 天前",
        // ── Test Points tab ─────────────────────────────────
        "tp.tree.title" => "测试点（{filter}）",
        "tp.filter.all" => "全部",
        "tp.filter.proposed" => "待批准",
        "tp.filter.approved" => "已批准",
        "tp.filter.rejected" => "已驳回",
        "tp.filter.needs_review" => "需复审",
        "tp.tree.empty" => "还没有测试点。请从“需求 [4]”中建立关联。",
        "tp.review.proposed" => "待批准",
        "tp.review.approved" => "已批准",
        "tp.review.rejected" => "已驳回",
        "tp.review.needs_review" => "需复审",
        "tp.details.title.edit" => "意图与评审 [插入]",
        "tp.details.title" => "意图与评审 [普通]",
        "tp.details.empty" => "在左侧树中选择一个测试点。",
        "tp.details.unavailable" => "选中的测试点不可用。",
        "tp.field.title" => "标题",
        "tp.field.objective" => "目标",
        "tp.field.preconditions" => "前置条件",
        "tp.field.expected" => "预期结果",
        "tp.field.hierarchy" => "层级",
        "tp.realized" => "已实现场景 [o]：",
        "tp.line" => "第 {n} 行",
        "tp.excerpts.title" => "需求摘录",
        "tp.excerpts.empty" => " (无关联摘录)",
        "tp.excerpt.ok" => "有效",
        "tp.excerpt.stale" => "已过期",
        // ── Model profiles ──────────────────────────────────
        "model.panel.title" => " 模型配置 ",
        "model.panel.add" => " 添加模型配置 ",
        "model.panel.edit" => " 编辑模型配置 ",
        "model.empty" => "  未找到模型配置。",
        "model.empty.hint" => "  按 a 添加，或将 TOML 文件放到",
        "model.active" => "  [当前]",
        "model.add" => "添加 [a] ",
        "model.edit" => "编辑 [e] ",
        "model.delete" => "删除 [dd/Del] ",
        "model.use" => "使用 [Enter] ",
        "model.form.save" => "保存 [Enter] ",
        "model.form.type_here" => "(在此输入...)",
        "model.form.empty" => "(空)",
        "model.field.name" => "名称",
        "model.field.provider" => "提供方",
        "model.field.model" => "模型",
        "model.field.base_url" => "Base URL",
        "model.field.api_key" => "API Key",
        "model.field.max_tokens" => "最大 Token",
        "model.field.temperature" => "温度",
        // ── Auth overlay ────────────────────────────────────
        "auth.title" => " 认证管理 ",
        "auth.add.title" => "添加提供方",
        "auth.add.hint" => "请在命令行使用 'teshi auth login' 添加提供方 API Key。",
        "auth.remove.title" => "移除提供方：{provider}",
        "auth.remove.hint" => "请在命令行使用 'teshi auth remove <provider>' 移除。",
        "auth.overview.title" => "模型配置（共享存储）",
        "auth.empty" => "  尚未配置模型。",
        "auth.empty.hint" => "  使用 'teshi auth login' 或按 m 添加。",
        "auth.active_id" => "  当前 ID：",
        "auth.key" => " 密钥：",
        // ── Project picker and switch overlays ──────────────
        "project.recent.empty" => "没有最近项目",
        "project.picker.hint" => "[O] 打开路径 · ↑/↓ 选择 · Enter 打开 · Esc 取消",
        "project.open" => " 打开项目 ",
        "project.open_path" => " 打开项目路径 ",
        "project.open_path.prompt" => "输入项目目录后按 Enter：",
        "project.open_path.hint" => "Enter 打开 · Backspace 编辑 · Esc 取消",
        "project.dirty.title" => " 处理项目状态 ",
        "project.dirty.body" => "需先处理未保存的项目状态。",
        "project.dirty.hint" => "[S] 保存并切换   [D] 放弃并切换   [Esc] 取消",
        "project.busy.title" => " 停止项目任务 ",
        "project.busy.body" => "仍有属于该项目的任务在运行。",
        "project.busy.hint" => "[S] 停止任务并切换   [Esc] 取消",
        "project.busy.wait" => "请等待所属任务退出后再切换。",
        "project.switch.to" => "切换到 {path}",
        // ── Requirements tree and editor ────────────────────
        "req.scope.all" => "全部项目",
        "req.scope.none" => "无项目",
        "req.scope.current" => "当前项目",
        "req.view.iteration" => "迭代",
        "req.view.folders" => "文件夹",
        "req.view.path" => "路径",
        "req.iteration.all" => "全部",
        "req.iteration.unassigned" => "未分配",
        "req.states.all" => " · 全部状态",
        "req.states.pending" => " · 仅未完成",
        "req.tree.title" => "{status} · [/] 搜索",
        "req.tree.status" => "{scope} · {view} · 迭代：{iteration}",
        "req.tree.identity_error" => "项目标识：{error}",
        "req.tree.none" => "还没有需求文档。Ctrl+n 新建；范围 [o]：全部 / 无项目。",
        "req.tree.no_match" => "没有匹配 /{query} 的需求\nEsc：清除搜索   Enter：保留筛选",
        "req.tree.empty" => "此范围内没有需求。\no：切换范围   b：关联所选\nCtrl+n：为当前项目新建",
        "req.editor.title" => "需求",
        "req.source.title" => "源文件",
        "req.missing_suffix" => "（文件缺失）",
        "req.missing_file" => "索引的文档文件在磁盘上缺失。",
        "req.priority.title" => " 优先级：{value} ",
        "req.priority.unset" => "未设置",
        "req.linked.title" => "关联测试点",
        "req.status.planned" => "[计划中]",
        "req.status.implemented" => "[已实现]",
        "req.status.verified" => "[已验证]",
        // ── Agent panels ────────────────────────────────────
        "agent.panel.select" => " 为本次会话选择 Agent ",
        "agent.panel.installed" => " 已安装的 ACP Agent（{count}） ",
        "agent.panel.registry" => " 官方 ACP Registry（{count} 个 Agent） ",
        "agent.panel.title" => " Agent ",
        "agent.install" => "安装 [Enter] ",
        "agent.use" => "使用 [Enter] ",
        "agent.registry.browse" => "Registry [i] ",
        "agent.native" => "Teshi Native",
        "agent.selected" => " [已选]",
        "agent.installed" => "[已安装]",
        "agent.available" => "[可安装]",
        "agent.no_description" => "无描述",
        "agent.registry.loading" => "正在加载官方 ACP Registry...",
        "agent.registry.empty" => "Registry 中还没有 Agent",
        "agent.installed.empty" => "未安装 ACP Agent · 按 i 浏览 Registry",
        "agent.empty" => "没有可用的 Agent",
        "agent.config.title" => " Agent 配置 ",
        "agent.apply" => "应用 [Enter] ",
        "agent.profile.no_desc" => "(无描述)",
        "agent.profile.none" => "  没有配置",
        "agent.profile.empty" => "未选择配置",
        "agent.profile.section" => "配置",
        "agent.profile.name" => "  名称：",
        "agent.profile.description" => "  描述：",
        "agent.profile.empty_value" => "(空)",
        "agent.prompt.section" => "系统提示词（预览）",
        "agent.config.section" => "配置项",
        "agent.tools.all" => "全部工具",
        "agent.tools.none" => "无",
        "agent.tools.enabled" => "已启用 {count} 个",
        "agent.tools.label" => "  工具：",
        "agent.skills.hint" => "(在 agent.yaml 中配置)",
        "agent.skills.label" => "  技能：",
        "agent.model.active" => "(当前模型配置)",
        "agent.model.label" => "  模型：",
        "agent.apply_hint" => "Enter 应用 · Esc 关闭",
        // ── Chat panel and composer ─────────────────────────
        "chat.panel.title" => " AI 对话 ",
        "chat.session.prefix" => "· 会话：",
        "chat.agent.suffix" => "· Agent：{name} ",
        "chat.chats_hidden" => "· 会话列表已隐藏 [Ctrl+g] ",
        "chat.greeting" => "欢迎使用 AI 对话！在下方输入消息后按 Enter。",
        "chat.greeting.embedded" => "欢迎使用 AI 对话！",
        "chat.hint.preselected" => "已预选 Agent，尚未连接。聚焦输入框开始，或使用 /agent。",
        "chat.hint.no_agent" => "未选择 Agent。用 /agent 选择，或聚焦输入框使用 Teshi Native。",
        "chat.hint.auth" => {
            "提示：运行 'teshi auth login' 或设置 TESHI_LLM_API_KEY 以启用 AI 回复。"
        }
        "chat.ready" => "就绪",
        "chat.agent_native" => " Agent Native ▾ ",
        "chat.agent" => " Agent ▾ ",
        "chat.model" => " 模型 {model} ▾ ",
        "chat.model.select" => "选择",
        "chat.waiting" => "等待 Agent…",
        "chat.activity" => "活动",
        "chat.role.you" => "你",
        "chat.role.teshi" => "Teshi",
        "chat.queued" => "已排队 {count}：",
        "chat.queued.tail" => " · /unqueue 可编辑 · {state}",
        "chat.queue.more" => "Enter 继续排队",
        "chat.queue.resume" => "发送消息以继续",
        "chat.image.loading" => "正在加载图片…",
        "chat.image.error" => "无法显示图片：{reason}",
        "chat.image.scroll" => "滚动以显示图片",
        "chat.chats.title" => " 会话 [Ctrl+g] ",
        "chat.new" => " 新建 [a]",
        "chat.close" => " 关闭 [x]",
        "chat.send" => " 发送",
        "chat.send.key" => " 发送 [Enter]",
        "chat.focus" => " 聚焦",
        "chat.focus.key" => " 聚焦 [i/Enter]",
        "chat.blur" => " 失焦",
        "chat.blur.key" => " 失焦 [Esc]",
        "chat.model_chip" => " 模型",
        "chat.model_chip.key" => " 模型 [m]",
        "chat.chats_chip" => " 会话",
        "chat.chats_chip.key" => " 会话 [Ctrl+g]",
        "mode.chats" => "[会话] ",
        "chat.hint.chats_normal" => "Esc 后 j/k 滚动",
        "chat.hint.chats_browse" => "j/k 会话 · Enter 打开 · l 返回",
        "chat.hint.normal_sidebar" => {
            "h 会话 · j/k/l/方向键 滚动 · g/G 到首尾 · Ctrl+D/U 半页 · [/] 会话"
        }
        "chat.hint.normal" => "hjkl/方向键 滚动 · g/G 到首尾 · Ctrl+D/U 半页 · [/] 会话",
        "chat.pending.title" => " 待确认的 Agent 变更 ",
        "chat.permission.title" => " ACP 请求权限 ",
        "chat.change.accept" => " 接受",
        "chat.change.reject" => " 驳回",
        "chat.change.diff" => " 差异",
        "chat.change.summary" => " 摘要",
        "chat.permission.allow" => " 允许 [Enter]",
        "chat.permission.reject" => " 驳回 [Esc]",
        // ── Requirement overlays ────────────────────────────
        "req.overlay.unsaved.title" => "需求未保存",
        "req.overlay.unsaved.body" => "{title} 有未保存的更改。",
        "req.overlay.save" => "[S] 保存",
        "req.overlay.discard" => "[D] 放弃",
        "req.overlay.cancel" => "[Esc] 取消",
        "req.overlay.select" => "选择 [Enter]",
        "req.overlay.toggle" => "切换 [Space]",
        "req.overlay.save_enter" => "保存 [Enter]",
        "req.overlay.add" => "添加 [Enter]",
        "req.overlay.confirm" => "确认 [Enter]",
        "req.overlay.delete" => "删除 [Enter]",
        "req.overlay.color" => "颜色 [c]",
        "req.overlay.front" => "前缀 [f]",
        "req.overlay.next_field" => "下一字段 [Tab]",
        "req.overlay.field.name" => "名称",
        "req.overlay.field.condition" => "条件",
        "req.overlay.filter.title" => "筛选迭代",
        "req.overlay.folders.title" => "{title} 的文件夹",
        "req.overlay.folders.none" => "还没有文件夹。关闭后选择 Inbox，再选择“新建文件夹…”。",
        "req.overlay.folders.none_checked" => "未勾选文件夹：需求会列在 Inbox 中。",
        "req.overlay.folders.checked" => "需求会列在每一个勾选的文件夹中。",
        "req.overlay.tags.title" => "{title} 的标签",
        "req.overlay.tags.new" => " ＋ 新标签…",
        "req.overlay.tags.new_buffer" => " 新标签：{buffer}",
        "req.overlay.tags.none" => "还没有标签。",
        "req.overlay.tags.note" => "标签属于文档，所有项目看到的是同一组。",
        "req.overlay.tags.no_color" => "无颜色",
        "req.overlay.tags.before" => "标题前",
        "req.overlay.tags.after" => "标题后",
        "req.overlay.folder.new" => "新建文件夹",
        "req.overlay.folder.new_child" => "在 {parent} 中新建子文件夹",
        "req.overlay.folder.hint" => "用 / 创建嵌套文件夹，例如 Chat/Archive。",
        "req.overlay.rename.query" => "重命名保存的查询",
        "req.overlay.rename.folder" => "重命名文件夹",
        "req.overlay.rename.hint" => "需求与查询的引用保持不变。",
        "req.overlay.query.matches_one" => "匹配 1 条需求",
        "req.overlay.query.matches" => "匹配 {count} 条需求",
        "req.overlay.query.error" => "! {error}",
        "req.overlay.query.help1" => "字段：status、priority、iteration、title、folder",
        "req.overlay.query.help2" => "& = 与，| = 或，! = 非；& 优先级高于 |",
        "req.overlay.query.help3" => "示例：status=planned & priority=P1 | folder=AI/Chat",
        "req.overlay.query.edit" => "编辑保存的查询",
        "req.overlay.query.new" => "新建保存的查询",
        "req.overlay.query.delete.title" => "删除保存的查询",
        "req.overlay.query.delete.body" => "删除保存的查询“{name}”？",
        "req.overlay.query.delete.note" => "需求不会改变。",
        "req.overlay.folder.delete.title" => "删除文件夹",
        "req.overlay.folder.delete.body" => "删除文件夹“{name}”及其子文件夹？",
        "req.overlay.folder.delete.none" => "没有需求回到 Inbox。",
        "req.overlay.folder.delete.one" => "1 条需求回到 Inbox。",
        "req.overlay.folder.delete.many" => "{n} 条需求回到 Inbox。",
        "req.overlay.folder.delete.note" => "需求文档本身不会被删除。",
        "req.overlay.requirement.delete.title" => "删除需求",
        "req.overlay.requirement.delete.body" => "删除需求“{title}”（{id}）？",
        "req.overlay.requirement.delete.note1" => "这会将它从全局需求库中移除。",
        "req.overlay.requirement.delete.note2" => "关联的测试点会保留，并成为缺失来源的链接。",
        "req.overlay.iteration.title" => "设置迭代",
        "req.overlay.iteration.field" => "迭代",
        "req.overlay.iteration.hint" => "留空表示该需求未分配。",
        "req.overlay.priority.title" => "需求优先级",
        "req.overlay.scope.title" => " 确认生成来源范围 ",
        "req.overlay.scope.store" => "需求库：{path}",
        "req.overlay.scope.store_id" => "库 ID：{id}",
        "req.overlay.scope.iteration" => "迭代范围：{filter}",
        "req.overlay.scope.project" => "项目范围：{scope}",
        "req.overlay.scope.confirm" => "确认 [Enter] ",
        "req.overlay.scope.iteration_key" => "迭代 [i] ",
        "req.overlay.scope.uninitialized" => "(未初始化)",
        _ => return None,
    })
}

/// Substitute `{name}` placeholders in one pass.
///
/// Substituted values are never re-scanned, so a session title that happens to
/// contain `{count}` stays literal instead of being replaced by a later pass.
pub fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(offset) = rest[start..].find('}') else {
            break;
        };
        let end = start + offset;
        let name = &rest[start + 1..end];
        match pairs.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&rest[start..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Localized description for a slash command.
///
/// Returns `None` for commands the catalog does not know, so a caller can keep
/// its static English table entry instead of failing to render.
pub fn slash_description(name: &str) -> Option<&'static str> {
    let key = match name {
        "new" => "command.new",
        "exit" => "command.exit",
        "resume" => "command.resume",
        "retry" => "command.retry",
        "copy" => "command.copy",
        "details" => "command.details",
        "unqueue" => "command.unqueue",
        "rename" => "command.rename",
        "models" => "command.models",
        "sessions" => "command.sessions",
        "project" => "command.project",
        "worktree" => "command.worktree",
        "approval" => "command.approval",
        "agent" => "command.agent",
        "acp" => "command.acp",
        "settings" => "command.settings",
        "preferences" => "command.preferences",
        "agent-profile" => "command.agent-profile",
        "generate" => "command.generate",
        "continue" => "command.continue",
        _ => return None,
    };
    Some(t(key))
}

/// Localized label for a persisted locale preference.
pub fn preference_label(preference: UiLocalePreference) -> &'static str {
    match preference {
        UiLocalePreference::System => t("settings.locale.system"),
        UiLocalePreference::En => t("settings.locale.en"),
        UiLocalePreference::ZhCn => t("settings.locale.zh_cn"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_catalog_covers_tabs_and_settings() {
        set_effective_locale(EffectiveUiLocale::ZhCn);
        assert_eq!(t("tab.explore").trim(), "探索 [1]");
        assert_eq!(t("settings.ui_locale"), "界面语言");
        set_effective_locale(EffectiveUiLocale::En);
        assert_eq!(t("tab.explore"), " Explore [1] ");
    }

    #[test]
    fn unknown_keys_fall_back_to_the_key() {
        set_effective_locale(EffectiveUiLocale::En);
        assert_eq!(t("missing.key"), "missing.key");
    }

    #[test]
    fn fill_replaces_placeholders_in_one_pass() {
        assert_eq!(fill("{a}-{b}", &[("a", "1"), ("b", "2")]), "1-2");
        // Substituted values are not re-scanned for placeholders.
        assert_eq!(fill("{a}", &[("a", "{b}"), ("b", "2")]), "{b}");
        assert_eq!(fill("{missing}", &[]), "{missing}");
        assert_eq!(fill("plain", &[("a", "1")]), "plain");
    }

    #[test]
    fn chinese_catalog_covers_action_bars_and_dialogs() {
        set_effective_locale(EffectiveUiLocale::ZhCn);
        for (key, expected) in [
            ("mode.normal", "[普通] "),
            ("tp.approve.key", " 批准 [a]"),
            ("explore.run.key", " 运行 [r]"),
            ("editor.hint.save", " 保存 [Ctrl+s] "),
            ("dialog.delete.confirm", "删除 [Enter/y]"),
            ("dialog.cancel.esc", "取消 [Esc]"),
            ("dialog.close.esc", "关闭 [Esc]"),
            ("req.new.doc.key", " 新建文档 [n]"),
            ("change_summary.title", "变更摘要"),
            ("features.title.search", "功能 [/] 搜索"),
            ("scenarios.title", "场景（{count}）"),
            ("sessions.title.legacy", " 旧版 / 未分配 · {count} "),
            ("age.minutes", "{n} 分钟前"),
            ("tp.tree.title", "测试点（{filter}）"),
            ("tp.field.expected", "预期结果"),
            ("model.field.name", "名称"),
            ("auth.active_id", "  当前 ID："),
            ("project.dirty.body", "需先处理未保存的项目状态。"),
            ("req.tree.status", "{scope} · {view} · 迭代：{iteration}"),
            ("req.priority.title", " 优先级：{value} "),
            ("agent.panel.select", " 为本次会话选择 Agent "),
            ("agent.profile.name", "  名称："),
            ("agent.tools.enabled", "已启用 {count} 个"),
            ("chat.panel.title", " AI 对话 "),
            ("chat.role.you", "你"),
            ("chat.queue.more", "Enter 继续排队"),
            ("chat.permission.reject", " 驳回 [Esc]"),
            ("req.overlay.unsaved.title", "需求未保存"),
            ("req.overlay.tags.title", "{title} 的标签"),
            ("req.overlay.query.matches", "匹配 {count} 条需求"),
            ("req.overlay.folder.delete.many", "{n} 条需求回到 Inbox。"),
            ("req.overlay.scope.store", "需求库：{path}"),
            ("mode.input", "[输入] "),
            ("req.iteration.unassigned", "未分配"),
        ] {
            assert_eq!(t(key), expected, "{key}");
        }
        set_effective_locale(EffectiveUiLocale::En);
    }

    #[test]
    fn slash_descriptions_follow_the_locale() {
        set_effective_locale(EffectiveUiLocale::En);
        assert_eq!(
            slash_description("preferences"),
            Some(t("command.preferences"))
        );
        assert_eq!(
            slash_description("generate"),
            Some("Start requirements → test points → scenarios generation")
        );
        assert_eq!(slash_description("not-a-command"), None);

        set_effective_locale(EffectiveUiLocale::ZhCn);
        assert_eq!(
            slash_description("preferences"),
            Some("Teshi 设置：界面语言与 AI 提示附加段")
        );
        assert_eq!(slash_description("continue"), Some("批准测试点后继续生成"));
        set_effective_locale(EffectiveUiLocale::En);
    }
}
