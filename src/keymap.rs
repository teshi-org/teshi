use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    ActivateStepInput,
    ClearInputState,
}

impl Action {
    pub fn from_key_event(event: KeyEvent, step_input_active: bool) -> Option<Self> {
        if step_input_active {
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
            (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
            (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Save),
            (KeyCode::Char(' '), KeyModifiers::NONE) => Some(Self::ActivateStepInput),
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
