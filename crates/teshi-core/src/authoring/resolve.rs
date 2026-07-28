//! Anchor creation and deterministic re-resolution for requirement links.

use crate::authoring::positions::{document_char_len, slice_by_char_range};
use crate::authoring::testpoints::{ReviewState, TestPoint};
use crate::authoring::{CharPosition, QuoteSelector, RequirementLink, ResolutionState, TextRange};

/// Maximum prefix/suffix context length stored with each anchor.
pub const ANCHOR_CONTEXT_CHARS: usize = 32;

/// Creates a requirement link from a non-empty scalar range in `text`.
///
/// # Errors
///
/// Returns `None` when the range is empty or invalid for `text`.
pub fn create_requirement_link(
    document_id: impl Into<String>,
    document_revision: impl Into<String>,
    text: &str,
    range: TextRange,
) -> Option<RequirementLink> {
    let quote = slice_by_char_range(text, range)?;
    if quote.trim().is_empty() {
        return None;
    }

    let prefix = context_before(text, range.start, ANCHOR_CONTEXT_CHARS);
    let suffix = context_after(text, range.end, ANCHOR_CONTEXT_CHARS);

    Some(RequirementLink {
        document_id: document_id.into(),
        document_revision: document_revision.into(),
        position: range,
        quote: QuoteSelector {
            quote,
            prefix,
            suffix,
        },
        resolution: ResolutionState::Resolved,
    })
}

/// Re-resolves a link against the current document body and revision.
///
/// Resolution order:
/// 1. Same revision + position still matches quote → resolved at stored position
/// 2. Unique exact quote match → resolved at new position
/// 3. Multiple quote matches disambiguated by prefix/suffix → resolved
/// 4. Otherwise → stale
pub fn resolve_requirement_link(
    text: &str,
    current_revision: &str,
    link: &RequirementLink,
) -> RequirementLink {
    let mut resolved = link.clone();

    if link.document_revision == current_revision
        && position_matches_quote(text, link.position, &link.quote.quote)
    {
        resolved.resolution = ResolutionState::Resolved;
        return resolved;
    }

    let matches = find_quote_matches(text, &link.quote.quote);
    let unique = match matches.len() {
        0 => None,
        1 => Some(matches[0]),
        _ => disambiguate_matches(text, &matches, &link.quote),
    };

    if let Some(range) = unique {
        resolved.position = range;
        resolved.document_revision = current_revision.to_string();
        resolved.resolution = ResolutionState::Resolved;
    } else {
        resolved.resolution = ResolutionState::Stale;
    }

    resolved
}

/// Re-resolves every link on test points that reference `document_id`.
///
/// When a previously approved test point acquires a stale link, its review state
/// becomes `NeedsReview`. Unrelated approvals are preserved.
pub fn re_resolve_document_links(
    text: &str,
    document_id: &str,
    current_revision: &str,
    test_points: &mut [TestPoint],
) {
    for tp in test_points.iter_mut() {
        let mut any_stale = false;
        let mut any_changed = false;

        for link in tp.requirement_links.iter_mut() {
            if link.document_id != document_id {
                continue;
            }
            let before = (link.position, link.resolution);
            *link = resolve_requirement_link(text, current_revision, link);
            if (link.position, link.resolution) != before {
                any_changed = true;
            }
            if link.resolution == ResolutionState::Stale {
                any_stale = true;
            }
        }

        if any_stale && tp.review_state == ReviewState::Approved {
            tp.review_state = ReviewState::NeedsReview;
        } else if any_changed && tp.review_state == ReviewState::Approved && !any_stale {
            // Quote unchanged but position moved — approval remains valid per spec.
        }
    }
}

fn position_matches_quote(text: &str, position: TextRange, quote: &str) -> bool {
    slice_by_char_range(text, position).as_deref() == Some(quote)
}

fn find_quote_matches(text: &str, quote: &str) -> Vec<TextRange> {
    if quote.is_empty() {
        return Vec::new();
    }

    let quote_len = quote.chars().count() as u32;
    let mut matches = Vec::new();
    let mut search_from = CharPosition::new(0);

    while let Some(start) = find_substring_at(text, quote, search_from) {
        let end = CharPosition::new(start.offset() + quote_len);
        matches.push(TextRange { start, end });
        if end.offset() >= document_char_len(text) {
            break;
        }
        search_from = CharPosition::new(start.offset() + 1);
    }
    matches
}

fn find_substring_at(text: &str, needle: &str, from: CharPosition) -> Option<CharPosition> {
    let start_byte = crate::authoring::positions::char_position_to_byte_offset(text, from)?;
    let relative = text[start_byte..].find(needle)?;
    let prefix_chars = text[start_byte..start_byte + relative].chars().count() as u32;
    Some(CharPosition::new(from.offset() + prefix_chars))
}

fn disambiguate_matches(
    text: &str,
    matches: &[TextRange],
    quote: &QuoteSelector,
) -> Option<TextRange> {
    let candidates: Vec<TextRange> = matches
        .iter()
        .copied()
        .filter(|range| context_matches(text, *range, quote))
        .collect();

    if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    }
}

fn context_matches(text: &str, range: TextRange, quote: &QuoteSelector) -> bool {
    let prefix = context_before(text, range.start, quote.prefix.chars().count());
    let suffix = context_after(text, range.end, quote.suffix.chars().count());
    prefix == quote.prefix && suffix == quote.suffix
}

fn context_before(text: &str, start: CharPosition, max_chars: usize) -> String {
    if max_chars == 0 || start.offset() == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().take(start.offset() as usize).collect();
    let from = chars.len().saturating_sub(max_chars);
    chars[from..].iter().collect()
}

fn context_after(text: &str, end: CharPosition, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    text.chars()
        .skip(end.offset() as usize)
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::testpoints::HierarchyPath;

    fn link_at(text: &str, start: u32, end: u32) -> RequirementLink {
        create_requirement_link("doc-1", "rev-1", text, TextRange::new(start, end)).unwrap()
    }

    #[test]
    fn create_rejects_empty_selection() {
        let text = "hello";
        assert!(create_requirement_link("doc-1", "rev-1", text, TextRange::new(2, 2)).is_none());
        assert!(create_requirement_link("doc-1", "rev-1", text, TextRange::new(2, 2)).is_none());
    }

    #[test]
    fn resolve_by_position_when_revision_matches() {
        let text = "User can log in";
        let link = link_at(text, 0, 4);
        let resolved = resolve_requirement_link(text, "rev-1", &link);
        assert_eq!(resolved.resolution, ResolutionState::Resolved);
        assert_eq!(resolved.position.start.offset(), 0);
    }

    #[test]
    fn resolve_relocates_unchanged_quote_after_move() {
        let original = "prefix login suffix";
        let link = link_at(original, 7, 12);
        let edited = "login prefix suffix";
        let resolved = resolve_requirement_link(edited, "rev-2", &link);
        assert_eq!(resolved.resolution, ResolutionState::Resolved);
        assert_eq!(
            slice_by_char_range(edited, resolved.position).as_deref(),
            Some("login")
        );
    }

    #[test]
    fn duplicate_quotes_without_context_become_stale() {
        let text = "foo bar foo";
        let mut link = link_at(text, 0, 3);
        link.quote.prefix.clear();
        link.quote.suffix.clear();
        let resolved = resolve_requirement_link(text, "rev-2", &link);
        assert_eq!(resolved.resolution, ResolutionState::Stale);
    }

    #[test]
    fn duplicate_quotes_disambiguated_by_context() {
        let text = "alpha foo beta foo gamma";
        let range = TextRange::new(6, 9);
        let mut link = create_requirement_link("doc-1", "rev-1", text, range).unwrap();
        link.quote.prefix = "alpha ".into();
        link.quote.suffix = " beta".into();

        let edited = "beta foo gamma alpha foo beta";
        let resolved = resolve_requirement_link(edited, "rev-2", &link);
        assert_eq!(resolved.resolution, ResolutionState::Resolved);
        assert_eq!(
            slice_by_char_range(edited, resolved.position).as_deref(),
            Some("foo")
        );
    }

    #[test]
    fn deleted_text_becomes_stale() {
        let text = "remove me";
        let link = link_at(text, 0, 6);
        let edited = "keep";
        let resolved = resolve_requirement_link(edited, "rev-2", &link);
        assert_eq!(resolved.resolution, ResolutionState::Stale);
    }

    #[test]
    fn approved_test_point_moves_to_needs_review_on_stale_link() {
        let text = "auth login flow";
        let mut tp = TestPoint {
            id: "tp-1".into(),
            title: "Login".into(),
            objective: "Verify login".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
            review_state: ReviewState::Approved,
            requirement_links: vec![link_at(text, 5, 10)],
            scenario_refs: Vec::new(),
        };

        let edited = "auth flow";
        re_resolve_document_links(edited, "doc-1", "rev-2", std::slice::from_mut(&mut tp));
        assert_eq!(tp.review_state, ReviewState::NeedsReview);
        assert_eq!(tp.requirement_links[0].resolution, ResolutionState::Stale);
    }

    #[test]
    fn unrelated_edit_keeps_approval_when_quote_still_unique() {
        let text = "login is required for users";
        let mut tp = TestPoint {
            id: "tp-1".into(),
            title: "Login".into(),
            objective: "Verify login".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
            review_state: ReviewState::Approved,
            requirement_links: vec![link_at(text, 0, 5)],
            scenario_refs: Vec::new(),
        };

        let edited = "login is required for admins";
        re_resolve_document_links(edited, "doc-1", "rev-2", std::slice::from_mut(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Approved);
        assert_eq!(
            tp.requirement_links[0].resolution,
            ResolutionState::Resolved
        );
    }
}
