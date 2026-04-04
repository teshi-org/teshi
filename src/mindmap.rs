//! Mind-map tree construction and state for the three-stage MindMap tab.
//!
//! Builds a step-sequence prefix tree (trie) so shared step prefixes collapse
//! into a single path. Each step node records all source locations that map
//! to that path, enabling location selection in the preview panel.

use std::collections::HashMap;

use crate::gherkin::{BddProject, ScenarioKind};

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

/// Result of a closest-node lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeMatch {
    pub node_id: String,
    pub location_index: usize,
}

/// Precomputed tree items and lookup maps for MindMap behavior.
#[derive(Debug, Clone)]
pub struct MindMapIndex {
    pub items: Vec<tui_tree_widget::TreeItem<'static, String>>,
    node_paths: HashMap<String, Vec<String>>,
    node_locations: HashMap<String, Vec<NodeLocation>>,
    node_child_counts: HashMap<String, usize>,
    occurrences_by_feature: Vec<Vec<NodeOccurrence>>,
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

    /// Returns whether `id` is a leaf in the MindMap widget (no child trie nodes).
    pub fn is_leaf(&self, id: &str) -> bool {
        self.node_child_counts.get(id).copied().unwrap_or(0) == 0
    }

    /// Lists node occurrences ordered for closest-line lookup within one feature file.
    pub fn occurrences_for_feature(&self, feature_idx: usize) -> Option<&[NodeOccurrence]> {
        self.occurrences_by_feature
            .get(feature_idx)
            .map(|v| v.as_slice())
    }
}

#[derive(Debug, Clone)]
struct TrieNode {
    label: String,
    children: Vec<usize>,
    child_by_label: HashMap<String, usize>,
    locations: Vec<NodeLocation>,
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
    let mut node_child_counts: HashMap<String, usize> = HashMap::new();
    let mut next_id = 0usize;

    let mut ctx = BuildItemsCtx {
        arena: &arena,
        root_label: &root_label,
        next_id: &mut next_id,
        node_paths: &mut node_paths,
        node_locations: &mut node_locations,
        node_child_counts: &mut node_child_counts,
    };
    let root_item = build_items(0, &mut ctx, &[]);

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
        node_child_counts,
        occurrences_by_feature,
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
    node_child_counts: &'a mut HashMap<String, usize>,
}

fn build_items(
    node_idx: usize,
    ctx: &mut BuildItemsCtx<'_>,
    parent_path: &[String],
) -> tui_tree_widget::TreeItem<'static, String> {
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
    ctx.node_child_counts
        .insert(id.clone(), ctx.arena[node_idx].children.len());

    let label = if node_idx == 0 {
        ctx.root_label.to_string()
    } else {
        ctx.arena[node_idx].label.clone()
    };

    let mut children = Vec::new();
    for &child_idx in &ctx.arena[node_idx].children {
        children.push(build_items(child_idx, ctx, &path));
    }

    // `TreeItem::new` only fails on invalid widget invariants; our tree is built consistently.
    tui_tree_widget::TreeItem::new(id, label, children).expect("tree item construction")
}

/// Creates a [`TreeState`] with default open nodes (root + all non-leaf nodes).
pub fn init_tree_state(index: &MindMapIndex) -> TreeState<String> {
    let mut state = TreeState::default();

    if let Some(path) = index.path_for("root") {
        state.open(path.clone());
    }

    for (id, path) in &index.node_paths {
        if id == "root" {
            continue;
        }
        if !index.is_leaf(id) {
            state.open(path.clone());
        }
    }

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

/// Returns `true` when the node has no children.
pub fn is_leaf_node(id: &str, index: &MindMapIndex) -> bool {
    index.is_leaf(id)
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

/// Formats a location label for the preview location selector.
pub fn format_location_label(project: &BddProject, location: &NodeLocation) -> String {
    let feature = project
        .features
        .get(location.feature_idx)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "Unknown Feature".to_string());

    let scenario_label = match location.context {
        LocationContext::Background => "Background".to_string(),
        LocationContext::Scenario(sci) => project
            .features
            .get(location.feature_idx)
            .and_then(|f| f.scenarios.get(sci))
            .map(|sc| match sc.kind {
                ScenarioKind::Scenario => format!("Scenario: {}", sc.name),
                ScenarioKind::ScenarioOutline => format!("Scenario Outline: {}", sc.name),
            })
            .unwrap_or_else(|| "Scenario".to_string()),
    };

    format!("{} > {}", feature, scenario_label)
}
