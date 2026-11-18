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
        "status.a_project_switch_is_already_being_prepared" => {
            "A project switch is already being prepared"
        }
        "status.acp_agents_move_enter_select_i_registry_esc_close" => {
            "ACP Agents: ↑↓ move · Enter select · i Registry · Esc close"
        }
        "status.acp_registry_unavailable_fetch_worker_stopped" => {
            "ACP Registry unavailable: fetch worker stopped"
        }
        "status.acp_registry_move_enter_install_esc_close" => {
            "ACP Registry: ↑↓ move · Enter install · Esc close"
        }
        "status.acp_agent_requests_permission_enter_to_allow_esc_to_reject" => {
            "ACP agent requests permission — Enter to allow, Esc to reject"
        }
        "status.acp_install_failed_worker_stopped" => "ACP install failed: worker stopped",
        "status.acp_permission_request_is_no_longer_pending" => {
            "ACP permission request is no longer pending"
        }
        "status.ai_cleared_filter" => "AI cleared filter",
        "status.ai_cleared_highlights" => "AI cleared highlights",
        "status.ai_error_llm_handle_not_available" => "AI error: LLM handle not available",
        "status.ai_error_background_llm_thread_has_exited" => {
            "AI error: background LLM thread has exited"
        }
        "status.ai_error_model_kept_calling_tools_after_final_response_limit" => {
            "AI error: model kept calling tools after final-response limit"
        }
        "status.ai_not_configured" => "AI not configured",
        "status.ai_preview_panel_closed" => "AI preview panel closed",
        "status.ai_request_cancelled" => "AI request cancelled",
        "status.ai_suggest_not_implemented" => "AI suggest: not implemented",
        "status.add_model" => "Add model",
        "status.agent_profiles_select_enter_apply_esc_close" => {
            "Agent profiles: ↑↓ select · Enter apply · Esc close"
        }
        "status.all_scenarios_folded" => "All scenarios folded",
        "status.already_on_that_project_active_work_was_preserved" => {
            "Already on that project; active work was preserved"
        }
        "status.approval_mode" => "Approval mode",
        "status.auth_panel_opened_press_esc_to_close" => "Auth panel opened. Press Esc to close.",
        "status.auth_showing_provider_status" => "Auth: showing provider status.",
        "status.back_to_tree_preview" => "Back to tree + preview",
        "status.background_run_started" => "Background run started",
        "status.cannot_approve_resolve_stale_links_or_review_state" => {
            "Cannot approve: resolve stale links or review state"
        }
        "status.cannot_close_the_last_agent" => "Cannot close the last agent",
        "status.cannot_reject_this_test_point" => "Cannot reject this test point",
        "status.chat_selection_is_no_longer_available" => "Chat selection is no longer available",
        "status.chat_sidebar_hidden" => "Chat sidebar hidden",
        "status.chat_sidebar_shown" => "Chat sidebar shown",
        "status.clipboard_is_empty" => "Clipboard is empty",
        "status.code_block_is_no_longer_available" => "Code block is no longer available",
        "status.completed_filter_applies_to_current_project_only" => {
            "Completed filter applies to Current project only"
        }
        "status.confirm_generation_source_scope_enter_confirm_esc_cancel_i_c" => {
            "Confirm generation source scope (Enter confirm · Esc cancel · i change iteration)"
        }
        "status.conversation_title_cannot_be_empty" => "Conversation title cannot be empty",
        "status.copy_works_on_steps_only" => "Copy works on steps only",
        "status.cycled_test_point_review_filter" => "Cycled test-point review filter",
        "status.delete_works_on_steps_or_scenario_headers" => {
            "Delete works on steps or scenario headers"
        }
        "status.deleted_feature_cannot_be_edited" => "Deleted Feature cannot be edited",
        "status.deleted_feature_cannot_be_run" => "Deleted Feature cannot be run",
        "status.deleted_step_is_read_only" => "Deleted Step is read-only",
        "status.deleted_node" => "Deleted node",
        "status.diff_view_explore_tab_shows_changes_d_to_close" => {
            "Diff view — explore tab shows changes. [D] to close."
        }
        "status.direct_no_review_generation_has_no_test_points_review_gate_s" => {
            "Direct (no-review) generation has no Test Points review gate; send the generation request to continue."
        }
        "status.edit_committed" => "Edit committed",
        "status.edit_the_fields_and_press_enter_to_save" => {
            "Edit the fields and press Enter to save."
        }
        "status.editing_active" => "Editing active",
        "status.enter_a_project_directory_or_press_esc_to_cancel" => {
            "Enter a project directory or press Esc to cancel"
        }
        "status.enter_editor_mode_first" => "Enter editor mode first",
        "status.exited_diff_view" => "Exited diff view",
        "status.explore_edit_mode" => "Explore edit mode",
        "status.explore_mode" => "Explore mode",
        "status.feature_load_deferred_save_edits_and_finish_pending_work_the" => {
            "Feature load deferred: save edits and finish pending work, then select again"
        }
        "status.feature_loaded_ast_and_mindmap_scope_current_feature" => {
            "Feature loaded; AST and MindMap scope: current Feature"
        }
        "status.fill_in_the_fields_and_press_enter_to_save" => {
            "Fill in the fields and press Enter to save."
        }
        "status.focus_returned_to_tree" => "Focus returned to tree",
        "status.focused_scenario" => "Focused scenario",
        "status.fold_works_inside_a_scenario" => "Fold works inside a scenario",
        "status.generation_canceled" => "Generation canceled",
        "status.generation_source_scope_confirmed" => "Generation source scope confirmed",
        "status.hiding_plaintext_secrets" => "Hiding plaintext secrets",
        "status.image_attached" => "Image attached",
        "status.image_removed" => "Image removed",
        "status.input_cleared" => "Input cleared",
        "status.input_deactivated" => "Input deactivated",
        "status.input_state_cleared" => "Input state cleared",
        "status.inserted_new_step_line" => "Inserted new step line",
        "status.inserted_scenario" => "Inserted scenario",
        "status.inserted_step_above" => "Inserted step above",
        "status.inserted_step_below" => "Inserted step below",
        "status.interrupt_the_current_run" => "Interrupt the current run?",
        "status.keep_or_clear_the_current_draft_before_editing_a_queued_mess" => {
            "Keep or clear the current draft before editing a queued message"
        }
        "status.llm_not_configured_set_an_api_key_on_a_model_profile_or_tesh" => {
            "LLM not configured: set an API key on a model profile or TESHI_LLM_API_KEY"
        }
        "status.legacy_unassigned_loading_keeps_history_unassigned_enter_loa" => {
            "Legacy / Unassigned. Loading keeps history unassigned. Enter load · Esc close"
        }
        "status.linked_requirement_to_current_project" => "Linked requirement to current project",
        "status.loaded_legacy_session_history_remains_unassigned" => {
            "Loaded legacy session; history remains unassigned"
        }
        "status.loading_official_acp_registry" => "Loading official ACP Registry...",
        "status.location_switched" => "Location switched",
        "status.mindmap_is_display_only" => "MindMap is display-only",
        "status.model_identifier_is_required" => "Model identifier is required.",
        "status.model_profiles_m_a_add_e_edit_select_enter_activate_esc_clos" => {
            "Model profiles [m]. a add · e edit · ↑↓ select · Enter activate · Esc close"
        }
        "status.moved_step_down" => "Moved step down",
        "status.moved_step_up" => "Moved step up",
        "status.name_is_required" => "Name is required.",
        "status.new_line_is_available_on_step_lines_only" => {
            "New line is available on step lines only"
        }
        "status.new_session_started" => "New session started",
        "status.no_http_exchanges_to_expand" => "No HTTP exchanges to expand",
        "status.no_assistant_response_to_copy_a_code_block_from" => {
            "No assistant response to copy a code block from"
        }
        "status.no_capture_for_this_step_showing_the_first_one" => {
            "No capture for this step; showing the first one"
        }
        "status.no_captured_screenshots_for_this_scenario" => {
            "No captured screenshots for this scenario"
        }
        "status.no_chat_selection_to_copy" => "No chat selection to copy",
        "status.no_completed_assistant_responses_to_copy" => {
            "No completed assistant responses to copy"
        }
        "status.no_custom_conversation_title_to_clear" => "No custom conversation title to clear",
        "status.no_editable_text_region_on_this_line" => "No editable text region on this line",
        "status.no_file_path_run_with_cargo_run_path_to_file_feature" => {
            "No file path: run with `cargo run -- path/to/file.feature`"
        }
        "status.no_interrupted_acp_task_to_continue" => "No interrupted ACP task to continue",
        "status.no_queued_messages" => "No queued messages",
        "status.no_recent_project_is_available" => "No recent project is available",
        "status.no_run_details_for_selection" => "No run details for selection",
        "status.no_saved_sessions_found" => "No saved sessions found",
        "status.no_scenario_selected" => "No scenario selected",
        "status.no_scenario_selected_for_step_insertion" => {
            "No scenario selected for step insertion"
        }
        "status.no_scenario_selected_to_run" => "No scenario selected to run",
        "status.no_scenarios_to_run" => "No scenarios to run",
        "status.no_selection_to_copy" => "No selection to copy",
        "status.no_step_to_edit" => "No step to edit",
        "status.nothing_to_redo" => "Nothing to redo",
        "status.nothing_to_undo" => "Nothing to undo",
        "status.open_run_details_first_enter_then_press_p_to_expand_secrets" => {
            "Open run details first (Enter), then press p to expand secrets"
        }
        "status.paste_works_inside_a_scenario" => "Paste works inside a scenario",
        "status.pending_command" => "Pending command",
        "status.permission_sent_to_acp_agent" => "Permission sent to ACP agent...",
        "status.preview_closed" => "Preview closed",
        "status.profile_not_found_for_editing" => "Profile not found for editing.",
        "status.project_picker_select_enter_open_esc_cancel" => {
            "Project picker: ↑/↓ select · Enter open · Esc cancel"
        }
        "status.project_switch_canceled" => "Project switch canceled",
        "status.project_switch_needs_running_tasks_stopped_s_stop_esc_cancel" => {
            "Project switch needs running tasks stopped: S stop · Esc cancel"
        }
        "status.project_owned_sessions_cannot_be_loaded_from_the_legacy_view" => {
            "Project-owned sessions cannot be loaded from the Legacy view"
        }
        "status.project_owned_tasks_must_stop_before_switching" => {
            "Project-owned tasks must stop before switching"
        }
        "status.queued_message_paused_configure_an_agent_then_send_a_message" => {
            "Queued message paused: configure an Agent, then send a message to resume"
        }
        "status.queued_message_restored_for_editing" => "Queued message restored for editing",
        "status.removed_externally_deleted_feature_on_disk_run_evidence_reta" => {
            "Removed externally deleted Feature; on-disk run evidence retained"
        }
        "status.requirement_changed_on_disk_unsaved_edits_kept" => {
            "Requirement changed on disk; unsaved edits kept"
        }
        "status.requirement_library_refreshed" => "Requirement library refreshed",
        "status.requirement_library_reloaded" => "Requirement library reloaded",
        "status.requirement_link_is_stale_resolve_in_requirements_tab" => {
            "Requirement link is stale — resolve in Requirements tab"
        }
        "status.requirements_view_updated" => "Requirements view updated",
        "status.review_proposed_test_points_in_test_points_tab_5_then_press" => {
            "Review proposed test points in Test Points tab [5], then press c to continue generation"
        }
        "status.run_started" => "Run started",
        "status.runner_already_active" => "Runner already active",
        "status.runner_exited_with_error" => "Runner exited with error",
        "status.runner_is_still_active_wait_for_its_owned_process_to_exit" => {
            "Runner is still active; wait for its owned process to exit"
        }
        "status.save_current_edits_and_finish_pending_work_before_switching" => {
            "Save current edits and finish pending work before switching Feature"
        }
        "status.saved_requirement_document" => "Saved requirement document",
        "status.saved_test_points" => "Saved test points",
        "status.scenario_expanded" => "Scenario expanded",
        "status.scenario_folded" => "Scenario folded",
        "status.select_agent_move_enter_select_esc_close" => {
            "Select Agent: ↑↓ move · Enter select · Esc close"
        }
        "status.select_a_feature_and_wait_for_it_to_load_before_editing" => {
            "Select a Feature and wait for it to load before editing"
        }
        "status.select_a_feature_and_wait_for_it_to_load_before_running" => {
            "Select a Feature and wait for it to load before running"
        }
        "status.select_a_requirement_document_to_set_its_iteration" => {
            "Select a requirement document to set its iteration"
        }
        "status.select_a_requirement_document_to_set_its_priority" => {
            "Select a requirement document to set its priority"
        }
        "status.select_a_test_point_to_approve" => "Select a test point to approve",
        "status.select_a_test_point_to_reject" => "Select a test point to reject",
        "status.select_non_empty_requirement_text_to_create_a_test_point" => {
            "Select non-empty requirement text to create a test point"
        }
        "status.select_step_keyword_enter_esc_cancel" => {
            "Select step keyword (↑↓ Enter, Esc cancel)"
        }
        "status.selected_teshi_native_remembered_for_new_sessions" => {
            "Selected Teshi Native; remembered for new sessions"
        }
        "status.sending_mindmap_context_to_ai" => "Sending MindMap context to AI...",
        "status.sending_message_to_ai" => "Sending message to AI...",
        "status.sessions_for_the_active_project_sessions_legacy_shows_unassi" => {
            "Sessions for the active project. /sessions legacy shows unassigned history"
        }
        "status.showing_implemented_and_verified_requirements" => {
            "Showing implemented and verified requirements"
        }
        "status.showing_pending_requirements_only" => "Showing pending requirements only",
        "status.showing_plaintext_secrets_p_to_hide" => "Showing plaintext secrets (p to hide)",
        "status.step_cannot_move_further_down" => "Step cannot move further down",
        "status.step_cannot_move_further_up" => "Step cannot move further up",
        "status.step_copied" => "Step copied",
        "status.step_keyword_list_is_available_on_step_lines_only" => {
            "Step keyword list is available on step lines only"
        }
        "status.step_keyword_selection_canceled" => "Step keyword selection canceled",
        "status.step_keyword_shortcuts_work_on_step_lines_only" => {
            "Step keyword shortcuts work on step lines only"
        }
        "status.step_keyword_updated" => "Step keyword updated",
        "status.step_pasted" => "Step pasted",
        "status.stop_requested_for_the_project_owned_runner_waiting_for_exit" => {
            "Stop requested for the project-owned runner; waiting for exit"
        }
        "status.switched_back_to_strict_review_human_approval_required_resta" => {
            "Switched back to Strict review (human approval required). Restart generation with /generate to follow the review flow."
        }
        "status.switched_to_ai_tab" => "Switched to AI tab",
        "status.switched_to_explore_tab" => "Switched to Explore tab",
        "status.switched_to_mindmap_tab" => "Switched to MindMap tab",
        "status.switched_to_requirements_tab" => "Switched to Requirements tab",
        "status.switched_to_test_points_tab" => "Switched to Test Points tab",
        "status.task_interrupted_retry_to_continue" => "Task interrupted — /retry to continue",
        "status.tasks_stopped_resolve_unsaved_state_before_switching" => {
            "Tasks stopped; resolve unsaved state before switching"
        }
        "status.the_clipboard_holds_no_text_or_image" => "The clipboard holds no text or image",
        "status.the_project_directory_has_no_feature_files_add_one_to_begin" => {
            "The project directory has no .feature files. Add one to begin."
        }
        "status.thinking_and_tool_details_toggled" => "Thinking and tool details toggled",
        "status.tool_call_limit_reached_requesting_final_response" => {
            "Tool call limit reached, requesting final response…"
        }
        "status.type_a_project_directory_and_press_enter_to_open_it_esc_canc" => {
            "Type a project directory and press Enter to open it; Esc cancels"
        }
        "status.type_q_w_wq_or_x" => "Type q, w, wq, or x",
        "status.unassigned_session_use_sessions_legacy_to_load_it_explicitly" => {
            "Unassigned session: use /sessions legacy to load it explicitly"
        }
        "status.unknown_slash_command_try_new_exit_resume_retry_copy_copy_co" => {
            "Unknown slash command. Try /new, /exit, /resume, /retry, /copy, /copy-code, /models, /sessions, /project, /approval, /agent, /preferences, /generate, /generate-direct, /review-strict, /continue"
        }
        "status.unlinked_requirement_from_current_project" => {
            "Unlinked requirement from current project"
        }
        "status.usage_acp_add_id_program_arg" => "Usage: /acp add <id> [\"program\",\"arg\",...]",
        "status.usage_copy_n_where_n_is_a_positive_integer" => {
            "Usage: /copy [N] where N is a positive integer"
        }
        "status.usage_copy_code_n_where_n_is_a_positive_integer" => {
            "Usage: /copy-code [N] where N is a positive integer"
        }
        "status.usage_rename_title_rename_clear" => "Usage: /rename <title> · /rename --clear",
        "status.usage_worktree_branch_path" => "Usage: /worktree <branch> [path]",
        "status.vim_pending_command" => "Vim pending command",
        "status.vim_pending_dd_delete" => "Vim pending: dd delete",
        "status.vim_pending_gg_top" => "Vim pending: gg top",
        "status.vim_pending_yy_yank" => "Vim pending: yy yank",
        "status.waiting_for_project_owned_agent_backends_to_exit" => {
            "Waiting for project-owned Agent backends to exit"
        }
        "status.waiting_for_the_previous_agent_to_exit_before_loading_histor" => {
            "Waiting for the previous Agent to exit before loading history; Esc cancels"
        }
        "status.yanked_requirement_line" => "Yanked requirement line",
        "status.yanked_requirement_selection" => "Yanked requirement selection",
        "status.dd_to_delete" => "`dd` to delete",
        "status.yy_to_copy" => "`yy` to copy",
        "status.acp_registry_unavailable" => "ACP Registry unavailable: {error}",
        "status.acp_approval_preference_could_not_be_restored" => {
            "ACP approval preference could not be restored: {error}"
        }
        "status.acp_install_failed" => "ACP install failed: {error}",
        "status.acp_installations_unavailable" => "ACP installations unavailable: {error}",
        "status.acp_storage_unavailable" => "ACP storage unavailable: {error}",
        "status.ai_applied_change_in" => "AI applied change \"{arg0}\" in {arg1}",
        "status.ai_applied_filter" => "AI applied filter: {desc}",
        "status.ai_applied_highlight_rule_s" => "AI applied {count} highlight rule(s)",
        "status.ai_backend_unavailable" => "AI backend unavailable: {error}",
        "status.ai_created_file" => "AI created file: {file_name}",
        "status.ai_error" => "AI error: {error}",
        "status.ai_error_message" => "AI error: {message}",
        "status.ai_response_received" => "AI response received ({model})",
        "status.ai_wants_to_y_accept_n_reject" => "AI wants to {arg0}. [Y] accept [N] reject",
        "status.added_and_activated_profile" => "Added and activated profile: {arg0}",
        "status.added_custom_acp_agent_use_agent_to_select_it" => {
            "Added custom ACP Agent {name}. Use /agent to select it."
        }
        "status.advanced_to_scenario_planning_with_approved_test_point_s" => {
            "Advanced to Scenario Planning with {arg0} approved test point(s)"
        }
        "status.agent_selected_but_preference_could_not_be_saved" => {
            "Agent selected, but preference could not be saved: {error}"
        }
        "status.approved_test_point" => "Approved test point {tp_id}",
        "status.batch_approved_test_point_s" => "Batch approved {count} test point(s)",
        "status.cannot_derive_a_worktree_path_for_pass_one_explicitly" => {
            "Cannot derive a worktree path for {arg0}; pass one explicitly"
        }
        "status.cannot_open" => "Cannot open {noun}: {error}",
        "status.cannot_resolve_session_project" => "Cannot resolve session project: {error}",
        "status.cannot_save_before_switching" => "Cannot save before switching: {error}",
        "status.cannot_send_queued_message" => "Cannot send queued message: {error}",
        "status.change_summary_change_s_navigate_enter_jump_esc_close" => {
            "Change Summary — {arg0} change(s). ↑↓ navigate, Enter jump, Esc close."
        }
        "status.closed" => "Closed {name}",
        "status.context_compacted_message_s_summary_token_window" => {
            "Context compacted: {removed_count} message(s) → summary ({arg0} token window)"
        }
        "status.conversation_renamed_to" => "Conversation renamed to '{title}'",
        "status.conversation_title_reset_to" => "Conversation title reset to '{arg0}'",
        "status.copied_code_block_line_s" => "Copied code block ({lines} line(s))",
        "status.copied_code_block_line_s_2" => "Copied code block {arg0} ({lines} line(s))",
        "status.copied_last_assistant_response_s_to_clipboard" => {
            "Copied last {arg0} assistant response(s) to clipboard"
        }
        "status.copied_session_id" => "Copied session ID {id}",
        "status.copied_character_s" => "Copied {arg0} character(s)",
        "status.copied_line_s" => "Copied {line_count} line(s)",
        "status.could_not_apply_acp_file_change" => "Could not apply ACP file change: {error}",
        "status.could_not_checkpoint_project_a" => "Could not checkpoint project A: {error}",
        "status.could_not_copy_to_clipboard" => "Could not copy to clipboard: {error}",
        "status.could_not_create_requirement" => "Could not create requirement: {error}",
        "status.could_not_delete_requirement" => "Could not delete requirement: {error}",
        "status.could_not_prepare_project_b" => "Could not prepare project B: {error}",
        "status.could_not_reconnect_acp_for_profile" => {
            "Could not reconnect ACP for profile {arg0}: {error}"
        }
        "status.could_not_resolve_project" => "Could not resolve project: {error}",
        "status.could_not_save_requirement" => "Could not save requirement: {error}",
        "status.could_not_save_test_point" => "Could not save test point: {error}",
        "status.created_agent" => "Created agent {id}",
        "status.created_proposed_test_point" => "Created proposed test point {id}",
        "status.created_requirement_document_md" => "Created requirement document {id}.md",
        "status.custom_acp_agent_failed" => "Custom ACP Agent failed: {error}",
        "status.cut_character_s" => "Cut {arg0} character(s)",
        "status.deleted_global_requirement" => "Deleted global requirement {document_id}",
        "status.deleted_profile" => "Deleted profile: {name}",
        "status.deleted_session" => "Deleted session {id}",
        "status.direct_no_review_generation_traceability_confirm_source_scop" => {
            "Direct (no-review) generation, traceability: {arg0} — confirm source scope (Enter confirm · Esc cancel · i change iteration)"
        }
        "status.exploration_terminated_url_is_not_in_the_whitelist" => {
            "Exploration terminated: URL '{arg0}' is not in the whitelist"
        }
        "status.exploration_terminated_step_limit_exceeded" => {
            "Exploration terminated: step limit ({arg0}) exceeded"
        }
        "status.failed_to_activate_profile" => "Failed to activate profile: {e}",
        "status.failed_to_delete_profile" => "Failed to delete profile: {e}",
        "status.failed_to_delete_session" => "Failed to delete session: {e}",
        "status.failed_to_reload" => "Failed to reload {arg0}: {err}",
        "status.failed_to_restore_generation_state" => "Failed to restore generation state: {e}",
        "status.failed_to_save_active_profile" => "Failed to save active profile: {e}",
        "status.failed_to_save_profile" => "Failed to save profile: {e}",
        "status.feature_changed_on_disk_reload_enter_r_or_keep_local_esc_k" => {
            "Feature changed on disk: {arg0}. Reload [Enter/r] or keep local [Esc/k]."
        }
        "status.feature_discovery_failed" => "Feature discovery failed: {error}",
        "status.feature_loading_failed" => "Feature loading failed: {error}",
        "status.generation_paused" => "Generation paused: {err}",
        "status.installed_use_agent_to_select_it" => "Installed {arg0}. Use /agent to select it.",
        "status.installing" => "Installing {arg0}...",
        "status.iteration_is_not_in_the_current_requirement_store_choose_ano" => {
            "Iteration '{name}' is not in the current requirement store; choose another scope"
        }
        "status.kept_local_buffer_for" => "Kept local buffer for {arg0}",
        "status.llm_configured_model_base_url_provider" => {
            "LLM configured: model={arg0}, base_url={arg1}, provider={arg2}"
        }
        "status.llm_not_configured" => "LLM not configured: {e}",
        "status.loaded_session_with_messages" => "Loaded session with {arg0} messages",
        "status.message_queued_unqueue_to_edit" => "Message queued ({arg0}) — /unqueue to edit",
        "status.no_code_block_in_the_last_assistant_response" => {
            "No code block {arg0} in the last assistant response"
        }
        "status.opened_requirement_excerpt_in" => "Opened requirement excerpt in {arg0}",
        "status.opened_scenario_in" => "Opened scenario {arg0} in {arg1}",
        "status.opened_in_the_system_viewer" => "Opened {noun} in the system viewer",
        "status.plaintext_expand_needs_a_running_api_sidecar" => {
            "Plaintext expand needs a running API sidecar: {err}"
        }
        "status.profile_has_no_api_key_set_one_or_use_teshi_llm_api_key" => {
            "Profile '{arg0}' has no API key; set one or use TESHI_LLM_API_KEY"
        }
        "status.profile_saved_but_failed_to_activate" => {
            "Profile saved but failed to activate: {e}"
        }
        "status.project_directory_does_not_exist" => "Project directory does not exist: {arg0}",
        "status.project_switch_could_not_continue" => "Project switch could not continue: {error}",
        "status.project_switch_stopped_owned_task_did_not_exit_within_s_proj" => {
            "Project switch stopped: owned task did not exit within {arg0}s; project A remains active"
        }
        "status.rejected_ai_change" => "Rejected AI change: {desc}",
        "status.rejected_test_point" => "Rejected test point {tp_id}",
        "status.reloaded_from_disk" => "Reloaded from disk: {arg0}",
        "status.requirement_library_refresh_failed" => {
            "Requirement library refresh failed: {error}"
        }
        "status.requirement_library_reload_failed" => "Requirement library reload failed: {error}",
        "status.requirement_scope" => "Requirement scope: {arg0}",
        "status.resumed_session_from_messages" => "Resumed session from {arg0} messages",
        "status.run_complete_passed_failed" => "Run complete: {passed} passed, {failed} failed",
        "status.run_complete_passed_failed_blocked" => {
            "Run complete: {passed} passed, {failed} failed, {blocked} blocked"
        }
        "status.run_not_started" => "Run not started: {first_line}",
        "status.runner_error" => "Runner error: {message}",
        "status.saved" => "Saved {arg0}",
        "status.selected_acp_agent_remembered_for_new_sessions" => {
            "Selected ACP Agent {arg0}; remembered for new sessions"
        }
        "status.session_belongs_to_another_project" => "Session belongs to another project: {arg0}",
        "status.settings_applied_but_not_saved" => "Settings applied but not saved: {error}",
        "status.step_keyword_set_to" => "Step keyword set to {keyword}",
        "status.switched_project_but_recent_history_was_not_saved" => {
            "Switched project, but recent history was not saved: {error}"
        }
        "status.switched_project" => "Switched project: {arg0}",
        "status.switched_to_agent" => "Switched to agent: {arg0}",
        "status.switched_to_model" => "Switched to model: {arg0}",
        "status.switched_to" => "Switched to {arg0}",
        "status.unknown_auth_subcommand_try_auth_auth_add_auth_status_auth_r" => {
            "Unknown auth subcommand: '{arg0}'. Try: auth, auth add, auth status, auth remove <provider>"
        }
        "status.unknown_command" => "Unknown command: {other}",
        "status.unsaved_project_state_s_save_switch_d_discard_switch_esc_can" => {
            "Unsaved project state: {arg0} · S save & switch · D discard & switch · Esc cancel"
        }
        "status.updated_profile" => "Updated profile: {arg0}",
        "status.view" => "View: {arg0}",
        "status.workspace_has_a_single_member" => "Workspace {arg0} has a single member",
        "status.worktree_not_created" => "Worktree not created: {error}",
        "status.already_works_in_this_member_press_a_for_a_new_agent_in_anot" => {
            "{arg0} already works in this member; press a for a new agent in another member"
        }
        "status.belongs_to_no_workspace" => "{arg0} belongs to no workspace",
        "status.is_already_installed_use_agent_to_select_it" => {
            "{arg0} is already installed. Use /agent to select it."
        }
        "status.now_works_in" => "{arg0} now works in {label}",
        "status.now_works_in_workspace_created_for_it" => {
            "{arg0} now works in {label}; workspace {arg1} created for it"
        }
        "status." => "{arg0} — {error}",
        "status.images_attached" => "{count} images attached",
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
        "status.a_project_switch_is_already_being_prepared" => "正在准备项目切换",
        "status.acp_agents_move_enter_select_i_registry_esc_close" => {
            "ACP Agent：↑↓ 移动 · Enter 选择 · i Registry · Esc 关闭"
        }
        "status.acp_registry_unavailable_fetch_worker_stopped" => {
            "ACP Registry 不可用：获取线程已停止"
        }
        "status.acp_registry_move_enter_install_esc_close" => {
            "ACP Registry：↑↓ 移动 · Enter 安装 · Esc 关闭"
        }
        "status.acp_agent_requests_permission_enter_to_allow_esc_to_reject" => {
            "ACP Agent 请求权限 — Enter 允许，Esc 拒绝"
        }
        "status.acp_install_failed_worker_stopped" => "ACP 安装失败：线程已停止",
        "status.acp_permission_request_is_no_longer_pending" => "ACP 权限请求已失效",
        "status.ai_cleared_filter" => "AI 已清除筛选",
        "status.ai_cleared_highlights" => "AI 已清除高亮",
        "status.ai_error_llm_handle_not_available" => "AI 错误：LLM 句柄不可用",
        "status.ai_error_background_llm_thread_has_exited" => "AI 错误：后台 LLM 线程已退出",
        "status.ai_error_model_kept_calling_tools_after_final_response_limit" => {
            "AI 错误：模型在最终回复上限后仍持续调用工具"
        }
        "status.ai_not_configured" => "AI 未配置",
        "status.ai_preview_panel_closed" => "AI 预览面板已关闭",
        "status.ai_request_cancelled" => "AI 请求已取消",
        "status.ai_suggest_not_implemented" => "AI 建议：尚未实现",
        "status.add_model" => "添加模型",
        "status.agent_profiles_select_enter_apply_esc_close" => {
            "Agent 配置：↑↓ 选择 · Enter 应用 · Esc 关闭"
        }
        "status.all_scenarios_folded" => "已折叠全部场景",
        "status.already_on_that_project_active_work_was_preserved" => {
            "已在该项目中；现有工作已保留"
        }
        "status.approval_mode" => "审批模式",
        "status.auth_panel_opened_press_esc_to_close" => "已打开认证面板。按 Esc 关闭。",
        "status.auth_showing_provider_status" => "认证：正在显示提供方状态。",
        "status.back_to_tree_preview" => "返回树 + 预览",
        "status.background_run_started" => "后台运行已开始",
        "status.cannot_approve_resolve_stale_links_or_review_state" => {
            "无法批准：请先处理失效链接或评审状态"
        }
        "status.cannot_close_the_last_agent" => "无法关闭最后一个 Agent",
        "status.cannot_reject_this_test_point" => "无法驳回该测试点",
        "status.chat_selection_is_no_longer_available" => "聊天选区已失效",
        "status.chat_sidebar_hidden" => "会话侧栏已隐藏",
        "status.chat_sidebar_shown" => "会话侧栏已显示",
        "status.clipboard_is_empty" => "剪贴板为空",
        "status.code_block_is_no_longer_available" => "代码块已失效",
        "status.completed_filter_applies_to_current_project_only" => "“已完成”筛选仅适用于当前项目",
        "status.confirm_generation_source_scope_enter_confirm_esc_cancel_i_c" => {
            "确认生成来源范围（Enter 确认 · Esc 取消 · i 修改迭代）"
        }
        "status.conversation_title_cannot_be_empty" => "会话标题不能为空",
        "status.copy_works_on_steps_only" => "只能在步骤上复制",
        "status.cycled_test_point_review_filter" => "已切换测试点评审筛选",
        "status.delete_works_on_steps_or_scenario_headers" => "只能删除步骤或场景标题",
        "status.deleted_feature_cannot_be_edited" => "已删除的功能无法编辑",
        "status.deleted_feature_cannot_be_run" => "已删除的功能无法运行",
        "status.deleted_step_is_read_only" => "已删除的步骤为只读",
        "status.deleted_node" => "已删除节点",
        "status.diff_view_explore_tab_shows_changes_d_to_close" => {
            "差异视图 — 探索页显示变更。[D] 关闭。"
        }
        "status.direct_no_review_generation_has_no_test_points_review_gate_s" => {
            "直接生成（免评审）没有测试点评审环节；发送生成请求即可继续。"
        }
        "status.edit_committed" => "编辑已提交",
        "status.edit_the_fields_and_press_enter_to_save" => "编辑字段后按 Enter 保存。",
        "status.editing_active" => "正在编辑",
        "status.enter_a_project_directory_or_press_esc_to_cancel" => "输入项目目录，或按 Esc 取消",
        "status.enter_editor_mode_first" => "请先进入编辑器模式",
        "status.exited_diff_view" => "已退出差异视图",
        "status.explore_edit_mode" => "探索编辑模式",
        "status.explore_mode" => "探索模式",
        "status.feature_load_deferred_save_edits_and_finish_pending_work_the" => {
            "功能加载已推迟：请保存编辑并完成待处理工作后重新选择"
        }
        "status.feature_loaded_ast_and_mindmap_scope_current_feature" => {
            "功能已加载；AST 与思维导图范围为当前功能"
        }
        "status.fill_in_the_fields_and_press_enter_to_save" => "填写字段后按 Enter 保存。",
        "status.focus_returned_to_tree" => "焦点已回到树",
        "status.focused_scenario" => "已聚焦场景",
        "status.fold_works_inside_a_scenario" => "折叠仅适用于场景内",
        "status.generation_canceled" => "生成已取消",
        "status.generation_source_scope_confirmed" => "生成来源范围已确认",
        "status.hiding_plaintext_secrets" => "正在隐藏明文密钥",
        "status.image_attached" => "已添加图片",
        "status.image_removed" => "已移除图片",
        "status.input_cleared" => "输入已清空",
        "status.input_deactivated" => "输入已失焦",
        "status.input_state_cleared" => "输入状态已清除",
        "status.inserted_new_step_line" => "已插入新步骤行",
        "status.inserted_scenario" => "已插入场景",
        "status.inserted_step_above" => "已在上方插入步骤",
        "status.inserted_step_below" => "已在下方插入步骤",
        "status.interrupt_the_current_run" => "中断当前运行？",
        "status.keep_or_clear_the_current_draft_before_editing_a_queued_mess" => {
            "编辑排队消息前，请保留或清除当前草稿"
        }
        "status.llm_not_configured_set_an_api_key_on_a_model_profile_or_tesh" => {
            "LLM 未配置：请在模型配置中设置 API Key，或设置 TESHI_LLM_API_KEY"
        }
        "status.legacy_unassigned_loading_keeps_history_unassigned_enter_loa" => {
            "旧版 / 未分配。加载后历史仍保持未分配。Enter 加载 · Esc 关闭"
        }
        "status.linked_requirement_to_current_project" => "已关联需求到当前项目",
        "status.loaded_legacy_session_history_remains_unassigned" => {
            "已加载旧版会话；历史仍保持未分配"
        }
        "status.loading_official_acp_registry" => "正在加载官方 ACP Registry...",
        "status.location_switched" => "位置已切换",
        "status.mindmap_is_display_only" => "思维导图仅供查看",
        "status.model_identifier_is_required" => "模型标识为必填项。",
        "status.model_profiles_m_a_add_e_edit_select_enter_activate_esc_clos" => {
            "模型配置 [m]。a 添加 · e 编辑 · ↑↓ 选择 · Enter 启用 · Esc 关闭"
        }
        "status.moved_step_down" => "步骤已下移",
        "status.moved_step_up" => "步骤已上移",
        "status.name_is_required" => "名称为必填项。",
        "status.new_line_is_available_on_step_lines_only" => "仅步骤行支持换行",
        "status.new_session_started" => "已开始新会话",
        "status.no_http_exchanges_to_expand" => "没有可展开的 HTTP 交互",
        "status.no_assistant_response_to_copy_a_code_block_from" => "没有可复制代码块的助手回复",
        "status.no_capture_for_this_step_showing_the_first_one" => "该步骤没有截图；显示第一张",
        "status.no_captured_screenshots_for_this_scenario" => "该场景没有截图",
        "status.no_chat_selection_to_copy" => "没有可复制的聊天选区",
        "status.no_completed_assistant_responses_to_copy" => "没有可复制的已完成助手回复",
        "status.no_custom_conversation_title_to_clear" => "没有可清除的自定义会话标题",
        "status.no_editable_text_region_on_this_line" => "该行没有可编辑的文本区域",
        "status.no_file_path_run_with_cargo_run_path_to_file_feature" => {
            "没有文件路径：请使用 `cargo run -- path/to/file.feature` 运行"
        }
        "status.no_interrupted_acp_task_to_continue" => "没有可继续的中断 ACP 任务",
        "status.no_queued_messages" => "没有排队消息",
        "status.no_recent_project_is_available" => "没有可用的最近项目",
        "status.no_run_details_for_selection" => "当前选择没有运行详情",
        "status.no_saved_sessions_found" => "未找到已保存的会话",
        "status.no_scenario_selected" => "未选择场景",
        "status.no_scenario_selected_for_step_insertion" => "未选择用于插入步骤的场景",
        "status.no_scenario_selected_to_run" => "未选择要运行的场景",
        "status.no_scenarios_to_run" => "没有可运行的场景",
        "status.no_selection_to_copy" => "没有可复制的选区",
        "status.no_step_to_edit" => "没有可编辑的步骤",
        "status.nothing_to_redo" => "没有可重做的操作",
        "status.nothing_to_undo" => "没有可撤销的操作",
        "status.open_run_details_first_enter_then_press_p_to_expand_secrets" => {
            "请先打开运行详情（Enter），再按 p 展开密钥"
        }
        "status.paste_works_inside_a_scenario" => "粘贴仅适用于场景内",
        "status.pending_command" => "待处理命令",
        "status.permission_sent_to_acp_agent" => "权限已发送给 ACP Agent...",
        "status.preview_closed" => "预览已关闭",
        "status.profile_not_found_for_editing" => "未找到要编辑的配置。",
        "status.project_picker_select_enter_open_esc_cancel" => {
            "项目选择：↑/↓ 选择 · Enter 打开 · Esc 取消"
        }
        "status.project_switch_canceled" => "项目切换已取消",
        "status.project_switch_needs_running_tasks_stopped_s_stop_esc_cancel" => {
            "切换项目前需停止运行中的任务：S 停止 · Esc 取消"
        }
        "status.project_owned_sessions_cannot_be_loaded_from_the_legacy_view" => {
            "属于项目的会话不能从旧版视图加载"
        }
        "status.project_owned_tasks_must_stop_before_switching" => "切换前必须停止属于该项目的任务",
        "status.queued_message_paused_configure_an_agent_then_send_a_message" => {
            "排队消息已暂停：请配置 Agent 后发送消息以继续"
        }
        "status.queued_message_restored_for_editing" => "排队消息已取回编辑",
        "status.removed_externally_deleted_feature_on_disk_run_evidence_reta" => {
            "已移除外部删除的功能；磁盘上的运行证据仍保留"
        }
        "status.requirement_changed_on_disk_unsaved_edits_kept" => {
            "需求在磁盘上已变更；未保存的编辑已保留"
        }
        "status.requirement_library_refreshed" => "需求库已刷新",
        "status.requirement_library_reloaded" => "需求库已重新加载",
        "status.requirement_link_is_stale_resolve_in_requirements_tab" => {
            "需求链接已失效 — 请在需求页处理"
        }
        "status.requirements_view_updated" => "需求视图已更新",
        "status.review_proposed_test_points_in_test_points_tab_5_then_press" => {
            "请在测试点页 [5] 审阅提议的测试点，然后按 c 继续生成"
        }
        "status.run_started" => "运行已开始",
        "status.runner_already_active" => "运行器已在运行",
        "status.runner_exited_with_error" => "运行器异常退出",
        "status.runner_is_still_active_wait_for_its_owned_process_to_exit" => {
            "运行器仍在运行；请等待其所属进程退出"
        }
        "status.save_current_edits_and_finish_pending_work_before_switching" => {
            "切换功能前请保存当前编辑并完成待处理工作"
        }
        "status.saved_requirement_document" => "需求文档已保存",
        "status.saved_test_points" => "测试点已保存",
        "status.scenario_expanded" => "场景已展开",
        "status.scenario_folded" => "场景已折叠",
        "status.select_agent_move_enter_select_esc_close" => {
            "选择 Agent：↑↓ 移动 · Enter 选择 · Esc 关闭"
        }
        "status.select_a_feature_and_wait_for_it_to_load_before_editing" => {
            "请选择功能并等待加载完成后再编辑"
        }
        "status.select_a_feature_and_wait_for_it_to_load_before_running" => {
            "请选择功能并等待加载完成后再运行"
        }
        "status.select_a_requirement_document_to_set_its_iteration" => "请选择需求文档以设置迭代",
        "status.select_a_requirement_document_to_set_its_priority" => "请选择需求文档以设置优先级",
        "status.select_a_test_point_to_approve" => "请选择要批准的测试点",
        "status.select_a_test_point_to_reject" => "请选择要驳回的测试点",
        "status.select_non_empty_requirement_text_to_create_a_test_point" => {
            "请选择非空需求文本以创建测试点"
        }
        "status.select_step_keyword_enter_esc_cancel" => "选择步骤关键字（↑↓ Enter，Esc 取消）",
        "status.selected_teshi_native_remembered_for_new_sessions" => {
            "已选择 Teshi Native；将用于新会话"
        }
        "status.sending_mindmap_context_to_ai" => "正在向 AI 发送思维导图上下文...",
        "status.sending_message_to_ai" => "正在向 AI 发送消息...",
        "status.sessions_for_the_active_project_sessions_legacy_shows_unassi" => {
            "当前项目的会话。旧版历史请用 /sessions legacy"
        }
        "status.showing_implemented_and_verified_requirements" => "正在显示已实现和已验证的需求",
        "status.showing_pending_requirements_only" => "仅显示未完成的需求",
        "status.showing_plaintext_secrets_p_to_hide" => "正在显示明文密钥（按 p 隐藏）",
        "status.step_cannot_move_further_down" => "步骤无法继续下移",
        "status.step_cannot_move_further_up" => "步骤无法继续上移",
        "status.step_copied" => "步骤已复制",
        "status.step_keyword_list_is_available_on_step_lines_only" => {
            "步骤关键字列表仅在步骤行可用"
        }
        "status.step_keyword_selection_canceled" => "已取消步骤关键字选择",
        "status.step_keyword_shortcuts_work_on_step_lines_only" => "步骤关键字快捷键仅在步骤行生效",
        "status.step_keyword_updated" => "步骤关键字已更新",
        "status.step_pasted" => "步骤已粘贴",
        "status.stop_requested_for_the_project_owned_runner_waiting_for_exit" => {
            "已请求停止项目运行器；正在等待退出"
        }
        "status.switched_back_to_strict_review_human_approval_required_resta" => {
            "已切回严格评审（需人工批准）。使用 /generate 重新开始以走评审流程。"
        }
        "status.switched_to_ai_tab" => "已切换到 AI 页",
        "status.switched_to_explore_tab" => "已切换到探索页",
        "status.switched_to_mindmap_tab" => "已切换到思维导图页",
        "status.switched_to_requirements_tab" => "已切换到需求页",
        "status.switched_to_test_points_tab" => "已切换到测试点页",
        "status.task_interrupted_retry_to_continue" => "任务已中断 — 使用 /retry 继续",
        "status.tasks_stopped_resolve_unsaved_state_before_switching" => {
            "任务已停止；请先处理未保存状态再切换"
        }
        "status.the_clipboard_holds_no_text_or_image" => "剪贴板中没有文本或图片",
        "status.the_project_directory_has_no_feature_files_add_one_to_begin" => {
            "项目目录中没有 .feature 文件。添加一个即可开始。"
        }
        "status.thinking_and_tool_details_toggled" => "已切换思考与工具详情",
        "status.tool_call_limit_reached_requesting_final_response" => {
            "已达工具调用上限，正在请求最终回复…"
        }
        "status.type_a_project_directory_and_press_enter_to_open_it_esc_canc" => {
            "输入项目目录并按 Enter 打开；Esc 取消"
        }
        "status.type_q_w_wq_or_x" => "输入 q、w、wq 或 x",
        "status.unassigned_session_use_sessions_legacy_to_load_it_explicitly" => {
            "未分配会话：请用 /sessions legacy 显式加载"
        }
        "status.unknown_slash_command_try_new_exit_resume_retry_copy_copy_co" => {
            "未知斜杠命令。可尝试 /new、/exit、/resume、/retry、/copy、/copy-code、/models、/sessions、/project、/approval、/agent、/preferences、/generate、/generate-direct、/review-strict、/continue"
        }
        "status.unlinked_requirement_from_current_project" => "已取消需求与当前项目的关联",
        "status.usage_acp_add_id_program_arg" => "用法：/acp add <id> [\"program\",\"arg\",...]",
        "status.usage_copy_n_where_n_is_a_positive_integer" => "用法：/copy [N]，N 为正整数",
        "status.usage_copy_code_n_where_n_is_a_positive_integer" => {
            "用法：/copy-code [N]，N 为正整数"
        }
        "status.usage_rename_title_rename_clear" => "用法：/rename <title> · /rename --clear",
        "status.usage_worktree_branch_path" => "用法：/worktree <branch> [path]",
        "status.vim_pending_command" => "Vim 待处理命令",
        "status.vim_pending_dd_delete" => "Vim 待处理：dd 删除",
        "status.vim_pending_gg_top" => "Vim 待处理：gg 顶部",
        "status.vim_pending_yy_yank" => "Vim 待处理：yy 复制",
        "status.waiting_for_project_owned_agent_backends_to_exit" => {
            "正在等待项目所属 Agent 后端退出"
        }
        "status.waiting_for_the_previous_agent_to_exit_before_loading_histor" => {
            "正在等待上一个 Agent 退出后再加载历史；Esc 取消"
        }
        "status.yanked_requirement_line" => "已复制需求行",
        "status.yanked_requirement_selection" => "已复制需求选区",
        "status.dd_to_delete" => "`dd` 删除",
        "status.yy_to_copy" => "`yy` 复制",
        "status.acp_registry_unavailable" => "ACP Registry 不可用：{error}",
        "status.acp_approval_preference_could_not_be_restored" => "无法恢复 ACP 审批偏好：{error}",
        "status.acp_install_failed" => "ACP 安装失败：{error}",
        "status.acp_installations_unavailable" => "ACP 安装信息不可用：{error}",
        "status.acp_storage_unavailable" => "ACP 存储不可用：{error}",
        "status.ai_applied_change_in" => "AI 已在 {arg1} 中应用变更“{arg0}”",
        "status.ai_applied_filter" => "AI 已应用筛选：{desc}",
        "status.ai_applied_highlight_rule_s" => "AI 已应用 {count} 条高亮规则",
        "status.ai_backend_unavailable" => "AI 后端不可用：{error}",
        "status.ai_created_file" => "AI 已创建文件：{file_name}",
        "status.ai_error" => "AI 错误：{error}",
        "status.ai_error_message" => "AI 错误：{message}",
        "status.ai_response_received" => "已收到 AI 回复（{model}）",
        "status.ai_wants_to_y_accept_n_reject" => "AI 想要{arg0}。[Y] 接受 [N] 拒绝",
        "status.added_and_activated_profile" => "已添加并启用配置：{arg0}",
        "status.added_custom_acp_agent_use_agent_to_select_it" => {
            "已添加自定义 ACP Agent {name}。使用 /agent 选择。"
        }
        "status.advanced_to_scenario_planning_with_approved_test_point_s" => {
            "已进入场景规划，共批准 {arg0} 个测试点"
        }
        "status.agent_selected_but_preference_could_not_be_saved" => {
            "Agent 已选择，但偏好保存失败：{error}"
        }
        "status.approved_test_point" => "已批准测试点 {tp_id}",
        "status.batch_approved_test_point_s" => "已批量批准 {count} 个测试点",
        "status.cannot_derive_a_worktree_path_for_pass_one_explicitly" => {
            "无法为 {arg0} 推导 worktree 路径；请显式指定"
        }
        "status.cannot_open" => "无法打开 {noun}：{error}",
        "status.cannot_resolve_session_project" => "无法解析会话项目：{error}",
        "status.cannot_save_before_switching" => "切换前保存失败：{error}",
        "status.cannot_send_queued_message" => "无法发送排队消息：{error}",
        "status.change_summary_change_s_navigate_enter_jump_esc_close" => {
            "变更摘要 — {arg0} 项变更。↑↓ 导航，Enter 跳转，Esc 关闭。"
        }
        "status.closed" => "已关闭 {name}",
        "status.context_compacted_message_s_summary_token_window" => {
            "上下文已压缩：{removed_count} 条消息 → 摘要（{arg0} token 窗口）"
        }
        "status.conversation_renamed_to" => "会话已重命名为“{title}”",
        "status.conversation_title_reset_to" => "会话标题已重置为“{arg0}”",
        "status.copied_code_block_line_s" => "已复制代码块（{lines} 行）",
        "status.copied_code_block_line_s_2" => "已复制代码块 {arg0}（{lines} 行）",
        "status.copied_last_assistant_response_s_to_clipboard" => {
            "已复制最近 {arg0} 条助手回复到剪贴板"
        }
        "status.copied_session_id" => "已复制会话 ID {id}",
        "status.copied_character_s" => "已复制 {arg0} 个字符",
        "status.copied_line_s" => "已复制 {line_count} 行",
        "status.could_not_apply_acp_file_change" => "无法应用 ACP 文件变更：{error}",
        "status.could_not_checkpoint_project_a" => "无法检查点项目 A：{error}",
        "status.could_not_copy_to_clipboard" => "无法复制到剪贴板：{error}",
        "status.could_not_create_requirement" => "无法创建需求：{error}",
        "status.could_not_delete_requirement" => "无法删除需求：{error}",
        "status.could_not_prepare_project_b" => "无法准备项目 B：{error}",
        "status.could_not_reconnect_acp_for_profile" => "无法为配置 {arg0} 重连 ACP：{error}",
        "status.could_not_resolve_project" => "无法解析项目：{error}",
        "status.could_not_save_requirement" => "无法保存需求：{error}",
        "status.could_not_save_test_point" => "无法保存测试点：{error}",
        "status.created_agent" => "已创建 Agent {id}",
        "status.created_proposed_test_point" => "已创建待批准测试点 {id}",
        "status.created_requirement_document_md" => "已创建需求文档 {id}.md",
        "status.custom_acp_agent_failed" => "自定义 ACP Agent 失败：{error}",
        "status.cut_character_s" => "已剪切 {arg0} 个字符",
        "status.deleted_global_requirement" => "已删除全局需求 {document_id}",
        "status.deleted_profile" => "已删除配置：{name}",
        "status.deleted_session" => "已删除会话 {id}",
        "status.direct_no_review_generation_traceability_confirm_source_scop" => {
            "直接生成（免评审），可追溯性：{arg0} — 确认来源范围（Enter 确认 · Esc 取消 · i 修改迭代）"
        }
        "status.exploration_terminated_url_is_not_in_the_whitelist" => {
            "探索已终止：URL“{arg0}”不在白名单中"
        }
        "status.exploration_terminated_step_limit_exceeded" => "探索已终止：超过步骤上限（{arg0}）",
        "status.failed_to_activate_profile" => "启用配置失败：{e}",
        "status.failed_to_delete_profile" => "删除配置失败：{e}",
        "status.failed_to_delete_session" => "删除会话失败：{e}",
        "status.failed_to_reload" => "重新加载 {arg0} 失败：{err}",
        "status.failed_to_restore_generation_state" => "恢复生成状态失败：{e}",
        "status.failed_to_save_active_profile" => "保存当前配置失败：{e}",
        "status.failed_to_save_profile" => "保存配置失败：{e}",
        "status.feature_changed_on_disk_reload_enter_r_or_keep_local_esc_k" => {
            "功能在磁盘上已变更：{arg0}。重新加载 [Enter/r] 或保留本地 [Esc/k]。"
        }
        "status.feature_discovery_failed" => "功能发现失败：{error}",
        "status.feature_loading_failed" => "功能加载失败：{error}",
        "status.generation_paused" => "生成已暂停：{err}",
        "status.installed_use_agent_to_select_it" => "已安装 {arg0}。使用 /agent 选择。",
        "status.installing" => "正在安装 {arg0}...",
        "status.iteration_is_not_in_the_current_requirement_store_choose_ano" => {
            "迭代“{name}”不在当前需求库中；请选择其他范围"
        }
        "status.kept_local_buffer_for" => "已保留 {arg0} 的本地缓冲区",
        "status.llm_configured_model_base_url_provider" => {
            "LLM 已配置：model={arg0}，base_url={arg1}，provider={arg2}"
        }
        "status.llm_not_configured" => "LLM 未配置：{e}",
        "status.loaded_session_with_messages" => "已加载含 {arg0} 条消息的会话",
        "status.message_queued_unqueue_to_edit" => "消息已排队（{arg0}）— /unqueue 可编辑",
        "status.no_code_block_in_the_last_assistant_response" => {
            "最近一条助手回复中没有代码块 {arg0}"
        }
        "status.opened_requirement_excerpt_in" => "已在 {arg0} 中打开需求摘录",
        "status.opened_scenario_in" => "已在 {arg1} 中打开场景 {arg0}",
        "status.opened_in_the_system_viewer" => "已在系统查看器中打开 {noun}",
        "status.plaintext_expand_needs_a_running_api_sidecar" => {
            "展开明文需要运行中的 API sidecar：{err}"
        }
        "status.profile_has_no_api_key_set_one_or_use_teshi_llm_api_key" => {
            "配置“{arg0}”没有 API Key；请设置或使用 TESHI_LLM_API_KEY"
        }
        "status.profile_saved_but_failed_to_activate" => "配置已保存但启用失败：{e}",
        "status.project_directory_does_not_exist" => "项目目录不存在：{arg0}",
        "status.project_switch_could_not_continue" => "项目切换无法继续：{error}",
        "status.project_switch_stopped_owned_task_did_not_exit_within_s_proj" => {
            "项目切换已停止：所属任务未在 {arg0} 秒内退出；项目 A 仍处于活动状态"
        }
        "status.rejected_ai_change" => "已拒绝 AI 变更：{desc}",
        "status.rejected_test_point" => "已驳回测试点 {tp_id}",
        "status.reloaded_from_disk" => "已从磁盘重新加载：{arg0}",
        "status.requirement_library_refresh_failed" => "需求库刷新失败：{error}",
        "status.requirement_library_reload_failed" => "需求库重新加载失败：{error}",
        "status.requirement_scope" => "需求范围：{arg0}",
        "status.resumed_session_from_messages" => "已从 {arg0} 条消息恢复会话",
        "status.run_complete_passed_failed" => "运行完成：{passed} 通过，{failed} 失败",
        "status.run_complete_passed_failed_blocked" => {
            "运行完成：{passed} 通过，{failed} 失败，{blocked} 阻塞"
        }
        "status.run_not_started" => "运行未启动：{first_line}",
        "status.runner_error" => "运行器错误：{message}",
        "status.saved" => "已保存 {arg0}",
        "status.selected_acp_agent_remembered_for_new_sessions" => {
            "已选择 ACP Agent {arg0}；将用于新会话"
        }
        "status.session_belongs_to_another_project" => "会话属于另一个项目：{arg0}",
        "status.settings_applied_but_not_saved" => "设置已应用但未保存：{error}",
        "status.step_keyword_set_to" => "步骤关键字已设为 {keyword}",
        "status.switched_project_but_recent_history_was_not_saved" => {
            "已切换项目，但最近记录未保存：{error}"
        }
        "status.switched_project" => "已切换项目：{arg0}",
        "status.switched_to_agent" => "已切换到 Agent：{arg0}",
        "status.switched_to_model" => "已切换到模型：{arg0}",
        "status.switched_to" => "已切换到 {arg0}",
        "status.unknown_auth_subcommand_try_auth_auth_add_auth_status_auth_r" => {
            "未知 auth 子命令：“{arg0}”。可尝试：auth、auth add、auth status、auth remove <provider>"
        }
        "status.unknown_command" => "未知命令：{other}",
        "status.unsaved_project_state_s_save_switch_d_discard_switch_esc_can" => {
            "未保存的项目状态：{arg0} · S 保存并切换 · D 放弃并切换 · Esc 取消"
        }
        "status.updated_profile" => "已更新配置：{arg0}",
        "status.view" => "视图：{arg0}",
        "status.workspace_has_a_single_member" => "工作区 {arg0} 只有一个成员",
        "status.worktree_not_created" => "未创建 worktree：{error}",
        "status.already_works_in_this_member_press_a_for_a_new_agent_in_anot" => {
            "{arg0} 已在此成员中工作；按 a 在其他成员中新建 Agent"
        }
        "status.belongs_to_no_workspace" => "{arg0} 不属于任何工作区",
        "status.is_already_installed_use_agent_to_select_it" => "{arg0} 已安装。使用 /agent 选择。",
        "status.now_works_in" => "{arg0} 现在工作于 {label}",
        "status.now_works_in_workspace_created_for_it" => {
            "{arg0} 现在工作于 {label}；已为其创建工作区 {arg1}"
        }
        "status." => "{arg0} — {error}",
        "status.images_attached" => "已附加 {count} 张图片",
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

/// Format one [`fill`] value for a status template.
///
/// Borrowing the value keeps field expressions such as `self.agent().title`
/// movable-free; the helper also avoids the `format!("{}", value)` pattern that
/// clippy flags as a useless format.
pub fn arg<T: std::fmt::Display + ?Sized>(value: &T) -> String {
    value.to_string()
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
    fn status_templates_fill_in_both_locales() {
        set_effective_locale(EffectiveUiLocale::ZhCn);
        assert_eq!(
            fill(t("status.ai_error"), &[("error", "boom")]),
            "AI 错误：boom"
        );
        assert_eq!(
            fill(
                t("status.run_complete_passed_failed"),
                &[("passed", "2"), ("failed", "1")]
            ),
            "运行完成：2 通过，1 失败"
        );
        set_effective_locale(EffectiveUiLocale::En);
        assert_eq!(
            fill(t("status.ai_error"), &[("error", "boom")]),
            "AI error: boom"
        );
        assert_eq!(
            fill(
                t("status.run_complete_passed_failed"),
                &[("passed", "2"), ("failed", "1")]
            ),
            "Run complete: 2 passed, 1 failed"
        );
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
