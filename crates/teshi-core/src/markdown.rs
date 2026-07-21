//! Pure Markdown text-processing utilities.
//!
//! Block-parsing and rendering logic that depends on UI types (ratatui) lives
//! in the TUI crate.

/// Strips `<think>...</think>` blocks (with content inside) from reasoning
/// model output.
pub fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_think = false;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        if bytes[i..].starts_with(b"<think>") {
            in_think = true;
            i += 7;
        } else if in_think && bytes[i..].starts_with(b"</think>") {
            in_think = false;
            i += 8;
        } else if !in_think {
            result.push(text[i..].chars().next().unwrap());
            i += text[i..].chars().next().unwrap().len_utf8();
        } else {
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_think_tags_removes_single_block() {
        let input = "Hello <think>internal reasoning</think> world";
        let result = strip_think_tags(input);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn test_strip_think_tags_no_tags() {
        let input = "Plain text without any tags";
        let result = strip_think_tags(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_strip_think_tags_nested_handled_as_single() {
        // Nested <think> tags: the first <think> opens, first </think> closes.
        // Remaining "tail</think>" is outside the block and rendered literally.
        let input = "A <think>outer <think>inner</think> tail</think> B";
        let result = strip_think_tags(input);
        assert_eq!(result, "A  tail</think> B");
    }
}
