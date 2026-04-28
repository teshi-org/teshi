//! Mind-map tree construction and state for the three-stage MindMap tab.
//!
//! Builds a step-sequence prefix tree (trie) so shared step prefixes collapse
//! into a single path. Each step node records all source locations that map
//! to that path, enabling location selection in the preview panel.

use std::collections::HashMap;

use ratatui::style::{Color, Style};
use ratatui::text::Text as TuiText;
use tui_tree_widget::TreeItem;

use crate::gherkin::BddProject;

pub use tui_tree_widget::TreeState;

/// Where a step node appears in the source project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationContext {
    Background,
    Scenario(usize),
}

/// A single occurrence of a step node in the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeLocation {
    pub feature_idx: usize,
    pub context: LocationContext,
    pub step_idx: usize,
    pub line_number: usize,
}

/// One occurrence used for closest-node lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeOccurrence {
    pub node_id: String,
    pub location_index: usize,
    pub line_number: usize,
}

/// Structured context extracted from the currently selected MindMap node.
#[derive(Debug, Clone)]
pub struct MindMapContext {
    /// The step text (the trie node label for this node).
    pub step_text: String,
    /// Labels from root to this node, forming the full step sequence.
    pub path_labels: Vec<String>,
    /// The number of source locations referencing this node.
    pub location_count: usize,
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

/// Result of a closest-node lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeMatch {
    pub node_id: String,
    pub location_index: usize,
}

/// A condition for matching MindMap nodes.
#[derive(Debug, Clone)]
pub enum MatchCondition {
    /// Match nodes whose step text contains the given substring (case-insensitive).
    StepContains(String),
}

/// A highlight rule: nodes matching `condition` are styled with `color`.
#[derive(Debug, Clone)]
pub struct HighlightRule {
    pub condition: MatchCondition,
    pub color: Color,
}

/// A filter that restricts which nodes are visible in the tree.
#[derive(Debug, Clone)]
pub enum MindMapFilter {
    /// Show only nodes whose label contains the string (case-insensitive),
    /// plus ancestors to preserve tree structure.
    NameContains(String),
}

/// Precomputed tree items and lookup maps for MindMap behavior.
#[derive(Debug, Clone)]
pub struct MindMapIndex {
    pub items: Vec<TreeItem<'static, String>>,
    node_paths: HashMap<String, Vec<String>>,
    node_locations: HashMap<String, Vec<NodeLocation>>,
    node_labels: HashMap<String, String>,
    occurrences_by_feature: Vec<Vec<NodeOccurrence>>,
    /// Preserved trie arena for rebuilding items with different styling.
    arena: Vec<TrieNode>,
    /// Active highlight rules.
    highlights: Vec<HighlightRule>,
    /// Active filter.
    filter: Option<MindMapFilter>,
}

impl MindMapIndex {
    /// Returns every source location recorded for a trie node id.
    pub fn locations_for(&self, id: &str) -> Option<&[NodeLocation]> {
        self.node_locations.get(id).map(|v| v.as_slice())
    }

    /// Returns the path from root to `id` for [`TreeState`] selection.
    pub fn path_for(&self, id: &str) -> Option<&Vec<String>> {
        self.node_paths.get(id)
    }

    /// Returns the display label for a node id.
    pub fn label_for(&self, id: &str) -> Option<&String> {
        self.node_labels.get(id)
    }

    /// Lists node occurrences ordered for closest-line lookup within one feature file.
    pub fn occurrences_for_feature(&self, feature_idx: usize) -> Option<&[NodeOccurrence]> {
        self.occurrences_by_feature
            .get(feature_idx)
            .map(|v| v.as_slice())
    }

    // ── Highlight methods ──────────────────────────────────────────────

    /// Replace active highlights with `rules` and rebuild tree items.
    pub fn apply_highlights(&mut self, rules: Vec<HighlightRule>) {
        self.highlights = rules;
        self.rebuild_items_from_arena();
    }

    /// Remove all highlights and rebuild tree items.
    pub fn clear_highlights(&mut self) {
        self.highlights.clear();
        self.rebuild_items_from_arena();
    }

    /// Whether any highlight rules are active.
    pub fn has_active_highlights(&self) -> bool {
        !self.highlights.is_empty()
    }

    // ── Filter methods ─────────────────────────────────────────────────

    /// Set the active filter and rebuild tree items.
    pub fn apply_filter(&mut self, filter: MindMapFilter) {
        self.filter = Some(filter);
        self.rebuild_items_from_arena();
    }

    /// Remove the active filter and rebuild tree items.
    pub fn clear_filter(&mut self) {
        self.filter = None;
        self.rebuild_items_from_arena();
    }

    /// Whether a filter is currently active.
    pub fn has_active_filter(&self) -> bool {
        self.filter.is_some()
    }

    // ── Internal rebuild ───────────────────────────────────────────────

    fn rebuild_items_from_arena(&mut self) {
        let label_colors = Self::build_label_colors(&self.highlights, &self.node_labels);
        let root_label = self.node_labels.get("root").cloned().unwrap_or_default();
        let mut next_id = 0usize;
        let mut ctx = BuildItemsCtx {
            arena: &self.arena,
            root_label: &root_label,
            next_id: &mut next_id,
            node_paths: &mut self.node_paths,
            node_locations: &mut self.node_locations,
            node_labels: &mut self.node_labels,
            label_colors: &label_colors,
            filter: &self.filter,
        };
        let root_item = build_items(0, &mut ctx, &[])
            .unwrap_or_else(|| TreeItem::new_leaf("root".to_string(), "(no matching nodes)"));
        self.items = vec![root_item];
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
                    map.insert(id.clone(), rule.color);
                    break;
                }
            }
        }
        map
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TrieNode {
    pub(crate) label: String,
    pub(crate) children: Vec<usize>,
    pub(crate) child_by_label: HashMap<String, usize>,
    pub(crate) locations: Vec<NodeLocation>,
}

impl TrieNode {
    fn new(label: String) -> Self {
        Self {
            label,
            children: Vec::new(),
            child_by_label: HashMap::new(),
            locations: Vec::new(),
        }
    }
}

/// Evaluate whether a label matches a match condition.
fn evaluate_condition(cond: &MatchCondition, label: &str) -> bool {
    match cond {
        MatchCondition::StepContains(text) => label.to_lowercase().contains(&text.to_lowercase()),
    }
}

/// Check whether a node label matches the active filter.
fn matches_filter(filter: &MindMapFilter, label: &str) -> bool {
    match filter {
        MindMapFilter::NameContains(text) => label.to_lowercase().contains(&text.to_lowercase()),
    }
}

/// Parse a named color string into a ratatui [`Color`].
pub fn parse_color(s: &str) -> Option<Color> {
    match s.to_lowercase().as_str() {
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "black" => Some(Color::Black),
        _ => None,
    }
}

/// Builds the MindMap index and tree items from a parsed project.
pub fn build_index(project: &BddProject) -> MindMapIndex {
    let root_label = project
        .root_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.root_dir.display().to_string());

    let mut arena: Vec<TrieNode> = vec![TrieNode::new(String::new())];

    for (fi, feature) in project.features.iter().enumerate() {
        let bg_steps: Vec<(String, NodeLocation)> = feature
            .background
            .as_ref()
            .map(|bg| {
                bg.steps
                    .iter()
                    .enumerate()
                    .map(|(si, step)| {
                        (
                            step.text.clone(),
                            NodeLocation {
                                feature_idx: fi,
                                context: LocationContext::Background,
                                step_idx: si,
                                line_number: step.line_number,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        for (sci, scenario) in feature.scenarios.iter().enumerate() {
            let mut node_idx = 0usize; // root

            for (text, loc) in &bg_steps {
                node_idx = insert_step(&mut arena, node_idx, text, *loc, true);
            }

            for (sti, step) in scenario.steps.iter().enumerate() {
                let loc = NodeLocation {
                    feature_idx: fi,
                    context: LocationContext::Scenario(sci),
                    step_idx: sti,
                    line_number: step.line_number,
                };
                node_idx = insert_step(&mut arena, node_idx, &step.text, loc, false);
            }
        }
    }

    let mut node_paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut node_locations: HashMap<String, Vec<NodeLocation>> = HashMap::new();
    let mut node_labels: HashMap<String, String> = HashMap::new();
    let mut next_id = 0usize;

    // Build the initial label-to-color map (empty during initial construction)
    let empty_colors: HashMap<String, Color> = HashMap::new();
    let empty_filter: Option<MindMapFilter> = None;

    let mut ctx = BuildItemsCtx {
        arena: &arena,
        root_label: &root_label,
        next_id: &mut next_id,
        node_paths: &mut node_paths,
        node_locations: &mut node_locations,
        node_labels: &mut node_labels,
        label_colors: &empty_colors,
        filter: &empty_filter,
    };
    let root_item = build_items(0, &mut ctx, &[])
        .expect("root item must always be constructable during initial build");

    let mut occurrences_by_feature = vec![Vec::new(); project.features.len()];
    for (node_id, locations) in &node_locations {
        for (idx, loc) in locations.iter().enumerate() {
            if let Some(list) = occurrences_by_feature.get_mut(loc.feature_idx) {
                list.push(NodeOccurrence {
                    node_id: node_id.clone(),
                    location_index: idx,
                    line_number: loc.line_number,
                });
            }
        }
    }

    MindMapIndex {
        items: vec![root_item],
        node_paths,
        node_locations,
        node_labels,
        occurrences_by_feature,
        arena,
        highlights: Vec::new(),
        filter: None,
    }
}

fn insert_step(
    arena: &mut Vec<TrieNode>,
    parent_idx: usize,
    text: &str,
    loc: NodeLocation,
    dedupe_background: bool,
) -> usize {
    let child_idx = if let Some(&idx) = arena[parent_idx].child_by_label.get(text) {
        idx
    } else {
        let idx = arena.len();
        arena.push(TrieNode::new(text.to_string()));
        arena[parent_idx].children.push(idx);
        arena[parent_idx]
            .child_by_label
            .insert(text.to_string(), idx);
        idx
    };

    if dedupe_background && loc.context == LocationContext::Background {
        let already = arena[child_idx].locations.iter().any(|existing| {
            existing.context == LocationContext::Background
                && existing.feature_idx == loc.feature_idx
                && existing.line_number == loc.line_number
        });
        if !already {
            arena[child_idx].locations.push(loc);
        }
    } else {
        arena[child_idx].locations.push(loc);
    }

    child_idx
}

/// Mutable state shared while converting the trie arena into `TreeItem`s.
struct BuildItemsCtx<'a> {
    arena: &'a [TrieNode],
    root_label: &'a str,
    next_id: &'a mut usize,
    node_paths: &'a mut HashMap<String, Vec<String>>,
    node_locations: &'a mut HashMap<String, Vec<NodeLocation>>,
    node_labels: &'a mut HashMap<String, String>,
    /// Cached map from node ID to highlight color.
    label_colors: &'a HashMap<String, Color>,
    /// Active filter, if any.
    filter: &'a Option<MindMapFilter>,
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

    // Apply highlight styling if this node has a color
    let label_text = if let Some(&color) = ctx.label_colors.get(&id) {
        TuiText::styled(label, Style::default().fg(color))
    } else {
        TuiText::from(label)
    };

    TreeItem::new(id, label_text, children).ok()
}

/// Creates a [`TreeState`] with all nodes collapsed by default; only the root is selected.
pub fn init_tree_state(_index: &MindMapIndex) -> TreeState<String> {
    let mut state = TreeState::default();
    state.select(vec!["root".to_string()]);
    state
}

/// Returns the last identifier in the current selection path (the selected node's own ID).
pub fn selected_node_id(state: &TreeState<String>) -> Option<&str> {
    state.selected().last().map(|s| s.as_str())
}

/// Returns the tree path for a node identifier.
pub fn node_id_to_path(id: &str, index: &MindMapIndex) -> Option<Vec<String>> {
    index.path_for(id).cloned()
}

/// Resolves a node identifier + location index to `(feature_idx, line_number)`.
pub fn parse_node_line_number(
    id: &str,
    index: &MindMapIndex,
    location_index: usize,
) -> Option<(usize, usize)> {
    let locations = index.locations_for(id)?;
    let loc = locations.get(location_index)?;
    Some((loc.feature_idx, loc.line_number))
}

/// Finds the closest node to a given editor cursor line within a feature.
pub fn find_closest_node(
    index: &MindMapIndex,
    feature_idx: usize,
    cursor_line_1based: usize,
) -> Option<NodeMatch> {
    let list = index.occurrences_for_feature(feature_idx)?;
    let mut best: Option<NodeMatch> = None;
    let mut best_dist = usize::MAX;

    for occ in list {
        let d = cursor_line_1based.abs_diff(occ.line_number);
        if d < best_dist {
            best_dist = d;
            best = Some(NodeMatch {
                node_id: occ.node_id.clone(),
                location_index: occ.location_index,
            });
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{build_index, init_tree_state, selected_node_context};
    use crate::gherkin::{self, BddProject};

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

    #[test]
    fn test_init_tree_state_collapses_all_nodes_by_default() {
        let project = sample_project();
        let index = build_index(&project);
        assert!(
            index.items.len() == 1 && !index.node_paths.is_empty(),
            "index should have built non-trivial node paths for the test"
        );

        let state = init_tree_state(&index);

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
        let index = build_index(&project);
        let state = init_tree_state(&index);

        let ctx = selected_node_context(&state, &index).expect("root should be selectable");
        assert_eq!(ctx.step_text, ".", "root label is the project dir name");
        assert_eq!(ctx.path_labels, &["."], "root path is just the root label");
        assert_eq!(ctx.location_count, 0, "root has no source locations");
    }
}
