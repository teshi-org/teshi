//! Mind-map tree construction and state for the three-stage MindMap tab.
//!
//! Uses `tui-tree-widget`'s [`TreeState`] for navigation and open/close tracking,
//! which correctly handles visibility of nested nodes.

use crate::gherkin::BddProject;
use crate::step_index::StepIndex;

pub use tui_tree_widget::TreeState;

/// Creates a [`TreeState`] with default open nodes: root, all files, all features, and all scenarios.
pub fn init_tree_state(project: &BddProject) -> TreeState<String> {
    let mut state = TreeState::default();

    state.open(vec!["root".to_string()]);

    for (fi, feature) in project.features.iter().enumerate() {
        let file_path = vec!["root".to_string(), format!("file-{fi}")];
        state.open(file_path.clone());

        let mut feat_path = file_path;
        feat_path.push(format!("feat-{fi}"));
        state.open(feat_path.clone());

        if feature.background.is_some() {
            let mut bg_path = feat_path.clone();
            bg_path.push(format!("bg-{fi}"));
            state.open(bg_path);
        }

        for (sci, _) in feature.scenarios.iter().enumerate() {
            let mut sc_path = feat_path.clone();
            sc_path.push(format!("sc-{fi}-{sci}"));
            state.open(sc_path);
        }
    }

    state.select(vec!["root".to_string()]);
    state
}

/// Returns the last identifier in the current selection path (the selected node's own ID).
pub fn selected_node_id(state: &TreeState<String>) -> Option<&str> {
    state.selected().last().map(|s| s.as_str())
}

/// Computes the full tree path (root → … → node) for a given node identifier.
///
/// Required by `TreeState::select` and `TreeState::open` which use path-based addressing.
pub fn node_id_to_path(id: &str) -> Vec<String> {
    // step-{fi}-{sci}-{si}
    if let Some(rest) = id.strip_prefix("step-") {
        let parts: Vec<&str> = rest.splitn(3, '-').collect();
        if parts.len() == 3 {
            let fi = parts[0];
            let sci = parts[1];
            return vec![
                "root".to_string(),
                format!("file-{fi}"),
                format!("feat-{fi}"),
                format!("sc-{fi}-{sci}"),
                id.to_string(),
            ];
        }
    }
    // bg-{fi}-s{si}  (background step)
    if id.starts_with("bg-") && id.contains("-s") {
        if let Some(rest) = id.strip_prefix("bg-") {
            let parts: Vec<&str> = rest.splitn(2, "-s").collect();
            if parts.len() == 2 {
                let fi = parts[0];
                return vec![
                    "root".to_string(),
                    format!("file-{fi}"),
                    format!("feat-{fi}"),
                    format!("bg-{fi}"),
                    id.to_string(),
                ];
            }
        }
    }
    // bg-{fi}  (background header)
    if let Some(rest) = id.strip_prefix("bg-") {
        if !rest.contains('-') {
            let fi = rest;
            return vec![
                "root".to_string(),
                format!("file-{fi}"),
                format!("feat-{fi}"),
                id.to_string(),
            ];
        }
    }
    // sc-{fi}-{sci}
    if let Some(rest) = id.strip_prefix("sc-") {
        let parts: Vec<&str> = rest.splitn(2, '-').collect();
        if parts.len() == 2 {
            let fi = parts[0];
            return vec![
                "root".to_string(),
                format!("file-{fi}"),
                format!("feat-{fi}"),
                id.to_string(),
            ];
        }
    }
    // feat-{fi}
    if let Some(fi) = id.strip_prefix("feat-") {
        return vec![
            "root".to_string(),
            format!("file-{fi}"),
            id.to_string(),
        ];
    }
    // file-{fi}
    if id.starts_with("file-") {
        return vec!["root".to_string(), id.to_string()];
    }
    // root
    vec![id.to_string()]
}

/// Returns `true` when the node is a leaf (step or background step) that has no children.
pub fn is_leaf_node(id: &str) -> bool {
    id.starts_with("step-") || (id.starts_with("bg-") && id.contains("-s"))
}

/// Resolves a node identifier to `(feature_idx, line_number)`.
pub fn parse_node_line_number(id: &str, project: &BddProject) -> Option<(usize, usize)> {
    // step-{fi}-{sci}-{si}
    if let Some(rest) = id.strip_prefix("step-") {
        let parts: Vec<&str> = rest.splitn(3, '-').collect();
        if parts.len() == 3 {
            let fi: usize = parts[0].parse().ok()?;
            let sci: usize = parts[1].parse().ok()?;
            let si: usize = parts[2].parse().ok()?;
            let feat = project.features.get(fi)?;
            let sc = feat.scenarios.get(sci)?;
            let step = sc.steps.get(si)?;
            return Some((fi, step.line_number));
        }
    }
    // bg-{fi}-s{si}
    if let Some(rest) = id.strip_prefix("bg-") {
        if rest.contains("-s") {
            let parts: Vec<&str> = rest.splitn(2, "-s").collect();
            if parts.len() == 2 {
                let fi: usize = parts[0].parse().ok()?;
                let si: usize = parts[1].parse().ok()?;
                let feat = project.features.get(fi)?;
                let bg = feat.background.as_ref()?;
                let step = bg.steps.get(si)?;
                return Some((fi, step.line_number));
            }
        } else {
            let fi: usize = rest.parse().ok()?;
            let feat = project.features.get(fi)?;
            let bg = feat.background.as_ref()?;
            return Some((fi, bg.line_number));
        }
    }
    // sc-{fi}-{sci}
    if let Some(rest) = id.strip_prefix("sc-") {
        let parts: Vec<&str> = rest.splitn(2, '-').collect();
        if parts.len() == 2 {
            let fi: usize = parts[0].parse().ok()?;
            let sci: usize = parts[1].parse().ok()?;
            let feat = project.features.get(fi)?;
            let sc = feat.scenarios.get(sci)?;
            return Some((fi, sc.line_number));
        }
    }
    // feat-{fi}
    if let Some(rest) = id.strip_prefix("feat-") {
        let fi: usize = rest.parse().ok()?;
        return Some((fi, 1));
    }
    // file-{fi}
    if let Some(rest) = id.strip_prefix("file-") {
        let fi: usize = rest.parse().ok()?;
        return Some((fi, 1));
    }
    None
}

/// Builds the tree widget items from the project for rendering with `tui-tree-widget`.
pub fn build_tree_items<'a>(
    project: &'a BddProject,
    step_index: &'a StepIndex,
) -> Vec<tui_tree_widget::TreeItem<'a, String>> {
    use tui_tree_widget::TreeItem;

    let root_label = project
        .root_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.root_dir.display().to_string());

    let mut file_items = Vec::new();
    for (fi, feature) in project.features.iter().enumerate() {
        let file_label = feature
            .file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| feature.file_path.display().to_string());

        let mut feat_children = Vec::new();

        // Background
        if let Some(bg) = &feature.background {
            let bg_steps: Vec<TreeItem<'_, String>> = bg
                .steps
                .iter()
                .enumerate()
                .map(|(si, step)| {
                    let label = step_label(&step.text, step_index);
                    TreeItem::new_leaf(format!("bg-{fi}-s{si}"), label)
                })
                .collect();
            feat_children.push(
                TreeItem::new(format!("bg-{fi}"), "Background:".to_string(), bg_steps)
                    .expect("tree item construction"),
            );
        }

        // Scenarios
        for (sci, scenario) in feature.scenarios.iter().enumerate() {
            let sc_label = match scenario.kind {
                crate::gherkin::ScenarioKind::Scenario => {
                    format!("Scenario: {}", scenario.name)
                }
                crate::gherkin::ScenarioKind::ScenarioOutline => {
                    format!("Scenario Outline: {}", scenario.name)
                }
            };
            let step_items: Vec<TreeItem<'_, String>> = scenario
                .steps
                .iter()
                .enumerate()
                .map(|(si, step)| {
                    let label = step_label(&step.text, step_index);
                    TreeItem::new_leaf(format!("step-{fi}-{sci}-{si}"), label)
                })
                .collect();
            feat_children.push(
                TreeItem::new(format!("sc-{fi}-{sci}"), sc_label, step_items)
                    .expect("tree item construction"),
            );
        }

        let feat_label = format!("Feature: {}", feature.name);
        let feat_item = TreeItem::new(format!("feat-{fi}"), feat_label, feat_children)
            .expect("tree item construction");
        let file_item = TreeItem::new(format!("file-{fi}"), file_label, vec![feat_item])
            .expect("tree item construction");
        file_items.push(file_item);
    }

    let root =
        TreeItem::new("root".to_string(), root_label, file_items).expect("tree item construction");
    vec![root]
}

/// Formats a step node label: body text + optional `[×N]` reuse suffix.
fn step_label(step_text: &str, step_index: &StepIndex) -> String {
    let count = step_index.reuse_count(step_text);
    if count >= 2 {
        format!("{step_text} [×{count}]")
    } else {
        step_text.to_string()
    }
}

/// Finds the node ID closest to a given editor cursor line within a feature.
pub fn find_closest_node_id(
    project: &BddProject,
    feature_idx: usize,
    cursor_line_1based: usize,
) -> Option<String> {
    let feat = project.features.get(feature_idx)?;
    let mut best_id: Option<String> = None;
    let mut best_dist = usize::MAX;

    if let Some(bg) = &feat.background {
        let d = cursor_line_1based.abs_diff(bg.line_number);
        if d < best_dist {
            best_dist = d;
            best_id = Some(format!("bg-{feature_idx}"));
        }
        for (si, step) in bg.steps.iter().enumerate() {
            let d = cursor_line_1based.abs_diff(step.line_number);
            if d < best_dist {
                best_dist = d;
                best_id = Some(format!("bg-{feature_idx}-s{si}"));
            }
        }
    }

    for (sci, sc) in feat.scenarios.iter().enumerate() {
        let d = cursor_line_1based.abs_diff(sc.line_number);
        if d < best_dist {
            best_dist = d;
            best_id = Some(format!("sc-{feature_idx}-{sci}"));
        }
        for (si, step) in sc.steps.iter().enumerate() {
            let d = cursor_line_1based.abs_diff(step.line_number);
            if d < best_dist {
                best_dist = d;
                best_id = Some(format!("step-{feature_idx}-{sci}-{si}"));
            }
        }
    }

    best_id
}
