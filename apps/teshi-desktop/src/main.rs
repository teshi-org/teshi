//! Native GPUI desktop shell for teshi.

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(windows)]
use base64::Engine as _;
use gpui::{App, AppContext, Bounds, Entity, WindowBounds, WindowOptions, px, size};
use teshi_engine::{ApiStyle, ModelProfile, ModelProfileList, ModelProfilePublic, PROVIDER_OPENAI};
#[cfg(windows)]
use teshi_engine::{
    BrowserMode, RuntimeConfig, TeshiEngine, default_browser_service_script,
    default_winapp_service_script, open_project, start_browser_sidecar,
};
use teshi_ui::{
    ApiStyleDto, AppShell, LlmConfigBackend, LlmConfigSnapshot, LlmConfigUpdate,
    ModelProfileListSnapshot, ModelProfileSnapshot, ModelProfileUpdate, WinAppPreview,
    bind_llm_config_keys,
};

enum PreviewEvent {
    Waiting(String),
    Frame(Vec<u8>),
    Error(String),
}

type LatestPreviewEvent = Arc<Mutex<Option<PreviewEvent>>>;

fn replace_latest(slot: &LatestPreviewEvent, event: PreviewEvent) {
    *slot.lock().unwrap() = Some(event);
}

#[cfg(windows)]
fn run_winapp_stream(slot: LatestPreviewEvent, process_name: String) -> Result<(), String> {
    use tungstenite::{Message, connect};

    let configured_url = std::env::var("TESHI_WINAPP_WS_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let (ws_url, _owned_engine) = if let Some(ws_url) = configured_url {
        (ws_url, None)
    } else {
        let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
        let engine = TeshiEngine::new(
            RuntimeConfig {
                browser_service_script: default_browser_service_script(),
                winapp_service_script: default_winapp_service_script(),
                embedded_no_preview_stream: false,
            },
            None,
        );
        let project_root = std::env::current_dir()
            .map_err(|error| format!("resolve current project: {error}"))?
            .to_string_lossy()
            .into_owned();
        runtime.block_on(open_project(Arc::clone(&engine), project_root))?;
        let started = runtime
            .block_on(start_browser_sidecar(
                Arc::clone(&engine),
                BrowserMode::WinApp,
            ))
            .map_err(|error| match error.hint {
                Some(hint) => format!("{} ({hint})", error.message),
                None => error.message,
            })?;
        (started.ws_url, Some(engine))
    };

    replace_latest(
        &slot,
        PreviewEvent::Waiting(format!("Connected; attaching to {process_name}…")),
    );
    let (mut socket, _) = connect(&ws_url).map_err(|error| format!("connect {ws_url}: {error}"))?;
    let attach = serde_json::json!({
        "cmd": "attach_window",
        "request_id": "gpui-preview-attach",
        "process_name": process_name,
    });
    socket
        .send(Message::Text(attach.to_string()))
        .map_err(|error| format!("send attach command: {error}"))?;

    loop {
        let message = socket
            .read()
            .map_err(|error| format!("preview WebSocket closed: {error}"))?;
        let Message::Text(text) = message else {
            continue;
        };
        let payload: serde_json::Value = match serde_json::from_str(&text) {
            Ok(payload) => payload,
            Err(error) => {
                replace_latest(
                    &slot,
                    PreviewEvent::Error(format!("invalid preview message: {error}")),
                );
                continue;
            }
        };
        match payload.get("type").and_then(|value| value.as_str()) {
            Some("frame") => {
                let Some(data) = payload.get("data").and_then(|value| value.as_str()) else {
                    continue;
                };
                match base64::engine::general_purpose::STANDARD.decode(data) {
                    Ok(jpeg) => replace_latest(&slot, PreviewEvent::Frame(jpeg)),
                    Err(error) => replace_latest(
                        &slot,
                        PreviewEvent::Error(format!("invalid JPEG frame: {error}")),
                    ),
                }
            }
            Some("frame_error") => replace_latest(
                &slot,
                PreviewEvent::Error(
                    payload
                        .get("error")
                        .and_then(|value| value.as_str())
                        .unwrap_or("WinApp capture failed")
                        .to_string(),
                ),
            ),
            Some("response")
                if payload.get("request_id").and_then(|v| v.as_str())
                    == Some("gpui-preview-attach") =>
            {
                if payload.get("ok").and_then(|value| value.as_bool()) == Some(true) {
                    replace_latest(
                        &slot,
                        PreviewEvent::Waiting("Attached; waiting for first frame…".into()),
                    );
                } else {
                    replace_latest(
                        &slot,
                        PreviewEvent::Error(
                            payload
                                .get("error")
                                .and_then(|value| value.as_str())
                                .unwrap_or("could not attach to target application")
                                .to_string(),
                        ),
                    );
                }
            }
            _ => {}
        }
    }
}

fn start_native_preview(process_name: String) -> LatestPreviewEvent {
    let slot = Arc::new(Mutex::new(None));
    let worker_slot = Arc::clone(&slot);
    std::thread::spawn(move || {
        #[cfg(windows)]
        if let Err(error) = run_winapp_stream(Arc::clone(&worker_slot), process_name) {
            replace_latest(&worker_slot, PreviewEvent::Error(error));
        }

        #[cfg(not(windows))]
        {
            let _ = process_name;
            replace_latest(
                &worker_slot,
                PreviewEvent::Error("WinApp preview is only available on Windows".into()),
            );
        }
    });
    slot
}

fn poll_preview_events(preview: Entity<WinAppPreview>, slot: LatestPreviewEvent, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(32))
                .await;
            let event = slot.lock().unwrap().take();
            if let Some(event) = event {
                preview.update(cx, |preview, cx| match event {
                    PreviewEvent::Waiting(detail) => preview.set_waiting(detail, cx),
                    PreviewEvent::Frame(jpeg) => preview.set_jpeg(jpeg, cx),
                    PreviewEvent::Error(error) => preview.set_error(error, cx),
                });
            }
        }
    })
    .detach();
}

struct NativeLlmBackend;

fn map_api_style(style: ApiStyle) -> ApiStyleDto {
    match style {
        ApiStyle::ChatCompletions => ApiStyleDto::ChatCompletions,
        ApiStyle::Responses => ApiStyleDto::Responses,
    }
}

fn map_api_style_in(style: ApiStyleDto) -> ApiStyle {
    match style {
        ApiStyleDto::ChatCompletions => ApiStyle::ChatCompletions,
        ApiStyleDto::Responses => ApiStyle::Responses,
    }
}

fn map_profile(p: ModelProfilePublic) -> ModelProfileSnapshot {
    ModelProfileSnapshot {
        id: p.id,
        name: p.name,
        provider: p.provider,
        api_style: map_api_style(p.api_style),
        model_id: p.model_id,
        max_context_tokens: p.max_context_tokens,
        max_output_tokens: p.max_output_tokens,
        base_url: p.base_url,
        api_key_configured: p.api_key_configured,
        api_key_masked: p.api_key_masked,
        stream: p.stream,
        http_headers: p.http_headers,
        chat_options: p.chat_options,
        active: p.active,
    }
}

fn map_list(list: ModelProfileList) -> ModelProfileListSnapshot {
    ModelProfileListSnapshot {
        profiles: list.profiles.into_iter().map(map_profile).collect(),
        active_id: list.active_id,
    }
}

impl LlmConfigBackend for NativeLlmBackend {
    fn get_llm_config(&self) -> Result<LlmConfigSnapshot, String> {
        let public = teshi_engine::load_llm_config_public().map_err(|e| e.to_string())?;
        Ok(LlmConfigSnapshot {
            base_url: public.base_url,
            model: public.model,
            api_key_configured: public.api_key_configured,
            api_key_masked: public.api_key_masked,
        })
    }

    fn set_llm_config(&self, update: LlmConfigUpdate) -> Result<(), String> {
        let write = teshi_engine::LlmConfigWrite {
            base_url: update.base_url,
            model: update.model,
            api_key: update.api_key,
        };
        teshi_engine::save_stored_llm_config(&write).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list_profiles(&self) -> Result<ModelProfileListSnapshot, String> {
        let list = teshi_engine::list_profiles().map_err(|e| e.to_string())?;
        Ok(map_list(list))
    }

    fn get_profile(&self, id: &str) -> Result<ModelProfileSnapshot, String> {
        let p = teshi_engine::get_profile_public(id).map_err(|e| e.to_string())?;
        Ok(map_profile(p))
    }

    fn save_profile(&self, update: ModelProfileUpdate) -> Result<ModelProfileSnapshot, String> {
        let id = if update.id.trim().is_empty() {
            teshi_engine::generate_id()
        } else {
            update.id
        };
        let mut profile = ModelProfile {
            id,
            name: update.name,
            provider: if update.provider.is_empty() {
                PROVIDER_OPENAI.into()
            } else {
                update.provider
            },
            api_style: map_api_style_in(update.api_style),
            model_id: update.model_id,
            max_context_tokens: update.max_context_tokens,
            max_output_tokens: update.max_output_tokens,
            base_url: update.base_url,
            api_key: update.api_key,
            stream: update.stream,
            http_headers: update.http_headers,
            chat_options: update.chat_options,
        };
        let public = teshi_engine::save_profile(&mut profile).map_err(|e| e.to_string())?;
        Ok(map_profile(public))
    }

    fn delete_profile(&self, id: &str) -> Result<(), String> {
        teshi_engine::delete_profile(id).map_err(|e| e.to_string())
    }

    fn activate_profile(&self, id: &str) -> Result<(), String> {
        teshi_engine::set_active_id(id).map_err(|e| e.to_string())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        bind_llm_config_keys(cx);
        let bounds = Bounds::centered(None, size(px(960.0), px(720.0)), cx);
        let backend: Rc<dyn LlmConfigBackend> = Rc::new(NativeLlmBackend);
        let process_name = std::env::var("TESHI_WINAPP_PROCESS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "TargetApp.exe".into());
        let preview = cx.new(|_| WinAppPreview::new(process_name.clone()));
        let events = start_native_preview(process_name);
        poll_preview_events(preview.clone(), events, cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| AppShell::new(backend.clone(), preview.clone(), window, cx)),
        )
        .expect("open teshi-desktop window");
        cx.activate(true);
    });
}
