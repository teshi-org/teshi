//! Mind-map tree construction data types and pure logic.
//!
//! Builds a step-sequence prefix tree (trie) so shared step prefixes collapse
//! into a single path. Each step node records all source locations that map
//! to that path, enabling location selection in the preview panel.
//!
//! TreeItem rendering (ratatui/tui-tree-widget) lives in the TUI crate.

use std::collections::HashMap;

use crate::gherkin::BddProject;
use crate::gherkin_lang::StepKeywordType;

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
    pub color: HighlightColor,
}

/// A named color for highlighting — kept UI-crate-agnostic so core has no
/// ratatui dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightColor {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Black,
}

/// A filter that restricts which nodes are visible in the tree.
#[derive(Debug, Clone)]
pub enum MindMapFilter {
    /// Show only nodes whose label contains the string (case-insensitive),
    /// plus ancestors to preserve tree structure.
    NameContains(String),
}

/// Node highlighting category based on relationship to the selected node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightCategory {
    /// The currently selected node.
    Selected,
    /// Ancestors from parent up to root (warm trace path).
    Ancestor,
    /// Descendants: children, grandchildren, etc. (growth direction).
    Descendant,
    /// Siblings sharing the same parent as the selected node.
    Sibling,
    /// All other nodes unrelated to the selection.
    GrayedOut,
}

#[derive(Debug, Clone)]
pub struct TrieNode {
    pub label: String,
    pub children: Vec<usize>,
    pub child_by_label: HashMap<String, usize>,
    pub locations: Vec<NodeLocation>,
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

/// Precomputed tree data and lookup maps for MindMap behavior.
///
/// Tree items for the TUI widget are stored separately by the TUI crate
/// (via a wrapper that derefs to this struct).
#[derive(Debug, Clone)]
pub struct MindMapIndex {
    pub node_paths: HashMap<String, Vec<String>>,
    pub node_locations: HashMap<String, Vec<NodeLocation>>,
    pub node_labels: HashMap<String, String>,
    pub occurrences_by_feature: Vec<Vec<NodeOccurrence>>,
    /// Preserved trie arena for rebuilding items with different styling.
    pub arena: Vec<TrieNode>,
    /// Active highlight rules.
    pub highlights: Vec<HighlightRule>,
    /// Maps child node ID → parent node ID. Root has no entry.
    pub parent_map: HashMap<String, String>,
    /// Maps node ID → list of child node IDs.
    pub children_map: HashMap<String, Vec<String>>,
    /// Per-node highlight category computed from the current selection.
    pub node_categories: HashMap<String, HighlightCategory>,
    /// Active filter.
    pub filter: Option<MindMapFilter>,
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

    /// Whether the node identified by `id` has any children.
    pub fn has_children(&self, id: &str) -> bool {
        self.children_map.get(id).is_some_and(|c| !c.is_empty())
    }

    /// Returns the id of the previous sibling of `id`, if any.
    /// Siblings are nodes that share the same parent.
    pub fn prev_sibling(&self, id: &str) -> Option<String> {
        let parent = self.parent_map.get(id)?;
        let siblings = self.children_map.get(parent)?;
        let pos = siblings.iter().position(|s| s == id)?;
        if pos > 0 {
            Some(siblings[pos - 1].clone())
        } else {
            None
        }
    }

    /// Returns the id of the next sibling of `id`, if any.
    /// Siblings are nodes that share the same parent.
    pub fn next_sibling(&self, id: &str) -> Option<String> {
        let parent = self.parent_map.get(id)?;
        let siblings = self.children_map.get(parent)?;
        let pos = siblings.iter().position(|s| s == id)?;
        siblings.get(pos + 1).cloned()
    }

    /// Lists node occurrences ordered for closest-line lookup within one feature file.
    pub fn occurrences_for_feature(&self, feature_idx: usize) -> Option<&[NodeOccurrence]> {
        self.occurrences_by_feature
            .get(feature_idx)
            .map(|v| v.as_slice())
    }

    /// Computes a per-node [`HighlightCategory`] map based on the selected node.
    pub fn compute_node_categories(&self, selected_id: &str) -> HashMap<String, HighlightCategory> {
        let mut categories: HashMap<String, HighlightCategory> = HashMap::new();

        // Selected
        categories.insert(selected_id.to_string(), HighlightCategory::Selected);

        // Ancestors — walk up parent_map
        let mut current = selected_id.to_string();
        while let Some(parent) = self.parent_map.get(&current) {
            categories.insert(parent.clone(), HighlightCategory::Ancestor);
            current = parent.clone();
        }

        // Siblings — direct children of the selected node's parent
        if let Some(parent) = self.parent_map.get(selected_id)
            && let Some(siblings) = self.children_map.get(parent)
        {
            for sib in siblings {
                categories
                    .entry(sib.clone())
                    .or_insert(HighlightCategory::Sibling);
            }
        }

        // Descendants — stack-based traversal from selected_id
        let mut stack: Vec<String> = vec![selected_id.to_string()];
        while let Some(id) = stack.pop() {
            if let Some(children) = self.children_map.get(&id) {
                for child in children {
                    categories
                        .entry(child.clone())
                        .or_insert(HighlightCategory::Descendant);
                    stack.push(child.clone());
                }
            }
        }

        // Remaining nodes → GrayedOut
        for id in self.node_labels.keys() {
            categories
                .entry(id.clone())
                .or_insert(HighlightCategory::GrayedOut);
        }

        categories
    }
}

/// Evaluate whether a label matches a match condition.
pub fn evaluate_condition(cond: &MatchCondition, label: &str) -> bool {
    match cond {
        MatchCondition::StepContains(text) => label.to_lowercase().contains(&text.to_lowercase()),
    }
}

/// Check whether a node label matches the active filter.
pub fn matches_filter(filter: &MindMapFilter, label: &str) -> bool {
    match filter {
        MindMapFilter::NameContains(text) => label.to_lowercase().contains(&text.to_lowercase()),
    }
}

/// Parse a named color string into a [`HighlightColor`].
pub fn parse_color(s: &str) -> Option<HighlightColor> {
    match s.to_lowercase().as_str() {
        "red" => Some(HighlightColor::Red),
        "green" => Some(HighlightColor::Green),
        "yellow" => Some(HighlightColor::Yellow),
        "blue" => Some(HighlightColor::Blue),
        "magenta" => Some(HighlightColor::Magenta),
        "cyan" => Some(HighlightColor::Cyan),
        "white" => Some(HighlightColor::White),
        "black" => Some(HighlightColor::Black),
        _ => None,
    }
}

/// Builds the MindMap index from a parsed project.
///
/// The returned index has an empty `items` vector — the TUI crate
/// must call its tree-item builder afterwards.
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
            let mut node_idx = 0usize;
            let mut parent_idx = 0usize;
            let mut effective_keyword: Option<StepKeywordType> = None;
            let mut last_when_parent: usize = 0;
            let mut has_then_since_when = false;

            for (text, loc) in &bg_steps {
                parent_idx = node_idx;
                node_idx = insert_step(&mut arena, node_idx, text, *loc, true);
            }

            for (sti, step) in scenario.steps.iter().enumerate() {
                let loc = NodeLocation {
                    feature_idx: fi,
                    context: LocationContext::Scenario(sci),
                    step_idx: sti,
                    line_number: step.line_number,
                };

                let kw_type = step.keyword_type;

                let parent = if kw_type == StepKeywordType::When && has_then_since_when {
                    last_when_parent
                } else if matches!(kw_type, StepKeywordType::And | StepKeywordType::But) {
                    if effective_keyword == Some(StepKeywordType::Then) {
                        parent_idx
                    } else {
                        node_idx
                    }
                } else if Some(kw_type) == effective_keyword {
                    if kw_type == StepKeywordType::Then {
                        parent_idx
                    } else {
                        node_idx
                    }
                } else {
                    node_idx
                };

                let new_idx = insert_step(&mut arena, parent, &step.text, loc, false);

                match kw_type {
                    StepKeywordType::And | StepKeywordType::But => {
                        node_idx = new_idx;
                    }
                    StepKeywordType::When => {
                        if has_then_since_when {
                            parent_idx = last_when_parent;
                            node_idx = new_idx;
                            has_then_since_when = false;
                        } else {
                            last_when_parent = parent;
                            parent_idx = node_idx;
                            node_idx = new_idx;
                        }
                        effective_keyword = Some(StepKeywordType::When);
                    }
                    StepKeywordType::Then => {
                        has_then_since_when = true;
                        if effective_keyword != Some(StepKeywordType::Then) {
                            parent_idx = node_idx;
                        }
                        node_idx = new_idx;
                        effective_keyword = Some(StepKeywordType::Then);
                    }
                    StepKeywordType::Given => {
                        parent_idx = node_idx;
                        node_idx = new_idx;
                        effective_keyword = Some(StepKeywordType::Given);
                    }
                }
            }
        }
    }

    let mut node_paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut node_locations: HashMap<String, Vec<NodeLocation>> = HashMap::new();
    let mut node_labels: HashMap<String, String> = HashMap::new();
    let mut parent_map: HashMap<String, String> = HashMap::new();
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut next_id = 0usize;

    let mut ctx = PopulateCtx {
        arena: &arena,
        root_label: &root_label,
        next_id: &mut next_id,
        node_paths: &mut node_paths,
        node_locations: &mut node_locations,
        node_labels: &mut node_labels,
        parent_map: &mut parent_map,
        children_map: &mut children_map,
    };
    traverse_and_populate(0, &mut ctx, &[]);

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
        node_paths,
        node_locations,
        node_labels,
        occurrences_by_feature,
        arena,
        highlights: Vec::new(),
        parent_map,
        children_map,
        node_categories: HashMap::new(),
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

/// Mutable state for trie-arena traversal to populate index maps.
struct PopulateCtx<'a> {
    arena: &'a [TrieNode],
    root_label: &'a str,
    next_id: &'a mut usize,
    node_paths: &'a mut HashMap<String, Vec<String>>,
    node_locations: &'a mut HashMap<String, Vec<NodeLocation>>,
    node_labels: &'a mut HashMap<String, String>,
    parent_map: &'a mut HashMap<String, String>,
    children_map: &'a mut HashMap<String, Vec<String>>,
}

/// Walk the trie arena and populate node_paths, node_labels, node_locations,
/// parent_map, and children_map. Returns the node id.
fn traverse_and_populate(
    node_idx: usize,
    ctx: &mut PopulateCtx<'_>,
    parent_path: &[String],
) -> String {
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
    ctx.node_labels.insert(id.clone(), label);

    for &child_idx in &ctx.arena[node_idx].children {
        traverse_and_populate(child_idx, ctx, &path);
    }

    id
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{build_index, find_closest_node};
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
    fn test_index_builds_node_paths() {
        let project = sample_project();
        let index = build_index(&project);
        assert!(
            !index.node_paths.is_empty(),
            "index should have built non-trivial node paths for the test"
        );
    }

    #[test]
    fn test_find_closest_node() {
        let project = sample_project();
        let index = build_index(&project);
        let node = find_closest_node(&index, 0, 3);
        assert!(node.is_some(), "should find a node near line 3");
    }
}
