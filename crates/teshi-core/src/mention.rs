//! Headless `@` mention popup state machine for AI composer inputs.
//!
//! Tracks `@` triggers at line start, after whitespace, or after CJK text,
//! supports the four core context sources (`@last-fail`, `@issue`, `@tp`,
//! `@feature`) with simple case-insensitive substring filtering, and guards
//! keyboard confirmation while an IME composition session is active (so Web
//! and desktop Chinese IME `Enter` commits never trigger unintended selections).

use std::ops::Range;

/// The four core context sources supported by the `@` mention popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MentionSource {
    /// Most recent failed run/scenario recorded in `Explore`.
    LastFail,
    /// Document from the `Issues` store.
    Issue,
    /// Test point from `Test Points`.
    TestPoint,
    /// `.feature` file from `Explore`.
    Feature,
}

impl MentionSource {
    /// Canonical trigger prefix (e.g. `"@last-fail"`, `"@issue"`).
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::LastFail => "@last-fail",
            Self::Issue => "@issue",
            Self::TestPoint => "@tp",
            Self::Feature => "@feature",
        }
    }

    /// Trigger token without the leading `@`.
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::LastFail => "last-fail",
            Self::Issue => "issue",
            Self::TestPoint => "tp",
            Self::Feature => "feature",
        }
    }

    /// Canonical display order for the 4 core mention sources.
    pub const ALL: [Self; 4] = [Self::LastFail, Self::Issue, Self::TestPoint, Self::Feature];

    fn from_scope_token(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "last-fail" => Some(Self::LastFail),
            "issue" => Some(Self::Issue),
            "tp" => Some(Self::TestPoint),
            "feature" => Some(Self::Feature),
            _ => None,
        }
    }
}

/// One selectable context candidate supplied by the frontend host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionCandidate {
    /// Stable identifier used to resolve and mount the backing context capsule.
    pub id: String,
    /// Source category of this candidate.
    pub source: MentionSource,
    /// Concise display title shown in the popup row.
    pub label: String,
    /// Optional muted secondary metadata (e.g. path, issue tag, error summary).
    pub detail: Option<String>,
    /// Extra text included in simple substring matching (e.g. id or relative path).
    pub search_text: String,
}

impl MentionCandidate {
    /// Returns `true` when the lowercase substring `needle` appears in the
    /// candidate's label, detail, search text, or source keyword.
    pub fn matches_substring(&self, needle_lower: &str) -> bool {
        if needle_lower.is_empty() {
            return true;
        }
        self.label.to_lowercase().contains(needle_lower)
            || self
                .detail
                .as_deref()
                .is_some_and(|d| d.to_lowercase().contains(needle_lower))
            || self.search_text.to_lowercase().contains(needle_lower)
            || self.source.keyword().contains(needle_lower)
    }
}

/// One visible row rendered inside the `@` mention popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionItem {
    /// Source shortcut row (e.g. `@issue`, `@tp`, `@feature`, or empty `@last-fail`).
    Category {
        source: MentionSource,
        count: usize,
        preview: Option<String>,
    },
    /// Concrete selectable item that mounts directly into the context capsule bar.
    Candidate(MentionCandidate),
}

/// Active `@...` trigger token parsed from the composer around the caret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionTrigger {
    /// Character range in the composer covering `@` through the caret.
    pub range: Range<usize>,
    /// Raw query text after `@` up to the caret.
    pub raw_query: String,
    /// Explicit source scope when the user typed `<source>:` or an exact source keyword.
    pub scoped_source: Option<MentionSource>,
    /// Lowercase substring filter applied to candidate items.
    pub filter: String,
}

/// High-level navigation/action keys handled by [`MentionPopupState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentionKey {
    Prev,
    Next,
    Confirm,
    Dismiss,
}

/// Result produced when handling a key or pointer confirmation on [`MentionPopupState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionOutcome {
    /// Popup is closed, IME composition is active, or nothing matched.
    Ignored,
    /// Highlighted row changed.
    Navigated,
    /// Popup was dismissed via `Esc`.
    Dismissed,
    /// User chose a source category shortcut (`@issue` -> `@issue:`) to drill down.
    ExpandScope {
        range: Range<usize>,
        replacement: String,
    },
    /// User confirmed a concrete candidate; host should remove `range` and mount the capsule.
    Selected {
        range: Range<usize>,
        candidate: MentionCandidate,
    },
    /// User confirmed a source that currently has no available entries (e.g. `@last-fail` with no failure).
    EmptySource(MentionSource),
}

/// Headless state machine for the AI composer's `@` context-mention popup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionPopupState {
    active_trigger: Option<MentionTrigger>,
    selection: usize,
    ime_composing: bool,
    /// Start char index of an `@` trigger explicitly dismissed with `Esc`.
    dismissed_at: Option<usize>,
}

impl MentionPopupState {
    /// Creates a closed mention popup state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when an `@` trigger is active and the popup should be shown.
    pub fn is_open(&self) -> bool {
        self.active_trigger.is_some()
    }

    /// Returns `true` when keyboard navigation/confirmation should be intercepted.
    ///
    /// Returns `false` while an IME composition session is active so pressing
    /// `Enter` to commit CJK candidates never misfires popup confirmation.
    pub fn accepts_keyboard(&self) -> bool {
        self.is_open() && !self.ime_composing
    }

    /// Returns whether an IME composition session is currently active.
    pub fn is_ime_composing(&self) -> bool {
        self.ime_composing
    }

    /// Updates the IME composition flag (e.g. from Web `compositionstart`/`compositionend`
    /// or `KeyboardEvent.isComposing`).
    pub fn set_ime_composing(&mut self, composing: bool) {
        self.ime_composing = composing;
    }

    /// Currently active trigger metadata, if the popup is open.
    pub fn active_trigger(&self) -> Option<&MentionTrigger> {
        self.active_trigger.as_ref()
    }

    /// Character range of the active `@...` trigger in the composer, if open.
    pub fn active_range(&self) -> Option<Range<usize>> {
        self.active_trigger.as_ref().map(|t| t.range.clone())
    }

    /// Current highlighted row index (clamped against visible items on render/confirm).
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// Dismisses the popup for the current `@` trigger without editing the draft.
    pub fn dismiss(&mut self) {
        if let Some(trigger) = self.active_trigger.take() {
            self.dismissed_at = Some(trigger.range.start);
        }
        self.selection = 0;
    }

    /// Fully resets the popup state (e.g. on message send, blur, or tab switch).
    pub fn reset(&mut self) {
        self.active_trigger = None;
        self.selection = 0;
        self.dismissed_at = None;
    }

    /// Re-evaluates whether the caret sits inside an `@` trigger token.
    ///
    /// Call this after every text edit or cursor movement. When `ime_composing`
    /// is true, the popup remains synchronized without resetting composition state.
    pub fn sync_input(&mut self, text: &str, cursor_char_idx: usize) {
        let Some(trigger) = detect_mention_trigger(text, cursor_char_idx) else {
            self.active_trigger = None;
            self.selection = 0;
            self.dismissed_at = None;
            return;
        };

        if self.dismissed_at == Some(trigger.range.start) {
            self.active_trigger = None;
            self.selection = 0;
            return;
        }
        self.dismissed_at = None;

        let query_changed = self.active_trigger.as_ref().is_none_or(|prev| {
            prev.raw_query != trigger.raw_query || prev.range.start != trigger.range.start
        });
        if query_changed {
            self.selection = 0;
        }
        self.active_trigger = Some(trigger);
    }

    /// Computes the visible rows for the active trigger using simple substring filtering.
    pub fn visible_items(&self, candidates: &[MentionCandidate]) -> Vec<MentionItem> {
        let Some(trigger) = self.active_trigger.as_ref() else {
            return Vec::new();
        };

        // Explicit scope (`@issue:...`, `@tp:...`, `@feature:...`, `@last-fail:...`, or exact `@issue`/`@tp`/`@feature`).
        if let Some(source) = trigger.scoped_source {
            let matching: Vec<MentionItem> = candidates
                .iter()
                .filter(|c| c.source == source && c.matches_substring(&trigger.filter))
                .cloned()
                .map(MentionItem::Candidate)
                .collect();
            if matching.is_empty() && source == MentionSource::LastFail && trigger.filter.is_empty()
            {
                return vec![MentionItem::Category {
                    source: MentionSource::LastFail,
                    count: 0,
                    preview: None,
                }];
            }
            return matching;
        }

        // Empty query (`@`): present the 4 core sources in canonical order.
        if trigger.raw_query.is_empty() {
            return MentionSource::ALL
                .into_iter()
                .map(|source| category_or_last_fail_item(source, candidates))
                .collect();
        }

        let needle = trigger.raw_query.to_lowercase();
        let mut items = Vec::new();
        let mut included_ids = Vec::new();

        // First include any matching source shortcut categories (e.g. `@la` -> `@last-fail`, `@is` -> `@issue`).
        for source in MentionSource::ALL {
            if source.keyword().contains(&needle) {
                let item = category_or_last_fail_item(source, candidates);
                if let MentionItem::Candidate(ref c) = item {
                    included_ids.push(c.id.clone());
                }
                items.push(item);
            }
        }

        // Then append concrete candidates matching the substring across all 4 sources.
        for candidate in candidates {
            if included_ids.iter().any(|id| id == &candidate.id) {
                continue;
            }
            if candidate.matches_substring(&needle) {
                included_ids.push(candidate.id.clone());
                items.push(MentionItem::Candidate(candidate.clone()));
            }
        }

        items
    }

    /// Handles a keyboard navigation or confirmation event.
    ///
    /// Always returns [`MentionOutcome::Ignored`] when the popup is closed or
    /// when `ime_composing` is true.
    pub fn handle_key(
        &mut self,
        key: MentionKey,
        candidates: &[MentionCandidate],
    ) -> MentionOutcome {
        if !self.accepts_keyboard() {
            return MentionOutcome::Ignored;
        }
        match key {
            MentionKey::Dismiss => {
                self.dismiss();
                MentionOutcome::Dismissed
            }
            MentionKey::Prev => {
                let items = self.visible_items(candidates);
                if items.is_empty() {
                    self.selection = 0;
                    return MentionOutcome::Ignored;
                }
                let clamped = self.selection.min(items.len() - 1);
                self.selection = clamped.saturating_sub(1);
                MentionOutcome::Navigated
            }
            MentionKey::Next => {
                let items = self.visible_items(candidates);
                if items.is_empty() {
                    self.selection = 0;
                    return MentionOutcome::Ignored;
                }
                let max = items.len() - 1;
                self.selection = (self.selection + 1).min(max);
                MentionOutcome::Navigated
            }
            MentionKey::Confirm => {
                let items = self.visible_items(candidates);
                if items.is_empty() {
                    return MentionOutcome::Ignored;
                }
                let index = self.selection.min(items.len() - 1);
                self.confirm_item(items.into_iter().nth(index))
            }
        }
    }

    /// Confirms a specific visible row index (used by mouse/pointer clicks).
    pub fn choose_index(
        &mut self,
        index: usize,
        candidates: &[MentionCandidate],
    ) -> MentionOutcome {
        if !self.accepts_keyboard() {
            return MentionOutcome::Ignored;
        }
        let items = self.visible_items(candidates);
        if index >= items.len() {
            return MentionOutcome::Ignored;
        }
        self.selection = index;
        self.confirm_item(items.into_iter().nth(index))
    }

    fn confirm_item(&mut self, item: Option<MentionItem>) -> MentionOutcome {
        let Some(trigger) = self.active_trigger.clone() else {
            return MentionOutcome::Ignored;
        };
        match item {
            Some(MentionItem::Candidate(candidate)) => {
                self.reset();
                MentionOutcome::Selected {
                    range: trigger.range,
                    candidate,
                }
            }
            Some(MentionItem::Category {
                source: MentionSource::LastFail,
                ..
            }) => {
                self.dismiss();
                MentionOutcome::EmptySource(MentionSource::LastFail)
            }
            Some(MentionItem::Category { source, count, .. }) => {
                if count == 0 {
                    self.dismiss();
                    return MentionOutcome::EmptySource(source);
                }
                self.selection = 0;
                MentionOutcome::ExpandScope {
                    range: trigger.range,
                    replacement: format!("{}:", source.prefix()),
                }
            }
            None => MentionOutcome::Ignored,
        }
    }
}

fn category_or_last_fail_item(
    source: MentionSource,
    candidates: &[MentionCandidate],
) -> MentionItem {
    if source == MentionSource::LastFail {
        if let Some(last_fail) = candidates
            .iter()
            .find(|c| c.source == MentionSource::LastFail)
        {
            return MentionItem::Candidate(last_fail.clone());
        }
        return MentionItem::Category {
            source: MentionSource::LastFail,
            count: 0,
            preview: None,
        };
    }
    let mut count = 0usize;
    let mut preview = None;
    for candidate in candidates.iter().filter(|c| c.source == source) {
        if preview.is_none() {
            preview = Some(candidate.label.clone());
        }
        count += 1;
    }
    MentionItem::Category {
        source,
        count,
        preview,
    }
}

/// Detects an active `@` mention trigger at `cursor_char_idx`.
///
/// Triggers only when `@` (or full-width `＠`) sits at the start of the text,
/// immediately after whitespace (including line start `\n`), or immediately
/// after a CJK character. Emails like `user@example.com` do not trigger.
pub fn detect_mention_trigger(text: &str, cursor_char_idx: usize) -> Option<MentionTrigger> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor_char_idx.min(chars.len());
    if cursor == 0 {
        return None;
    }

    let mut idx = cursor;
    while idx > 0 {
        idx -= 1;
        let ch = chars[idx];
        if ch.is_whitespace() {
            return None;
        }
        if matches!(ch, '@' | '＠') {
            let valid_boundary =
                idx == 0 || chars[idx - 1].is_whitespace() || is_cjk(chars[idx - 1]);
            if !valid_boundary {
                return None;
            }
            let raw_query: String = chars[idx + 1..cursor].iter().collect();
            let (scoped_source, filter) = parse_scope_and_filter(&raw_query);
            return Some(MentionTrigger {
                range: idx..cursor,
                raw_query,
                scoped_source,
                filter,
            });
        }
    }
    None
}

fn parse_scope_and_filter(raw_query: &str) -> (Option<MentionSource>, String) {
    if let Some((prefix, rest)) = raw_query.split_once(':')
        && let Some(source) = MentionSource::from_scope_token(prefix)
    {
        return (Some(source), rest.to_lowercase());
    }
    if let Some(source) = MentionSource::from_scope_token(raw_query) {
        return (Some(source), String::new());
    }
    (None, raw_query.to_lowercase())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_candidates() -> Vec<MentionCandidate> {
        vec![
            MentionCandidate {
                id: "last-fail:0:0".into(),
                source: MentionSource::LastFail,
                label: "导出超时".into(),
                detail: Some("TimeoutError: 30s".into()),
                search_text: "features/export.feature".into(),
            },
            MentionCandidate {
                id: "issue:doc-1".into(),
                source: MentionSource::Issue,
                label: "用户登录超时".into(),
                detail: Some("[BUG]".into()),
                search_text: "issues/login-timeout.md".into(),
            },
            MentionCandidate {
                id: "issue:doc-2".into(),
                source: MentionSource::Issue,
                label: "购物车结算".into(),
                detail: Some("[FEAT]".into()),
                search_text: "issues/cart.md".into(),
            },
            MentionCandidate {
                id: "tp:tp-1".into(),
                source: MentionSource::TestPoint,
                label: "密码错误锁定时长".into(),
                detail: Some("Auth / Login".into()),
                search_text: "tp-1".into(),
            },
            MentionCandidate {
                id: "feature:features/export.feature".into(),
                source: MentionSource::Feature,
                label: "数据导出".into(),
                detail: Some("features/export.feature".into()),
                search_text: "features/export.feature".into(),
            },
        ]
    }

    #[test]
    fn trigger_detects_line_start_space_and_cjk_but_ignores_emails() {
        assert_eq!(
            detect_mention_trigger("@", 1).map(|t| (t.range, t.raw_query)),
            Some((0..1, String::new()))
        );
        assert_eq!(
            detect_mention_trigger("check @iss", 10).map(|t| (t.range, t.raw_query)),
            Some((6..10, "iss".into()))
        );
        assert_eq!(
            detect_mention_trigger("line1\n@tp:登录", 12).map(|t| (
                t.range,
                t.scoped_source,
                t.filter
            )),
            Some((6..12, Some(MentionSource::TestPoint), "登录".into()))
        );
        assert_eq!(
            detect_mention_trigger("帮我看看@issue", 10).map(|t| (t.range, t.scoped_source)),
            Some((4..10, Some(MentionSource::Issue)))
        );
        // Email address must NOT open mention popup.
        assert!(detect_mention_trigger("alice@example.com", 17).is_none());
        // Whitespace after @ closes the trigger.
        assert!(detect_mention_trigger("@issue foo", 10).is_none());
    }

    #[test]
    fn empty_at_lists_four_core_sources_and_mounts_last_fail_or_drills_into_issue() {
        let candidates = sample_candidates();
        let mut state = MentionPopupState::new();
        state.sync_input("@", 1);
        assert!(state.is_open());

        let items = state.visible_items(&candidates);
        assert_eq!(items.len(), 4);
        assert!(matches!(
            items[0],
            MentionItem::Candidate(MentionCandidate {
                source: MentionSource::LastFail,
                ..
            })
        ));
        assert!(matches!(
            items[1],
            MentionItem::Category {
                source: MentionSource::Issue,
                count: 2,
                ..
            }
        ));
        assert!(matches!(
            items[2],
            MentionItem::Category {
                source: MentionSource::TestPoint,
                count: 1,
                ..
            }
        ));
        assert!(matches!(
            items[3],
            MentionItem::Category {
                source: MentionSource::Feature,
                count: 1,
                ..
            }
        ));

        // Pressing Enter on index 0 immediately selects @last-fail.
        let outcome = state.handle_key(MentionKey::Confirm, &candidates);
        assert!(matches!(
            outcome,
            MentionOutcome::Selected {
                range,
                candidate: MentionCandidate {
                    source: MentionSource::LastFail,
                    ..
                }
            } if range == (0..1)
        ));
        assert!(!state.is_open());

        // Drilling into @issue expands scope to `@issue:`.
        state.sync_input("@", 1);
        assert_eq!(
            state.handle_key(MentionKey::Next, &candidates),
            MentionOutcome::Navigated
        );
        assert_eq!(
            state.handle_key(MentionKey::Confirm, &candidates),
            MentionOutcome::ExpandScope {
                range: 0..1,
                replacement: "@issue:".into(),
            }
        );

        // After expanding to `@issue:超时`, substring filter narrows to the matching Issue.
        state.sync_input("@issue:超时", 9);
        let filtered = state.visible_items(&candidates);
        assert_eq!(filtered.len(), 1);
        assert!(matches!(
            &filtered[0],
            MentionItem::Candidate(c) if c.id == "issue:doc-1"
        ));
    }

    #[test]
    fn ime_composing_blocks_enter_confirmation_and_navigation() {
        let candidates = sample_candidates();
        let mut state = MentionPopupState::new();
        state.sync_input("@issue:", 7);
        assert!(state.is_open());
        assert!(state.accepts_keyboard());

        // While Chinese IME composition is active (e.g. Web `compositionstart`),
        // pressing Enter/Down/Esc must NOT confirm or close the mention popup.
        state.set_ime_composing(true);
        assert!(!state.accepts_keyboard());
        assert_eq!(
            state.handle_key(MentionKey::Confirm, &candidates),
            MentionOutcome::Ignored
        );
        assert_eq!(
            state.handle_key(MentionKey::Next, &candidates),
            MentionOutcome::Ignored
        );
        assert_eq!(
            state.handle_key(MentionKey::Dismiss, &candidates),
            MentionOutcome::Ignored
        );
        assert!(state.is_open());

        // Once composition finishes, Enter confirms cleanly.
        state.set_ime_composing(false);
        assert!(state.accepts_keyboard());
        let outcome = state.handle_key(MentionKey::Confirm, &candidates);
        assert!(matches!(
            outcome,
            MentionOutcome::Selected { candidate, .. } if candidate.id == "issue:doc-1"
        ));
    }

    #[test]
    fn dismiss_keeps_current_at_closed_until_new_trigger() {
        let candidates = sample_candidates();
        let mut state = MentionPopupState::new();
        state.sync_input("hello @is", 9);
        assert!(state.is_open());
        assert_eq!(
            state.handle_key(MentionKey::Dismiss, &candidates),
            MentionOutcome::Dismissed
        );
        assert!(!state.is_open());

        // Continuing to type on the dismissed `@` stays closed.
        state.sync_input("hello @iss", 10);
        assert!(!state.is_open());

        // Typing a space and a fresh `@` re-opens cleanly.
        state.sync_input("hello @iss @", 12);
        assert!(state.is_open());
    }
}
