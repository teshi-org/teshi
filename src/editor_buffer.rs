use ropey::Rope;

#[derive(Debug, Clone)]
pub struct EditorBuffer {
    rope: Rope,
}

impl EditorBuffer {
    pub fn from_string(content: String) -> Self {
        Self {
            rope: Rope::from_str(&content),
        }
    }

    pub fn as_string(&self) -> String {
        self.rope.to_string()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines().max(1)
    }

    pub fn line(&self, row: usize) -> String {
        if row >= self.line_count() {
            return String::new();
        }
        let mut line = self.rope.line(row).to_string();
        if line.ends_with('\n') {
            line.pop();
        }
        line
    }

    pub fn line_len_chars(&self, row: usize) -> usize {
        self.line(row).chars().count()
    }

    pub fn clamp_col(&self, row: usize, col: usize) -> usize {
        col.min(self.line_len_chars(row))
    }

    pub fn insert_char(&mut self, row: usize, col: usize, ch: char) {
        let idx = self.line_col_to_char_idx(row, col);
        self.rope.insert_char(idx, ch);
    }

    pub fn backspace(&mut self, row: usize, col: usize) -> (usize, usize, bool) {
        if row == 0 && col == 0 {
            return (0, 0, false);
        }

        if col > 0 {
            let idx = self.line_col_to_char_idx(row, col);
            self.rope.remove(idx - 1..idx);
            return (row, col - 1, true);
        }

        let current_line_start = self.rope.line_to_char(row);
        let prev_row = row - 1;
        let prev_col = self.line_len_chars(prev_row);
        self.rope.remove(current_line_start - 1..current_line_start);
        (prev_row, prev_col, true)
    }

    pub fn delete(&mut self, row: usize, col: usize) -> bool {
        let idx = self.line_col_to_char_idx(row, col);
        if idx < self.rope.len_chars() {
            self.rope.remove(idx..idx + 1);
            return true;
        }
        false
    }

    fn line_col_to_char_idx(&self, row: usize, col: usize) -> usize {
        let safe_row = row.min(self.line_count() - 1);
        let line_start = self.rope.line_to_char(safe_row);
        let safe_col = self.clamp_col(safe_row, col);
        line_start + safe_col
    }
}

#[cfg(test)]
mod tests {
    use super::EditorBuffer;

    #[test]
    fn test_insert_delete_and_newline() {
        let mut buffer = EditorBuffer::from_string("Feature: x".to_string());
        buffer.insert_char(0, 8, '\n');
        buffer.insert_char(1, 0, 'A');
        assert_eq!(buffer.line(0), "Feature:");
        assert_eq!(buffer.line(1), "A x");
        assert!(buffer.delete(1, 0));
        assert_eq!(buffer.line(1), " x");
    }

    #[test]
    fn test_backspace_merges_lines() {
        let mut buffer = EditorBuffer::from_string("a\nb".to_string());
        let (row, col, changed) = buffer.backspace(1, 0);
        assert!(changed);
        assert_eq!((row, col), (0, 1));
        assert_eq!(buffer.as_string(), "ab");
    }
}
