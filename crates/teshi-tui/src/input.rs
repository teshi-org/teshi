//! Terminal input events.
//!
//! On Windows, terminal input is read as VT through an owned console handle
//! rather than through crossterm's console-record backend. That backend never
//! reports bracketed paste, so a terminal's empty paste for an image-only
//! clipboard (how Ctrl+V reaches an application with a screenshot) would be
//! lost. Set `TESHI_LEGACY_CONSOLE_INPUT` to use crossterm's backend instead.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};

#[cfg(windows)]
mod win32_keys;

pub struct EventSource {
    #[cfg(windows)]
    vt: Option<windows_vt::VtInput>,
}

impl EventSource {
    pub fn new() -> Result<Self> {
        Ok(Self {
            #[cfg(windows)]
            vt: windows_vt::VtInput::start_if_needed()?,
        })
    }

    pub fn next(&mut self, timeout: Duration) -> Result<Option<Event>> {
        #[cfg(windows)]
        if let Some(input) = self.vt.as_mut() {
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
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
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
        ENABLE_WINDOW_INPUT, GetConsoleMode, GetStdHandle, ReadConsoleW, STD_INPUT_HANDLE,
        SetConsoleMode,
    };

    use super::win32_keys::{self, Decoder, Piece};

    const ESCAPE_FLUSH_INTERVAL: Duration = Duration::from_millis(50);
    /// Opts out of VT input and keeps crossterm's console-record backend.
    const LEGACY_INPUT_ENV: &str = "TESHI_LEGACY_CONSOLE_INPUT";

    pub struct VtInput {
        events: Receiver<io::Result<Event>>,
        stop: Arc<AtomicBool>,
        reader: Option<thread::JoinHandle<()>>,
        parser: Option<thread::JoinHandle<()>>,
        _console_mode: ConsoleModeGuard,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CancelSynchronousIo(thread: windows_sys::Win32::Foundation::HANDLE) -> i32;
    }

    fn stop_reader(reader: thread::JoinHandle<()>) {
        while !reader.is_finished() {
            unsafe {
                CancelSynchronousIo(reader.as_raw_handle());
            }
            thread::sleep(Duration::from_millis(1));
        }
        let _ = reader.join();
    }

    impl Drop for VtInput {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            while self.events.try_recv().is_ok() {}
            if let Some(parser) = self.parser.take() {
                let _ = parser.join();
            }
            if let Some(reader) = self.reader.take() {
                // Cancel the owned synchronous console read before restoring
                // its input mode. Retrying covers the read-start race.
                stop_reader(reader);
            }
        }
    }

    impl VtInput {
        /// Starts VT input unless opted out.
        ///
        /// Zellij needs VT input to deliver mouse and paste at all, so a
        /// failure there is an error. Elsewhere a console that refuses VT
        /// input falls back to crossterm's backend. Zellij keeps plain VT
        /// keys; direct consoles also get win32-input-mode for full key
        /// fidelity.
        pub fn start_if_needed() -> Result<Option<Self>> {
            if std::env::var_os("ZELLIJ").is_some() {
                return Self::start(false).map(Some);
            }
            if std::env::var_os(LEGACY_INPUT_ENV).is_some() {
                return Ok(None);
            }
            match Self::start(true) {
                Ok(input) => Ok(Some(input)),
                Err(error) => {
                    tracing::debug!(error = %format!("{error:#}"), "console VT input unavailable");
                    Ok(None)
                }
            }
        }

        fn start(key_records: bool) -> Result<Self> {
            let console_mode = ConsoleModeGuard::enable(key_records)
                .context("failed to enable VT terminal input on Windows")?;
            // Cancellation must not leave a read pending on the process-wide
            // stdin handle: later consumers need a fresh console read state.
            let console_input = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("CONIN$")
                .context("failed to open owned console input")?;
            let (event_tx, event_rx) = mpsc::sync_channel(256);
            let (bytes_tx, bytes_rx) = mpsc::sync_channel(32);
            let stop = Arc::new(AtomicBool::new(false));
            let reader_stop = stop.clone();
            let reader = thread::Builder::new()
                .name("teshi-stdin-pump".into())
                .spawn(move || {
                    let stdin = console_input.as_raw_handle();
                    let mut buffer = [0u16; 4096];
                    let mut pending_surrogate = None;
                    while !reader_stop.load(Ordering::Acquire) {
                        let mut read = 0;
                        // Read UTF-16 directly: std::io::Stdin retries a
                        // cancelled read, while byte console reads can split
                        // supplementary Unicode characters incorrectly.
                        let succeeded = unsafe {
                            ReadConsoleW(
                                stdin,
                                buffer.as_mut_ptr().cast(),
                                buffer.len() as u32,
                                &mut read,
                                std::ptr::null(),
                            )
                        };
                        if reader_stop.load(Ordering::Acquire) {
                            break;
                        }
                        if succeeded == 0 {
                            let _ = bytes_tx.send(Err(io::Error::last_os_error()));
                            break;
                        }
                        if read == 0 {
                            break;
                        }
                        if bytes_tx
                            .send(Ok(decode_console_units(
                                &buffer[..read as usize],
                                &mut pending_surrogate,
                            )))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .context("failed to start the VT stdin reader")?;
            let parser_stop = stop.clone();
            let parser = thread::Builder::new()
                .name("teshi-vt-input".to_string())
                .spawn(move || {
                    parse_stdin(
                        event_tx,
                        bytes_rx,
                        parser_stop,
                        key_records.then(Decoder::default),
                    )
                });
            let parser = match parser {
                Ok(parser) => parser,
                Err(error) => {
                    stop.store(true, Ordering::Release);
                    stop_reader(reader);
                    return Err(error).context("failed to start the VT input parser");
                }
            };

            Ok(Self {
                events: event_rx,
                stop,
                reader: Some(reader),
                parser: Some(parser),
                _console_mode: console_mode,
            })
        }

        pub fn next(&mut self, timeout: Duration) -> Result<Option<Event>> {
            match self.events.recv_timeout(timeout) {
                Ok(Ok(event)) => Ok(Some(event)),
                Ok(Err(error)) => Err(error.into()),
                Err(RecvTimeoutError::Timeout) => Ok(None),
                Err(RecvTimeoutError::Disconnected) => {
                    Err(anyhow::anyhow!("VT input reader stopped"))
                }
            }
        }
    }

    fn decode_console_units(chunk: &[u16], pending_surrogate: &mut Option<u16>) -> String {
        let mut units = Vec::with_capacity(chunk.len() + 1);
        units.extend(pending_surrogate.take());
        units.extend_from_slice(chunk);
        // ReadConsoleW can stop between the two units of a supplementary
        // character. Keep that high surrogate until the next read arrives.
        if units
            .last()
            .is_some_and(|unit| (0xd800..=0xdbff).contains(unit))
        {
            *pending_surrogate = units.pop();
        }
        String::from_utf16_lossy(&units)
    }

    /// Console input mode for VT reading, restored on drop.
    struct ConsoleModeGuard {
        original_mode: u32,
        /// Whether win32-input-mode was requested and must be turned off.
        key_records: bool,
    }

    /// Writes a mode switch to the console host. A console that does not
    /// know the mode ignores it, which leaves plain VT input.
    fn write_mode(sequence: &str) -> io::Result<()> {
        use std::io::Write;
        let mut stdout = io::stdout();
        stdout.write_all(sequence.as_bytes())?;
        stdout.flush()
    }

    impl ConsoleModeGuard {
        fn enable(key_records: bool) -> io::Result<Self> {
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
                let guard = Self {
                    original_mode,
                    key_records,
                };
                if key_records {
                    write_mode(win32_keys::ENABLE)?;
                }
                Ok(guard)
            }
        }
    }

    impl Drop for ConsoleModeGuard {
        fn drop(&mut self) {
            if self.key_records {
                let _ = write_mode(win32_keys::DISABLE);
            }
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetConsoleMode(handle, self.original_mode);
                }
            }
        }
    }

    fn parse_stdin(
        event_tx: SyncSender<io::Result<Event>>,
        text_rx: Receiver<io::Result<String>>,
        stop: Arc<AtomicBool>,
        mut key_records: Option<Decoder>,
    ) {
        let mut parser = InputParser::new();
        let mut pressed_button = None;
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let mut events = Vec::new();
            match text_rx.recv_timeout(ESCAPE_FLUSH_INTERVAL) {
                Ok(Ok(text)) => {
                    let pieces = match key_records.as_mut() {
                        Some(decoder) => decoder.feed(&text),
                        None => vec![Piece::Text(text)],
                    };
                    // Pieces keep arrival order: text parsed so far is
                    // emitted before a key that followed it.
                    for piece in pieces {
                        match piece {
                            Piece::Text(text) => events.extend(
                                parser
                                    .parse_as_vec(text.as_bytes(), true)
                                    .into_iter()
                                    .filter_map(|event| convert_event(event, &mut pressed_button)),
                            ),
                            Piece::Key(key) => events.push(Event::Key(key)),
                        }
                    }
                }
                Ok(Err(error)) => {
                    let _ = event_tx.send(Err(error));
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Input paused: a held Escape is a key, not a sequence start.
                    let held = key_records.as_mut().map(Decoder::flush).unwrap_or_default();
                    events.extend(
                        parser
                            .parse_as_vec(held.as_bytes(), false)
                            .into_iter()
                            .filter_map(|event| convert_event(event, &mut pressed_button)),
                    );
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }

            for event in events {
                if stop.load(Ordering::Acquire) {
                    return;
                }
                if event_tx.send(Ok(event)).is_err() {
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
        fn console_read_boundary_preserves_supplementary_unicode() {
            let mut pending = None;
            assert_eq!(decode_console_units(&[0x7532, 0xd83d], &mut pending), "甲");
            assert_eq!(
                decode_console_units(&[0xde42, 0x5c3e], &mut pending),
                "🙂尾"
            );
            assert_eq!(pending, None);
            assert_eq!(decode_console_units(&[0xd83d, 0x61], &mut pending), "�a");
        }

        /// Feeds `input` to a parser thread and returns the first `count` events.
        fn parse_events(input: &str, key_records: bool, count: usize) -> Vec<Event> {
            let (text_tx, text_rx) = mpsc::sync_channel(1);
            let (event_tx, event_rx) = mpsc::sync_channel(16);
            let stop = Arc::new(AtomicBool::new(false));
            let parser_stop = stop.clone();
            let decoder = key_records.then(Decoder::default);
            let parser =
                thread::spawn(move || parse_stdin(event_tx, text_rx, parser_stop, decoder));
            text_tx.send(Ok(input.to_string())).unwrap();
            let events = (0..count)
                .map(|_| {
                    event_rx
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap()
                        .unwrap()
                })
                .collect();
            stop.store(true, Ordering::Release);
            drop(text_tx);
            parser.join().unwrap();
            events
        }

        #[test]
        fn parser_flushes_lone_escape_without_waiting_for_another_key() {
            for key_records in [false, true] {
                assert_eq!(
                    parse_events("\x1b", key_records, 1),
                    [Event::Key(KeyEvent::new(
                        KeyCode::Esc,
                        KeyModifiers::empty()
                    ))],
                    "key records: {key_records}"
                );
            }
        }

        #[test]
        fn key_records_keep_order_and_deliver_an_empty_paste() {
            let synthetic: String = "\x1b[200~\x1b[201~"
                .encode_utf16()
                .map(|unit| format!("\x1b[0;0;{unit};1;0;1_"))
                .collect();
            let input = format!("\x1b[65;0;97;1;0;1_{synthetic}\x1b[13;0;13;1;16;1_");
            assert_eq!(
                parse_events(&input, true, 3),
                [
                    Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty())),
                    Event::Paste(String::new()),
                    Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
                ]
            );
        }

        #[test]
        #[ignore = "real console fixture; launched by the Windows PTY input test"]
        fn console_lifecycle_fixture() {
            if std::env::var_os("TESHI_VT_INPUT_FIXTURE").is_none() {
                return;
            }
            use std::io::Write;
            // The application establishes raw mode before constructing its
            // event source; reproduce that console lifecycle here as well.
            crossterm::terminal::enable_raw_mode().unwrap();
            let stdin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
            let mut original_mode = 0;
            assert_ne!(unsafe { GetConsoleMode(stdin, &mut original_mode) }, 0);
            // Without Zellij the source also runs win32-input-mode, and the
            // PTY test sends an empty bracketed paste after the text.
            let key_records = std::env::var_os("ZELLIJ").is_none();
            if key_records {
                print!("\x1b[?2004h");
            }
            for cycle in 0..3 {
                let mut source = VtInput::start_if_needed().unwrap().unwrap();
                println!("vt-ready-{cycle}");
                io::stdout().flush().unwrap();
                let mut received = String::new();
                loop {
                    match source.next(Duration::from_secs(5)).unwrap() {
                        Some(Event::Key(key)) if key.code == KeyCode::Char('|') => break,
                        Some(Event::Key(key)) => {
                            if let KeyCode::Char(character) = key.code {
                                received.push(character);
                            }
                        }
                        Some(Event::Paste(text)) => received.push_str(&format!("[paste:{text}]")),
                        event => {
                            panic!("unexpected console event: {event:?}; received {received:?}")
                        }
                    }
                }
                let expected = if key_records {
                    "甲🙂尾[paste:]"
                } else {
                    "甲🙂尾"
                };
                assert_eq!(received, expected);
                let started = std::time::Instant::now();
                drop(source);
                assert!(
                    started.elapsed() < Duration::from_secs(2),
                    "console read was not cancelled"
                );
                let mut mode = 0;
                assert_ne!(unsafe { GetConsoleMode(stdin, &mut mode) }, 0);
                assert_eq!(mode, original_mode, "console mode leaked after drop");
                println!("vt-released-{cycle}");
                io::stdout().flush().unwrap();
            }
            if key_records {
                print!("\x1b[?2004l");
            }
            crossterm::terminal::disable_raw_mode().unwrap();
            println!("stdin-ready");
            io::stdout().flush().unwrap();
            let mut line = String::new();
            io::stdin().read_line(&mut line).unwrap();
            assert_eq!(line.trim_end(), "after-vt");
            println!("stdin-restored");
        }

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
        fn bracketed_paste_survives_a_split_read() {
            let mut parser = InputParser::new();
            let mut pasted = String::new();
            for chunk in [
                &b"\x1b[200~\xe7\x94\xb2\xf0\x9f\x99\x82\r\n\tsec"[..],
                &b"ond line\x1b[201~"[..],
            ] {
                for event in parser.parse_as_vec(chunk, true) {
                    if let InputEvent::Paste(text) = event {
                        pasted.push_str(&text);
                    }
                }
            }
            assert_eq!(pasted, "甲🙂\r\n\tsecond line");
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
