//! Thin Tauri command adapters over `teshi-runtime`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use teshi_core::FeatureRenderPayload;
use teshi_engine::llm::{call_llm_with_tool, llm_config_from_env};
use teshi_engine::{
    abandon_pending_locator, check_project_switch_allowed, confirm_locator, get_active_step,
    get_pending_locator, get_project_root as runtime_project_root, get_recent_projects,
    highlight_locator, list_dir as runtime_list_dir, load_project_settings,
    open_dialog_default_dir, open_project as runtime_open_project, reject_locator,
    render_feature as runtime_render_feature, resize_terminal as runtime_resize,
    set_browser_active, set_terminal_active, spawn_terminal as runtime_spawn_terminal,
    start_browser_sidecar as runtime_start_browser, step_binding_statuses,
    stop_browser_sidecar as runtime_stop_browser, stop_terminal as runtime_stop_terminal,
    sync_active_step, teardown_runtime as runtime_teardown, unbind_step,
    write_terminal as runtime_write_terminal, ActiveStep, BrowserError, BrowserMode,
    BrowserStartResult, DirEntry, PendingLocator, ProjectSettings, StepBinding, StepBindingStatus,
    TeshiEngine,
};

#[tauri::command]
pub async fn check_project_switch_allowed_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
) -> Result<bool, String> {
    Ok(check_project_switch_allowed(&rt))
}

#[tauri::command]
pub async fn open_project(rt: State<'_, Arc<TeshiEngine>>, path: String) -> Result<(), String> {
    runtime_open_project(Arc::clone(&rt), path).await
}

#[tauri::command]
pub fn list_dir(rt: State<'_, Arc<TeshiEngine>>, path: String) -> Result<Vec<DirEntry>, String> {
    runtime_list_dir(&rt, path)
}

#[tauri::command]
pub fn get_project_root(rt: State<'_, Arc<TeshiEngine>>) -> Option<String> {
    runtime_project_root(&rt)
}

#[tauri::command]
pub async fn render_feature_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
    path: String,
) -> Result<FeatureRenderPayload, String> {
    runtime_render_feature(&rt, path)
}

#[tauri::command]
pub async fn sync_active_step_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
    feature_path: String,
    step_line: u32,
) -> Result<ActiveStep, String> {
    sync_active_step(&rt, feature_path, step_line).await
}

#[tauri::command]
pub fn get_active_step_cmd(rt: State<'_, Arc<TeshiEngine>>) -> Result<Option<ActiveStep>, String> {
    get_active_step(&rt)
}

#[tauri::command]
pub fn get_pending_locator_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
) -> Result<Option<PendingLocator>, String> {
    get_pending_locator(&rt)
}

#[tauri::command]
pub fn get_step_binding_statuses_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
    feature_path: String,
) -> Result<Vec<StepBindingStatus>, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    step_binding_statuses(&project_root, &feature_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn confirm_locator_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
    candidate_rank: u32,
    edited_value: Option<String>,
) -> Result<(), String> {
    confirm_locator(&rt, candidate_rank, edited_value).await
}

#[tauri::command]
pub async fn reject_locator_cmd(rt: State<'_, Arc<TeshiEngine>>) -> Result<(), String> {
    reject_locator(&rt).await
}

#[tauri::command]
pub async fn highlight_locator_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
    selector: String,
) -> Result<(), String> {
    highlight_locator(&rt, selector).await
}

#[tauri::command]
pub async fn abandon_pending_locator_cmd(rt: State<'_, Arc<TeshiEngine>>) -> Result<(), String> {
    abandon_pending_locator(&rt).await
}

#[tauri::command]
pub async fn unbind_step_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
    feature_path: String,
    step_line: u32,
) -> Result<Option<StepBinding>, String> {
    unbind_step(&rt, feature_path, step_line).await
}

#[tauri::command]
pub fn get_project_settings_cmd(
    rt: State<'_, Arc<TeshiEngine>>,
) -> Result<ProjectSettings, String> {
    let project_root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    load_project_settings(&project_root).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_browser_sidecar(
    rt: State<'_, Arc<TeshiEngine>>,
    mode: String,
) -> Result<BrowserStartResult, BrowserError> {
    let mode = match mode.as_str() {
        "chrome" => BrowserMode::Chrome,
        "winapp" => BrowserMode::WinApp,
        _ => BrowserMode::Embedded,
    };
    runtime_start_browser(Arc::clone(&rt), mode).await
}

#[tauri::command]
pub async fn stop_browser_sidecar(rt: State<'_, Arc<TeshiEngine>>) -> Result<(), String> {
    runtime_stop_browser(&rt).await
}

#[tauri::command]
pub async fn spawn_terminal(
    rt: State<'_, Arc<TeshiEngine>>,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    runtime_spawn_terminal(Arc::clone(&rt), cols, rows).await
}

#[tauri::command]
pub fn stop_terminal(rt: State<'_, Arc<TeshiEngine>>) -> Result<(), String> {
    runtime_stop_terminal(&rt)
}

#[tauri::command]
pub fn resize_terminal(
    rt: State<'_, Arc<TeshiEngine>>,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    runtime_resize(&rt, cols, rows)
}

#[tauri::command]
pub fn write_terminal(rt: State<'_, Arc<TeshiEngine>>, data: String) -> Result<(), String> {
    runtime_write_terminal(&rt, data)
}

#[tauri::command]
pub async fn teardown_runtime(rt: State<'_, Arc<TeshiEngine>>) -> Result<(), String> {
    runtime_teardown(&rt).await
}

#[tauri::command]
pub fn set_browser_active_cmd(rt: State<'_, Arc<TeshiEngine>>, active: bool) {
    set_browser_active(&rt, active);
}

#[tauri::command]
pub fn set_terminal_active_cmd(rt: State<'_, Arc<TeshiEngine>>, active: bool) {
    set_terminal_active(&rt, active);
}

#[tauri::command]
pub fn get_recent_projects_cmd() -> Result<Vec<String>, String> {
    get_recent_projects()
}

/// Opens the native folder picker and returns the selected path.
#[tauri::command]
pub async fn open_project_dir(app: AppHandle) -> Result<Option<String>, String> {
    let default = open_dialog_default_dir();
    let picked = app
        .dialog()
        .file()
        .set_title("Open Project")
        .set_directory(default.unwrap_or_else(|| PathBuf::from(".")))
        .blocking_pick_folder();

    Ok(picked.map(|p| p.to_string()))
}

/// Read a text file from disk (used for `.teshi/cdp-endpoint.json` polling).
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))
}

/// Read a file and return it as a data URL (base64-encoded with MIME prefix).
#[tauri::command]
pub fn read_file_base64(path: String) -> Result<String, String> {
    let data = std::fs::read(&path).map_err(|e| format!("read {path}: {e}"))?;
    let mime = if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".png") {
        "image/png"
    } else {
        "application/octet-stream"
    };
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
    Ok(format!("data:{mime};base64,{b64}"))
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GenerateRequirementsSegment {
    pub id: String,
    pub text: String,
    pub pos: (usize, usize),
}

#[derive(Debug, serde::Serialize)]
pub struct GenerateRequirementsResult {
    pub slug: String,
    pub segments: Vec<GenerateRequirementsSegment>,
    pub mindmap_xml: String,
    pub mock_html: String,
}

#[tauri::command]
pub async fn generate_requirements_cmd(text: String) -> Result<GenerateRequirementsResult, String> {
    if text.trim().is_empty() {
        return Err("Requirements text is empty".to_string());
    }

    let (api_key, base_url, model) = llm_config_from_env()?;

    let system_prompt = r#"You are a requirements analysis assistant. Given a free-text requirements document, you must:

1. **Segment the text** into word-level semantic units. Each segment gets a unique ID (w1, w2, ...), the text content, and character position range [start, end]. Segments must cover the ENTIRE input text exactly once with no gaps or overlaps.

2. **Generate test points** as a FreeMind XML mindmap. The root node is the system/module name. Intermediate nodes are feature categories. Leaf nodes are individual test points. Each leaf node MUST have a LINK attribute with comma-separated segment IDs that this test point verifies.

3. **Generate mock HTML** - a complete, self-contained HTML document with inline CSS that demonstrates the user interface described by the requirements. Include realistic form elements, buttons, navigation, and content. Make it look like a real application.

IMPORTANT RULES:
- Only generate test points for requirements that are ACTUALLY mentioned in the text. Do not invent test points for unmentioned features.
- Segment the text at word/phrase level, not character-by-character.
- The mindmap XML must be valid FreeMind format (version 1.0.1).
- LINK attributes use comma-separated segment IDs like LINK="w1,w3,w5".
- The mock HTML must be complete with <!DOCTYPE html> and all styles inline.
"#;

    let tool_params = serde_json::json!({
        "type": "object",
        "properties": {
            "segments": {
                "type": "array",
                "description": "Word-level segments of the requirements text, covering every character exactly once",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "text": { "type": "string" },
                        "pos": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "minItems": 2,
                            "maxItems": 2
                        }
                    },
                    "required": ["id", "text", "pos"]
                }
            },
            "mindmap_xml": {
                "type": "string",
                "description": "FreeMind-compatible XML mindmap with test points"
            },
            "mock_html": {
                "type": "string",
                "description": "Complete self-contained high-fidelity HTML page"
            }
        },
        "required": ["segments", "mindmap_xml", "mock_html"]
    });

    let result = call_llm_with_tool(
        &api_key,
        &base_url,
        &model,
        system_prompt,
        &text,
        "generate_testpoints",
        "Generate test points, mindmap, and mock HTML from requirements text",
        tool_params,
    )
    .await?;

    let segments: Vec<GenerateRequirementsSegment> =
        serde_json::from_value(result.get("segments").cloned().unwrap_or_default())
            .map_err(|e| format!("Failed to parse segments: {}", e))?;

    let mindmap_xml = result
        .get("mindmap_xml")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mock_html = result
        .get("mock_html")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Validate
    let text_len = text.chars().count();
    for seg in &segments {
        if seg.pos.0 >= seg.pos.1 || seg.pos.1 > text_len {
            return Err(format!("Segment {} has invalid position range", seg.id));
        }
    }
    if mindmap_xml.is_empty() {
        return Err("Mindmap XML is empty".to_string());
    }

    let slug = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();

    Ok(GenerateRequirementsResult {
        slug,
        segments,
        mindmap_xml,
        mock_html,
    })
}

/// Confirms destructive switch/exit when runtime is active.
#[tauri::command]
pub async fn confirm_teardown(
    rt: State<'_, Arc<TeshiEngine>>,
    app: AppHandle,
) -> Result<bool, String> {
    if check_project_switch_allowed(&rt) {
        return Ok(true);
    }
    let answer = app
        .dialog()
        .message("Browser/Terminal is running. Continuing will stop them.")
        .title("Confirm")
        .buttons(MessageDialogButtons::OkCancel)
        .kind(MessageDialogKind::Warning)
        .blocking_show();

    Ok(answer)
}
