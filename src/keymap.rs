use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{MainTab, ViewStage};

/// Inputs for [`Action::from_key_event`] to resolve mode-specific bindings.
#[derive(Debug, Clone, Copy)]
pub struct KeyContext {
    pub step_keyword_picker_active: bool,
    pub step_input_active: bool,
    pub active_tab: MainTab,
    pub view_stage: ViewStage,
    pub explore_edit_mode: bool,
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
    // Global
    Save,
    Quit,
    SelectTab(MainTab),
    ActivateStepInput,
    ClearInputState,
    // Explore navigation
    FocusNextColumn,
    FocusPrevColumn,
    RunScenario,
    AiSuggest,
    EnterEdit,
    // Step keyword picker overlay
    StepKeywordPickerUp,
    StepKeywordPickerDown,
    StepKeywordPickerConfirm,
    StepKeywordPickerCancel,
    // Tree navigation (stages 1 & 2)
    TreeUp,
    TreeDown,
    TreeExpand,
    TreeCollapse,
    TreeToggle,
    TreeOpen,
    TreeHome,
    TreeEnd,
    /// Cycle the stage-2 preview to the previous source location for a shared step path (left bracket).
    TreeLocationPrev,
    /// Cycle the stage-2 preview to the next source location for a shared step path (right bracket).
    TreeLocationNext,
    /// Go back one stage in the three-stage model.
    StageBack,
}

impl Action {
    pub fn from_key_event(event: KeyEvent, context: KeyContext) -> Option<Self> {
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
                (KeyCode::Up, _) => Some(Self::MoveUp),
                (KeyCode::Down, _) => Some(Self::MoveDown),
                (KeyCode::Left, _) => Some(Self::MoveLeft),
                (KeyCode::Right, _) => Some(Self::MoveRight),
                (KeyCode::Home, _) => Some(Self::MoveHome),
                (KeyCode::End, _) => Some(Self::MoveEnd),
                (KeyCode::PageUp, _) => Some(Self::PageUp),
                (KeyCode::PageDown, _) => Some(Self::PageDown),
                (KeyCode::Enter, _) => Some(Self::Enter),
                (KeyCode::Backspace, _) => Some(Self::Backspace),
                (KeyCode::Delete, _) => Some(Self::Delete),
                (KeyCode::Char(ch), modifiers) if modifiers.is_empty() => Some(Self::Insert(ch)),
                _ => None,
            };
        }

        // Explore tab: three-column navigation
        if context.active_tab == MainTab::Explore && !context.explore_edit_mode {
            return match (event.code, event.modifiers) {
                (KeyCode::Tab, _) => Some(Self::FocusNextColumn),
                (KeyCode::BackTab, _) => Some(Self::FocusPrevColumn),
                (KeyCode::Left, _) => Some(Self::FocusPrevColumn),
                (KeyCode::Right, _) => Some(Self::FocusNextColumn),
                (KeyCode::Up, _) => Some(Self::MoveUp),
                (KeyCode::Down, _) => Some(Self::MoveDown),
                (KeyCode::Home, _) => Some(Self::MoveHome),
                (KeyCode::End, _) => Some(Self::MoveEnd),
                (KeyCode::Char('r'), KeyModifiers::NONE) => Some(Self::RunScenario),
                (KeyCode::Char('a'), KeyModifiers::NONE) => Some(Self::AiSuggest),
                (KeyCode::Char('e'), KeyModifiers::NONE) => Some(Self::EnterEdit),
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                (KeyCode::Esc, _) => Some(Self::ClearInputState),
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
                (KeyCode::Up, _) => Some(Self::TreeUp),
                (KeyCode::Down, _) => Some(Self::TreeDown),
                (KeyCode::Left, _) => Some(Self::TreeCollapse),
                (KeyCode::Right, _) => Some(Self::TreeExpand),
                (KeyCode::Char(' '), _) => Some(Self::TreeToggle),
                (KeyCode::Enter, _) => Some(Self::TreeOpen),
                (KeyCode::Home, _) => Some(Self::TreeHome),
                (KeyCode::End, _) => Some(Self::TreeEnd),
                (KeyCode::Char('['), _) => Some(Self::TreeLocationPrev),
                (KeyCode::Char(']'), _) => Some(Self::TreeLocationNext),
                (KeyCode::Esc, _) => Some(Self::StageBack),
                (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
                (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
                (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
                (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
                (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
                _ => None,
            };
        }

        // Default: editor (stage 3) and global keys
        match (event.code, event.modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::MindMap)),
            (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Explore)),
            (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
            (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
            (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
            (KeyCode::Char(' '), KeyModifiers::NONE)
                if (context.active_tab == MainTab::MindMap
                    && context.view_stage == ViewStage::EditorAndPanel)
                    || (context.active_tab == MainTab::Explore && context.explore_edit_mode) =>
            {
                Some(Self::ActivateStepInput)
            }
            (KeyCode::Esc, _) => Some(Self::ClearInputState),
            (KeyCode::Up, _) => Some(Self::MoveUp),
            (KeyCode::Down, _) => Some(Self::MoveDown),
            (KeyCode::Left, _) => Some(Self::MoveLeft),
            (KeyCode::Right, _) => Some(Self::MoveRight),
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
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
            context,
        );
        assert_eq!(action, Some(Action::SelectTab(MainTab::Explore)));
    }

    #[test]
    fn test_tab_switch_shortcuts_disabled_in_step_input_state() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: true,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
            context,
        );
        assert_eq!(action, Some(Action::Insert('2')));
    }

    #[test]
    fn test_tree_nav_keys_in_stage1() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
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
            Some(Action::TreeOpen)
        );
    }

    #[test]
    fn test_editor_keys_in_stage3() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            active_tab: MainTab::MindMap,
            view_stage: ViewStage::EditorAndPanel,
            explore_edit_mode: false,
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
            Some(Action::ActivateStepInput)
        );
    }

    #[test]
    fn test_explore_tab_navigation_keys() {
        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            active_tab: MainTab::Explore,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
        };
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), context),
            Some(Action::FocusNextColumn)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), context),
            Some(Action::FocusNextColumn)
        );
        assert_eq!(
            Action::from_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), context),
            Some(Action::FocusPrevColumn)
        );
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
                context
            ),
            Some(Action::EnterEdit)
        );
    }
}
