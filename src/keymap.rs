use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{MainTab, MindMapFocus, ViewStage};

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
}

/// High-level editor command derived from keyboard input.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    // Global
    Save,
    Quit,
    SelectTab(MainTab),
    ActivateStepInput,
    ClearInputState,
    // Explore navigation
    FocusNextColumn,
    FocusPrevColumn,
    ExploreRight,
    RunScenario,
    AiSuggest,
    EnterEdit,
    ToggleFailureDetail,
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
    // ── Slash command suggestion popup ───────────────
    AiSlashPrev,
    AiSlashNext,
    AiSlashSelect,
    AiSlashDismiss,
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
    // Mouse-driven selection actions (no key bindings — triggered by mouse events)
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
}

impl Action {
    pub fn from_key_event(event: KeyEvent, context: KeyContext) -> Option<Self> {
        if let Some(pending_char) = context.pending_char {
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
        if context.step_input_active {
            return match (event.code, event.modifiers) {
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                (KeyCode::Char('s'), KeyModifiers::CONTROL) => Some(Self::Save),
                (KeyCode::Char('/'), KeyModifiers::CONTROL)
                | (KeyCode::Char('_'), KeyModifiers::CONTROL) => Some(Self::Undo),
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
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::AiSuggest),
                (KeyCode::Char('e'), KeyModifiers::NONE) => Some(Self::EnterEdit),
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
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
                (KeyCode::Char(ch), _) if !ch.is_control() => Some(Self::AiSendChar(ch)),
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
                    (KeyCode::Char(ch), _) if !ch.is_control() => Some(Self::AiSendChar(ch)),
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
            | (KeyCode::Char('_'), KeyModifiers::CONTROL) => Some(Self::Undo),
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

#[cfg(test)]
mod tests {
    use super::{Action, KeyContext};
    use crate::app::{MainTab, MindMapFocus, ViewStage};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
            context,
        );
        assert_eq!(action, Some(Action::Insert('1')));
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
}
