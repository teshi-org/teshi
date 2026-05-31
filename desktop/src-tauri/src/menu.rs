//! Native application menu (File > Open Project / Open Recent).

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Result, Wry};

use crate::app_data::get_recent_projects;

const MENU_ID_OPEN_PROJECT: &str = "open_project";
const MENU_ID_RECENT_EMPTY: &str = "recent_empty";
const RECENT_ID_PREFIX: &str = "recent:";

/// Builds the root application menu with a dynamic Open Recent submenu.
pub fn build_app_menu(app: &AppHandle, recent_paths: &[String]) -> Result<Menu<Wry>> {
    let accelerator = if cfg!(target_os = "macos") {
        "Cmd+O"
    } else {
        "Ctrl+O"
    };

    let open_project = MenuItemBuilder::with_id(MENU_ID_OPEN_PROJECT, "Open Project…")
        .accelerator(accelerator)
        .build(app)?;

    let mut recent_builder = SubmenuBuilder::new(app, "Open Recent");
    if recent_paths.is_empty() {
        let empty = MenuItemBuilder::with_id(MENU_ID_RECENT_EMPTY, "No recent projects")
            .enabled(false)
            .build(app)?;
        recent_builder = recent_builder.item(&empty);
    } else {
        for (idx, path) in recent_paths.iter().enumerate() {
            let id = format!("{RECENT_ID_PREFIX}{idx}");
            recent_builder = recent_builder.text(&id, truncate_menu_label(path));
        }
    }
    let open_recent = recent_builder.build()?;

    let file_submenu = SubmenuBuilder::new(app, "File")
        .item(&open_project)
        .item(&open_recent)
        .build()?;

    MenuBuilder::new(app).item(&file_submenu).build()
}

/// Rebuilds the application menu from persisted recent projects.
pub fn rebuild_app_menu(app: &AppHandle) -> Result<()> {
    let recent = get_recent_projects().unwrap_or_default();
    let menu = build_app_menu(app, &recent)?;
    app.set_menu(menu)?;
    Ok(())
}

/// Handles native menu clicks and forwards them to the webview as events.
pub fn handle_menu_event(app: &AppHandle, event_id: &str) {
    match event_id {
        MENU_ID_OPEN_PROJECT => {
            let _ = app.emit("menu-open-project", ());
        }
        MENU_ID_RECENT_EMPTY => {}
        id if id.starts_with(RECENT_ID_PREFIX) => {
            let Some(idx_str) = id.strip_prefix(RECENT_ID_PREFIX) else {
                return;
            };
            let Ok(idx) = idx_str.parse::<usize>() else {
                return;
            };
            if let Ok(paths) = get_recent_projects() {
                if let Some(path) = paths.get(idx) {
                    let _ = app.emit("menu-open-recent", path.clone());
                }
            }
        }
        _ => {}
    }
}

fn truncate_menu_label(path: &str) -> String {
    const MAX_CHARS: usize = 60;
    let char_count = path.chars().count();
    if char_count <= MAX_CHARS {
        return path.to_string();
    }
    let tail: String = path
        .chars()
        .skip(char_count.saturating_sub(MAX_CHARS - 1))
        .collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::truncate_menu_label;

    #[test]
    fn truncate_menu_label_short_path_unchanged() {
        assert_eq!(truncate_menu_label("C:\\proj"), "C:\\proj");
    }

    #[test]
    fn truncate_menu_label_long_path_truncated() {
        let long = "C:\\".to_string() + &"a".repeat(80);
        let out = truncate_menu_label(&long);
        assert!(out.chars().count() <= 60);
        assert!(out.starts_with('…'));
    }
}
