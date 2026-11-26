//! Pasted-text references: a long paste folds into one `[Pasted text #N +L lines]`
//! label in the draft and expands back to the exact text when the message is sent.
//!
//! A label is plain draft characters. Whether it means anything is decided by
//! [`DraftPastes`]: the entry must exist *and* the text must read exactly that
//! entry's label, so undo, cut and a paste back need no rules of their own, and
//! label-shaped text nobody registered stays plain text. Sent text never turns
//! back into a label, so the queue, prompt history and a recalled draft always
//! hold the real text.

use std::ops::Range;

/// A paste folds once it is longer than this many characters.
pub const FOLD_MIN_CHARS: usize = 800;

/// Whether a paste of `text` folds into one label instead of showing inline.
///
/// It folds past [`FOLD_MIN_CHARS`] or when it holds more visible lines than
/// the composer can spare: five on a normal terminal, fewer on a short one, so
/// the rest of the UI keeps ten rows. A trailing newline does not add a line.
pub fn folds(text: &str, screen_rows: u16) -> bool {
    let spare = screen_rows.saturating_sub(10).min(4);
    let newlines = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .matches('\n')
        .count();
    text.chars().count() > FOLD_MIN_CHARS || newlines > usize::from(spare)
}

/// The label a paste of `text` is written with: `[Pasted text #3 +12 lines]`,
/// or `[Pasted text #3]` when it holds a single line.
///
/// The label is written once, in the UI language of that moment, and kept with
/// the entry, so switching language later leaves earlier labels working.
pub fn label(number: usize, text: &str) -> String {
    let lines = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .matches('\n')
        .count();
    let template = crate::i18n::t(if lines == 0 {
        "draft.pasted_text"
    } else {
        "draft.pasted_text_lines"
    });
    let body = crate::i18n::fill(
        template,
        &[
            ("number", &crate::i18n::arg(&number)),
            ("lines", &crate::i18n::arg(&lines)),
        ],
    );
    format!("[{body}]")
}

/// One folded paste: the text it stands for and the label naming it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Paste {
    number: usize,
    text: String,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PasteEdit {
    /// `Composer::revision()` immediately after the edit was applied.
    revision: u64,
    /// Whether `after` is currently installed in `DraftPastes::pastes`.
    applied: bool,
    before: Paste,
    after: Paste,
}

/// Pastes folded into one draft, keyed by their label number.
///
/// Numbers only grow within one draft, so deleting `[Pasted text #1]` and
/// pasting again yields `#2` rather than reviving a stale number. An entry
/// whose label is no longer in the text matches nothing but stays attached:
/// undo and a paste back must still find what the label stood for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftPastes {
    pastes: Vec<Paste>,
    edits: Vec<PasteEdit>,
}

impl DraftPastes {
    /// Registers `text` and returns the label to insert for it.
    pub fn attach(&mut self, text: String) -> String {
        let number = self
            .pastes
            .iter()
            .map(|paste| paste.number)
            .max()
            .unwrap_or(0)
            + 1;
        let label = label(number, &text);
        self.pastes.push(Paste {
            number,
            text,
            label: label.clone(),
        });
        label
    }

    /// The text behind label `number`.
    pub fn text(&self, number: usize) -> Option<&str> {
        self.pastes
            .iter()
            .find(|paste| paste.number == number)
            .map(|paste| paste.text.as_str())
    }

    /// The label naming `number`.
    pub fn label(&self, number: usize) -> Option<&str> {
        self.pastes
            .iter()
            .find(|paste| paste.number == number)
            .map(|paste| paste.label.as_str())
    }

    /// Replaces the text behind `number` and records the transition at
    /// `revision_after` so `Composer` undo/redo restores both label and text.
    pub fn replace_at_revision(
        &mut self,
        number: usize,
        text: String,
        revision_after: u64,
    ) -> Option<String> {
        // A new edit invalidates any previously undone redo branch.
        self.edits.retain(|edit| edit.applied);
        let paste = self
            .pastes
            .iter_mut()
            .find(|paste| paste.number == number)?;
        let before = paste.clone();
        paste.text = text;
        paste.label = label(paste.number, &paste.text);
        let after = paste.clone();
        let new_label = after.label.clone();
        self.edits.push(PasteEdit {
            revision: revision_after,
            applied: true,
            before,
            after,
        });
        Some(new_label)
    }

    /// Reverts any paste edits recorded above `revision` after `Composer::undo()`.
    pub fn on_undo(&mut self, revision: u64) {
        for edit in self.edits.iter_mut().rev() {
            if edit.applied && revision < edit.revision {
                if let Some(paste) = self
                    .pastes
                    .iter_mut()
                    .find(|p| p.number == edit.before.number)
                {
                    *paste = edit.before.clone();
                }
                edit.applied = false;
            }
        }
    }

    /// Re-applies any undone paste edits up to `revision` after `Composer::redo()`.
    pub fn on_redo(&mut self, revision: u64) {
        for edit in &mut self.edits {
            if !edit.applied && revision >= edit.revision {
                if let Some(paste) = self
                    .pastes
                    .iter_mut()
                    .find(|p| p.number == edit.after.number)
                {
                    *paste = edit.after.clone();
                }
                edit.applied = true;
            }
        }
    }

    /// Drops undone paste edits once a new edit clears the composer redo stack.
    pub fn discard_redo(&mut self) {
        self.edits.retain(|edit| edit.applied);
    }

    /// Clears recorded undo/redo edit transitions when replacing the whole draft.
    pub fn clear_edits(&mut self) {
        self.edits.clear();
    }
}

/// Byte ranges and numbers of the live `[Pasted text #N ...]` labels in `text`,
/// in text order.
pub fn placeholders(text: &str, pastes: &DraftPastes) -> Vec<(Range<usize>, usize)> {
    let mut found: Vec<(Range<usize>, usize)> = Vec::new();
    for paste in &pastes.pastes {
        let mut from = 0;
        while let Some(offset) = text[from..].find(&paste.label) {
            let start = from + offset;
            found.push((start..start + paste.label.len(), paste.number));
            from = start + paste.label.len();
        }
    }
    found.sort_by_key(|(range, _)| (range.start, range.end));
    let mut labels: Vec<(Range<usize>, usize)> = Vec::new();
    for entry in found {
        if labels
            .last()
            .is_none_or(|(last, _)| entry.0.start >= last.end)
        {
            labels.push(entry);
        }
    }
    labels
}

/// `text` as it is sent: each live label replaced by the paste it stands for.
///
/// Nothing is added around the text. A label stood exactly where the paste
/// would have gone, so sending puts the same characters back.
#[cfg(test)]
pub fn expand(text: &str, pastes: &DraftPastes) -> String {
    let labels = placeholders(text, pastes);
    if labels.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, number) in labels {
        let Some(paste) = pastes.text(number) else {
            continue;
        };
        out.push_str(&text[last..range.start]);
        out.push_str(paste);
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folded(text: &str) -> (DraftPastes, String) {
        let mut pastes = DraftPastes::default();
        let label = pastes.attach(text.to_string());
        (pastes, label)
    }

    #[test]
    fn numbers_grow_within_one_draft_and_are_never_reused() {
        let (mut pastes, first) = folded("one\ntwo");
        let second = pastes.attach("three\nfour".to_string());

        assert_eq!(first, "[Pasted text #1 +1 lines]");
        assert_eq!(second, "[Pasted text #2 +1 lines]");
        assert_eq!(pastes.text(1), Some("one\ntwo"));
        assert_eq!(pastes.text(2), Some("three\nfour"));
    }

    #[test]
    fn a_paste_folds_past_800_characters_or_five_visible_lines() {
        assert!(!folds(&"x".repeat(FOLD_MIN_CHARS), 40));
        assert!(folds(&"x".repeat(FOLD_MIN_CHARS + 1), 40));
        assert!(!folds("1\n2\n3\n4\n5", 40));
        assert!(
            !folds("1\n2\n3\n4\n5\n", 40),
            "a trailing newline adds no line"
        );
        assert!(folds("1\n2\n3\n4\n5\n6", 40));
    }

    #[test]
    fn a_short_terminal_folds_sooner_but_keeps_ten_rows_for_the_rest() {
        assert!(!folds("1\n2", 40));
        assert!(!folds("1\n2", 11));
        assert!(folds("1\n2\n3", 11));
        assert!(folds("1\n2", 8));
        assert!(
            !folds("one line", 5),
            "a long single line still folds by length"
        );
    }

    #[test]
    fn a_single_line_paste_and_an_empty_one_name_no_line_count() {
        assert_eq!(
            label(3, &"x".repeat(FOLD_MIN_CHARS + 1)),
            "[Pasted text #3]"
        );
        assert_eq!(label(4, ""), "[Pasted text #4]");
    }

    #[test]
    fn each_label_sends_the_paste_it_stands_for_in_place() {
        let (pastes, label) = folded("one\ntwo");
        let text = format!("before {label} after");

        assert_eq!(expand(&text, &pastes), "before one\ntwo after");
    }

    #[test]
    fn a_label_named_twice_sends_its_paste_twice() {
        let (pastes, label) = folded("a\nb");
        let text = format!("{label} and {label}");

        assert_eq!(expand(&text, &pastes), "a\nb and a\nb");
    }

    #[test]
    fn text_without_live_labels_sends_unchanged() {
        let (pastes, label) = folded("one\ntwo");
        let unregistered = "[Pasted text #99 +3 lines]";

        assert_eq!(expand("plain text", &pastes), "plain text");
        assert_eq!(expand(unregistered, &pastes), unregistered);
        assert_eq!(
            expand("[Pasted text #1]", &pastes),
            "[Pasted text #1]",
            "the label of another line count is not this entry"
        );
        assert_eq!(
            expand(&label.to_uppercase(), &pastes),
            "[PASTED TEXT #1 +1 LINES]"
        );
    }

    #[test]
    fn placeholders_report_live_labels_in_text_order() {
        let (mut pastes, first) = folded("a\nb");
        let second = pastes.attach("c\nd".to_string());
        let text = format!("{second} x {first}");

        let found: Vec<(usize, &str)> = placeholders(&text, &pastes)
            .into_iter()
            .map(|(range, number)| (number, &text[range]))
            .collect();
        assert_eq!(found, [(2, second.as_str()), (1, first.as_str())]);
    }

    #[test]
    fn expansion_keeps_indentation_and_markdown_verbatim() {
        let original = "  - item\n\n```rust\nfn main() {}\n```\n";
        let (pastes, label) = folded(original);

        assert_eq!(expand(&label, &pastes), original);
        assert_eq!(pastes.text(1), Some(original));
    }
}
