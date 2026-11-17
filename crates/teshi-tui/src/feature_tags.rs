//! Gherkin tag filtering for the Explore Features pane.
//!
//! Startup scans paths only, so the tag pool is built by a background worker
//! that parses each Feature once and keeps just tag names plus the tags that
//! apply to each scenario. Results arrive one file at a time and are cached by
//! a lightweight file stamp, so editing a file reparses only that file.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Color;
use teshi_core::authoring::TESHI_TP_TAG_PREFIX;
use teshi_core::gherkin::BddFeature;

use crate::app::{App, ColumnFocus, MainTab};
use crate::theme;

/// Maximum rows the tag strip may occupy inside the Features pane.
pub(crate) const MAX_TAG_ROWS: usize = 2;

/// Metadata-only stamp; a Feature is reparsed only when it changes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn file_stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Stamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Compact tag projection of one Feature file.
struct FileTagInfo {
    /// Distinct tags appearing anywhere in the file, excluding `@teshi-tp:`.
    pool: Vec<String>,
    /// Effective tags per scenario, in `all_scenarios()` order.
    scenarios: Vec<Vec<String>>,
}

struct TagResult {
    path: PathBuf,
    info: Option<FileTagInfo>,
    stamp: Option<Stamp>,
}

/// Effective tags of one scenario: its own, its Rule's, its Feature's, and its
/// Examples' tags. File-level tags therefore apply to every scenario.
pub(crate) fn effective_scenario_tags(feature: &BddFeature) -> Vec<Vec<String>> {
    let mut tagged: Vec<(usize, Vec<String>)> = Vec::new();
    for rule in &feature.rules {
        for scenario in &rule.scenarios {
            let mut tags = feature.tags.clone();
            tags.extend(rule.tags.iter().cloned());
            tags.extend(scenario.tags.iter().cloned());
            for table in &scenario.examples {
                tags.extend(table.tags.iter().cloned());
            }
            tagged.push((scenario.line_number, tags));
        }
    }
    for scenario in &feature.scenarios {
        let mut tags = feature.tags.clone();
        tags.extend(scenario.tags.iter().cloned());
        for table in &scenario.examples {
            tags.extend(table.tags.iter().cloned());
        }
        tagged.push((scenario.line_number, tags));
    }
    tagged.sort_by_key(|(line, _)| *line);
    tagged.into_iter().map(|(_, tags)| tags).collect()
}

fn is_indexable_tag(tag: &str) -> bool {
    !tag.starts_with(TESHI_TP_TAG_PREFIX)
}

fn collect_pool(into: &mut BTreeSet<String>, tags: &[String]) {
    for tag in tags.iter().filter(|tag| is_indexable_tag(tag)) {
        into.insert(tag.clone());
    }
}

fn pool_tags(feature: &BddFeature) -> Vec<String> {
    let mut pool = BTreeSet::new();
    collect_pool(&mut pool, &feature.tags);
    for rule in &feature.rules {
        collect_pool(&mut pool, &rule.tags);
        for scenario in &rule.scenarios {
            collect_pool(&mut pool, &scenario.tags);
            for table in &scenario.examples {
                collect_pool(&mut pool, &table.tags);
            }
        }
    }
    for scenario in &feature.scenarios {
        collect_pool(&mut pool, &scenario.tags);
        for table in &scenario.examples {
            collect_pool(&mut pool, &table.tags);
        }
    }
    pool.into_iter().collect()
}

fn tags_have_all(tags: &[String], selected: &BTreeSet<String>) -> bool {
    selected
        .iter()
        .all(|wanted| tags.iter().any(|tag| tag == wanted))
}

fn parse_file_tags(path: &Path) -> TagResult {
    let stamp = file_stamp(path);
    let info = crate::lazy_features::read_content(path)
        .ok()
        .map(|(feature, _)| FileTagInfo {
            pool: pool_tags(&feature),
            scenarios: effective_scenario_tags(&feature),
        });
    TagResult {
        path: path.to_path_buf(),
        info,
        stamp,
    }
}

/// Progressive, cached tag index fed by a single background worker.
#[derive(Default)]
pub(crate) struct TagIndex {
    files: HashMap<PathBuf, FileTagInfo>,
    /// `None` means the path was attempted but metadata/stamp was unavailable.
    /// Presence still counts toward [`Self::is_complete`].
    stamps: HashMap<PathBuf, Option<Stamp>>,
    pool_counts: BTreeMap<String, usize>,
    revision: u64,
    work_tx: Option<Sender<PathBuf>>,
    result_rx: Option<Receiver<TagResult>>,
    pending: HashSet<PathBuf>,
    checked: HashSet<PathBuf>,
    last_check: Option<Instant>,
}

impl TagIndex {
    const CHECK_INTERVAL: Duration = Duration::from_millis(1500);

    fn start_worker(&mut self) {
        if self.work_tx.is_some() {
            return;
        }
        let (work_tx, work_rx) = mpsc::channel::<PathBuf>();
        let (result_tx, result_rx) = mpsc::channel::<TagResult>();
        if std::thread::Builder::new()
            .name("feature-tags".into())
            .spawn(move || {
                while let Ok(path) = work_rx.recv() {
                    if result_tx.send(parse_file_tags(&path)).is_err() {
                        break;
                    }
                }
            })
            .is_ok()
        {
            self.work_tx = Some(work_tx);
            self.result_rx = Some(result_rx);
        }
    }

    fn remove_file(&mut self, path: &Path) {
        if let Some(info) = self.files.remove(path) {
            for tag in info.pool {
                let empty = if let Some(count) = self.pool_counts.get_mut(&tag) {
                    *count -= 1;
                    *count == 0
                } else {
                    false
                };
                if empty {
                    self.pool_counts.remove(&tag);
                }
            }
        }
        self.stamps.remove(path);
        self.revision = self.revision.wrapping_add(1);
    }

    fn install(&mut self, result: TagResult) {
        self.remove_file(&result.path);
        // Always record the attempt so unreadable paths do not leave the
        // index permanently incomplete with a blank Features pane.
        self.stamps.insert(result.path.clone(), result.stamp);
        if let Some(info) = result.info {
            for tag in &info.pool {
                *self.pool_counts.entry(tag.clone()).or_insert(0) += 1;
            }
            self.files.insert(result.path, info);
        }
    }

    /// Installs finished results and queues paths whose stamp changed.
    pub(crate) fn poll(&mut self, paths: &[PathBuf]) {
        let finished: Vec<TagResult> = self
            .result_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for result in finished {
            self.pending.remove(&result.path);
            self.install(result);
        }
        let now = Instant::now();
        let current: HashSet<PathBuf> = paths.iter().cloned().collect();
        let due = self
            .last_check
            .is_none_or(|at| now.duration_since(at) >= Self::CHECK_INTERVAL);
        if !due && current == self.checked {
            return;
        }
        self.last_check = Some(now);
        self.checked = current.clone();
        let stale: Vec<_> = self
            .stamps
            .keys()
            .filter(|path| !current.contains(*path))
            .cloned()
            .collect();
        for path in stale {
            self.remove_file(&path);
        }
        let mut needs = Vec::new();
        for path in paths {
            if self.pending.contains(path) {
                continue;
            }
            let stamp = file_stamp(path);
            // Queue when unseen or when the on-disk stamp no longer matches.
            if !self.stamps.get(path).is_some_and(|stored| stored == &stamp) {
                needs.push(path.clone());
            }
        }
        if needs.is_empty() {
            return;
        }
        self.start_worker();
        if let Some(tx) = &self.work_tx {
            for path in needs {
                if tx.send(path.clone()).is_err() {
                    break;
                }
                self.pending.insert(path);
            }
        }
    }

    /// Bumped whenever the pool or any file projection changes.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Distinct tags in alphabetical order, with stable positions.
    pub(crate) fn all_tags(&self) -> Vec<String> {
        self.pool_counts.keys().cloned().collect()
    }

    /// Whether every known path has been parsed (used to avoid a false
    /// "no matches" message while the first pass is still in flight).
    pub(crate) fn is_complete(&self) -> bool {
        self.pending.is_empty()
            && self
                .checked
                .iter()
                .all(|path| self.stamps.contains_key(path))
    }

    /// A file survives the filter when at least one scenario carries every
    /// selected tag. Unparsed files are hidden until their result arrives.
    pub(crate) fn file_matches(&self, path: &Path, selected: &BTreeSet<String>) -> bool {
        if selected.is_empty() {
            return true;
        }
        self.files.get(path).is_some_and(|info| {
            info.scenarios
                .iter()
                .any(|tags| tags_have_all(tags, selected))
        })
    }
}

/// One tag's position in the wrapped logical layout.
#[derive(Clone, Copy)]
pub(crate) struct TagSlot {
    /// Index into [`TagIndex::all_tags`].
    pub index: usize,
    /// Wrapped line within the logical tag flow.
    pub line: usize,
    /// Column offset of the chip within that line.
    pub col: usize,
}

/// Selection, keyboard focus and per-frame layout of the tag strip.
#[derive(Default)]
pub(crate) struct FeatureTagFilter {
    /// Progressive disk-backed tag index for the Feature catalog.
    pub index: TagIndex,
    /// Tags currently ANDed into the Explore filter.
    pub selected: BTreeSet<String>,
    /// Whether keyboard focus is on the tag strip rather than the tree.
    pub focused: bool,
    /// Cursor index into the wrapped [`Self::slots`] list.
    pub cursor: usize,
    /// First visible logical line of the tag strip.
    pub scroll_line: usize,
    /// Hit-test area of the tag strip for the current frame.
    pub area: Option<Rect>,
    /// Wrapped chip positions for the current frame.
    pub slots: Vec<TagSlot>,
    /// Total wrapped lines in the tag flow.
    pub line_count: usize,
    /// Visible row budget reserved for the strip this frame.
    pub visible_rows: usize,
    /// Hit target for scrolling the strip up.
    pub up_hit: Option<Rect>,
    /// Hit target for scrolling the strip down.
    pub down_hit: Option<Rect>,
    /// Hit target for clearing every selected tag.
    pub clear_hit: Option<Rect>,
}

impl FeatureTagFilter {
    /// Clears per-frame hit targets before Explore is redrawn.
    pub(crate) fn begin_frame(&mut self) {
        self.area = None;
        self.slots.clear();
        self.line_count = 0;
        self.visible_rows = 0;
        self.up_hit = None;
        self.down_hit = None;
        self.clear_hit = None;
    }

    /// Whether at least one tag is selected (AND filter is live).
    pub(crate) fn active(&self) -> bool {
        !self.selected.is_empty()
    }
}

/// Stable accent color for a tag, shared by the chip and the title label.
pub(crate) fn tag_accent(tag: &str) -> Color {
    const PALETTE: [Color; 6] = [
        theme::PRIMARY,
        theme::ACCENT,
        theme::SUCCESS,
        theme::WARNING,
        theme::SECONDARY,
        theme::ERROR,
    ];
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in tag.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    PALETTE[(hash as usize) % PALETTE.len()]
}

impl App {
    fn feature_tag_paths(&self) -> Vec<PathBuf> {
        if let Some(catalog) = &self.lazy_features {
            catalog.paths.clone()
        } else {
            self.project
                .features
                .iter()
                .map(|feature| feature.file_path.clone())
                .collect()
        }
    }

    /// Advances the background tag index; called once per event-loop frame.
    ///
    /// When the pool revision changes under an active filter, remaps the
    /// Explore selection so progressive parse results do not leave the cursor
    /// on a vanished row.
    pub(crate) fn poll_feature_tags(&mut self) {
        let paths = self.feature_tag_paths();
        let before = self.feature_tags.index.revision();
        let had_filter = self.filter_active();
        self.feature_tags.index.poll(&paths);
        if self.feature_tags.index.revision() == before {
            return;
        }
        // Drop selections that left the pool once indexing has settled.
        if self.feature_tags.index.is_complete() {
            let pool: BTreeSet<String> = self.feature_tags.index.all_tags().into_iter().collect();
            self.feature_tags.selected.retain(|tag| pool.contains(tag));
        }
        if had_filter || self.filter_active() {
            self.after_feature_tag_change();
        }
    }

    /// Whether the Explore tag AND-filter currently hides some Features.
    pub(crate) fn filter_active(&self) -> bool {
        self.feature_tags.active()
    }

    /// Whether `path` survives the active tag filter.
    ///
    /// Prefers the in-memory AST when the Feature is already loaded so unsaved
    /// edits stay consistent between the tree and the Scenario pane; falls
    /// back to the disk index otherwise.
    pub(crate) fn feature_path_matches_filter(&self, path: &Path) -> bool {
        if !self.filter_active() {
            return true;
        }
        if let Some(feature) = self
            .project
            .features
            .iter()
            .find(|feature| feature.file_path == path)
        {
            return effective_scenario_tags(feature)
                .iter()
                .any(|tags| tags_have_all(tags, &self.feature_tags.selected));
        }
        self.feature_tags
            .index
            .file_matches(path, &self.feature_tags.selected)
    }

    /// Scenario indices (into `all_scenarios()`) that satisfy the tag filter.
    pub(crate) fn explore_visible_scenario_indices(&self) -> Vec<usize> {
        let Some(feature) = self.project.features.get(self.explore_selected_feature) else {
            return Vec::new();
        };
        let count = feature.scenario_count();
        if !self.filter_active() {
            return (0..count).collect();
        }
        let tags = effective_scenario_tags(feature);
        (0..count)
            .filter(|&index| {
                tags.get(index)
                    .is_some_and(|tags| tags_have_all(tags, &self.feature_tags.selected))
            })
            .collect()
    }

    /// Keyboard handling for the tag strip, before the shared keymap.
    pub(crate) fn handle_feature_tag_key(&mut self, event: KeyEvent) -> bool {
        if self.active_tab != MainTab::Explore {
            self.feature_tags.focused = false;
            return false;
        }
        if self.explore_edit_mode
            || self.explore_focus != ColumnFocus::Feature
            || self.pending_delete.is_some()
            || self.quit_pending_confirm
            || self.auth_panel_active
            || self.model_panel_active
            || self.session_panel_active
            || self.approval_panel_active
            || self.settings_panel_active
            || self.agent_profile_panel_active
            || self.change_summary_visible
            || self.scenario_dropdown_open
            || self.step_keyword_picker.is_some()
            || self.step_input_active
            || self.generation_scope_prompt.is_some()
            || self.has_external_change_prompt()
            || self.has_agent_change_prompt()
            || self.has_acp_permission_prompt()
            || !matches!(
                self.project_switch.overlay,
                crate::project_context::SwitchOverlay::None
            )
            || self.feature_browser.searching
        {
            return false;
        }

        // Ctrl+W j/k mirror the Ctrl+W h/l pane chord: j enters the tag
        // strip, k returns to the tree. Any other chord key clears the focus
        // and keeps its existing pane-navigation meaning.
        if self.pending_char == Some('w') {
            let plain = event.modifiers.is_empty()
                || event.modifiers == KeyModifiers::SHIFT
                || event.modifiers == KeyModifiers::CONTROL
                || event.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT);
            if plain {
                match event.code {
                    KeyCode::Char('j' | 'J') | KeyCode::Down => {
                        self.pending_char = None;
                        self.enter_feature_tags();
                        return true;
                    }
                    KeyCode::Char('k' | 'K') | KeyCode::Up => {
                        self.pending_char = None;
                        self.feature_tags.focused = false;
                        return true;
                    }
                    _ => {}
                }
            }
            self.feature_tags.focused = false;
            return false;
        }

        // Esc ladder: search input, then selected tags, then tag-strip focus.
        if event.code == KeyCode::Esc {
            if !self.feature_browser.query.text().is_empty() {
                return false;
            }
            if self.feature_tags.active() {
                self.clear_feature_tags();
                return true;
            }
            if self.feature_tags.focused {
                self.feature_tags.focused = false;
                return true;
            }
            return false;
        }

        if !self.feature_tags.focused {
            return false;
        }

        // Consume both keys of gg so the global keymap cannot reinterpret the
        // second one as a first/last navigation of the tree.
        if self.pending_char == Some('g') {
            self.pending_char = None;
            if matches!(event.code, KeyCode::Char('g') | KeyCode::Char('G'))
                && event.modifiers.is_empty()
            {
                self.move_tag_cursor(0);
            }
            return true;
        }

        match (event.code, event.modifiers) {
            (KeyCode::Char('g'), KeyModifiers::NONE) => {
                self.pending_char = Some('g');
                true
            }
            (KeyCode::Char('G'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.move_tag_cursor(usize::MAX);
                true
            }
            (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _) => {
                self.step_tag_cursor(-1, 0);
                true
            }
            (KeyCode::Char('l'), KeyModifiers::NONE) | (KeyCode::Right, _) => {
                self.step_tag_cursor(1, 0);
                true
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                self.step_tag_cursor(0, 1);
                true
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                self.step_tag_cursor(0, -1);
                true
            }
            (KeyCode::Enter, _) | (KeyCode::Char(' '), KeyModifiers::NONE) => {
                self.toggle_feature_tag_at_cursor();
                true
            }
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                self.feature_tags.focused = false;
                false
            }
            _ => false,
        }
    }

    fn enter_feature_tags(&mut self) {
        if self.feature_tags.index.all_tags().is_empty() {
            return;
        }
        self.explore_focus = ColumnFocus::Feature;
        self.feature_tags.focused = true;
        self.feature_tags.cursor = 0;
        self.feature_tags.scroll_line = 0;
    }

    fn move_tag_cursor(&mut self, target: usize) {
        let len = self.feature_tags.slots.len();
        if len == 0 {
            return;
        }
        self.feature_tags.cursor = if target == usize::MAX {
            len - 1
        } else {
            target.min(len - 1)
        };
        self.reveal_tag_cursor();
    }

    fn step_tag_cursor(&mut self, dx: isize, dy: isize) {
        let Some(current) = self
            .feature_tags
            .slots
            .iter()
            .find(|slot| slot.index == self.feature_tags.cursor)
            .copied()
        else {
            self.move_tag_cursor(0);
            return;
        };
        // The top row of the tag strip is the tree's lower neighbour.
        if dy < 0 && current.line == 0 {
            self.feature_tags.focused = false;
            if let Some(last) = self.feature_browser_rows().len().checked_sub(1) {
                self.explore_set_feature_row(last);
            }
            return;
        }
        let target_line = current.line.saturating_add_signed(dy);
        let target = if dy == 0 {
            let index = self.feature_tags.cursor.saturating_add_signed(dx);
            self.feature_tags
                .slots
                .iter()
                .find(|slot| slot.index == index)
                .copied()
        } else {
            self.feature_tags
                .slots
                .iter()
                .filter(|slot| slot.line == target_line)
                .min_by_key(|slot| slot.col.abs_diff(current.col))
                .copied()
        };
        if let Some(slot) = target {
            self.feature_tags.cursor = slot.index;
            self.reveal_tag_cursor();
        }
    }

    fn reveal_tag_cursor(&mut self) {
        let Some(slot) = self
            .feature_tags
            .slots
            .iter()
            .find(|slot| slot.index == self.feature_tags.cursor)
            .copied()
        else {
            return;
        };
        let rows = self.feature_tags.visible_rows.max(1);
        if slot.line < self.feature_tags.scroll_line {
            self.feature_tags.scroll_line = slot.line;
        } else if slot.line >= self.feature_tags.scroll_line + rows {
            self.feature_tags.scroll_line = slot.line + 1 - rows;
        }
        let max = self.feature_tags.line_count.saturating_sub(rows);
        self.feature_tags.scroll_line = self.feature_tags.scroll_line.min(max);
    }

    fn toggle_feature_tag_at_cursor(&mut self) {
        let tags = self.feature_tags.index.all_tags();
        let Some(tag) = tags.get(self.feature_tags.cursor).cloned() else {
            return;
        };
        self.toggle_feature_tag(&tag);
    }

    /// Adds or removes one tag; multiple selections are ANDed.
    pub(crate) fn toggle_feature_tag(&mut self, tag: &str) {
        if !self.feature_tags.selected.remove(tag) {
            self.feature_tags.selected.insert(tag.to_string());
        }
        self.after_feature_tag_change();
    }

    /// Clears every selected tag and remaps the Explore selection.
    pub(crate) fn clear_feature_tags(&mut self) {
        self.feature_tags.selected.clear();
        self.after_feature_tag_change();
    }

    fn after_feature_tag_change(&mut self) {
        // Filter changes rebuild the tree; heal the cursor onto a still-visible row.
        if self.feature_browser_selected_row().is_none() && !self.feature_browser_rows().is_empty()
        {
            self.explore_set_feature_row(0);
        }
        self.normalize_explore_selection();
    }

    /// Wheel or arrow paging inside the tag strip.
    pub(crate) fn scroll_feature_tags(&mut self, down: bool, lines: usize) {
        let max = self
            .feature_tags
            .line_count
            .saturating_sub(self.feature_tags.visible_rows.max(1));
        self.feature_tags.scroll_line = if down {
            self.feature_tags.scroll_line.saturating_add(lines).min(max)
        } else {
            self.feature_tags.scroll_line.saturating_sub(lines)
        };
    }

    /// Enters the tag strip when the tree cursor walks past its last row.
    pub(crate) fn maybe_enter_feature_tags(&mut self) -> bool {
        if self.feature_tags.index.all_tags().is_empty() || self.feature_tags.focused {
            return false;
        }
        self.enter_feature_tags();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature() -> BddFeature {
        // Rule children must be indented deeper than the Rule header or the
        // parser treats them as top-level and drops Rule-scoped tags. Use a
        // raw string so line-continuation stripping cannot erase that indent.
        teshi_core::gherkin::parse_feature(
            r#"@file @smoke
Feature: Auth

  @rule
  Rule: Access

    @login @fast
    Scenario: Valid user
      Given a user

    @wip
    Scenario Outline: Reset
      Given a user
    @examples
    Examples: Cases
      | name |
      | a    |

  @logout
  Scenario: Locked account
    Given a user
"#,
            "auth.feature".into(),
        )
    }

    #[test]
    fn effective_tags_merge_file_rule_scenario_and_examples() {
        let effective = effective_scenario_tags(&feature());
        assert_eq!(effective.len(), 3);
        assert!(effective[0].contains(&"@file".to_string()));
        assert!(effective[0].contains(&"@rule".to_string()));
        assert!(effective[0].contains(&"@login".to_string()));
        assert!(effective[1].contains(&"@examples".to_string()));
        assert!(!effective[2].contains(&"@login".to_string()));
    }

    #[test]
    fn pool_excludes_teshi_tp_and_is_sorted() {
        let mut source = feature();
        source.tags.push("@teshi-tp:TP-12".into());
        let pool = pool_tags(&source);
        assert_eq!(
            pool,
            vec![
                "@examples",
                "@fast",
                "@file",
                "@login",
                "@logout",
                "@rule",
                "@smoke",
                "@wip"
            ]
        );
    }

    #[test]
    fn and_filter_requires_every_selected_tag_on_one_scenario() {
        let effective = effective_scenario_tags(&feature());
        let selected: BTreeSet<String> =
            ["@file", "@login"].iter().map(|s| s.to_string()).collect();
        assert!(tags_have_all(&effective[0], &selected));
        assert!(!tags_have_all(&effective[2], &selected));
        let mixed: BTreeSet<String> = ["@login", "@logout"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!effective.iter().any(|tags| tags_have_all(tags, &mixed)));
    }
}
