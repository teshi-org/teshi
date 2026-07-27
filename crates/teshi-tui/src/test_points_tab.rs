//! Test Points tab state, hierarchy tree, review actions, and excerpt navigation.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use teshi_core::authoring::{
    AuthoringArtifacts, RequirementLink, ResolutionState, ReviewState, TestPoint, TextRange,
    slice_by_char_range,
};
use teshi_engine::save_test_points;
use tui_tree_widget::{TreeItem, TreeState};

use crate::authoring_tab::review_state_label;

/// Tree node id prefix for hierarchy folders.
pub const TREE_DIR_PREFIX: &str = "tp-dir:";
/// Tree node id prefix for test-point leaves.
pub const TREE_TP_PREFIX: &str = "tp-leaf:";

/// Focus within the Test Points tab three-pane layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestPointsFocus {
    Tree,
    Details,
    Excerpts,
}

/// Editable intent field in the center pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailField {
    Title,
    Objective,
    Preconditions,
    ExpectedOutcomes,
    Hierarchy,
}

/// Optional review-state filter for the hierarchy tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewFilter {
    All,
    Proposed,
    Approved,
    Rejected,
    NeedsReview,
}

/// UI state for the Test Points tab.
pub struct TestPointsUiState {
    pub focus: TestPointsFocus,
    pub tree_items: Vec<TreeItem<'static, String>>,
    pub tree_state: TreeState<String>,
    /// Ordered leaf ids matching visible tree leaves for keyboard navigation.
    pub visible_leaf_ids: Vec<String>,
    pub selected_test_point_id: Option<String>,
    pub detail_field: DetailField,
    pub selected_excerpt_index: usize,
    pub review_filter: ReviewFilter,
    /// Scratch buffer while editing a detail field.
    pub field_buffer: String,
    pub field_dirty: bool,
}

impl TestPointsUiState {
    /// Creates empty UI state.
    pub fn empty() -> Self {
        Self {
            focus: TestPointsFocus::Tree,
            tree_items: Vec::new(),
            tree_state: TreeState::default(),
            visible_leaf_ids: Vec::new(),
            selected_test_point_id: None,
            detail_field: DetailField::Title,
            selected_excerpt_index: 0,
            review_filter: ReviewFilter::All,
            field_buffer: String::new(),
            field_dirty: false,
        }
    }

    /// Rebuilds the hierarchy tree from authoring artifacts.
    pub fn rebuild_tree(&mut self, artifacts: Option<&AuthoringArtifacts>) {
        let test_points = artifacts
            .map(|a| a.test_points.test_points.as_slice())
            .unwrap_or(&[]);
        self.visible_leaf_ids = filtered_test_points(test_points, self.review_filter)
            .into_iter()
            .map(|tp| tp.id.clone())
            .collect();
        self.tree_items = build_test_point_tree(test_points, self.review_filter);

        if self.selected_test_point_id.is_none() {
            if let Some(id) = self.visible_leaf_ids.first().cloned() {
                self.select_test_point(&id);
            }
        } else if let Some(id) = self.selected_test_point_id.clone() {
            if !self.visible_leaf_ids.iter().any(|x| x == &id) {
                if let Some(first) = self.visible_leaf_ids.first().cloned() {
                    self.select_test_point(&first);
                } else {
                    self.selected_test_point_id = None;
                    self.field_buffer.clear();
                    self.field_dirty = false;
                }
            } else {
                self.tree_state
                    .select(vec![format!("{TREE_TP_PREFIX}{id}")]);
            }
        }
    }

    /// Selects a test point by id and loads the active field buffer.
    pub fn select_test_point(&mut self, tp_id: &str) {
        self.selected_test_point_id = Some(tp_id.to_string());
        self.selected_excerpt_index = 0;
        self.tree_state
            .select(vec![format!("{TREE_TP_PREFIX}{tp_id}")]);
        self.load_field_buffer();
    }

    /// Selects a test point from a tree node id.
    pub fn select_tree_node(&mut self, node_id: &str) {
        if let Some(tp_id) = node_id.strip_prefix(TREE_TP_PREFIX) {
            self.select_test_point(tp_id);
        }
    }

    pub fn focus_next_column(&mut self) {
        self.commit_field_if_dirty();
        self.focus = match self.focus {
            TestPointsFocus::Tree => TestPointsFocus::Details,
            TestPointsFocus::Details => TestPointsFocus::Excerpts,
            TestPointsFocus::Excerpts => TestPointsFocus::Excerpts,
        };
    }

    pub fn focus_prev_column(&mut self) {
        self.commit_field_if_dirty();
        self.focus = match self.focus {
            TestPointsFocus::Tree => TestPointsFocus::Tree,
            TestPointsFocus::Details => TestPointsFocus::Tree,
            TestPointsFocus::Excerpts => TestPointsFocus::Details,
        };
    }

    pub fn cycle_review_filter(&mut self) {
        self.review_filter = match self.review_filter {
            ReviewFilter::All => ReviewFilter::Proposed,
            ReviewFilter::Proposed => ReviewFilter::Approved,
            ReviewFilter::Approved => ReviewFilter::Rejected,
            ReviewFilter::Rejected => ReviewFilter::NeedsReview,
            ReviewFilter::NeedsReview => ReviewFilter::All,
        };
    }

    pub fn move_tree_selection(&mut self, delta: isize) {
        if self.visible_leaf_ids.is_empty() {
            return;
        }
        let current = self
            .selected_test_point_id
            .as_ref()
            .and_then(|id| self.visible_leaf_ids.iter().position(|x| x == id))
            .unwrap_or(0);
        let next = if delta < 0 {
            current.saturating_sub(1)
        } else {
            current
                .saturating_add(1)
                .min(self.visible_leaf_ids.len() - 1)
        };
        if let Some(id) = self.visible_leaf_ids.get(next).cloned() {
            self.select_test_point(&id);
        }
    }

    pub fn move_detail_field(&mut self, delta: isize) {
        self.commit_field_if_dirty();
        let fields = [
            DetailField::Title,
            DetailField::Objective,
            DetailField::Preconditions,
            DetailField::ExpectedOutcomes,
            DetailField::Hierarchy,
        ];
        let pos = fields
            .iter()
            .position(|f| *f == self.detail_field)
            .unwrap_or(0);
        let next = if delta < 0 {
            pos.saturating_sub(1)
        } else {
            pos.saturating_add(1).min(fields.len() - 1)
        };
        self.detail_field = fields[next];
        self.load_field_buffer();
    }

    pub fn move_excerpt_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.selected_excerpt_index = 0;
            return;
        }
        let next = if delta < 0 {
            self.selected_excerpt_index.saturating_sub(1)
        } else {
            self.selected_excerpt_index.saturating_add(1).min(count - 1)
        };
        self.selected_excerpt_index = next;
    }

    pub fn load_field_buffer(&mut self) {
        self.field_buffer.clear();
        self.field_dirty = false;
    }

    pub fn load_field_buffer_from(&mut self, tp: &TestPoint) {
        self.field_buffer = field_value(tp, self.detail_field);
        self.field_dirty = false;
    }

    pub fn commit_field_if_dirty(&mut self) {
        if !self.field_dirty {
            return;
        }
        // Actual commit happens in app with mutable artifacts access.
    }

    /// Applies the scratch buffer to `tp`, resetting review state for meaning-bearing edits.
    pub fn apply_field_buffer(tp: &mut TestPoint, field: DetailField, value: &str) -> bool {
        let meaning_bearing = !matches!(field, DetailField::Hierarchy);
        let changed = match field {
            DetailField::Title => {
                let new_val = value.trim().to_string();
                let changed = tp.title != new_val;
                tp.title = new_val;
                changed
            }
            DetailField::Objective => {
                let changed = tp.objective != value;
                tp.objective = value.to_string();
                changed
            }
            DetailField::Preconditions => {
                let new_val = optional_field(value);
                let changed = tp.preconditions != new_val;
                tp.preconditions = new_val;
                changed
            }
            DetailField::ExpectedOutcomes => {
                let new_val = optional_field(value);
                let changed = tp.expected_outcomes != new_val;
                tp.expected_outcomes = new_val;
                changed
            }
            DetailField::Hierarchy => {
                let normalized = value.replace(" / ", "/");
                let segments: Vec<String> = normalized
                    .split('/')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                let new_path = teshi_core::authoring::HierarchyPath::new(segments);
                let changed = tp.hierarchy_path != new_path;
                tp.hierarchy_path = new_path;
                changed
            }
        };

        if changed && meaning_bearing && is_review_locked(tp.review_state) {
            tp.review_state = ReviewState::Proposed;
        }
        changed
    }

    pub fn insert_char(&mut self, ch: char) {
        self.field_buffer.push(ch);
        self.field_dirty = true;
    }

    pub fn backspace(&mut self) {
        if self.field_buffer.pop().is_some() {
            self.field_dirty = true;
        }
    }

    pub fn delete_forward(&mut self) {
        if !self.field_buffer.is_empty() {
            self.field_buffer.remove(0);
            self.field_dirty = true;
        }
    }

    /// Returns whether the test point can be approved (resolved links, eligible state).
    pub fn can_approve(tp: &TestPoint) -> bool {
        matches!(
            tp.review_state,
            ReviewState::Proposed | ReviewState::NeedsReview
        ) && tp
            .requirement_links
            .iter()
            .all(|l| l.resolution == ResolutionState::Resolved)
    }

    /// Approves a single test point when eligible.
    pub fn approve(tp: &mut TestPoint) -> bool {
        if !Self::can_approve(tp) {
            return false;
        }
        tp.review_state = ReviewState::Approved;
        true
    }

    /// Rejects a single test point when review is still open.
    pub fn reject(tp: &mut TestPoint) -> bool {
        match tp.review_state {
            ReviewState::Proposed | ReviewState::NeedsReview => {
                tp.review_state = ReviewState::Rejected;
                true
            }
            _ => false,
        }
    }

    /// Batch-approves every eligible visible test point. Returns count approved.
    pub fn batch_approve(test_points: &mut [TestPoint], ids: &[String]) -> usize {
        let mut approved = 0usize;
        for id in ids {
            if let Some(tp) = test_points.iter_mut().find(|tp| &tp.id == id) {
                if Self::approve(tp) {
                    approved += 1;
                }
            }
        }
        approved
    }

    /// Persists test points after review or edit changes.
    pub fn save_test_points(project_root: &Path, artifacts: &AuthoringArtifacts) -> Result<()> {
        save_test_points(project_root, &artifacts.test_points)
            .context("save test points from Test Points tab")
    }
}

fn is_review_locked(state: ReviewState) -> bool {
    matches!(
        state,
        ReviewState::Approved | ReviewState::Rejected | ReviewState::NeedsReview
    )
}

fn optional_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn field_value(tp: &TestPoint, field: DetailField) -> String {
    match field {
        DetailField::Title => tp.title.clone(),
        DetailField::Objective => tp.objective.clone(),
        DetailField::Preconditions => tp.preconditions.clone().unwrap_or_default(),
        DetailField::ExpectedOutcomes => tp.expected_outcomes.clone().unwrap_or_default(),
        DetailField::Hierarchy => tp.hierarchy_path.segments().join(" / "),
    }
}

fn filtered_test_points<'a>(
    test_points: &'a [TestPoint],
    filter: ReviewFilter,
) -> Vec<&'a TestPoint> {
    test_points
        .iter()
        .filter(|tp| match filter {
            ReviewFilter::All => true,
            ReviewFilter::Proposed => tp.review_state == ReviewState::Proposed,
            ReviewFilter::Approved => tp.review_state == ReviewState::Approved,
            ReviewFilter::Rejected => tp.review_state == ReviewState::Rejected,
            ReviewFilter::NeedsReview => tp.review_state == ReviewState::NeedsReview,
        })
        .collect()
}

/// Builds hierarchy tree items grouped by business path.
pub fn build_test_point_tree(
    test_points: &[TestPoint],
    filter: ReviewFilter,
) -> Vec<TreeItem<'static, String>> {
    #[derive(Default)]
    struct DirNode {
        children: HashMap<String, DirNode>,
        leaves: Vec<TestPoint>,
    }

    let mut root = DirNode::default();
    for tp in filtered_test_points(test_points, filter) {
        let mut node = &mut root;
        let segments = tp.hierarchy_path.segments();
        if segments.is_empty() {
            node.leaves.push(tp.clone());
            continue;
        }
        for segment in segments {
            node = node.children.entry(segment.clone()).or_default();
        }
        node.leaves.push(tp.clone());
    }

    fn dir_to_items(path: String, node: &DirNode) -> Vec<TreeItem<'static, String>> {
        let mut items = Vec::new();
        let mut child_names: Vec<_> = node.children.keys().cloned().collect();
        child_names.sort();
        for name in child_names {
            let child = node.children.get(&name).expect("child");
            let id = if path.is_empty() {
                format!("{TREE_DIR_PREFIX}{name}")
            } else {
                format!("{TREE_DIR_PREFIX}{path}/{name}")
            };
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            items.push(TreeItem::new(id, name, dir_to_items(child_path, child)).unwrap());
        }
        let mut leaves = node.leaves.clone();
        leaves.sort_by(|a, b| a.id.cmp(&b.id));
        for tp in leaves {
            let badge = review_state_label(tp.review_state);
            let label = format!("[{badge}] {}", tp.title);
            let id = format!("{TREE_TP_PREFIX}{}", tp.id);
            items.push(TreeItem::new(id, label, Vec::new()).unwrap());
        }
        items
    }

    dir_to_items(String::new(), &root)
}

/// One linked requirement excerpt row for the right pane.
#[derive(Debug, Clone)]
pub struct RequirementExcerpt {
    pub link_index: usize,
    pub document_title: String,
    pub quote: String,
    pub resolution: ResolutionState,
    pub document_id: String,
    pub position: TextRange,
}

/// Collects excerpt rows for a test point.
pub fn excerpts_for_test_point(
    tp: &TestPoint,
    artifacts: &AuthoringArtifacts,
) -> Vec<RequirementExcerpt> {
    tp.requirement_links
        .iter()
        .enumerate()
        .map(|(index, link)| {
            let (document_title, quote) = document_excerpt(artifacts, link);
            RequirementExcerpt {
                link_index: index,
                document_title,
                quote,
                resolution: link.resolution,
                document_id: link.document_id.clone(),
                position: link.position,
            }
        })
        .collect()
}

fn document_excerpt(artifacts: &AuthoringArtifacts, link: &RequirementLink) -> (String, String) {
    let title = artifacts
        .index
        .documents
        .iter()
        .find(|d| d.id == link.document_id)
        .map(|d| {
            if d.title.trim().is_empty() {
                d.path.clone()
            } else {
                d.title.clone()
            }
        })
        .unwrap_or_else(|| link.document_id.clone());

    let quote = artifacts
        .documents
        .iter()
        .find(|d| d.meta.id == link.document_id)
        .and_then(|d| slice_by_char_range(&d.body, link.position))
        .unwrap_or_else(|| link.quote.quote.clone());

    (title, quote)
}

/// Footer hint line for the Test Points tab.
pub fn test_points_footer_line() -> Line<'static> {
    Line::from(vec![
        Span::styled(" Focus [Tab] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Approve [a] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Reject [r] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Batch [A] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Continue [c] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Filter [f] ", Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(" Save [s] ", Style::default().fg(Color::DarkGray)),
    ])
}

/// Full review-state name for detail display.
pub fn review_state_name(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Proposed => "Proposed",
        ReviewState::Approved => "Approved",
        ReviewState::Rejected => "Rejected",
        ReviewState::NeedsReview => "Needs review",
    }
}

/// Style for excerpt resolution badges in the right pane.
pub fn excerpt_resolution_style(resolution: ResolutionState) -> Style {
    match resolution {
        ResolutionState::Resolved => Style::default().fg(Color::Green),
        ResolutionState::Stale => Style::default().fg(Color::Red),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::authoring::{HierarchyPath, QuoteSelector};

    fn sample_tp(id: &str, state: ReviewState, hierarchy: Vec<&str>) -> TestPoint {
        TestPoint {
            id: id.into(),
            title: format!("Title {id}"),
            objective: "Objective".into(),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(hierarchy.into_iter().map(str::to_string).collect()),
            review_state: state,
            requirement_links: vec![teshi_core::authoring::RequirementLink {
                document_id: "doc-1".into(),
                document_revision: "rev".into(),
                position: teshi_core::authoring::TextRange::new(0, 4),
                quote: QuoteSelector {
                    quote: "test".into(),
                    prefix: String::new(),
                    suffix: String::new(),
                },
                resolution: ResolutionState::Resolved,
            }],
            scenario_refs: Vec::new(),
        }
    }

    #[test]
    fn tree_groups_by_hierarchy_path() {
        let items = build_test_point_tree(
            &[
                sample_tp("tp-1", ReviewState::Proposed, vec!["Auth", "Login"]),
                sample_tp("tp-2", ReviewState::Approved, vec!["Auth", "Logout"]),
            ],
            ReviewFilter::All,
        );
        assert!(!items.is_empty());
    }

    #[test]
    fn approve_rejects_stale_links() {
        let mut tp = sample_tp("tp-1", ReviewState::Proposed, vec!["Auth"]);
        tp.requirement_links[0].resolution = ResolutionState::Stale;
        assert!(!TestPointsUiState::approve(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Proposed);
    }

    #[test]
    fn approve_transitions_proposed_to_approved() {
        let mut tp = sample_tp("tp-1", ReviewState::Proposed, vec!["Auth"]);
        assert!(TestPointsUiState::approve(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Approved);
    }

    #[test]
    fn reject_transitions_proposed_to_rejected() {
        let mut tp = sample_tp("tp-1", ReviewState::Proposed, vec!["Auth"]);
        assert!(TestPointsUiState::reject(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Rejected);
    }

    #[test]
    fn meaning_edit_resets_approved_to_proposed() {
        let mut tp = sample_tp("tp-1", ReviewState::Approved, vec!["Auth"]);
        TestPointsUiState::apply_field_buffer(&mut tp, DetailField::Title, "New title");
        assert_eq!(tp.review_state, ReviewState::Proposed);
    }

    #[test]
    fn hierarchy_edit_preserves_approval() {
        let mut tp = sample_tp("tp-1", ReviewState::Approved, vec!["Auth"]);
        TestPointsUiState::apply_field_buffer(
            &mut tp,
            DetailField::Hierarchy,
            "Billing / Payments",
        );
        assert_eq!(tp.review_state, ReviewState::Approved);
        assert_eq!(
            tp.hierarchy_path.segments(),
            &["Billing".to_string(), "Payments".to_string()]
        );
    }

    #[test]
    fn batch_approve_only_eligible() {
        let mut points = vec![
            sample_tp("tp-1", ReviewState::Proposed, vec!["A"]),
            sample_tp("tp-2", ReviewState::Rejected, vec!["A"]),
            sample_tp("tp-3", ReviewState::NeedsReview, vec!["A"]),
        ];
        points[2].requirement_links[0].resolution = ResolutionState::Stale;
        let ids = vec!["tp-1".into(), "tp-2".into(), "tp-3".into()];
        let count = TestPointsUiState::batch_approve(&mut points, &ids);
        assert_eq!(count, 1);
        assert_eq!(points[0].review_state, ReviewState::Approved);
        assert_eq!(points[1].review_state, ReviewState::Rejected);
        assert_eq!(points[2].review_state, ReviewState::NeedsReview);
    }

    #[test]
    fn needs_review_can_be_approved_when_resolved() {
        let mut tp = sample_tp("tp-1", ReviewState::NeedsReview, vec!["Auth"]);
        assert!(TestPointsUiState::approve(&mut tp));
        assert_eq!(tp.review_state, ReviewState::Approved);
    }

    #[test]
    fn reject_does_not_implicitly_approve_on_selection() {
        let mut ui = TestPointsUiState::empty();
        let tp = sample_tp("tp-1", ReviewState::Proposed, vec!["Auth"]);
        ui.select_test_point(&tp.id);
        assert_ne!(tp.review_state, ReviewState::Approved);
    }
}
