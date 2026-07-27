//! Conversions between UTF-8 byte offsets and persisted Unicode scalar positions.

use crate::authoring::{CharPosition, TextRange};

/// Converts a UTF-8 byte offset to a Unicode scalar offset.
///
/// Returns `None` when `byte_offset` is not on a `char` boundary.
pub fn byte_offset_to_char_position(text: &str, byte_offset: usize) -> Option<CharPosition> {
    if byte_offset > text.len() || !text.is_char_boundary(byte_offset) {
        return None;
    }
    let char_offset = text[..byte_offset].chars().count() as u32;
    Some(CharPosition::new(char_offset))
}

/// Converts a Unicode scalar offset to a UTF-8 byte offset.
///
/// Returns `None` when `position` exceeds the document scalar length.
pub fn char_position_to_byte_offset(text: &str, position: CharPosition) -> Option<usize> {
    let mut char_idx = 0u32;
    for (byte_idx, _) in text.char_indices() {
        if char_idx == position.offset() {
            return Some(byte_idx);
        }
        char_idx += 1;
    }
    if char_idx == position.offset() && position.offset() == text.chars().count() as u32 {
        return Some(text.len());
    }
    None
}

/// Returns the number of Unicode scalars in `text`.
pub fn document_char_len(text: &str) -> u32 {
    text.chars().count() as u32
}

/// Extracts the substring covered by a half-open scalar range.
///
/// Returns `None` when the range is invalid for `text`.
pub fn slice_by_char_range(text: &str, range: TextRange) -> Option<String> {
    if !range.is_non_empty() {
        return None;
    }
    let len = document_char_len(text);
    if range.end.offset() > len {
        return None;
    }
    let start_byte = char_position_to_byte_offset(text, range.start)?;
    let end_byte = char_position_to_byte_offset(text, range.end)?;
    text.get(start_byte..end_byte).map(str::to_string)
}

/// Maps a `(line, col)` position in a newline-delimited document to a scalar offset.
///
/// Both `line` and `col` count Unicode scalars, matching the TUI editor buffer.
pub fn line_col_to_char_position(text: &str, line: usize, col: usize) -> Option<CharPosition> {
    let mut current_line = 0usize;
    let mut col_remaining = col;
    let mut char_offset = 0u32;

    for ch in text.chars() {
        if current_line == line {
            if col_remaining == 0 {
                return Some(CharPosition::new(char_offset));
            }
            col_remaining -= 1;
        }
        if ch == '\n' {
            if current_line == line && col_remaining == 0 {
                return Some(CharPosition::new(char_offset));
            }
            current_line += 1;
        }
        char_offset += 1;
    }

    if current_line == line && col_remaining == 0 {
        return Some(CharPosition::new(char_offset));
    }
    None
}

/// Maps a document scalar offset back to `(line, col)` using `\n` line breaks.
pub fn char_position_to_line_col(text: &str, position: CharPosition) -> Option<(usize, usize)> {
    if position.offset() > document_char_len(text) {
        return None;
    }

    let mut line = 0usize;
    let mut col = 0usize;
    let mut seen = 0u32;

    if position.offset() == 0 {
        return Some((0, 0));
    }

    for ch in text.chars() {
        if seen == position.offset() {
            return Some((line, col));
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
        seen += 1;
    }

    if seen == position.offset() {
        return Some((line, col));
    }
    None
}

/// Builds a scalar range from a line/column selection in the document.
pub fn line_col_range_to_char_range(
    text: &str,
    start: (usize, usize),
    end: (usize, usize),
) -> Option<TextRange> {
    let start_pos = line_col_to_char_position(text, start.0, start.1)?;
    let end_pos = line_col_to_char_position(text, end.0, end.1)?;
    let (lo, hi) = if start_pos.offset() <= end_pos.offset() {
        (start_pos, end_pos)
    } else {
        (end_pos, start_pos)
    };
    if lo.offset() == hi.offset() {
        return None;
    }
    Some(TextRange { start: lo, end: hi })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_byte_and_char_offsets_roundtrip() {
        let text = "hello world";
        for byte in (0..=text.len()).filter(|b| text.is_char_boundary(*b)) {
            let char_pos = byte_offset_to_char_position(text, byte).unwrap();
            let back = char_position_to_byte_offset(text, char_pos).unwrap();
            assert_eq!(back, byte);
        }
    }

    #[test]
    fn cjk_multibyte_offsets_roundtrip() {
        let text = "用户登录";
        assert!(byte_offset_to_char_position(text, 1).is_none());

        let pos = byte_offset_to_char_position(text, 0).unwrap();
        assert_eq!(pos.offset(), 0);
        let slice = slice_by_char_range(text, TextRange::new(1, 3)).unwrap();
        assert_eq!(slice, "户登");
    }

    #[test]
    fn emoji_counts_scalar_values() {
        let text = "a😀b";
        assert_eq!(document_char_len(text), 3);
        let range = TextRange::new(1, 2);
        assert_eq!(slice_by_char_range(text, range).unwrap(), "😀");
    }

    #[test]
    fn combining_characters_use_scalar_offsets() {
        let text = "e\u{0301}"; // e + combining acute
        assert_eq!(document_char_len(text), 2);
        assert_eq!(
            slice_by_char_range(text, TextRange::new(0, 1)).unwrap(),
            "e"
        );
    }

    #[test]
    fn line_col_roundtrip_across_boundaries() {
        let text = "line1\nline2\n😀end";
        let cases = [(0, 0), (0, 5), (1, 0), (1, 3), (2, 0), (2, 2), (2, 4)];
        for (line, col) in cases {
            let pos = line_col_to_char_position(text, line, col).unwrap();
            let back = char_position_to_line_col(text, pos).unwrap();
            assert_eq!(back, (line, col), "failed at ({line}, {col})");
        }
    }

    #[test]
    fn line_col_range_matches_editor_selection() {
        let text = "aaa\nbbb\nccc";
        let range = line_col_range_to_char_range(text, (0, 1), (2, 2)).unwrap();
        assert_eq!(slice_by_char_range(text, range).unwrap(), "aa\nbbb\ncc");
    }

    #[test]
    fn invalid_byte_boundary_returns_none() {
        let text = "用户";
        assert!(byte_offset_to_char_position(text, 1).is_none());
    }
}
