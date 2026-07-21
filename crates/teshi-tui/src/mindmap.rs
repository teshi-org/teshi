//! MindMap TreeItem rendering and TreeState management.
//!
//! Wraps `teshi_core::mindmap::MindMapIndex` (domain data) with TUI-specific
//! `TreeItem` construction, highlight/filter application, and `TreeState` helpers.

use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Text as TuiText;
use tui_tree_widget::TreeItem;

// ── Re-exports from core ──────────────────────────────────────────────
pub use teshi_core::mindmap::{
    HighlightCategory, HighlightColor, HighlightRule, LocationContext, MatchCondition,
    MindMapContext, MindMapFilter, NodeLocation, evaluate_condition, find_closest_node,
    matches_filter, node_id_to_path, parse_color, parse_node_line_number,
};

pub use tui_tree_widget::TreeState;

/// Build a TUI [`MindMapIndex`] from a parsed project.
/// Wraps the core `build_index` with TreeItem construction.
pub fn build_index(project: &teshi_core::gherkin::BddProject) -> MindMapIndex {
    MindMapIndex::new(teshi_core::mindmap::build_index(project))
}

// ── TUI wrapper around core MindMapIndex ─────────────────────────────

/// Wraps the core `MindMapIndex` with TUI `TreeItem`s for rendering.
pub struct MindMapIndex {
    pub inner: teshi_core::mindmap::MindMapIndex,
    pub items: Vec<TreeItem<'static, String>>,
}

impl std::ops::Deref for MindMapIndex {
    type Target = teshi_core::mindmap::MindMapIndex;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for MindMapIndex {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl MindMapIndex {
    /// Wrap a core index; caller must call `rebuild_items` afterwards.
    pub fn new(inner: teshi_core::mindmap::MindMapIndex) -> Self {
        Self {
            inner,
            items: Vec::new(),
        }
    }

    // ── Highlight methods ──────────────────────────────────────────

    /// Replace active highlights with `rules` and rebuild tree items.
    pub fn apply_highlights(&mut self, rules: Vec<HighlightRule>) {
        self.highlights = rules;
        self.rebuild_items();
    }

    /// Remove all highlights and rebuild tree items.
    pub fn clear_highlights(&mut self) {
        self.highlights.clear();
        self.rebuild_items();
    }

    /// Whether any highlight rules are active.
    pub fn has_active_highlights(&self) -> bool {
        !self.highlights.is_empty()
    }

    // ── Filter methods ─────────────────────────────────────────────

    /// Set the active filter and rebuild tree items.
    pub fn apply_filter(&mut self, filter: MindMapFilter) {
        self.filter = Some(filter);
        self.rebuild_items();
    }

    /// Remove the active filter and rebuild tree items.
    pub fn clear_filter(&mut self) {
        self.filter = None;
        self.rebuild_items();
    }

    /// Whether a filter is currently active.
    pub fn has_active_filter(&self) -> bool {
        self.filter.is_some()
    }

    // ── Highlight category methods ─────────────────────────────────

    /// Computes node categories from the current selection and rebuilds tree items.
    pub fn apply_highlight_categories(&mut self, selected_id: &str) {
        self.node_categories = self.compute_node_categories(selected_id);
        self.rebuild_items();
    }

    // ── Internal rebuild ───────────────────────────────────────────

    fn rebuild_items(&mut self) {
        let inner = &mut self.inner;
        let label_colors = build_label_colors(&inner.highlights, &inner.node_labels);
        let root_label = inner.node_labels.get("root").cloned().unwrap_or_default();
        let mut next_id = 0usize;
        inner.parent_map.clear();
        inner.children_map.clear();
        let mut ctx = BuildItemsCtx {
            arena: &inner.arena,
            root_label: &root_label,
            next_id: &mut next_id,
            node_paths: &mut inner.node_paths,
            node_locations: &mut inner.node_locations,
            node_labels: &mut inner.node_labels,
            label_colors: &label_colors,
            node_categories: &inner.node_categories,
            filter: &inner.filter,
            parent_map: &mut inner.parent_map,
            children_map: &mut inner.children_map,
        };
        let root_item = build_items(0, &mut ctx, &[])
            .unwrap_or_else(|| TreeItem::new_leaf("root".to_string(), "(no matching nodes)"));
        self.items = vec![root_item];
    }
}

/// Build a cached map from node label to highlight color.
/// First matching rule wins.
fn build_label_colors(
    rules: &[HighlightRule],
    node_labels: &HashMap<String, String>,
) -> HashMap<String, Color> {
    let mut map = HashMap::new();
    for (id, label) in node_labels {
        for rule in rules {
            if evaluate_condition(&rule.condition, label) {
                map.insert(id.clone(), highlight_color_to_ratatui(rule.color));
                break;
            }
        }
    }
    map
}

/// Convert a core [`HighlightColor`] to a ratatui [`Color`].
fn highlight_color_to_ratatui(c: HighlightColor) -> Color {
    match c {
        HighlightColor::Red => Color::Red,
        HighlightColor::Green => Color::Green,
        HighlightColor::Yellow => Color::Yellow,
        HighlightColor::Blue => Color::Blue,
        HighlightColor::Magenta => Color::Magenta,
        HighlightColor::Cyan => Color::Cyan,
        HighlightColor::White => Color::White,
        HighlightColor::Black => Color::Black,
    }
}

/// Mutable state shared while converting the trie arena into `TreeItem`s.
struct BuildItemsCtx<'a> {
    arena: &'a [teshi_core::mindmap::TrieNode],
    root_label: &'a str,
    next_id: &'a mut usize,
    node_paths: &'a mut HashMap<String, Vec<String>>,
    node_locations: &'a mut HashMap<String, Vec<NodeLocation>>,
    node_labels: &'a mut HashMap<String, String>,
    label_colors: &'a HashMap<String, Color>,
    node_categories: &'a HashMap<String, HighlightCategory>,
    filter: &'a Option<MindMapFilter>,
    parent_map: &'a mut HashMap<String, String>,
    children_map: &'a mut HashMap<String, Vec<String>>,
}

fn build_items(
    node_idx: usize,
    ctx: &mut BuildItemsCtx<'_>,
    parent_path: &[String],
) -> Option<TreeItem<'static, String>> {
    let id = if node_idx == 0 {
        "root".to_string()
    } else {
        *ctx.next_id += 1;
        format!("node-{}", *ctx.next_id)
    };

    let mut path = parent_path.to_vec();
    path.push(id.clone());
    ctx.node_paths.insert(id.clone(), path.clone());
    ctx.node_locations
        .insert(id.clone(), ctx.arena[node_idx].locations.clone());

    if let Some(parent_id) = parent_path.last() {
        ctx.parent_map.insert(id.clone(), parent_id.clone());
        ctx.children_map
            .entry(parent_id.clone())
            .or_default()
            .push(id.clone());
    }

    let label = if node_idx == 0 {
        ctx.root_label.to_string()
    } else {
        ctx.arena[node_idx].label.clone()
    };
    ctx.node_labels.insert(id.clone(), label.clone());

    let mut children: Vec<TreeItem<'static, String>> = Vec::new();
    for &child_idx in &ctx.arena[node_idx].children {
        if let Some(child_item) = build_items(child_idx, ctx, &path) {
            children.push(child_item);
        }
    }

    // Filtering: skip this node if it doesn't match and has no matching descendants
    if let Some(filter) = ctx.filter.as_ref() {
        let self_matches = matches_filter(filter, &label);
        let has_matching_children = !children.is_empty();
        if !self_matches && !has_matching_children {
            return None;
        }
    }

    // Apply semantic styling based on node category
    let label_text = if let Some(&color) = ctx.label_colors.get(&id) {
        TuiText::styled(label, Style::default().fg(color))
    } else {
        match ctx.node_categories.get(&id) {
            Some(HighlightCategory::Selected) => TuiText::from(label),
            Some(HighlightCategory::Ancestor) => {
                TuiText::styled(label, Style::default().fg(Color::Cyan))
            }
            Some(HighlightCategory::Descendant) => {
                TuiText::styled(label, Style::default().fg(Color::Green))
            }
            Some(HighlightCategory::Sibling) => TuiText::from(label),
            Some(HighlightCategory::GrayedOut) | None => TuiText::styled(
                label,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
        }
    };

    TreeItem::new(id, label_text, children).ok()
}

// ── TreeState helpers ─────────────────────────────────────────────────

/// Creates a [`TreeState`] with all nodes collapsed; only root is selected.
/// Also rebuilds tree items for the index.
pub fn init_tree_state(index: &mut MindMapIndex) -> TreeState<String> {
    index.rebuild_items();
    let mut state = TreeState::default();
    state.select(vec!["root".to_string()]);
    state
}

/// Returns the last identifier in the current selection path (selected node's own ID).
pub fn selected_node_id(state: &TreeState<String>) -> Option<&str> {
    state.selected().last().map(|s| s.as_str())
}

/// Extracts [`MindMapContext`] for the currently selected node in `state`.
pub fn selected_node_context(
    state: &TreeState<String>,
    index: &MindMapIndex,
) -> Option<MindMapContext> {
    let id = selected_node_id(state)?;
    let path_ids = index.path_for(id)?;
    let locations = index.locations_for(id).unwrap_or(&[]);
    let path_labels: Vec<String> = path_ids
        .iter()
        .map(|pid| index.label_for(pid).cloned().unwrap_or_default())
        .collect();
    let step_text = path_labels.last().cloned().unwrap_or_default();
    Some(MindMapContext {
        step_text,
        path_labels,
        location_count: locations.len(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{MindMapIndex, build_index, init_tree_state, selected_node_context};
    use teshi_core::gherkin::{self, BddProject};

    fn sample_project() -> BddProject {
        let content = "\
Feature: F
  Scenario: S1
    Given a
    When b
    Then c
  Scenario: S2
    Given a
    When d
";
        let feature = gherkin::parse_feature(content, PathBuf::from("sample.feature"));
        BddProject {
            root_dir: PathBuf::from("."),
            features: vec![feature],
        }
    }

    fn build_tui_index(project: &BddProject) -> MindMapIndex {
        build_index(project)
    }

    #[test]
    fn test_init_tree_state_collapses_all_nodes_by_default() {
        let project = sample_project();
        let mut index = build_tui_index(&project);
        assert!(
            !index.node_paths.is_empty(),
            "index should have built non-trivial node paths for the test"
        );

        let state = init_tree_state(&mut index);

        assert!(
            state.opened().is_empty(),
            "no tree nodes should be expanded on initialization"
        );
        assert_eq!(
            state.selected(),
            &["root".to_string()],
            "root should remain selected on initialization"
        );
    }

    #[test]
    fn test_selected_node_context_returns_root_context() {
        let project = sample_project();
        let mut index = build_tui_index(&project);
        let state = init_tree_state(&mut index);

        let ctx = selected_node_context(&state, &index).expect("root should be selectable");
        assert_eq!(ctx.step_text, ".", "root label is the project dir name");
        assert_eq!(ctx.path_labels, &["."], "root path is just the root label");
        assert_eq!(ctx.location_count, 0, "root has no source locations");
    }
}
