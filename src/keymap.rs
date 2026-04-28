use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{MainTab, ViewStage};

/// Inputs for [`Action::from_key_event`] to resolve mode-specific bindings.
#[derive(Debug, Clone, Copy)]
pub struct KeyContext {
    pub step_keyword_picker_active: bool,
    pub step_input_active: bool,
    pub external_change_prompt_active: bool,
    pub active_tab: MainTab,
    pub view_stage: ViewStage,
    pub explore_edit_mode: bool,
    pub pending_char: Option<char>,
}

/// High-level editor command derived from keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    // AI tab input
    AiSendChar(char),
    AiSendMessage,
    AiBackspace,
    /// Send the selected MindMap node context as a user message to the AI.
    MindMapSendToAi,
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
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                _ => None,
            };
        }

        // AI tab: text input
        if context.active_tab == MainTab::Ai {
            return match (event.code, event.modifiers) {
                (KeyCode::Enter, _) => Some(Self::AiSendMessage),
                (KeyCode::Backspace, _) => Some(Self::AiBackspace),
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Char(ch), _) if !ch.is_control() => Some(Self::AiSendChar(ch)),
                _ => None,
            };
        }

        // MindMap tab: tree navigation (stages 1 & 2)
        if context.active_tab == MainTab::MindMap
            && matches!(
                context.view_stage,
                ViewStage::TreeOnly | ViewStage::TreeAndEditor
            )
        {
            return match (event.code, event.modifiers) {
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
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
                (KeyCode::Char('4'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::MindMapSendToAi),
                _ => None,
            };
        }

        // Default: editor (stage 3) and global keys
        match (event.code, event.modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
            (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
            (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
            (KeyCode::Char('4'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Ai)),
            (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
            (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => Some(Self::Save),
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
    use crate::app::{MainTab, ViewStage};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn test_tab_switch_shortcuts_in_tree_mode() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::Explore,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: None,
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
            pending_char: Some('d'),
        };
        let copy_context = KeyContext {
            pending_char: Some('y'),
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
            active_tab: MainTab::Explore,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
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
