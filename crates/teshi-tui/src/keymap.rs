use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{MainTab, MindMapFocus, ViewStage};
use crate::authoring_tab::{RequirementsEditorMode, RequirementsFocus};
use crate::test_points_tab::TestPointsFocus;

/// Inputs for [`Action::from_key_event`] to resolve mode-specific bindings.
#[derive(Debug, Clone, Copy)]
pub struct KeyContext {
    pub step_keyword_picker_active: bool,
    pub step_input_active: bool,
    pub external_change_prompt_active: bool,
    pub agent_change_prompt_active: bool,
    pub active_tab: MainTab,
    pub view_stage: ViewStage,
    pub explore_edit_mode: bool,
    pub pending_char: Option<char>,
    pub mindmap_focus: MindMapFocus,
    pub mindmap_ai_panel_visible: bool,
    pub ai_input_focused: bool,
    pub slash_suggestion_active: bool,
    /// Whether the auth management panel is currently open (as an overlay).
    pub auth_panel_active: bool,
    /// Whether the model profile management panel is currently open.
    pub model_panel_active: bool,
    /// Whether the model panel is in "adding" form mode (vs list mode).
    pub model_panel_adding: bool,
    /// Whether the session browser panel is currently open.
    pub session_panel_active: bool,
    /// Whether the Change Summary overlay is visible.
    pub change_summary_visible: bool,
    /// Whether the AI agent is currently waiting for an LLM response.
    pub ai_status_waiting: bool,
    /// Whether the scenario location dropdown is open in the MindMap preview panel.
    pub scenario_dropdown_open: bool,
    /// Whether the approval mode selection panel is open.
    pub approval_panel_active: bool,
    /// Whether the agent profile selection panel is open.
    pub agent_profile_panel_active: bool,
    /// Active pane when the Requirements tab is selected.
    pub requirements_focus: RequirementsFocus,
    /// Independent Markdown editor mode.
    pub requirements_editor_mode: RequirementsEditorMode,
    /// Active pane when the Test Points tab is selected.
    pub test_points_focus: TestPointsFocus,
    /// Whether a Requirements filter/iteration overlay is open.
    pub requirements_overlay_active: bool,
    /// The Requirements overlay accepts text rather than list-navigation keys.
    pub requirements_text_overlay: bool,
    /// Whether the AI generation source-scope confirmation overlay is open.
    pub generation_scope_prompt_active: bool,

    /// Whether the quit confirmation panel is open.
    pub quit_pending_confirm: bool,

    /// Whether an external ACP agent has a pending permission prompt.
    pub acp_permission_pending: bool,
    /// Whether the project switcher owns keyboard input.
    pub project_switch_active: bool,
}

/// High-level editor command derived from keyboard input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Action {
    // Editor movement (stage 3)
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    PageUp,
    PageDown,
    // Text editing
    Insert(char),
    Enter,
    Backspace,
    Delete,
    InsertNewline,
    // BDD structural editing
    MoveStepUp,
    MoveStepDown,
    /// Navigate to the previous sibling step within the same scenario (Shift+Up).
    MoveSiblingUp,
    /// Navigate to the next sibling step within the same scenario (Shift+Down).
    MoveSiblingDown,
    SwitchKeyword(&'static str),
    InsertStepBelow,
    InsertStepAbove,
    NewScenario,
    DeleteNode,
    CopyStep,
    PasteStep,
    ToggleScenarioFold,
    FoldAllScenarios,
    RunBackground,
    Undo,
    Redo,
    PendingChar(char),
    // Requirements tab authoring
    ReqNewTestPoint,
    /// Enter Requirements text input.
    ReqEnterInsert,
    /// Return to Requirements navigation.
    ReqExitInsert,
    ReqNewDocument,
    /// Open confirmation to remove the selected document from the global library.
    ReqDeleteDocument,
    /// Open the iteration filter picker (`i` on the Requirements tree).
    ReqFilterOverlay,
    /// Toggle path vs iteration grouping (`g` on the Requirements tree).
    ReqGroupToggle,
    /// Edit the selected document's iteration (`I` on the Requirements tree).
    ReqEditIteration,
    /// Edit the selected document's optional priority (`p` on the Requirements tree).
    ReqEditPriority,
    /// Bounded Vim append/open/delete/yank operations for Requirements.
    ReqVimAppend,
    ReqVimOpenBelow,
    ReqVimOpenAbove,
    ReqVimDeleteChar,
    ReqVimDeleteLine,
    ReqVimYankLine,
    ReqVimPasteAfter,
    ReqVimPasteBefore,
    ReqVimWordForward,
    ReqVimWordBackward,
    ReqVimWordEnd,
    ReqVimLineStart,
    ReqVimFirstNonBlank,
    ReqVimLineEnd,
    ReqVimGoTop,
    ReqVimGoBottom,
    ReqVimVisualChar,
    ReqVimVisualLine,
    ReqVimVisualYank,
    ReqVimVisualDelete,
    /// Confirm the active Requirements overlay.
    OverlayConfirm,
    /// Cancel the active Requirements overlay.
    OverlayCancel,
    /// Move the overlay list selection up.
    OverlayMoveUp,
    /// Move the overlay list selection down.
    OverlayMoveDown,
    /// Insert into the overlay text field.
    OverlayInsert(char),
    /// Backspace in the overlay text field.
    OverlayBackspace,
    /// Confirm the pending generation source-scope overlay.
    GenerationScopeConfirm,
    /// Cancel the pending generation source-scope overlay.
    GenerationScopeCancel,
    /// Change the pending generation iteration without starting gathering.
    GenerationScopeChangeIteration,
    // Test Points tab review and navigation
    TpApprove,
    TpReject,
    TpBatchApprove,
    TpReviewFilter,
    TpFollowExcerpt,
    /// Human-only: advance from Reviewing Test Points to Planning.
    TpContinueGeneration,
    /// Open a realized Gherkin scenario linked from the selected test point.
    TpFollowScenario,
    // Global
    Save,
    Quit,
    SelectTab(MainTab),
    ActivateStepInput,
    /// Cancel only the pending pane-navigation chord, preserving pane state.
    CancelPaneNavigation,
    ClearInputState,
    // Explore navigation
    FocusNextColumn,
    FocusPrevColumn,
    /// Move to the nearest visible pane below the current pane.
    FocusPaneDown,
    /// Move to the nearest visible pane above the current pane.
    FocusPaneUp,
    ExploreRight,
    RunScenario,
    AiSuggest,
    EnterEdit,
    ToggleFailureDetail,
    /// Expand or hide redacted HTTP fields in Explore run details.
    ToggleInspectorPlaintext,
    // Step keyword picker overlay
    StepKeywordPickerUp,
    StepKeywordPickerDown,
    StepKeywordPickerConfirm,
    StepKeywordPickerCancel,
    ExternalChangeReload,
    ExternalChangeKeepLocal,
    /// Accept a pending agent-originated text change.
    AgentChangeAccept,
    /// Reject a pending agent-originated text change.
    AgentChangeReject,
    /// View the diff for the pending agent change (Explore tab).
    AgentChangeDiff,
    /// Toggle the Change Summary overlay (MindMap tab).
    ToggleChangeSummary,
    /// Navigate up in the Change Summary list.
    ChangeSummaryUp,
    /// Navigate down in the Change Summary list.
    ChangeSummaryDown,
    /// Jump to the selected Change Summary node in the MindMap tree.
    ChangeSummaryJump,
    // AI tab input
    AiSendChar(char),
    AiSendMessage,
    /// Continue an interrupted ACP task in the current session.
    AiContinueTask,
    AiBackspace,
    /// Scroll chat history up (Alt+Up).
    AiScrollUp,
    /// Scroll chat history down (Alt+Down).
    AiScrollDown,
    /// Scroll chat content left (Alt+Left).
    AiScrollLeft,
    /// Scroll chat content right (Alt+Right).
    AiScrollRight,
    /// Blur the AI input (Esc while focused, or Enter on empty input).
    AiBlurInput,
    /// Focus the AI input (Enter while blurred).
    AiFocusInput,
    AiDelete,
    AiCursorLeft,
    AiCursorRight,
    AiCursorHome,
    AiCursorEnd,
    /// Jump to top of chat history (Ctrl+Home).
    AiScrollTop,
    /// Jump to bottom of chat history (Ctrl+End).
    AiScrollBottom,
    /// Insert pasted text into the AI input buffer in one shot.
    AiPaste(String),
    /// Insert a literal newline into the AI input (Shift+Enter).
    AiNewline,
    /// Paste clipboard content into the AI input (Ctrl+V).
    AiClipboardPaste,
    /// Cancel the current AI request / tool-call loop.
    AiCancel,
    // ── Agent lifecycle ──────────────────────────────
    AgentNew,
    AgentClose,
    AgentNext,
    AgentPrev,
    /// Select the visible conversation at this index without creating a turn.
    AgentSelect(usize),
    // ── Slash command suggestion popup ───────────────
    AiSlashPrev,
    AiSlashNext,
    AiSlashSelect,
    AiSlashDismiss,
    /// Highlight a visible slash suggestion without executing it.
    AiSlashChoose(usize),
    // ── ACP agent permission prompt ───────────────────
    /// Move the ACP permission prompt selection up.
    AcpPermissionUp,
    /// Move the ACP permission prompt selection down.
    AcpPermissionDown,
    /// Allow the highlighted ACP permission option.
    AcpPermissionConfirm,
    /// Reject the ACP permission request.
    AcpPermissionReject,
    /// Highlight a permission choice without approving the pending request.
    AcpPermissionChoose(usize),
    /// Send the selected MindMap node context as a user message to the AI.
    MindMapSendToAi,
    /// Toggle the AI preview panel visibility (global `Ctrl+\`).
    ToggleMindMapAiPanel,
    /// Move keyboard focus from tree to the AI preview panel.
    MindMapFocusAiPanel,
    // Auth panel
    /// Close the auth management overlay.
    AuthPanelClose,
    // Model profile panel
    /// Open the model profile management overlay.
    ModelPanelOpen,
    /// Close the model profile management overlay.
    ModelPanelClose,
    /// Activate the currently selected profile.
    ModelPanelActivate,
    /// Navigate up in the profile list.
    ModelPanelUp,
    /// Navigate down in the profile list.
    ModelPanelDown,
    /// Switch to "Add model" form mode.
    ModelPanelAdd,
    /// Edit the currently selected profile.
    ModelPanelEdit,
    /// Delete the currently selected profile.
    ModelPanelDelete,
    /// Focus the next form field (Tab).
    ModelPanelFormNext,
    /// Focus the previous form field (Shift+Tab).
    ModelPanelFormPrev,
    /// Insert a character into the currently focused form field.
    ModelPanelFormInsert(char),
    /// Delete the character before the cursor in the focused form field.
    ModelPanelFormBackspace,
    /// Submit the form and create the profile.
    ModelPanelFormSubmit,
    /// Cancel the form and return to the list.
    ModelPanelFormCancel,
    // Tree navigation (stages 1 & 2)
    TreeUp,
    TreeDown,
    TreeExpand,
    TreeCollapse,
    TreeToggle,
    /// Reserved for tests and future bindings; the MindMap tree is display-only (no Enter preview).
    #[allow(dead_code)]
    TreeOpen,
    TreeHome,
    TreeEnd,
    /// Cycle the stage-2 preview to the previous source location for a shared step path (left bracket).
    TreeLocationPrev,
    /// Cycle the stage-2 preview to the next source location for a shared step path (right bracket).
    TreeLocationNext,
    /// Navigate to the previous sibling node in the MindMap tree (Shift+Up).
    TreeSiblingPrev,
    /// Navigate to the next sibling node in the MindMap tree (Shift+Down).
    TreeSiblingNext,
    // Chat/editor selection actions
    /// Copy the current mouse-drag selection to the system clipboard and clear it.
    CopySelection,
    /// Clear the mouse-drag selection without copying.
    #[allow(dead_code)]
    ClearSelection,
    // ── Session browser panel actions ─────────────────
    /// Open the session browser panel (triggered via `/sessions` slash command).
    #[allow(dead_code)]
    SessionPanelOpen,
    SessionPanelClose,
    SessionPanelUp,
    SessionPanelDown,
    SessionPanelActivate,
    SessionPanelDelete,
    // ── Scenario location dropdown ──────────────
    /// Open/close the scenario location dropdown in the MindMap preview panel.
    ToggleScenarioDropdown,
    /// Select the highlighted item in the scenario dropdown.
    ScenarioDropdownSelect,
    /// Move selection up in the scenario dropdown.
    ScenarioDropdownUp,
    /// Move selection down in the scenario dropdown.
    ScenarioDropdownDown,
    /// Close the scenario dropdown without selecting.
    ScenarioDropdownClose,
    // ── Approval mode panel actions ───────────
    /// Navigate up in the approval mode selection panel.
    ApprovalPanelUp,
    /// Navigate down in the approval mode selection panel.
    ApprovalPanelDown,
    /// Confirm the selected approval mode.
    ApprovalPanelSelect,
    // ── Agent profile panel actions ───────────
    /// Navigate up in the agent profile selection panel.
    AgentPanelUp,
    /// Navigate down in the agent profile selection panel.
    AgentPanelDown,
    /// Confirm the selected agent profile.
    AgentPanelSelect,
    /// Close the agent profile selection panel.
    AgentPanelClose,
    /// Switch to "Add agent" form mode.
    AgentPanelAdd,
    /// Edit the currently selected agent profile.
    AgentPanelEdit,
    /// Delete the currently selected agent profile.
    AgentPanelDelete,
    /// Focus the next form field (Tab).
    AgentPanelFormNext,
    /// Focus the previous form field (Shift+Tab).
    AgentPanelFormPrev,
    /// Switch to the previous config tab (Left).
    AgentPanelFormTabPrev,
    /// Switch to the next config tab (Right).
    AgentPanelFormTabNext,
    /// Insert a character into the currently focused form field.
    AgentPanelFormInsert(char),
    /// Delete the character before the cursor in the focused form field.
    AgentPanelFormBackspace,
    /// Submit the form and create the profile.
    AgentPanelFormSubmit,
    /// Cancel the form and return to the list.
    AgentPanelFormCancel,
}

impl Action {
    pub fn from_key_event(event: KeyEvent, context: KeyContext) -> Option<Self> {
        if context.project_switch_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                (KeyCode::Enter, _) => Some(Self::OverlayConfirm),
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    Some(Self::OverlayMoveUp)
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::OverlayMoveDown)
                }
                (KeyCode::Backspace, _) => Some(Self::OverlayBackspace),
                (KeyCode::Char(ch), modifiers)
                    if !ch.is_control()
                        && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
                {
                    Some(Self::OverlayInsert(ch))
                }
                _ => None,
            };
        }
        if let Some(pending_char) = context.pending_char {
            if pending_char == 'w' && pane_navigation_allowed(context) {
                // Some terminal/input paths retain CONTROL (or add SHIFT for
                // an uppercase character) on the second key of the chord.
                // Treat h/H and l/L equivalently so Ctrl-W h/l works across
                // the native and VT input paths.
                let has_only_supported_modifiers = event.modifiers.is_empty()
                    || event.modifiers == KeyModifiers::SHIFT
                    || event.modifiers == KeyModifiers::CONTROL
                    || event.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT);
                if has_only_supported_modifiers {
                    return match event.code {
                        KeyCode::Char('h') | KeyCode::Char('H') => Some(Self::FocusPrevColumn),
                        KeyCode::Char('l') | KeyCode::Char('L') => Some(Self::FocusNextColumn),
                        KeyCode::Char('j') | KeyCode::Char('J') => Some(Self::FocusPaneDown),
                        KeyCode::Char('k') | KeyCode::Char('K') => Some(Self::FocusPaneUp),
                        KeyCode::Esc => Some(Self::CancelPaneNavigation),
                        _ => Some(Self::CancelPaneNavigation),
                    };
                }
                return Some(Self::CancelPaneNavigation);
            }
            if context.active_tab == MainTab::Requirements
                && context.requirements_focus == RequirementsFocus::Editor
                && matches!(
                    context.requirements_editor_mode,
                    RequirementsEditorMode::Browse
                        | RequirementsEditorMode::VisualChar
                        | RequirementsEditorMode::VisualLine
                )
            {
                match (pending_char, event.code, event.modifiers) {
                    ('d', KeyCode::Char('d'), KeyModifiers::NONE) => {
                        return Some(
                            if context.requirements_editor_mode == RequirementsEditorMode::Browse {
                                Self::ReqVimDeleteLine
                            } else {
                                Self::ReqVimVisualDelete
                            },
                        );
                    }
                    ('y', KeyCode::Char('y'), KeyModifiers::NONE) => {
                        return Some(
                            if context.requirements_editor_mode == RequirementsEditorMode::Browse {
                                Self::ReqVimYankLine
                            } else {
                                Self::ReqVimVisualYank
                            },
                        );
                    }
                    ('g', KeyCode::Char('g'), KeyModifiers::NONE) => {
                        return Some(Self::ReqVimGoTop);
                    }
                    _ => {}
                }
            }
            match (pending_char, event.code, event.modifiers) {
                ('d', KeyCode::Char('d'), KeyModifiers::NONE) => return Some(Self::DeleteNode),
                ('y', KeyCode::Char('y'), KeyModifiers::NONE) => return Some(Self::CopyStep),
                _ => {}
            }
        }

        // Ctrl+C always maps to Quit, regardless of mode.
        if event.code == KeyCode::Char('c') && event.modifiers == KeyModifiers::CONTROL {
            return Some(Self::Quit);
        }

        if context.external_change_prompt_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Enter, _) | (KeyCode::Char('r'), KeyModifiers::NONE) => {
                    Some(Self::ExternalChangeReload)
                }
                (KeyCode::Esc, _)
                | (KeyCode::Char('k'), KeyModifiers::NONE)
                | (KeyCode::Char('K'), KeyModifiers::SHIFT) => Some(Self::ExternalChangeKeepLocal),
                _ => None,
            };
        }

        // Agent change confirmation prompt intercepts Y / N / D / S / Esc.
        // Esc while input is focused blurs first; a second Esc rejects.
        if context.agent_change_prompt_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Char('y'), KeyModifiers::NONE)
                | (KeyCode::Char('Y'), KeyModifiers::SHIFT) => Some(Self::AgentChangeAccept),
                (KeyCode::Char('n'), KeyModifiers::NONE)
                | (KeyCode::Char('N'), KeyModifiers::SHIFT) => Some(Self::AgentChangeReject),
                (KeyCode::Esc, _) if context.ai_input_focused => Some(Self::AiBlurInput),
                (KeyCode::Esc, _) => Some(Self::AgentChangeReject),
                (KeyCode::Char('d'), KeyModifiers::NONE)
                | (KeyCode::Char('D'), KeyModifiers::SHIFT) => Some(Self::AgentChangeDiff),
                (KeyCode::Char('s'), KeyModifiers::NONE)
                | (KeyCode::Char('S'), KeyModifiers::SHIFT) => Some(Self::ToggleChangeSummary),
                _ => None,
            };
        }

        // ACP permission prompt intercepts list navigation and the decision.
        // Mirrors Zed's agent permission panel: move, Enter to allow the
        // highlighted option, Esc to reject.
        if context.acp_permission_pending {
            return match (event.code, event.modifiers) {
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    Some(Self::AcpPermissionUp)
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::AcpPermissionDown)
                }
                (KeyCode::Enter, _) => Some(Self::AcpPermissionConfirm),
                (KeyCode::Esc, _) => Some(Self::AcpPermissionReject),
                _ => None,
            };
        }

        // Quit confirmation panel intercept
        if context.quit_pending_confirm {
            return match (event.code, event.modifiers) {
                (KeyCode::Char('q'), KeyModifiers::NONE)
                | (KeyCode::Char('y'), KeyModifiers::NONE)
                | (KeyCode::Enter, _) => Some(Self::Quit),
                (KeyCode::Char('n'), KeyModifiers::NONE)
                | (KeyCode::Char('N'), KeyModifiers::SHIFT)
                | (KeyCode::Esc, _) => Some(Self::ClearInputState),
                (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                    Some(Self::MoveLeft)
                }
                (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                    Some(Self::MoveRight)
                }
                _ => None,
            };
        }

        // Agent profile selection panel (list mode; form keys handled above).
        if context.agent_profile_panel_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    Some(Self::AgentPanelUp)
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::AgentPanelDown)
                }
                (KeyCode::Enter, _) => Some(Self::AgentPanelSelect),
                (KeyCode::Esc, _) => Some(Self::AgentPanelClose),
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::AgentPanelAdd),
                (KeyCode::Char('i'), KeyModifiers::NONE) => Some(Self::AgentPanelAdd),
                (KeyCode::Char('e'), KeyModifiers::NONE) => Some(Self::AgentPanelEdit),
                (KeyCode::Char('d'), KeyModifiers::NONE) => Some(Self::AgentPanelDelete),
                _ => None,
            };
        }

        // Approval mode selection panel intercepts keys while open
        if context.approval_panel_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    Some(Self::ApprovalPanelUp)
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::ApprovalPanelDown)
                }
                (KeyCode::Enter, _) => Some(Self::ApprovalPanelSelect),
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                _ => None,
            };
        }

        // Scenario location dropdown intercepts keys while open
        if context.scenario_dropdown_open {
            return match (event.code, event.modifiers) {
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => Some(Self::ScenarioDropdownDown),
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => Some(Self::ScenarioDropdownUp),
                (KeyCode::Enter, _) => Some(Self::ScenarioDropdownSelect),
                (KeyCode::Esc, _) | (KeyCode::Char('o'), _) => Some(Self::ScenarioDropdownClose),
                _ => None,
            };
        }

        // Auth panel intercepts all keys while open
        if context.auth_panel_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Esc, _) => Some(Self::AuthPanelClose),
                _ => None,
            };
        }

        // Model profile panel intercepts all keys while open
        if context.model_panel_active {
            if context.model_panel_adding {
                // ── Adding form mode ──
                return match (event.code, event.modifiers) {
                    (KeyCode::Esc, _) => Some(Self::ModelPanelFormCancel),
                    (KeyCode::Tab, _) => Some(Self::ModelPanelFormNext),
                    (KeyCode::BackTab, _) => Some(Self::ModelPanelFormPrev),
                    (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                        Some(Self::ModelPanelFormPrev)
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                        Some(Self::ModelPanelFormNext)
                    }
                    (KeyCode::Enter, _) => Some(Self::ModelPanelFormSubmit),
                    (KeyCode::Backspace, _) => Some(Self::ModelPanelFormBackspace),
                    (KeyCode::Char(ch), _) if !ch.is_control() => {
                        Some(Self::ModelPanelFormInsert(ch))
                    }
                    _ => None,
                };
            } else {
                // ── List mode ──
                return match (event.code, event.modifiers) {
                    (KeyCode::Esc, _) => Some(Self::ModelPanelClose),
                    (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                        Some(Self::ModelPanelUp)
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                        Some(Self::ModelPanelDown)
                    }
                    (KeyCode::Enter, _) => Some(Self::ModelPanelActivate),
                    (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::ModelPanelAdd),
                    (KeyCode::Char('e'), KeyModifiers::NONE) => Some(Self::ModelPanelEdit),
                    (KeyCode::Char('d'), KeyModifiers::NONE) => Some(Self::ModelPanelDelete),
                    _ => None,
                };
            }
        }

        // Session browser panel intercepts all keys while open
        if context.session_panel_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    Some(Self::SessionPanelUp)
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::SessionPanelDown)
                }
                (KeyCode::Enter, _) => Some(Self::SessionPanelActivate),
                (KeyCode::Char('d'), KeyModifiers::NONE) => Some(Self::SessionPanelDelete),
                (KeyCode::Esc, _) => Some(Self::SessionPanelClose),
                _ => None,
            };
        }

        // Change Summary panel intercepts all keys while visible
        if context.change_summary_visible {
            return match (event.code, event.modifiers) {
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    Some(Self::ChangeSummaryUp)
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::ChangeSummaryDown)
                }
                (KeyCode::Enter, _) => Some(Self::ChangeSummaryJump),
                _ => None,
            };
        }

        // Requirements overlay intercepts keys while open (including from generation scope).
        if context.requirements_overlay_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Char(ch), modifiers)
                    if context.requirements_text_overlay
                        && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
                {
                    Some(Self::OverlayInsert(ch))
                }
                (KeyCode::Esc, _) => Some(Self::OverlayCancel),
                (KeyCode::Enter, _) => Some(Self::OverlayConfirm),
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                    Some(Self::OverlayMoveUp)
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::OverlayMoveDown)
                }
                (KeyCode::Backspace, _) => Some(Self::OverlayBackspace),
                (KeyCode::Char(ch), modifiers)
                    if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
                {
                    Some(Self::OverlayInsert(ch))
                }
                _ => None,
            };
        }

        // Generation source-scope confirmation intercepts keys while open.
        if context.generation_scope_prompt_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Enter, _) => Some(Self::GenerationScopeConfirm),
                (KeyCode::Esc, _) => Some(Self::GenerationScopeCancel),
                (KeyCode::Char('i'), KeyModifiers::NONE) => {
                    Some(Self::GenerationScopeChangeIteration)
                }
                _ => None,
            };
        }

        // Step keyword picker intercepts all keys
        if context.step_keyword_picker_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Esc, _) => Some(Self::StepKeywordPickerCancel),
                (KeyCode::Up, _) => Some(Self::StepKeywordPickerUp),
                (KeyCode::Down, _) => Some(Self::StepKeywordPickerDown),
                (KeyCode::Enter, _) => Some(Self::StepKeywordPickerConfirm),
                _ => None,
            };
        }

        // Step text input mode
        if context.step_input_active && context.active_tab != MainTab::Requirements {
            return match (event.code, event.modifiers) {
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                (KeyCode::Char('s'), KeyModifiers::CONTROL) => Some(Self::Save),
                (KeyCode::Char('/'), KeyModifiers::CONTROL)
                | (KeyCode::Char('7'), KeyModifiers::CONTROL)
                | (KeyCode::Char('_'), KeyModifiers::CONTROL) => Some(Self::Undo),
                // ConPTY decodes the same legacy 0x1f byte as Ctrl+Shift+-.
                (KeyCode::Char('-' | '_'), modifiers)
                    if modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                {
                    Some(Self::Undo)
                }
                (KeyCode::Char('y'), KeyModifiers::CONTROL)
                | (KeyCode::Char('Y'), KeyModifiers::CONTROL) => Some(Self::Redo),
                (KeyCode::Up, _) => Some(Self::MoveUp),
                (KeyCode::Down, _) => Some(Self::MoveDown),
                (KeyCode::Left, _) => Some(Self::MoveLeft),
                (KeyCode::Right, _) => Some(Self::MoveRight),
                (KeyCode::Home, _) => Some(Self::MoveHome),
                (KeyCode::End, _) => Some(Self::MoveEnd),
                (KeyCode::PageUp, _) => Some(Self::PageUp),
                (KeyCode::PageDown, _) => Some(Self::PageDown),
                (KeyCode::Enter, _) => Some(Self::Enter),
                (KeyCode::Tab, _) => Some(Self::InsertNewline),
                (KeyCode::Backspace, _) => Some(Self::Backspace),
                (KeyCode::Delete, _) => Some(Self::Delete),
                (KeyCode::Char(ch), modifiers)
                    if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
                {
                    Some(Self::Insert(ch))
                }
                _ => None,
            };
        }

        // Ctrl-W starts the pane-navigation chord after all modal and text
        // input handlers have had precedence.  In particular, it must not
        // steal an editor insertion or AI prompt keystroke, and it must not
        // fall through to the legacy Ctrl-W keyword switch.
        let ctrl_w = matches!(event.code, KeyCode::Char('w') | KeyCode::Char('W'))
            && event.modifiers.contains(KeyModifiers::CONTROL)
            || event.code == KeyCode::Char('\u{17}');
        if pane_navigation_allowed(context) && ctrl_w {
            return Some(Self::PendingChar('w'));
        }

        // Requirements tab: three-pane navigation and plain-text editing
        if context.active_tab == MainTab::Requirements {
            // Windows console input can normalize Ctrl+W to an uppercase
            // character while retaining CONTROL. Normalize both spellings
            // before entering the two-key pane-navigation chord.
            if context.requirements_focus == RequirementsFocus::Editor {
                if context.requirements_editor_mode == RequirementsEditorMode::Insert {
                    return match (event.code, event.modifiers) {
                        (KeyCode::Esc, _) => Some(Self::ReqExitInsert),
                        (KeyCode::Char('s'), KeyModifiers::CONTROL) => Some(Self::Save),
                        (KeyCode::Left, _) => Some(Self::MoveLeft),
                        (KeyCode::Right, _) => Some(Self::MoveRight),
                        (KeyCode::Up, _) => Some(Self::MoveUp),
                        (KeyCode::Down, _) => Some(Self::MoveDown),
                        (KeyCode::Home, _) => Some(Self::MoveHome),
                        (KeyCode::End, _) => Some(Self::MoveEnd),
                        (KeyCode::Enter, _) => Some(Self::Enter),
                        (KeyCode::Tab, KeyModifiers::NONE) => Some(Self::Insert('\t')),
                        (KeyCode::Backspace, _) => Some(Self::Backspace),
                        (KeyCode::Delete, _) => Some(Self::Delete),
                        (KeyCode::Char(ch), modifiers)
                            if !ch.is_control()
                                && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
                        {
                            Some(Self::Insert(ch))
                        }
                        _ => None,
                    };
                }
                if matches!(
                    context.requirements_editor_mode,
                    RequirementsEditorMode::VisualChar | RequirementsEditorMode::VisualLine
                ) {
                    return match (event.code, event.modifiers) {
                        (KeyCode::Esc, _) => Some(Self::ReqExitInsert),
                        (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                            Some(Self::MoveLeft)
                        }
                        (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                            Some(Self::MoveRight)
                        }
                        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                            Some(Self::MoveUp)
                        }
                        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                            Some(Self::MoveDown)
                        }
                        (KeyCode::Char('w'), KeyModifiers::NONE) => Some(Self::ReqVimWordForward),
                        (KeyCode::Char('b'), KeyModifiers::NONE) => Some(Self::ReqVimWordBackward),
                        (KeyCode::Char('e'), KeyModifiers::NONE) => Some(Self::ReqVimWordEnd),
                        (KeyCode::Char('0'), KeyModifiers::NONE) => Some(Self::ReqVimLineStart),
                        (KeyCode::Char('^'), KeyModifiers::SHIFT)
                        | (KeyCode::Char('^'), KeyModifiers::NONE) => {
                            Some(Self::ReqVimFirstNonBlank)
                        }
                        (KeyCode::Char('$'), KeyModifiers::SHIFT)
                        | (KeyCode::Char('$'), KeyModifiers::NONE)
                        | (KeyCode::End, _) => Some(Self::ReqVimLineEnd),
                        (KeyCode::Char('y'), KeyModifiers::NONE) => Some(Self::ReqVimVisualYank),
                        (KeyCode::Char('d'), KeyModifiers::NONE) => Some(Self::ReqVimVisualDelete),
                        _ => None,
                    };
                }
                return match (event.code, event.modifiers) {
                    (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                        Some(Self::MoveLeft)
                    }
                    (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                        Some(Self::MoveRight)
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                        Some(Self::MoveUp)
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                        Some(Self::MoveDown)
                    }
                    (KeyCode::Char('i'), KeyModifiers::NONE) => Some(Self::ReqEnterInsert),
                    (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::ReqVimAppend),
                    (KeyCode::Char('o'), KeyModifiers::NONE) => Some(Self::ReqVimOpenBelow),
                    (KeyCode::Char('O'), KeyModifiers::SHIFT)
                    | (KeyCode::Char('O'), KeyModifiers::NONE) => Some(Self::ReqVimOpenAbove),
                    (KeyCode::Char('x'), KeyModifiers::NONE) => Some(Self::ReqVimDeleteChar),
                    (KeyCode::Char('d'), KeyModifiers::NONE) => Some(Self::PendingChar('d')),
                    (KeyCode::Char('y'), KeyModifiers::NONE) => Some(Self::PendingChar('y')),
                    (KeyCode::Char('p'), KeyModifiers::NONE) => Some(Self::ReqVimPasteAfter),
                    (KeyCode::Char('P'), KeyModifiers::SHIFT)
                    | (KeyCode::Char('P'), KeyModifiers::NONE) => Some(Self::ReqVimPasteBefore),
                    (KeyCode::Char('v'), KeyModifiers::NONE) => Some(Self::ReqVimVisualChar),
                    (KeyCode::Char('V'), KeyModifiers::SHIFT)
                    | (KeyCode::Char('V'), KeyModifiers::NONE) => Some(Self::ReqVimVisualLine),
                    (KeyCode::Char('w'), KeyModifiers::NONE) => Some(Self::ReqVimWordForward),
                    (KeyCode::Char('b'), KeyModifiers::NONE) => Some(Self::ReqVimWordBackward),
                    (KeyCode::Char('e'), KeyModifiers::NONE) => Some(Self::ReqVimWordEnd),
                    (KeyCode::Char('0'), KeyModifiers::NONE) => Some(Self::ReqVimLineStart),
                    (KeyCode::Char('^'), KeyModifiers::SHIFT)
                    | (KeyCode::Char('^'), KeyModifiers::NONE) => Some(Self::ReqVimFirstNonBlank),
                    (KeyCode::Char('$'), KeyModifiers::SHIFT)
                    | (KeyCode::Char('$'), KeyModifiers::NONE) => Some(Self::ReqVimLineEnd),
                    (KeyCode::Char('G'), KeyModifiers::SHIFT)
                    | (KeyCode::Char('G'), KeyModifiers::NONE) => Some(Self::ReqVimGoBottom),
                    (KeyCode::Char('g'), KeyModifiers::NONE) => Some(Self::PendingChar('g')),
                    (KeyCode::Char('u'), KeyModifiers::NONE) => Some(Self::Undo),
                    (KeyCode::Char('r'), KeyModifiers::CONTROL) => Some(Self::Redo),
                    (KeyCode::Char('I'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        Some(Self::ReqEditIteration)
                    }
                    (KeyCode::Home, _) => Some(Self::MoveHome),
                    (KeyCode::End, _) => Some(Self::MoveEnd),
                    (KeyCode::Char('s'), KeyModifiers::CONTROL) => Some(Self::Save),
                    (KeyCode::Char('n'), KeyModifiers::NONE) => Some(Self::ReqNewTestPoint),
                    (KeyCode::Char('n'), KeyModifiers::CONTROL) => Some(Self::ReqNewDocument),
                    (KeyCode::Char('1'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::Explore))
                    }
                    (KeyCode::Char('2'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::MindMap))
                    }
                    (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                    (KeyCode::Char('4'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::Requirements))
                    }
                    (KeyCode::Char('5'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::TestPoints))
                    }
                    (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                    (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                    (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
                    (KeyCode::Esc, _) => Some(Self::ReqExitInsert),
                    _ => None,
                };
            }
            return match (event.code, event.modifiers) {
                (KeyCode::Left, _) => Some(Self::FocusPrevColumn),
                (KeyCode::Right, _) => Some(Self::FocusNextColumn),
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => Some(Self::MoveUp),
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::MoveDown)
                }
                (KeyCode::Char('n'), KeyModifiers::CONTROL) => Some(Self::ReqNewDocument),
                (KeyCode::Delete, _) | (KeyCode::Char('d'), KeyModifiers::NONE) => {
                    Some(Self::ReqDeleteDocument)
                }
                (KeyCode::Char('i'), KeyModifiers::NONE) => Some(Self::ReqFilterOverlay),
                (KeyCode::Char('g'), KeyModifiers::NONE) => Some(Self::ReqGroupToggle),
                (KeyCode::Char('I'), KeyModifiers::SHIFT) => Some(Self::ReqEditIteration),
                (KeyCode::Char('p'), KeyModifiers::NONE) => Some(Self::ReqEditPriority),
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::Requirements))
                }
                (KeyCode::Char('5'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::TestPoints))
                }
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                _ => None,
            };
        }

        // Test Points tab: three-pane navigation, editing, and review
        if context.active_tab == MainTab::TestPoints {
            if context.test_points_focus == TestPointsFocus::Details {
                return match (event.code, event.modifiers) {
                    (KeyCode::Tab, _) => Some(Self::FocusNextColumn),
                    (KeyCode::BackTab, _) => Some(Self::FocusPrevColumn),
                    (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                        Some(Self::MoveLeft)
                    }
                    (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                        Some(Self::MoveRight)
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
                        Some(Self::MoveUp)
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                        Some(Self::MoveDown)
                    }
                    (KeyCode::Backspace, _) => Some(Self::Backspace),
                    (KeyCode::Delete, _) => Some(Self::Delete),
                    (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::TpApprove),
                    (KeyCode::Char('a'), KeyModifiers::SHIFT) => Some(Self::TpBatchApprove),
                    (KeyCode::Char('r'), KeyModifiers::NONE) => Some(Self::TpReject),
                    (KeyCode::Char('f'), KeyModifiers::NONE) => Some(Self::TpReviewFilter),
                    (KeyCode::Char('o'), KeyModifiers::NONE) => Some(Self::TpFollowScenario),
                    (KeyCode::Char('1'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::Explore))
                    }
                    (KeyCode::Char('2'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::MindMap))
                    }
                    (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                    (KeyCode::Char('4'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::Requirements))
                    }
                    (KeyCode::Char('5'), KeyModifiers::NONE) => {
                        Some(Self::SelectTab(MainTab::TestPoints))
                    }
                    (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                    (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                    (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
                    (KeyCode::Esc, _) => Some(Self::ClearInputState),
                    (KeyCode::Char(ch), modifiers)
                        if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
                    {
                        Some(Self::Insert(ch))
                    }
                    _ => None,
                };
            }
            return match (event.code, event.modifiers) {
                (KeyCode::Tab, _) => Some(Self::FocusNextColumn),
                (KeyCode::BackTab, _) => Some(Self::FocusPrevColumn),
                (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                    Some(Self::FocusPrevColumn)
                }
                (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                    Some(Self::FocusNextColumn)
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => Some(Self::MoveUp),
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::MoveDown)
                }
                (KeyCode::Enter, _) if context.test_points_focus == TestPointsFocus::Excerpts => {
                    Some(Self::TpFollowExcerpt)
                }
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::TpApprove),
                (KeyCode::Char('A'), KeyModifiers::SHIFT) => Some(Self::TpBatchApprove),
                (KeyCode::Char('r'), KeyModifiers::NONE) => Some(Self::TpReject),
                (KeyCode::Char('f'), KeyModifiers::NONE) => Some(Self::TpReviewFilter),
                (KeyCode::Char('c'), KeyModifiers::NONE) => Some(Self::TpContinueGeneration),
                (KeyCode::Char('o'), KeyModifiers::NONE) => Some(Self::TpFollowScenario),
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::Requirements))
                }
                (KeyCode::Char('5'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::TestPoints))
                }
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                _ => None,
            };
        }

        // Explore tab: three-column navigation
        if context.active_tab == MainTab::Explore && !context.explore_edit_mode {
            return match (event.code, event.modifiers) {
                (KeyCode::Tab, _) => Some(Self::FocusNextColumn),
                (KeyCode::BackTab, _) => Some(Self::FocusPrevColumn),
                (KeyCode::Left, _) => Some(Self::FocusPrevColumn),
                (KeyCode::Right, _) => Some(Self::ExploreRight),
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => Some(Self::MoveUp),
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::MoveDown)
                }
                (KeyCode::Char('h'), KeyModifiers::NONE) => Some(Self::FocusPrevColumn),
                (KeyCode::Char('l'), KeyModifiers::NONE) => Some(Self::ExploreRight),
                (KeyCode::Home, _) => Some(Self::MoveHome),
                (KeyCode::End, _) => Some(Self::MoveEnd),
                (KeyCode::Enter, _) => Some(Self::ToggleFailureDetail),
                (KeyCode::Char('r'), KeyModifiers::NONE) => Some(Self::RunScenario),
                (KeyCode::Char('p'), KeyModifiers::NONE) => Some(Self::ToggleInspectorPlaintext),
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::AiSuggest),
                (KeyCode::Char('e'), KeyModifiers::NONE) => Some(Self::EnterEdit),
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::Requirements))
                }
                (KeyCode::Char('5'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::TestPoints))
                }
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                _ => None,
            };
        }

        // AI tab: blurred mode — Enter to focus, 1-4 switch tabs,
        // Esc cancels thinking agent.
        if context.active_tab == MainTab::Ai && !context.ai_input_focused {
            return match (event.code, event.modifiers) {
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::Requirements))
                }
                (KeyCode::Char('5'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::TestPoints))
                }
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::AgentNew),
                (KeyCode::Char('x'), KeyModifiers::NONE) => Some(Self::AgentClose),
                (KeyCode::Char('j'), KeyModifiers::NONE) => Some(Self::AgentNext),
                (KeyCode::Char('k'), KeyModifiers::NONE) => Some(Self::AgentPrev),
                (KeyCode::Up, KeyModifiers::ALT) => Some(Self::AiScrollUp),
                (KeyCode::Down, KeyModifiers::ALT) => Some(Self::AiScrollDown),
                (KeyCode::Left, KeyModifiers::ALT) => Some(Self::AiScrollLeft),
                (KeyCode::Right, KeyModifiers::ALT) => Some(Self::AiScrollRight),
                (KeyCode::Char('c'), modifiers)
                    if modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                {
                    Some(Self::CopySelection)
                }
                (KeyCode::Esc, _) if context.ai_status_waiting => Some(Self::AiCancel),
                (KeyCode::Enter, _) => Some(Self::AiFocusInput),
                _ => None,
            };
        }

        // Slash suggestion popup active — intercept navigation/select/dismiss
        if context.slash_suggestion_active {
            let action = match (event.code, event.modifiers) {
                (KeyCode::Up, _) => Some(Self::AiSlashPrev),
                (KeyCode::Down, _) => Some(Self::AiSlashNext),
                (KeyCode::Enter, _) => Some(Self::AiSlashSelect),
                (KeyCode::Tab, _) => Some(Self::AiSlashSelect),
                (KeyCode::Esc, _) => Some(Self::AiSlashDismiss),
                _ => None,
            };
            if action.is_some() {
                return action;
            }
            // Fall through to normal AI tab focused handling
        }

        // AI tab: focused mode — text input, Esc blurs instead of clearing
        if context.active_tab == MainTab::Ai {
            return match (event.code, event.modifiers) {
                (KeyCode::Enter, KeyModifiers::SHIFT) => Some(Self::AiNewline),
                (KeyCode::Enter, _) => Some(Self::AiSendMessage),
                (KeyCode::Backspace, _) => Some(Self::AiBackspace),
                (KeyCode::Delete, _) => Some(Self::AiDelete),
                (KeyCode::Up, KeyModifiers::ALT) => Some(Self::AiScrollUp),
                (KeyCode::Down, KeyModifiers::ALT) => Some(Self::AiScrollDown),
                (KeyCode::Left, KeyModifiers::ALT) => Some(Self::AiScrollLeft),
                (KeyCode::Right, KeyModifiers::ALT) => Some(Self::AiScrollRight),
                (KeyCode::Left, _) => Some(Self::AiCursorLeft),
                (KeyCode::Right, _) => Some(Self::AiCursorRight),
                (KeyCode::Esc, _) if context.ai_status_waiting => Some(Self::AiBlurInput),
                (KeyCode::Esc, _) => Some(Self::AiBlurInput),
                (KeyCode::PageUp, _) | (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                    Some(Self::AiScrollUp)
                }
                (KeyCode::PageDown, _) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                    Some(Self::AiScrollDown)
                }
                (KeyCode::Home, KeyModifiers::CONTROL) => Some(Self::AiScrollTop),
                (KeyCode::End, KeyModifiers::CONTROL) => Some(Self::AiScrollBottom),
                (KeyCode::Home, _) => Some(Self::AiCursorHome),
                (KeyCode::End, _) => Some(Self::AiCursorEnd),
                (KeyCode::Char('v'), KeyModifiers::CONTROL) => Some(Self::AiClipboardPaste),
                (KeyCode::Char('c'), modifiers)
                    if modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                {
                    Some(Self::CopySelection)
                }
                (KeyCode::Char(ch), modifiers)
                    if !ch.is_control()
                        && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
                {
                    Some(Self::AiSendChar(ch))
                }
                _ => None,
            };
        }

        // MindMap tab: AI panel has focus
        if context.active_tab == MainTab::MindMap && context.mindmap_focus == MindMapFocus::AiPanel
        {
            if context.ai_input_focused {
                // AI input focused — same bindings as AI tab text input
                return match (event.code, event.modifiers) {
                    (KeyCode::Enter, KeyModifiers::SHIFT) => Some(Self::AiNewline),
                    (KeyCode::Enter, _) => Some(Self::AiSendMessage),
                    (KeyCode::Backspace, _) => Some(Self::AiBackspace),
                    (KeyCode::Delete, _) => Some(Self::AiDelete),
                    (KeyCode::Up, KeyModifiers::ALT) => Some(Self::AiScrollUp),
                    (KeyCode::Down, KeyModifiers::ALT) => Some(Self::AiScrollDown),
                    (KeyCode::Left, KeyModifiers::ALT) => Some(Self::AiScrollLeft),
                    (KeyCode::Right, KeyModifiers::ALT) => Some(Self::AiScrollRight),
                    (KeyCode::Left, _) => Some(Self::AiCursorLeft),
                    (KeyCode::Right, _) => Some(Self::AiCursorRight),
                    (KeyCode::Esc, _) if context.ai_status_waiting => Some(Self::AiBlurInput),
                    (KeyCode::Esc, _) => Some(Self::AiBlurInput),
                    (KeyCode::PageUp, _) | (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                        Some(Self::AiScrollUp)
                    }
                    (KeyCode::PageDown, _) | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                        Some(Self::AiScrollDown)
                    }
                    (KeyCode::Home, KeyModifiers::CONTROL) => Some(Self::AiScrollTop),
                    (KeyCode::End, KeyModifiers::CONTROL) => Some(Self::AiScrollBottom),
                    (KeyCode::Home, _) => Some(Self::AiCursorHome),
                    (KeyCode::End, _) => Some(Self::AiCursorEnd),
                    (KeyCode::Char('v'), KeyModifiers::CONTROL) => Some(Self::AiClipboardPaste),
                    (KeyCode::Char('c'), modifiers)
                        if modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                    {
                        Some(Self::CopySelection)
                    }
                    (KeyCode::Char(ch), modifiers)
                        if !ch.is_control()
                            && (modifiers.is_empty() || modifiers == KeyModifiers::SHIFT) =>
                    {
                        Some(Self::AiSendChar(ch))
                    }
                    _ => None,
                };
            } else {
                // AI panel focused but input not focused — scroll, focus input, return to tree
                return match (event.code, event.modifiers) {
                    (KeyCode::Esc, _) => Some(Self::ClearInputState),
                    (KeyCode::Up, KeyModifiers::ALT) => Some(Self::AiScrollUp),
                    (KeyCode::Down, KeyModifiers::ALT) => Some(Self::AiScrollDown),
                    (KeyCode::Left, KeyModifiers::ALT) => Some(Self::AiScrollLeft),
                    (KeyCode::Right, KeyModifiers::ALT) => Some(Self::AiScrollRight),
                    (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                        Some(Self::ClearInputState)
                    }
                    (KeyCode::Enter, _) => Some(Self::AiFocusInput),
                    (KeyCode::Char('c'), modifiers)
                        if modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
                    {
                        Some(Self::CopySelection)
                    }
                    (KeyCode::Char('o'), _) => Some(Self::ToggleScenarioDropdown),
                    _ => None,
                };
            }
        }

        // MindMap tab: tree navigation (stages 1 & 2)
        if context.active_tab == MainTab::MindMap
            && context.mindmap_focus == MindMapFocus::Main
            && matches!(
                context.view_stage,
                ViewStage::TreeOnly | ViewStage::TreeAndEditor
            )
        {
            return match (event.code, event.modifiers) {
                (KeyCode::Up, KeyModifiers::SHIFT) => Some(Self::TreeSiblingPrev),
                (KeyCode::Down, KeyModifiers::SHIFT) => Some(Self::TreeSiblingNext),
                (KeyCode::Char('K'), KeyModifiers::SHIFT)
                | (KeyCode::Char('K'), KeyModifiers::NONE) => Some(Self::TreeSiblingPrev),
                (KeyCode::Char('J'), KeyModifiers::SHIFT)
                | (KeyCode::Char('J'), KeyModifiers::NONE) => Some(Self::TreeSiblingNext),
                (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => Some(Self::TreeUp),
                (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Some(Self::TreeDown)
                }
                (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE) => {
                    Some(Self::TreeCollapse)
                }
                (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE) => {
                    Some(Self::TreeExpand)
                }
                (KeyCode::Char(' '), _) => Some(Self::TreeToggle),
                (KeyCode::Home, _) => Some(Self::TreeHome),
                (KeyCode::End, _) => Some(Self::TreeEnd),
                (KeyCode::Char('['), _) => Some(Self::TreeLocationPrev),
                (KeyCode::Char(']'), _) => Some(Self::TreeLocationNext),
                (KeyCode::Enter, _) if context.mindmap_ai_panel_visible => {
                    Some(Self::MindMapFocusAiPanel)
                }
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::Requirements))
                }
                (KeyCode::Char('5'), KeyModifiers::NONE) => {
                    Some(Self::SelectTab(MainTab::TestPoints))
                }
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::MindMapSendToAi),
                (KeyCode::Char('p'), KeyModifiers::NONE) => Some(Self::ToggleMindMapAiPanel),
                (KeyCode::Char('o'), _) => Some(Self::ToggleScenarioDropdown),
                (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
                _ => None,
            };
        }

        // Default: editor (stage 3) and global keys
        match (event.code, event.modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
            (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
            (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
            (KeyCode::Char('4'), KeyModifiers::NONE) => {
                Some(Self::SelectTab(MainTab::Requirements))
            }
            (KeyCode::Char('5'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::TestPoints)),
            (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
            (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => Some(Self::Save),
            (KeyCode::Char('m'), KeyModifiers::NONE) => Some(Self::ModelPanelOpen),
            (KeyCode::Char('\x1c'), _) => Some(Self::ToggleMindMapAiPanel),
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => Some(Self::RunBackground),
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => Some(Self::NewScenario),
            (KeyCode::Char('g'), KeyModifiers::CONTROL) => Some(Self::SwitchKeyword("Given")),
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => Some(Self::SwitchKeyword("When")),
            (KeyCode::Char('t'), KeyModifiers::CONTROL) => Some(Self::SwitchKeyword("Then")),
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => Some(Self::SwitchKeyword("And")),
            (KeyCode::Char('/'), KeyModifiers::CONTROL)
            | (KeyCode::Char('7'), KeyModifiers::CONTROL)
            | (KeyCode::Char('_'), KeyModifiers::CONTROL) => Some(Self::Undo),
            (KeyCode::Char('-' | '_'), modifiers)
                if modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                Some(Self::Undo)
            }
            (KeyCode::Char('y'), KeyModifiers::CONTROL)
            | (KeyCode::Char('Y'), KeyModifiers::CONTROL) => Some(Self::Redo),
            (KeyCode::Char(' '), KeyModifiers::NONE)
                if (context.active_tab == MainTab::MindMap
                    && context.view_stage == ViewStage::EditorAndPanel)
                    || (context.active_tab == MainTab::Explore && context.explore_edit_mode) =>
            {
                Some(Self::ToggleScenarioFold)
            }
            (KeyCode::Char(' '), KeyModifiers::CONTROL)
            | (KeyCode::Null, KeyModifiers::CONTROL)
                if (context.active_tab == MainTab::MindMap
                    && context.view_stage == ViewStage::EditorAndPanel)
                    || (context.active_tab == MainTab::Explore && context.explore_edit_mode) =>
            {
                Some(Self::FoldAllScenarios)
            }
            (KeyCode::Enter, _)
                if (context.active_tab == MainTab::MindMap
                    && context.view_stage == ViewStage::EditorAndPanel)
                    || (context.active_tab == MainTab::Explore && context.explore_edit_mode) =>
            {
                Some(Self::ActivateStepInput)
            }
            (KeyCode::Esc, _) => Some(Self::ClearInputState),
            (KeyCode::Up, KeyModifiers::CONTROL) | (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                Some(Self::MoveStepUp)
            }
            (KeyCode::Down, KeyModifiers::CONTROL)
            | (KeyCode::Char('j'), KeyModifiers::CONTROL) => Some(Self::MoveStepDown),
            (KeyCode::Up, KeyModifiers::SHIFT) => Some(Self::MoveSiblingUp),
            (KeyCode::Down, KeyModifiers::SHIFT) => Some(Self::MoveSiblingDown),
            (KeyCode::Char('K'), KeyModifiers::SHIFT)
            | (KeyCode::Char('K'), KeyModifiers::NONE) => Some(Self::MoveSiblingUp),
            (KeyCode::Char('J'), KeyModifiers::SHIFT)
            | (KeyCode::Char('J'), KeyModifiers::NONE) => Some(Self::MoveSiblingDown),
            (KeyCode::Up, _) => Some(Self::MoveUp),
            (KeyCode::Down, _) => Some(Self::MoveDown),
            (KeyCode::Left, _) => Some(Self::MoveLeft),
            (KeyCode::Right, _) => Some(Self::MoveRight),
            (KeyCode::Char('h'), KeyModifiers::NONE) => Some(Self::MoveLeft),
            (KeyCode::Char('j'), KeyModifiers::NONE) => Some(Self::MoveDown),
            (KeyCode::Char('k'), KeyModifiers::NONE) => Some(Self::MoveUp),
            (KeyCode::Char('l'), KeyModifiers::NONE) => Some(Self::MoveRight),
            (KeyCode::Char('o'), KeyModifiers::NONE) => Some(Self::InsertStepBelow),
            (KeyCode::Char('O'), KeyModifiers::SHIFT)
            | (KeyCode::Char('O'), KeyModifiers::NONE) => Some(Self::InsertStepAbove),
            (KeyCode::Char('p'), KeyModifiers::NONE) => Some(Self::PasteStep),
            (KeyCode::Char('d'), KeyModifiers::NONE) => Some(Self::PendingChar('d')),
            (KeyCode::Char('y'), KeyModifiers::NONE) => Some(Self::PendingChar('y')),
            (KeyCode::Home, _) => Some(Self::MoveHome),
            (KeyCode::End, _) => Some(Self::MoveEnd),
            (KeyCode::PageUp, _) => Some(Self::PageUp),
            (KeyCode::PageDown, _) => Some(Self::PageDown),
            _ => None,
        }
    }
}

/// Whether the global two-key pane-navigation prefix may consume this key.
/// Modal branches return earlier in [`Action::from_key_event`]; these checks
/// cover the remaining text editors whose input must win over pane movement.
fn pane_navigation_allowed(context: KeyContext) -> bool {
    let ai_prompt_active = context.ai_input_focused
        && (context.active_tab == MainTab::Ai
            || (context.active_tab == MainTab::MindMap
                && context.mindmap_focus == MindMapFocus::AiPanel));
    if context.step_input_active || ai_prompt_active {
        return false;
    }
    if context.active_tab == MainTab::Requirements
        && matches!(
            context.requirements_editor_mode,
            RequirementsEditorMode::Insert
                | RequirementsEditorMode::VisualChar
                | RequirementsEditorMode::VisualLine
        )
    {
        return false;
    }
    !(context.active_tab == MainTab::TestPoints
        && context.test_points_focus == TestPointsFocus::Details)
}

#[cfg(test)]
mod tests {
    use super::{Action, KeyContext};
    use crate::app::{MainTab, MindMapFocus, ViewStage};
    use crate::authoring_tab::RequirementsFocus;
    use crate::test_points_tab::TestPointsFocus;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn chat_context(
        active_tab: MainTab,
        mindmap_focus: MindMapFocus,
        ai_input_focused: bool,
    ) -> KeyContext {
        KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus,
            mindmap_ai_panel_visible: true,
            ai_input_focused,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        }
    }

    #[test]
    fn undo_accepts_legacy_pty_control_seven_in_editor_contexts() {
        let mut context = chat_context(MainTab::Explore, MindMapFocus::Main, false);
        context.view_stage = ViewStage::EditorAndPanel;
        context.explore_edit_mode = true;
        for step_input in [false, true] {
            context.step_input_active = step_input;
            for ch in ['/', '_', '7'] {
                assert_eq!(
                    Action::from_key_event(
                        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
                        context,
                    ),
                    Some(Action::Undo),
                );
            }
            assert_eq!(
                Action::from_key_event(
                    KeyEvent::new(
                        KeyCode::Char('-'),
                        KeyModifiers::CONTROL | KeyModifiers::SHIFT
                    ),
                    context,
                ),
                Some(Action::Undo),
            );
        }
    }

    #[test]
    fn chat_copy_shortcut_is_available_in_all_chat_focus_states() {
        let key = KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        for context in [
            chat_context(MainTab::Ai, MindMapFocus::Main, false),
            chat_context(MainTab::Ai, MindMapFocus::Main, true),
            chat_context(MainTab::MindMap, MindMapFocus::AiPanel, false),
            chat_context(MainTab::MindMap, MindMapFocus::AiPanel, true),
        ] {
            assert_eq!(
                Action::from_key_event(key, context),
                Some(Action::CopySelection)
            );
        }
    }

    #[test]
    fn test_tab_switch_shortcuts_in_tree_mode() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
            context,
        );
        assert_eq!(action, Some(Action::SelectTab(MainTab::Explore)));
    }

    #[test]
    fn test_tab_switch_shortcuts_disabled_in_step_input_state() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: true,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
            context,
        );
        assert_eq!(action, Some(Action::Insert('1')));
    }

    #[test]
    fn acp_permission_prompt_intercepts_navigation_and_decision() {
        let mut context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::Ai,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: true,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: true,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: true,
            project_switch_active: false,
        };
        let keys = [
            (KeyCode::Up, KeyModifiers::NONE, Action::AcpPermissionUp),
            (
                KeyCode::Char('k'),
                KeyModifiers::NONE,
                Action::AcpPermissionUp,
            ),
            (KeyCode::Down, KeyModifiers::NONE, Action::AcpPermissionDown),
            (
                KeyCode::Char('j'),
                KeyModifiers::NONE,
                Action::AcpPermissionDown,
            ),
            (
                KeyCode::Enter,
                KeyModifiers::NONE,
                Action::AcpPermissionConfirm,
            ),
            (
                KeyCode::Esc,
                KeyModifiers::NONE,
                Action::AcpPermissionReject,
            ),
        ];
        for (code, modifiers, expected) in keys {
            assert_eq!(
                Action::from_key_event(KeyEvent::new(code, modifiers), context),
                Some(expected.clone()),
                "{code:?} must be captured by the ACP permission prompt"
            );
        }
        // Ordinary typing must not leak into the input while prompting.
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                context
            ),
            None
        );
        context.acp_permission_pending = false;
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), context),
            Some(Action::AiSendMessage)
        );
    }

    #[test]
    fn test_tree_nav_keys_in_stage1() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), context),
            Some(Action::TreeUp)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                context
            ),
            Some(Action::TreeToggle)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), context),
            None
        );
    }

    #[test]
    fn test_editor_keys_in_stage3() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), context),
            Some(Action::MoveUp)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                context
            ),
            Some(Action::ToggleScenarioFold)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), context),
            Some(Action::ActivateStepInput)
        );
    }

    #[test]
    fn test_explore_tab_navigation_keys() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::Explore,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), context),
            Some(Action::FocusNextColumn)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), context),
            Some(Action::ExploreRight)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), context),
            Some(Action::FocusPrevColumn)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), context),
            Some(Action::ToggleFailureDetail)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
                context
            ),
            Some(Action::EnterEdit)
        );
    }

    #[test]
    fn test_step_input_allows_shift_insert() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: true,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
            context,
        );
        assert_eq!(action, Some(Action::Insert('A')));
    }

    #[test]
    fn test_step_input_rejects_control_modified_insert() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: true,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
            context,
        );
        assert_eq!(action, None);
    }

    #[test]
    fn test_editor_structural_shortcuts_in_stage3() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL), context),
            Some(Action::MoveStepUp)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
                context
            ),
            Some(Action::SwitchKeyword("Given"))
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                context
            ),
            Some(Action::NewScenario)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
                context
            ),
            Some(Action::PasteStep)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Null, KeyModifiers::CONTROL), context),
            Some(Action::FoldAllScenarios)
        );
    }

    #[test]
    fn test_pending_sequences_promote_dd_and_yy() {
        let delete_context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: Some('d'),
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        let copy_context = KeyContext {
            pending_char: Some('y'),
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            ..delete_context
        };
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                delete_context
            ),
            Some(Action::DeleteNode)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                copy_context
            ),
            Some(Action::CopyStep)
        );
    }

    #[test]
    fn test_external_change_prompt_intercepts_confirm_keys() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: true,
            agent_change_prompt_active: false,
            active_tab: MainTab::Explore,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        };
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), context),
            Some(Action::ExternalChangeReload)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), context),
            Some(Action::ExternalChangeKeepLocal)
        );
    }

    #[test]
    fn requirements_modes_keep_printable_text_and_commands_separate() {
        use crate::authoring_tab::RequirementsEditorMode;
        let mut context = requirements_tree_context();
        context.requirements_focus = RequirementsFocus::Editor;
        for ch in (32u8..=126).map(char::from).chain("中文🙂é".chars()) {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            assert!(!matches!(
                Action::from_key_event(key, context),
                Some(Action::Insert(_))
            ));
            let insert = KeyContext {
                requirements_editor_mode: RequirementsEditorMode::Insert,
                ..context
            };
            assert_eq!(
                Action::from_key_event(key, insert),
                Some(Action::Insert(ch))
            );
            assert_eq!(
                Action::from_key_event(
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::SHIFT),
                    insert
                ),
                Some(Action::Insert(ch))
            );
        }
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                context
            ),
            Some(Action::ReqEnterInsert)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), context),
            None
        );
        context.requirements_editor_mode = RequirementsEditorMode::Insert;
        for (key, modifiers, expected) in [
            (
                KeyCode::Esc,
                KeyModifiers::NONE,
                Some(Action::ReqExitInsert),
            ),
            (
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
                Some(Action::Save),
            ),
            (KeyCode::Tab, KeyModifiers::NONE, Some(Action::Insert('\t'))),
            (KeyCode::BackTab, KeyModifiers::SHIFT, None),
            (KeyCode::Char('w'), KeyModifiers::CONTROL, None),
            (KeyCode::Char('W'), KeyModifiers::CONTROL, None),
            (KeyCode::Home, KeyModifiers::NONE, Some(Action::MoveHome)),
            (KeyCode::End, KeyModifiers::NONE, Some(Action::MoveEnd)),
        ] {
            assert_eq!(
                Action::from_key_event(KeyEvent::new(key, modifiers), context),
                expected
            );
        }
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                KeyContext {
                    requirements_editor_mode: RequirementsEditorMode::Browse,
                    pending_char: Some('w'),
                    ..context
                }
            ),
            Some(Action::FocusPrevColumn)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
                KeyContext {
                    requirements_editor_mode: RequirementsEditorMode::Browse,
                    pending_char: Some('w'),
                    ..context
                }
            ),
            Some(Action::FocusNextColumn)
        );
        for (key, modifiers, expected) in [
            (
                KeyCode::Char('H'),
                KeyModifiers::SHIFT,
                Action::FocusPrevColumn,
            ),
            (
                KeyCode::Char('l'),
                KeyModifiers::CONTROL,
                Action::FocusNextColumn,
            ),
            (
                KeyCode::Char('L'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                Action::FocusNextColumn,
            ),
        ] {
            assert_eq!(
                Action::from_key_event(
                    KeyEvent::new(key, modifiers),
                    KeyContext {
                        requirements_editor_mode: RequirementsEditorMode::Browse,
                        pending_char: Some('w'),
                        ..context
                    }
                ),
                Some(expected)
            );
        }
    }

    fn requirements_tree_context() -> KeyContext {
        KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::Requirements,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Browse,
            test_points_focus: TestPointsFocus::Tree,
            requirements_overlay_active: false,
            requirements_text_overlay: false,
            generation_scope_prompt_active: false,
            quit_pending_confirm: false,
            acp_permission_pending: false, // project context covered
            project_switch_active: false,
        }
    }

    #[test]
    fn requirements_line_operators_use_prefixes_but_visual_operators_are_immediate() {
        use crate::authoring_tab::RequirementsEditorMode;
        let mut context = requirements_tree_context();
        context.requirements_focus = RequirementsFocus::Editor;
        for (ch, line, visual) in [
            ('d', Action::ReqVimDeleteLine, Action::ReqVimVisualDelete),
            ('y', Action::ReqVimYankLine, Action::ReqVimVisualYank),
        ] {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            assert_eq!(
                Action::from_key_event(key, context),
                Some(Action::PendingChar(ch))
            );
            let repeated = KeyContext {
                pending_char: Some(ch),
                ..context
            };
            assert_eq!(Action::from_key_event(key, repeated), Some(line));
            for mode in [
                RequirementsEditorMode::VisualChar,
                RequirementsEditorMode::VisualLine,
            ] {
                let selecting = KeyContext {
                    requirements_editor_mode: mode,
                    ..context
                };
                assert_eq!(Action::from_key_event(key, selecting), Some(visual.clone()));
            }
        }
    }

    #[test]
    fn requirements_tree_binds_filter_group_and_iteration() {
        let context = requirements_tree_context();
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                context
            ),
            None
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
                context
            ),
            None
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), context),
            Some(Action::FocusPrevColumn)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), context),
            Some(Action::FocusNextColumn)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                context
            ),
            Some(Action::MoveDown)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                context
            ),
            Some(Action::MoveUp)
        );
        assert!(!matches!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                context
            ),
            Some(Action::FocusNextColumn | Action::FocusPrevColumn)
        ));
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                context
            ),
            Some(Action::ReqFilterOverlay)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                context
            ),
            Some(Action::ReqGroupToggle)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('I'), KeyModifiers::SHIFT),
                context
            ),
            Some(Action::ReqEditIteration)
        );
    }

    #[test]
    fn pane_prefix_is_global_but_text_modes_keep_precedence() {
        let base = requirements_tree_context();
        for active_tab in [
            MainTab::Explore,
            MainTab::MindMap,
            MainTab::Ai,
            MainTab::Requirements,
            MainTab::TestPoints,
        ] {
            let context = KeyContext {
                active_tab,
                mindmap_ai_panel_visible: active_tab == MainTab::MindMap,
                ..base
            };
            for key in ['w', 'W'] {
                assert_eq!(
                    Action::from_key_event(
                        KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL),
                        context,
                    ),
                    Some(Action::PendingChar('w')),
                    "tab {active_tab:?}"
                );
            }
        }

        let browse = KeyContext {
            active_tab: MainTab::Explore,
            mindmap_ai_panel_visible: false,
            pending_char: Some('w'),
            ..base
        };
        for (key, expected) in [
            ('h', Action::FocusPrevColumn),
            ('l', Action::FocusNextColumn),
            ('j', Action::FocusPaneDown),
            ('k', Action::FocusPaneUp),
        ] {
            assert_eq!(
                Action::from_key_event(
                    KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                    browse,
                ),
                Some(expected)
            );
        }
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), browse,),
            Some(Action::CancelPaneNavigation)
        );

        for context in [
            KeyContext {
                active_tab: MainTab::Requirements,
                requirements_focus: RequirementsFocus::Editor,
                requirements_editor_mode: crate::authoring_tab::RequirementsEditorMode::Insert,
                ..base
            },
            KeyContext {
                active_tab: MainTab::Ai,
                ai_input_focused: true,
                ..base
            },
            KeyContext {
                active_tab: MainTab::TestPoints,
                test_points_focus: TestPointsFocus::Details,
                ..base
            },
            KeyContext {
                active_tab: MainTab::MindMap,
                mindmap_focus: MindMapFocus::AiPanel,
                mindmap_ai_panel_visible: true,
                ai_input_focused: true,
                ..base
            },
            KeyContext {
                active_tab: MainTab::Explore,
                step_input_active: true,
                ..base
            },
        ] {
            assert_eq!(
                Action::from_key_event(
                    KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                    context,
                ),
                None
            );
        }
    }

    #[test]
    fn requirements_overlay_intercepts_confirm_and_insert() {
        let mut context = requirements_tree_context();
        context.requirements_overlay_active = true;
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), context),
            Some(Action::OverlayConfirm)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), context),
            Some(Action::OverlayCancel)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
                context
            ),
            Some(Action::OverlayInsert('a'))
        );
        for ch in ['j', 'k'] {
            context.requirements_text_overlay = true;
            assert_eq!(
                Action::from_key_event(
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                    context
                ),
                Some(Action::OverlayInsert(ch))
            );
            context.requirements_text_overlay = false;
            assert_eq!(
                Action::from_key_event(
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                    context
                ),
                Some(if ch == 'j' {
                    Action::OverlayMoveDown
                } else {
                    Action::OverlayMoveUp
                })
            );
        }
    }
}
