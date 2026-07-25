use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};

pub struct EventSource {
    #[cfg(windows)]
    zellij_vt: Option<windows_vt::ZellijVtInput>,
}

impl EventSource {
    pub fn new() -> Result<Self> {
        Ok(Self {
            #[cfg(windows)]
            zellij_vt: windows_vt::ZellijVtInput::start_if_needed()?,
        })
    }

    pub fn next(&mut self, timeout: Duration) -> Result<Option<Event>> {
        #[cfg(windows)]
        if let Some(input) = self.zellij_vt.as_mut() {
            return input.next(timeout);
        }

        if event::poll(timeout)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(windows)]
mod windows_vt {
    use std::io::{self, Read};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
    use std::thread;
    use std::time::Duration;

    use anyhow::{Context, Result};
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use termwiz::input::{
        InputEvent, InputParser, KeyCode as TermwizKeyCode, KeyEvent as TermwizKeyEvent, Modifiers,
        MouseButtons, MouseEvent as TermwizMouseEvent,
    };
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ENABLE_EXTENDED_FLAGS, ENABLE_MOUSE_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_WINDOW_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE, SetConsoleMode,
    };

    const ESCAPE_FLUSH_INTERVAL: Duration = Duration::from_millis(50);

    pub struct ZellijVtInput {
        events: Receiver<io::Result<Event>>,
        _console_mode: ConsoleModeGuard,
    }

    impl ZellijVtInput {
        pub fn start_if_needed() -> Result<Option<Self>> {
            if std::env::var_os("ZELLIJ").is_none() {
                return Ok(None);
            }

            let console_mode = ConsoleModeGuard::enable()
                .context("failed to enable VT input for Zellij on Windows")?;
            let (event_tx, event_rx) = mpsc::sync_channel(256);
            thread::Builder::new()
                .name("teshi-vt-input".to_string())
                .spawn(move || parse_stdin(event_tx))
                .context("failed to start the Zellij VT input reader")?;

            Ok(Some(Self {
                events: event_rx,
                _console_mode: console_mode,
            }))
        }

        pub fn next(&mut self, timeout: Duration) -> Result<Option<Event>> {
            match self.events.recv_timeout(timeout) {
                Ok(Ok(event)) => Ok(Some(event)),
                Ok(Err(error)) => Err(error.into()),
                Err(RecvTimeoutError::Timeout) => Ok(None),
                Err(RecvTimeoutError::Disconnected) => {
                    Err(anyhow::anyhow!("Zellij VT input reader stopped"))
                }
            }
        }
    }

    struct ConsoleModeGuard {
        original_mode: u32,
    }

    impl ConsoleModeGuard {
        fn enable() -> io::Result<Self> {
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                    return Err(io::Error::last_os_error());
                }

                let mut original_mode = 0;
                if GetConsoleMode(handle, &mut original_mode) == 0 {
                    return Err(io::Error::last_os_error());
                }

                let vt_mode = ENABLE_WINDOW_INPUT
                    | ENABLE_MOUSE_INPUT
                    | ENABLE_EXTENDED_FLAGS
                    | ENABLE_VIRTUAL_TERMINAL_INPUT;
                if SetConsoleMode(handle, vt_mode) == 0 {
                    return Err(io::Error::last_os_error());
                }

                Ok(Self { original_mode })
            }
        }
    }

    impl Drop for ConsoleModeGuard {
        fn drop(&mut self) {
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetConsoleMode(handle, self.original_mode);
                }
            }
        }
    }

    fn parse_stdin(event_tx: SyncSender<io::Result<Event>>) {
        let (bytes_tx, bytes_rx) = mpsc::sync_channel(32);
        let _stdin_pump = thread::Builder::new()
            .name("teshi-stdin-pump".to_string())
            .spawn(move || {
                let stdin = io::stdin();
                let mut stdin = stdin.lock();
                let mut buffer = [0u8; 4096];
                loop {
                    match stdin.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => {
                            if bytes_tx.send(Ok(buffer[..read].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = bytes_tx.send(Err(error));
                            break;
                        }
                    }
                }
            });

        let mut parser = InputParser::new();
        let mut pressed_button = None;
        loop {
            let input_events = match bytes_rx.recv_timeout(ESCAPE_FLUSH_INTERVAL) {
                Ok(Ok(bytes)) => parser.parse_as_vec(&bytes, true),
                Ok(Err(error)) => {
                    let _ = event_tx.send(Err(error));
                    break;
                }
                Err(RecvTimeoutError::Timeout) => parser.parse_as_vec(&[], false),
                Err(RecvTimeoutError::Disconnected) => break,
            };

            for input_event in input_events {
                if let Some(event) = convert_event(input_event, &mut pressed_button)
                    && event_tx.send(Ok(event)).is_err()
                {
                    return;
                }
            }
        }
    }

    fn convert_event(event: InputEvent, pressed_button: &mut Option<MouseButton>) -> Option<Event> {
        match event {
            InputEvent::Key(key) => convert_key(key).map(Event::Key),
            InputEvent::Mouse(mouse) => Some(Event::Mouse(convert_mouse(mouse, pressed_button))),
            InputEvent::Paste(text) => Some(Event::Paste(text)),
            InputEvent::Resized { cols, rows } => Some(Event::Resize(
                u16::try_from(cols).unwrap_or(u16::MAX),
                u16::try_from(rows).unwrap_or(u16::MAX),
            )),
            InputEvent::PixelMouse(_) | InputEvent::Wake => None,
        }
    }

    fn convert_key(event: TermwizKeyEvent) -> Option<KeyEvent> {
        let mut modifiers = convert_modifiers(event.modifiers);
        let code = match event.key {
            TermwizKeyCode::Char(character) => {
                if character.is_uppercase() {
                    modifiers |= KeyModifiers::SHIFT;
                }
                KeyCode::Char(character)
            }
            TermwizKeyCode::Backspace => KeyCode::Backspace,
            TermwizKeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            TermwizKeyCode::Tab => KeyCode::Tab,
            TermwizKeyCode::Enter => KeyCode::Enter,
            TermwizKeyCode::Escape => KeyCode::Esc,
            TermwizKeyCode::PageUp | TermwizKeyCode::KeyPadPageUp => KeyCode::PageUp,
            TermwizKeyCode::PageDown | TermwizKeyCode::KeyPadPageDown => KeyCode::PageDown,
            TermwizKeyCode::End | TermwizKeyCode::KeyPadEnd => KeyCode::End,
            TermwizKeyCode::Home | TermwizKeyCode::KeyPadHome => KeyCode::Home,
            TermwizKeyCode::LeftArrow | TermwizKeyCode::ApplicationLeftArrow => KeyCode::Left,
            TermwizKeyCode::RightArrow | TermwizKeyCode::ApplicationRightArrow => KeyCode::Right,
            TermwizKeyCode::UpArrow | TermwizKeyCode::ApplicationUpArrow => KeyCode::Up,
            TermwizKeyCode::DownArrow | TermwizKeyCode::ApplicationDownArrow => KeyCode::Down,
            TermwizKeyCode::Insert => KeyCode::Insert,
            TermwizKeyCode::Delete => KeyCode::Delete,
            TermwizKeyCode::Function(number) => KeyCode::F(number),
            _ => return None,
        };
        Some(KeyEvent::new(code, modifiers))
    }

    fn convert_mouse(
        event: TermwizMouseEvent,
        pressed_button: &mut Option<MouseButton>,
    ) -> MouseEvent {
        let kind = if event.mouse_buttons.contains(MouseButtons::VERT_WHEEL) {
            if event.mouse_buttons.contains(MouseButtons::WHEEL_POSITIVE) {
                MouseEventKind::ScrollUp
            } else {
                MouseEventKind::ScrollDown
            }
        } else if event.mouse_buttons.contains(MouseButtons::HORZ_WHEEL) {
            if event.mouse_buttons.contains(MouseButtons::WHEEL_POSITIVE) {
                MouseEventKind::ScrollRight
            } else {
                MouseEventKind::ScrollLeft
            }
        } else if let Some(button) = current_button(&event.mouse_buttons) {
            let kind = if *pressed_button == Some(button) {
                MouseEventKind::Drag(button)
            } else {
                MouseEventKind::Down(button)
            };
            *pressed_button = Some(button);
            kind
        } else if let Some(button) = pressed_button.take() {
            MouseEventKind::Up(button)
        } else {
            MouseEventKind::Moved
        };

        MouseEvent {
            kind,
            column: event.x.saturating_sub(1),
            row: event.y.saturating_sub(1),
            modifiers: convert_modifiers(event.modifiers),
        }
    }

    fn current_button(buttons: &MouseButtons) -> Option<MouseButton> {
        if buttons.contains(MouseButtons::LEFT) {
            Some(MouseButton::Left)
        } else if buttons.contains(MouseButtons::RIGHT) {
            Some(MouseButton::Right)
        } else if buttons.contains(MouseButtons::MIDDLE) {
            Some(MouseButton::Middle)
        } else {
            None
        }
    }

    fn convert_modifiers(modifiers: Modifiers) -> KeyModifiers {
        let mut converted = KeyModifiers::empty();
        if modifiers.contains(Modifiers::SHIFT) {
            converted |= KeyModifiers::SHIFT;
        }
        if modifiers.contains(Modifiers::ALT) {
            converted |= KeyModifiers::ALT;
        }
        if modifiers.contains(Modifiers::CTRL) {
            converted |= KeyModifiers::CONTROL;
        }
        converted
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn zellij_vt_decodes_sgr_mouse_click() {
            let mut parser = InputParser::new();
            let input_events = parser.parse_as_vec(b"\x1b[<0;12;7M\x1b[<0;12;7m", false);
            let mut pressed_button = None;
            let events = input_events
                .into_iter()
                .filter_map(|event| convert_event(event, &mut pressed_button))
                .collect::<Vec<_>>();

            assert_eq!(
                events,
                vec![
                    Event::Mouse(MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: 11,
                        row: 6,
                        modifiers: KeyModifiers::empty(),
                    }),
                    Event::Mouse(MouseEvent {
                        kind: MouseEventKind::Up(MouseButton::Left),
                        column: 11,
                        row: 6,
                        modifiers: KeyModifiers::empty(),
                    }),
                ]
            );
        }

        #[test]
        fn shifted_tab_becomes_back_tab() {
            let event = convert_key(TermwizKeyEvent {
                key: TermwizKeyCode::Tab,
                modifiers: Modifiers::SHIFT,
            })
            .unwrap();

            assert_eq!(event, KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        }

        #[test]
        fn uppercase_character_implies_shift() {
            let event = convert_key(TermwizKeyEvent {
                key: TermwizKeyCode::Char('Y'),
                modifiers: Modifiers::empty(),
            })
            .unwrap();

            assert_eq!(
                event,
                KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT)
            );
        }
    }
}
