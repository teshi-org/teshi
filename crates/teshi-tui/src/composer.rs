//! Editable AI composer model: text, caret, selection, undo history and
//! placeholder-aware editing.
//!
//! The model uses character indices throughout. `[Image #N]` placeholders are
//! registered as atomic character ranges so backspace, delete and horizontal
//! movement treat one attachment as one unit.
//!
//! The wrapped-row layout used by the renderer and vertical caret movement also
//! lives here, keeping the model independently testable.

use std::ops::{Deref, Range};

/// Largest number of undo snapshots retained.
const MAX_UNDO: usize = 200;
/// Rough byte budget for retained snapshots.
const MAX_UNDO_BYTES: usize = 4 * 1024 * 1024;

/// Whether an edit is a single unit in the undo history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditUnit {
    /// Consecutive typing coalesces into one undo entry.
    Typing,
    /// Paste, cut, attachment insertion or a programmatic replacement.
    Atomic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    revision: u64,
}

/// Text, caret, selection and undo state for one composer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composer {
    text: String,
    /// Character index, always within `0..=char_len`.
    cursor: usize,
    /// Selection anchor in character indices.
    anchor: Option<usize>,
    /// Atomic character ranges, usually image placeholders.
    atoms: Vec<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The next typing insert joins the newest undo entry.
    coalesce: bool,
    /// Monotonic revision of the current text state.
    revision: u64,
    /// Next fresh revision id allocated on non-coalesced edits or resets.
    next_revision: u64,
}

impl Deref for Composer {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl std::fmt::Display for Composer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl Composer {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Places the caret and clears the selection.
    pub fn set_cursor(&mut self, position: usize) {
        let position = position.min(self.char_len());
        self.cursor = self.unit_edge(position, position >= self.cursor);
        self.anchor = None;
        self.coalesce = false;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Monotonic revision of the current text state.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether a redo step is currently available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The selected character range, if the caret differs from the anchor.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| anchor.min(self.cursor)..anchor.max(self.cursor))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|range| self.slice(range))
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    pub fn set_atoms(&mut self, atoms: Vec<Range<usize>>) {
        self.atoms = atoms;
    }

    /// Replaces the draft and resets history, selection and atoms.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.char_len();
        self.anchor = None;
        self.atoms.clear();
        self.undo.clear();
        self.redo.clear();
        self.coalesce = false;
        self.next_revision = self.next_revision.saturating_add(1);
        self.revision = self.next_revision;
    }

    /// Takes the draft out of the composer and resets the model.
    pub fn take_text(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.anchor = None;
        self.atoms.clear();
        self.undo.clear();
        self.redo.clear();
        self.coalesce = false;
        self.next_revision = self.next_revision.saturating_add(1);
        self.revision = self.next_revision;
        text
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.char_len();
        self.coalesce = false;
    }

    /// Inserts one typed character. Full-width command/mention triggers are
    /// normalized at the positions where they start a token.
    pub fn insert_char(&mut self, ch: char) {
        let ch = self.normalize_trigger(ch);
        self.insert_inner(&ch.to_string(), EditUnit::Typing);
    }

    pub fn insert(&mut self, text: &str, unit: EditUnit) {
        if text.is_empty() {
            return;
        }
        self.insert_inner(text, unit);
    }

    fn insert_inner(&mut self, text: &str, unit: EditUnit) {
        self.record(unit);
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        self.replace_range(range.clone(), text);
        self.cursor = range.start + text.chars().count();
        self.anchor = None;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if let Some(atom) = self.atom_ending_at(self.cursor) {
            self.remove_chars(atom);
            return;
        }
        if self.cursor > 0 {
            self.remove_chars(self.cursor - 1..self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        if let Some(atom) = self.atom_starting_at(self.cursor) {
            self.remove_chars(atom);
            return;
        }
        if self.cursor < self.char_len() {
            self.remove_chars(self.cursor..self.cursor + 1);
        }
    }

    pub fn delete_word_back(&mut self) {
        if self.delete_selection() {
            return;
        }
        let start = self.previous_word_position(self.cursor);
        let range = self.expand_over_atoms(start..self.cursor);
        if !range.is_empty() {
            self.remove_chars(range);
        }
    }

    pub fn delete_word_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let end = self.next_word_position(self.cursor);
        let range = self.expand_over_atoms(self.cursor..end);
        if !range.is_empty() {
            self.remove_chars(range);
        }
    }

    pub fn delete_to_line_start(&mut self) {
        if self.delete_selection() {
            return;
        }
        let start = self.line_start(self.cursor);
        if start < self.cursor {
            self.remove_chars(start..self.cursor);
        }
    }

    /// Removes a character range as one undo unit.
    pub fn remove_chars(&mut self, range: Range<usize>) {
        self.replace_chars(range, "");
    }

    /// Replaces a character range with `text` as one atomic undo unit and moves
    /// the caret to the end of the replacement.
    pub fn replace_chars(&mut self, range: Range<usize>, text: &str) {
        let start = range.start.min(self.char_len());
        let end = range.end.min(self.char_len()).max(start);
        if start == end && text.is_empty() {
            return;
        }
        self.record(EditUnit::Atomic);
        self.replace_range(start..end, text);
        self.cursor = start + text.chars().count();
        self.anchor = None;
    }

    /// Replaces every one of `ranges` with `text` as one atomic undo unit,
    /// leaving the caret after the first replacement.
    ///
    /// Used when one label names several places that must keep changing
    /// together, such as a folded paste edited from its own editor.
    pub fn replace_ranges(&mut self, ranges: &[Range<usize>], text: &str) {
        let mut ranges = ranges.to_vec();
        ranges.sort_by_key(|range| range.start);
        ranges.dedup();
        let Some(first) = ranges.first() else {
            return;
        };
        self.record(EditUnit::Atomic);
        let caret = first.start + text.chars().count();
        // Last first, so every earlier range keeps its offsets.
        for range in ranges.iter().rev() {
            self.replace_range(range.clone(), text);
        }
        self.cursor = caret.min(self.char_len());
        self.anchor = None;
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            return false;
        };
        self.remove_chars(range);
        true
    }

    pub fn move_left(&mut self, select: bool) {
        if !select && self.collapse_selection(false) {
            return;
        }
        let target = self
            .atom_ending_at(self.cursor)
            .map(|atom| atom.start)
            .or_else(|| self.cursor.checked_sub(1))
            .unwrap_or(0);
        self.move_to(target, select);
    }

    pub fn move_right(&mut self, select: bool) {
        if !select && self.collapse_selection(true) {
            return;
        }
        let target = self
            .atom_starting_at(self.cursor)
            .map(|atom| atom.end)
            .unwrap_or_else(|| (self.cursor + 1).min(self.char_len()));
        self.move_to(target, select);
    }

    pub fn move_word_left(&mut self, select: bool) {
        if !select && self.collapse_selection(false) {
            return;
        }
        let target = self.previous_word_position(self.cursor);
        self.move_to(target, select);
    }

    pub fn move_word_right(&mut self, select: bool) {
        if !select && self.collapse_selection(true) {
            return;
        }
        let target = self.next_word_position(self.cursor);
        self.move_to(target, select);
    }

    pub fn move_home(&mut self, select: bool) {
        if !select && self.collapse_selection(false) {
            return;
        }
        let target = self.line_start(self.cursor);
        self.move_to(target, select);
    }

    pub fn move_end(&mut self, select: bool) {
        if !select && self.collapse_selection(true) {
            return;
        }
        let target = self.line_end(self.cursor);
        self.move_to(target, select);
    }

    /// Moves by one wrapped display row. Returns false when the caret is
    /// already on the first/last display row, so the caller may recall history.
    pub fn move_up(&mut self, select: bool, width: u16) -> bool {
        self.move_vertical(-1, select, width)
    }

    pub fn move_down(&mut self, select: bool, width: u16) -> bool {
        self.move_vertical(1, select, width)
    }

    fn move_vertical(&mut self, delta: i32, select: bool, width: u16) -> bool {
        if width == 0 {
            return self.move_logical_line(delta, select);
        }
        if !select && self.collapse_selection(delta > 0) {
            // Collapsing is a movement, but the caret may still be on the
            // first/last row; decide below using its new position.
        }
        let layout = layout(&self.text, width);
        let (row, column) = layout.position_of(self.cursor);
        let last = layout.last_row();
        if (delta < 0 && row == 0) || (delta > 0 && row >= last) {
            return false;
        }
        let target_row = if delta < 0 { row - 1 } else { row + 1 };
        let target = layout.char_at(target_row, column);
        self.move_to(target, select);
        true
    }

    fn move_logical_line(&mut self, delta: i32, select: bool) -> bool {
        let chars: Vec<char> = self.text.chars().collect();
        let cursor = self.cursor.min(chars.len());
        let start = chars[..cursor]
            .iter()
            .rposition(|ch| *ch == '\n')
            .map_or(0, |index| index + 1);
        let column = cursor - start;
        let target = if delta < 0 {
            if start == 0 {
                return false;
            }
            let end = start - 1;
            let previous = chars[..end]
                .iter()
                .rposition(|ch| *ch == '\n')
                .map_or(0, |index| index + 1);
            previous + column.min(end - previous)
        } else {
            let end = chars[cursor..]
                .iter()
                .position(|ch| *ch == '\n')
                .map_or(chars.len(), |index| cursor + index);
            if end == chars.len() {
                return false;
            }
            let next = end + 1;
            let length = chars[next..]
                .iter()
                .position(|ch| *ch == '\n')
                .unwrap_or(chars.len() - next);
            next + column.min(length)
        };
        self.move_to(target, select);
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            revision: self.revision,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor.min(self.char_len());
        self.anchor = None;
        self.coalesce = false;
        self.revision = snapshot.revision;
    }

    fn record(&mut self, unit: EditUnit) {
        if unit == EditUnit::Typing && self.coalesce {
            return;
        }
        self.undo.push(self.snapshot());
        self.redo.clear();
        self.next_revision = self.next_revision.saturating_add(1);
        self.revision = self.next_revision;
        self.coalesce = unit == EditUnit::Typing;
        while self.undo.len() > MAX_UNDO
            || self
                .undo
                .iter()
                .map(|snapshot| snapshot.text.len())
                .sum::<usize>()
                > MAX_UNDO_BYTES
        {
            self.undo.remove(0);
        }
    }

    fn replace_range(&mut self, range: Range<usize>, replacement: &str) {
        let start = self.byte_index(range.start);
        let end = self.byte_index(range.end);
        self.text.replace_range(start..end, replacement);
    }

    fn slice(&self, range: Range<usize>) -> String {
        let start = self.byte_index(range.start);
        let end = self.byte_index(range.end);
        self.text[start..end].to_string()
    }

    fn byte_index(&self, char_idx: usize) -> usize {
        let char_idx = char_idx.min(self.char_len());
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(index, _)| index)
    }

    fn move_to(&mut self, target: usize, select: bool) {
        if select && self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        } else if !select {
            self.anchor = None;
        }
        let target = target.min(self.char_len());
        // Vertical movement and a click land on the edge of an attachment or
        // folded paste, never inside one: a caret there would let one
        // backspace tear the placeholder apart.
        self.cursor = self.unit_edge(target, target >= self.cursor);
        self.coalesce = false;
        debug_assert!(self.cursor <= self.char_len());
    }

    /// `position`, or the edge of an atomic range containing it, in the
    /// direction of travel.
    fn unit_edge(&self, position: usize, forward: bool) -> usize {
        let mut index = position.min(self.char_len());
        loop {
            let previous = index;
            for atom in &self.atoms {
                if atom.start < index && index < atom.end {
                    index = if forward { atom.end } else { atom.start };
                }
            }
            if index == previous {
                return index.min(self.char_len());
            }
        }
    }

    /// Collapses a selection to its start (left/up) or end (right/down).
    fn collapse_selection(&mut self, to_end: bool) -> bool {
        let Some(range) = self.selection() else {
            return false;
        };
        self.cursor = if to_end { range.end } else { range.start };
        self.anchor = None;
        self.coalesce = false;
        true
    }

    fn atom_ending_at(&self, position: usize) -> Option<Range<usize>> {
        self.atoms.iter().find(|atom| atom.end == position).cloned()
    }

    fn atom_starting_at(&self, position: usize) -> Option<Range<usize>> {
        self.atoms
            .iter()
            .find(|atom| atom.start == position)
            .cloned()
    }

    fn expand_over_atoms(&self, range: Range<usize>) -> Range<usize> {
        let mut start = range.start;
        let mut end = range.end;
        for atom in &self.atoms {
            if atom.start < end && atom.end > start {
                start = start.min(atom.start);
                end = end.max(atom.end);
            }
        }
        start..end
    }

    fn line_start(&self, position: usize) -> usize {
        let chars: Vec<char> = self
            .text
            .chars()
            .take(position.min(self.char_len()))
            .collect();
        chars
            .iter()
            .rposition(|ch| *ch == '\n')
            .map_or(0, |index| index + 1)
    }

    fn line_end(&self, position: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let position = position.min(chars.len());
        chars[position..]
            .iter()
            .position(|ch| *ch == '\n')
            .map_or(chars.len(), |index| position + index)
    }

    fn previous_word_position(&self, position: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut index = position.min(chars.len());
        while index > 0 && chars[index - 1].is_whitespace() {
            index -= 1;
        }
        if index == 0 {
            return 0;
        }
        if let Some(atom) = self.atom_ending_at(index) {
            return atom.start;
        }
        if is_ascii_word(chars[index - 1]) {
            while index > 0 && is_ascii_word(chars[index - 1]) {
                index -= 1;
            }
        } else {
            index -= 1;
        }
        index
    }

    fn next_word_position(&self, position: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut index = position.min(chars.len());
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= chars.len() {
            return chars.len();
        }
        if let Some(atom) = self.atom_starting_at(index) {
            return atom.end;
        }
        if is_ascii_word(chars[index]) {
            while index < chars.len() && is_ascii_word(chars[index]) {
                index += 1;
            }
        } else {
            index += 1;
        }
        index
    }

    fn normalize_trigger(&self, ch: char) -> char {
        match ch {
            '／' | '＃' | '！' if self.cursor == 0 && self.selection().is_none() => match ch {
                '／' => '/',
                '＃' => '#',
                _ => '!',
            },
            '＠' if self.mention_boundary() => '@',
            _ => ch,
        }
    }

    fn mention_boundary(&self) -> bool {
        if self.cursor == 0 {
            return true;
        }
        self.text
            .chars()
            .nth(self.cursor - 1)
            .is_none_or(|previous| previous.is_whitespace() || is_cjk(previous))
    }
}

/// ASCII word characters. CJK and other non-ASCII characters are single units,
/// so a Chinese phrase never becomes one word.
fn is_ascii_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '\\')
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{3000}'..='\u{303F}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF00}'..='\u{FFEF}'
    )
}

/// Composer text wrapped into display rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub rows: Vec<String>,
    /// Character range rendered on each row, excluding the newline itself.
    pub row_ranges: Vec<Range<usize>>,
    /// `(row, column, cells)` of every character, in char order.
    pub cells: Vec<(u16, u16, u16)>,
    /// Position just past the last character.
    pub end: (u16, u16),
}

impl Layout {
    pub fn position_of(&self, char_idx: usize) -> (u16, u16) {
        self.cells
            .get(char_idx)
            .map_or(self.end, |&(row, column, _)| (row, column))
    }

    pub fn last_row(&self) -> u16 {
        self.rows.len().saturating_sub(1) as u16
    }

    /// Nearest character in `row` at or after `column`.
    pub fn char_at(&self, row: u16, column: u16) -> usize {
        let Some(range) = self.row_ranges.get(row as usize) else {
            return self.cells.len();
        };
        for index in range.clone() {
            if let Some(&(cell_row, cell_column, _)) = self.cells.get(index)
                && cell_row == row
                && cell_column >= column
            {
                return index;
            }
        }
        range.end
    }
}

/// Wrapped layout used by the composer renderer and vertical caret movement.
pub fn layout(input: &str, width: u16) -> Layout {
    let width = usize::from(width.max(1));
    let mut rows = vec![String::new()];
    let mut row_ranges = Vec::new();
    let mut row_start = 0usize;
    let mut column = 0;
    let mut positions = Vec::new();
    let char_count = input.chars().count();
    let row_index = |rows: &Vec<String>| (rows.len() - 1).min(usize::from(u16::MAX)) as u16;

    for (char_idx, ch) in input.chars().enumerate() {
        let cells = if ch == '\t' {
            4 - column % 4
        } else {
            unicode_width::UnicodeWidthChar::width(ch)
                .unwrap_or(0)
                .min(width)
        };
        if ch != '\n' && column + cells > width {
            row_ranges.push(row_start..char_idx);
            rows.push(String::new());
            row_start = char_idx;
            column = 0;
        }
        positions.push((row_index(&rows), column as u16, cells as u16));
        if ch == '\n' {
            row_ranges.push(row_start..char_idx);
            rows.push(String::new());
            row_start = char_idx + 1;
            column = 0;
        } else {
            if let Some(row) = rows.last_mut() {
                if ch == '\t' {
                    row.push_str(&" ".repeat(cells));
                } else if !ch.is_control() {
                    row.push(ch);
                }
            }
            column += cells;
            if column == width {
                row_ranges.push(row_start..char_idx.saturating_add(1));
                rows.push(String::new());
                row_start = char_idx + 1;
                column = 0;
            }
        }
    }
    row_ranges.push(row_start..char_count);
    if row_ranges.len() < rows.len() {
        row_ranges.resize(rows.len(), char_count..char_count);
    }
    Layout {
        end: (row_index(&rows), column as u16),
        rows,
        row_ranges,
        cells: positions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composer(text: &str) -> Composer {
        let mut composer = Composer::default();
        composer.set_text(text.to_string());
        composer
    }

    #[test]
    fn select_all_and_typing_replaces_then_undo_restores() {
        let mut input = composer("first\nsecond");
        input.select_all();
        input.insert_char('x');
        assert_eq!(input.text(), "x");
        assert_eq!(input.cursor(), 1);
        assert!(input.undo());
        assert_eq!(input.text(), "first\nsecond");
        assert_eq!(input.cursor(), "first\nsecond".chars().count());
    }

    #[test]
    fn selection_backspace_and_paste_replace_the_selection() {
        let mut input = composer("abcdef");
        input.select_all();
        input.backspace();
        assert_eq!(input.text(), "");

        let mut input = composer("abcdef");
        input.select_all();
        input.insert("XYZ", EditUnit::Atomic);
        assert_eq!(input.text(), "XYZ");
        assert!(input.undo());
        assert_eq!(input.text(), "abcdef");
    }

    #[test]
    fn shift_selection_extends_and_plain_move_collapses() {
        let mut input = composer("abcdef");
        input.move_home(false);
        input.move_right(true);
        input.move_right(true);
        assert_eq!(input.selected_text().as_deref(), Some("ab"));
        input.move_right(false);
        assert!(!input.has_selection());
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn typing_coalesces_but_atomic_edits_do_not() {
        let mut input = composer("");
        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        assert!(input.undo());
        assert_eq!(input.text(), "");
        assert!(!input.undo());

        let mut input = composer("");
        input.insert("a", EditUnit::Atomic);
        input.insert("b", EditUnit::Atomic);
        assert!(input.undo());
        assert_eq!(input.text(), "a");
        assert!(input.undo());
        assert_eq!(input.text(), "");
    }

    #[test]
    fn word_editing_keeps_chinese_as_single_units() {
        let mut input = composer("abc你好def");
        input.move_end(false);
        input.delete_word_back();
        assert_eq!(input.text(), "abc你好");
        input.delete_word_back();
        assert_eq!(input.text(), "abc你");
        input.delete_word_back();
        assert_eq!(input.text(), "abc");
        input.delete_word_back();
        assert_eq!(input.text(), "");
    }

    #[test]
    fn word_movement_skips_ascii_words_and_chinese_characters() {
        let mut input = composer("one two 你好");
        input.move_home(false);
        input.move_word_right(false);
        assert_eq!(input.cursor(), 3);
        input.move_word_right(false);
        assert_eq!(input.cursor(), 7);
        input.move_word_right(false);
        assert_eq!(input.cursor(), 9);
        input.move_word_right(false);
        assert_eq!(input.cursor(), 10);
        input.move_word_left(false);
        assert_eq!(input.cursor(), 9);
        input.move_word_left(false);
        assert_eq!(input.cursor(), 8);
    }

    #[test]
    fn delete_to_line_start_stops_at_newline() {
        let mut input = composer("first\nsecond");
        input.move_end(false);
        input.delete_to_line_start();
        assert_eq!(input.text(), "first\n");
    }

    #[test]
    fn atom_is_deleted_and_skipped_as_one_unit() {
        let text = "[Image #1] tail";
        let mut input = composer(text);
        let end = "[Image #1]".chars().count();
        input.set_atoms(std::iter::once(0..end).collect());
        input.move_home(false);
        input.move_right(false);
        assert_eq!(input.cursor(), end);
        input.move_left(false);
        assert_eq!(input.cursor(), 0);
        input.move_right(false);
        input.backspace();
        assert_eq!(input.text(), " tail");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn a_caret_never_rests_inside_an_atom() {
        let text = "[Image #1] tail";
        let mut input = composer(text);
        let end = "[Image #1]".chars().count();
        input.set_atoms(std::iter::once(0..end).collect());

        // Moving into a unit snaps to its far edge in the direction of travel.
        input.set_cursor(0);
        input.set_cursor(2);
        assert_eq!(input.cursor(), end);
        input.set_cursor(end - 1);
        assert_eq!(input.cursor(), 0);

        // A vertical move that would land mid-unit goes to its edge too.
        let mut wrapped = composer("[Paste #1]\nbottom");
        let unit = "[Paste #1]".chars().count();
        wrapped.set_atoms(std::iter::once(0..unit).collect());
        wrapped.move_end(false);
        wrapped.move_up(true, 12);
        assert!(
            wrapped.cursor() == 0 || wrapped.cursor() == unit,
            "cursor {} must sit on a unit edge",
            wrapped.cursor()
        );
    }

    #[test]
    fn undo_redo_keeps_cursor_inside_the_text() {
        let mut input = composer("hello");
        input.select_all();
        input.backspace();
        assert!(input.undo());
        assert_eq!(input.text(), "hello");
        assert!(input.redo());
        assert_eq!(input.text(), "");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn moving_breaks_typing_coalescing() {
        let mut input = composer("");
        input.insert_char('a');
        input.insert_char('b');
        input.move_left(false);
        input.insert_char('c');
        assert_eq!(input.text(), "acb");
        assert!(input.undo());
        assert_eq!(input.text(), "ab");
        assert!(input.undo());
        assert_eq!(input.text(), "");
    }

    #[test]
    fn vertical_movement_uses_wrapped_rows() {
        let mut input = composer("abcdefgh");
        input.move_end(false);
        assert!(input.move_up(false, 4));
        assert_eq!(input.cursor(), 4);
        assert!(input.move_up(false, 4));
        assert_eq!(input.cursor(), 0);
        assert!(!input.move_up(false, 4));
        assert!(input.move_down(false, 4));
        assert_eq!(input.cursor(), 4);
        assert!(input.move_down(false, 4));
        assert_eq!(input.cursor(), 8);
        assert!(!input.move_down(false, 4));
    }

    #[test]
    fn full_width_triggers_normalize_only_at_token_start() {
        let mut input = composer("");
        input.insert_char('／');
        assert_eq!(input.text(), "/");

        let mut input = composer("");
        input.insert_char('＃');
        assert_eq!(input.text(), "#");

        let mut input = composer("");
        input.insert_char('！');
        assert_eq!(input.text(), "!");

        let mut input = composer("hello");
        input.move_end(false);
        input.insert_char('！');
        assert_eq!(input.text(), "hello！");

        let mut input = composer("中文");
        input.move_end(false);
        input.insert_char('＠');
        assert_eq!(input.text(), "中文@");
    }

    #[test]
    fn layout_tracks_row_ranges_for_wrapped_text() {
        let layout = layout("abcdefgh", 4);
        assert_eq!(layout.rows, ["abcd", "efgh", ""]);
        assert_eq!(layout.row_ranges[0], 0..4);
        assert_eq!(layout.row_ranges[1], 4..8);
        assert_eq!(layout.char_at(0, 2), 2);
        assert_eq!(layout.char_at(1, 2), 6);
    }
}
