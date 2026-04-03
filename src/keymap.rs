use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::MainTab;

#[derive(Debug, Clone, Copy)]
pub struct KeyContext {
    pub step_input_active: bool,
    pub active_tab: MainTab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    PageUp,
    PageDown,
    Insert(char),
    Enter,
    Backspace,
    Delete,
    Save,
    Quit,
    SelectTab(MainTab),
    ActivateStepInput,
    ClearInputState,
}

impl Action {
    pub fn from_key_event(event: KeyEvent, context: KeyContext) -> Option<Self> {
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

        match (event.code, event.modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Editor)),
            (KeyCode::Char('2'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Feature)),
            (KeyCode::Char('3'), KeyModifiers::NONE) => Some(Self::SelectTab(MainTab::Help)),
            (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
            (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
            (KeyCode::Char(' '), KeyModifiers::NONE) if context.active_tab == MainTab::Editor => {
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
    use crate::app::MainTab;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn test_tab_switch_shortcuts_in_non_input_state() {
        let context = KeyContext {
            step_input_active: false,
            active_tab: MainTab::Editor,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
            context,
        );
        assert_eq!(action, Some(Action::SelectTab(MainTab::Feature)));
    }

    #[test]
    fn test_tab_switch_shortcuts_disabled_in_step_input_state() {
        let context = KeyContext {
            step_input_active: true,
            active_tab: MainTab::Editor,
        };
        let action = Action::from_key_event(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
            context,
        );
        assert_eq!(action, Some(Action::Insert('2')));
    }
}
