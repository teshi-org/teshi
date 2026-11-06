//! Windows win32-input-mode key records.
//!
//! With `ENABLE_VIRTUAL_TERMINAL_INPUT`, conhost turns key presses into VT
//! text and drops whatever VT cannot express, such as Shift on Enter. After
//! the application writes [`ENABLE`], conhost reports every key record as
//! `CSI Vk;Sc;Uc;Kd;Cs;Rc _` instead. [`Decoder`] splits that stream into VT
//! text for the VT parser and ready key events.
//!
//! Records that produce text become text: typed characters, unmodified Enter
//! and Tab, and the synthetic records (virtual key 0 or `VK_PACKET`) that
//! carry terminal sequences such as bracketed-paste markers and mouse
//! reports. A paste therefore reaches the VT parser as one uninterrupted
//! sequence even when the console delivers its newlines as real key presses.
//! Every other key becomes a key event with the semantics of crossterm's
//! native Windows backend, which the keymap was written against.
//!
//! A console that ignores the mode sends plain VT, which passes through as
//! text unchanged.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Asks the console to report key records in win32-input-mode.
pub(super) const ENABLE: &str = "\x1b[?9001h";
/// Restores plain VT key input.
pub(super) const DISABLE: &str = "\x1b[?9001l";

/// Longest parameter run still considered a possible record.
const MAX_RECORD_LEN: usize = 64;

const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_PACKET: u16 = 0xE7;
/// Shift, Ctrl, Alt, Caps Lock, Windows, Num Lock, Scroll Lock, and their
/// left/right variants: they only change the state of other keys.
const MODIFIER_KEYS: [u16; 14] = [
    0x10, 0x11, 0x12, 0x14, 0x5B, 0x5C, 0x90, 0x91, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
];

const RIGHT_ALT: u32 = 0x01;
const LEFT_ALT: u32 = 0x02;
const RIGHT_CTRL: u32 = 0x04;
const LEFT_CTRL: u32 = 0x08;
const SHIFT: u32 = 0x10;
const CAPS_LOCK: u32 = 0x80;

/// One decoded part of the input stream, in arrival order.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Piece {
    /// VT text for the VT parser.
    Text(String),
    /// A key press decoded from its record.
    Key(KeyEvent),
}

/// Fields of one `KEY_EVENT_RECORD`.
struct Record {
    virtual_key: u16,
    unicode: u16,
    key_down: bool,
    state: u32,
    repeat: u16,
}

/// Incremental decoder; records and surrogate pairs may span reads.
#[derive(Default)]
pub(super) struct Decoder {
    /// A possible record start, held until it completes or proves to be VT.
    pending: String,
    high_surrogate: Option<u16>,
}

impl Decoder {
    /// Decodes the next chunk of console input.
    pub(super) fn feed(&mut self, input: &str) -> Vec<Piece> {
        let mut out = Output::default();
        for ch in input.chars() {
            let in_csi = self.pending.starts_with("\x1b[");
            if ch == '\x1b' {
                out.text(&std::mem::take(&mut self.pending));
                self.pending.push(ch);
            } else if (self.pending == "\x1b" && ch == '[')
                || (in_csi
                    && (ch.is_ascii_digit() || ch == ';')
                    && self.pending.len() < MAX_RECORD_LEN)
            {
                self.pending.push(ch);
            } else if in_csi && ch == '_' {
                let record = std::mem::take(&mut self.pending);
                // A malformed record is dropped: it is not VT either.
                if let Some(record) = parse_record(&record[2..]) {
                    self.decode(&record, &mut out);
                }
            } else {
                out.text(&std::mem::take(&mut self.pending));
                out.char(ch);
            }
        }
        out.pieces
    }

    /// Releases text held as a possible record start, once input pauses, so
    /// a lone Escape from plain VT input is not held forever.
    pub(super) fn flush(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }

    fn decode(&mut self, record: &Record, out: &mut Output) {
        let state = record.state;
        let alt = state & (LEFT_ALT | RIGHT_ALT) != 0;
        let ctrl = state & (LEFT_CTRL | RIGHT_CTRL) != 0;
        if !record.key_down {
            // An Alt code commits its character when Alt is released.
            if record.virtual_key == VK_MENU
                && let Some(ch) = self.unit(record.unicode)
            {
                out.char(ch);
            }
            return;
        }
        if MODIFIER_KEYS.contains(&record.virtual_key)
            // Alt plus numpad digits composes an Alt code; the digits are not keys.
            || (alt && !ctrl && (0x60..=0x69).contains(&record.virtual_key))
        {
            return;
        }
        let ch = self.unit(record.unicode);
        let repeat = usize::from(record.repeat.max(1));
        if matches!(record.virtual_key, 0 | VK_PACKET) {
            if let Some(ch) = ch {
                (0..repeat).for_each(|_| out.char(ch));
            }
            return;
        }
        let modifiers = modifiers(state);
        let plain = modifiers.is_empty();
        let text = match record.virtual_key {
            VK_RETURN if plain => Some('\r'),
            VK_TAB if plain => Some('\t'),
            VK_BACK | VK_TAB | VK_RETURN | VK_ESCAPE => None,
            // AltGr reports as Ctrl+Alt but types the character it shows.
            _ => ch.filter(|ch| !ch.is_control() && (!(alt || ctrl) || (alt && ctrl))),
        };
        if let Some(text) = text {
            (0..repeat).for_each(|_| out.char(text));
            return;
        }
        if let Some(code) = key_code(record, ch, modifiers) {
            let key = KeyEvent::new(code, modifiers);
            (0..repeat).for_each(|_| out.pieces.push(Piece::Key(key)));
        }
    }

    /// The character for one UTF-16 unit, pairing surrogates across records.
    fn unit(&mut self, unit: u16) -> Option<char> {
        match unit {
            0 => None,
            0xD800..=0xDBFF => {
                self.high_surrogate = Some(unit);
                None
            }
            0xDC00..=0xDFFF => {
                let high = self.high_surrogate.take()?;
                char::decode_utf16([high, unit]).next()?.ok()
            }
            unit => {
                self.high_surrogate = None;
                char::from_u32(u32::from(unit))
            }
        }
    }
}

/// Merges consecutive text so the VT parser sees whole sequences.
#[derive(Default)]
struct Output {
    pieces: Vec<Piece>,
}

impl Output {
    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(Piece::Text(last)) = self.pieces.last_mut() {
            last.push_str(text);
        } else {
            self.pieces.push(Piece::Text(text.to_string()));
        }
    }

    fn char(&mut self, ch: char) {
        self.text(ch.encode_utf8(&mut [0; 4]));
    }
}

fn parse_record(parameters: &str) -> Option<Record> {
    let mut values = [0u32, 0, 0, 0, 0, 1];
    let parts: Vec<&str> = parameters.split(';').collect();
    if parts.len() > values.len() {
        return None;
    }
    for (value, part) in values.iter_mut().zip(parts) {
        if !part.is_empty() {
            *value = part.parse().ok()?;
        }
    }
    let [virtual_key, _scan, unicode, key_down, state, repeat] = values;
    if key_down > 1 {
        return None;
    }
    Some(Record {
        virtual_key: u16::try_from(virtual_key).ok()?,
        unicode: u16::try_from(unicode).ok()?,
        key_down: key_down == 1,
        state,
        repeat: u16::try_from(repeat).ok()?,
    })
}

fn modifiers(state: u32) -> KeyModifiers {
    let mut modifiers = KeyModifiers::empty();
    if state & SHIFT != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if state & (LEFT_CTRL | RIGHT_CTRL) != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    if state & (LEFT_ALT | RIGHT_ALT) != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    modifiers
}

/// The key a non-text record names, as crossterm's Windows backend reports it.
fn key_code(record: &Record, ch: Option<char>, modifiers: KeyModifiers) -> Option<KeyCode> {
    Some(match record.virtual_key {
        VK_BACK => KeyCode::Backspace,
        VK_TAB if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        VK_TAB => KeyCode::Tab,
        VK_RETURN => KeyCode::Enter,
        VK_ESCAPE => KeyCode::Esc,
        0x21 => KeyCode::PageUp,
        0x22 => KeyCode::PageDown,
        0x23 => KeyCode::End,
        0x24 => KeyCode::Home,
        0x25 => KeyCode::Left,
        0x26 => KeyCode::Up,
        0x27 => KeyCode::Right,
        0x28 => KeyCode::Down,
        0x2D => KeyCode::Insert,
        0x2E => KeyCode::Delete,
        vk @ 0x70..=0x87 => KeyCode::F((vk - 0x6F) as u8),
        _ => KeyCode::Char(match ch.filter(|ch| !ch.is_control()) {
            Some(ch) => ch,
            // Ctrl turns letters into control codes; report the key's own
            // character, cased like crossterm does.
            None => layout_char(record)?,
        }),
    })
}

/// The character a key types without modifiers, for Ctrl combinations.
fn layout_char(record: &Record) -> Option<char> {
    let upper = (record.state & SHIFT != 0) ^ (record.state & CAPS_LOCK != 0);
    match record.virtual_key {
        vk @ 0x41..=0x5A => {
            let letter = char::from(vk as u8);
            Some(if upper {
                letter
            } else {
                letter.to_ascii_lowercase()
            })
        }
        vk @ 0x30..=0x39 => Some(char::from(vk as u8)),
        VK_SPACE => Some(' '),
        vk => punctuation_char(vk),
    }
}

/// Unshifted punctuation for `virtual_key` in the input thread's layout.
fn punctuation_char(virtual_key: u16) -> Option<char> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_CHAR, MapVirtualKeyW};
    // The high bit flags a dead key, which types nothing by itself.
    let mapped = unsafe { MapVirtualKeyW(u32::from(virtual_key), MAPVK_VK_TO_CHAR) };
    (mapped != 0 && mapped & 0x8000_0000 == 0)
        .then(|| char::from_u32(mapped & 0xFFFF))
        .flatten()
        .filter(|ch| !ch.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(vk: u16, unicode: u16, state: u32) -> String {
        format!("\x1b[{vk};0;{unicode};1;{state};1_")
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Piece {
        Piece::Key(KeyEvent::new(code, modifiers))
    }

    fn text(text: &str) -> Piece {
        Piece::Text(text.into())
    }

    #[test]
    fn test_records_decode_like_crossterm_windows_keys() {
        let cases: Vec<(String, Vec<Piece>)> = vec![
            (record(0x41, 'a' as u16, 0), vec![text("a")]),
            (record(0x41, 'A' as u16, SHIFT), vec![text("A")]),
            (record(0x0D, 13, 0), vec![text("\r")]),
            (
                record(0x0D, 13, SHIFT),
                vec![key(KeyCode::Enter, KeyModifiers::SHIFT)],
            ),
            (
                record(0x09, 9, SHIFT),
                vec![key(KeyCode::BackTab, KeyModifiers::SHIFT)],
            ),
            (
                record(0x56, 0x16, LEFT_CTRL),
                vec![key(KeyCode::Char('v'), KeyModifiers::CONTROL)],
            ),
            (
                record(0x56, 'v' as u16, LEFT_ALT),
                vec![key(KeyCode::Char('v'), KeyModifiers::ALT)],
            ),
            (
                record(0x31, 0, LEFT_CTRL | SHIFT),
                vec![key(
                    KeyCode::Char('1'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                )],
            ),
            // AltGr on a European layout types `@`.
            (
                record(0x51, '@' as u16, RIGHT_ALT | LEFT_CTRL),
                vec![text("@")],
            ),
            (
                record(0x1B, 27, 0),
                vec![key(KeyCode::Esc, KeyModifiers::NONE)],
            ),
            (
                record(0x25, 0, LEFT_CTRL),
                vec![key(KeyCode::Left, KeyModifiers::CONTROL)],
            ),
            (record(0x10, 0, SHIFT), vec![]),
            // Key releases are dropped; an Alt release with a character is an Alt code.
            ("\x1b[65;0;97;0;0;1_".into(), vec![]),
            ("\x1b[18;0;233;0;0;1_".into(), vec![text("é")]),
            ("\x1b[65;0;97;1;0;3_".into(), vec![text("aaa")]),
        ];
        for (input, expected) in cases {
            assert_eq!(Decoder::default().feed(&input), expected, "{input:?}");
        }
    }

    #[test]
    fn test_synthetic_records_rebuild_paste_and_mouse_sequences() {
        let synthetic = |text: &str| -> String {
            text.encode_utf16()
                .map(|unit| format!("\x1b[0;0;{unit};1;0;1_"))
                .collect()
        };
        let mut decoder = Decoder::default();
        // An empty paste is what a terminal sends for an image-only clipboard.
        let input = format!(
            "{}{}{}",
            synthetic("\x1b[200~a"),
            record(0x0D, 13, 0),
            synthetic("b🙂\x1b[201~\x1b[200~\x1b[201~\x1b[<0;3;4M"),
        );
        assert_eq!(
            decoder.feed(&input),
            vec![text(
                "\x1b[200~a\rb🙂\x1b[201~\x1b[200~\x1b[201~\x1b[<0;3;4M"
            )]
        );
    }

    #[test]
    fn test_plain_vt_passes_through_and_split_records_complete() {
        let mut decoder = Decoder::default();
        assert_eq!(
            decoder.feed("x\x1b[A\x1b[200~"),
            vec![text("x\x1b[A\x1b[200~")]
        );
        assert_eq!(decoder.feed("\x1b[13;0;13;1;"), vec![]);
        assert_eq!(
            decoder.feed("16;1_"),
            vec![key(KeyCode::Enter, KeyModifiers::SHIFT)]
        );
        assert_eq!(decoder.feed("\x1b"), vec![]);
        assert_eq!(decoder.flush(), "\x1b", "a lone Escape is released");
    }
}
