//! GPUI WASM shell for teshi (Path 1: served by teshi-daemon).

use std::rc::Rc;

use base64::Engine as _;
use gpui::{AppCell, Entity, prelude::*};
use teshi_ui::{
    AppShell, BrowserSessionListSnapshot, BrowserSessionsBackend, BrowserTabTarget,
    LlmConfigBackend, LlmConfigSnapshot, LlmConfigUpdate, ModelProfileListSnapshot,
    ModelProfileSnapshot, ModelProfileUpdate, WinAppPreview, bind_llm_config_keys,
};
use wasm_bindgen::prelude::*;

fn query_parameter(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get(name).filter(|value| !value.trim().is_empty())
}

fn same_origin_preview_ws_url() -> Result<String, String> {
    let location = web_sys::window()
        .ok_or_else(|| "browser window is unavailable".to_string())?
        .location();
    let protocol = location
        .protocol()
        .map_err(|error| format!("page protocol: {error:?}"))?;
    let scheme = match protocol.as_str() {
        "http:" => "ws",
        "https:" => "wss",
        _ => return Err(format!("unsupported page protocol {protocol}")),
    };
    let host = location
        .host()
        .map_err(|error| format!("page host: {error:?}"))?;
    Ok(format!("{scheme}://{host}/api/v1/browser/stream"))
}

fn update_preview(
    app: &Rc<AppCell>,
    preview: &Entity<WinAppPreview>,
    update: impl FnOnce(&mut WinAppPreview, &mut gpui::Context<WinAppPreview>),
) {
    // Browser callbacks run on the same thread as GPUI. If GPUI is already in
    // an update, dropping this superseded event is preferable to re-entrant borrowing.
    if let Ok(mut cx) = app.try_borrow_mut() {
        let app: &mut gpui::App = std::ops::DerefMut::deref_mut(&mut cx);
        preview.update(app, update);
    }
}

fn start_wasm_preview(preview: Entity<WinAppPreview>, app: Rc<AppCell>, cx: &mut gpui::App) {
    let ws_url = if let Some(ws_url) = query_parameter("winapp_ws") {
        ws_url
    } else {
        match WasmBackend::xhr_json(
            "POST",
            "/api/v1/browser/start",
            Some(r#"{"mode":"winapp"}"#),
        )
        .and_then(|_| same_origin_preview_ws_url())
        {
            Ok(ws_url) => ws_url,
            Err(error) => {
                preview.update(cx, |preview, cx| {
                    preview.set_error(format!("start WinApp sidecar: {error}"), cx);
                });
                return;
            }
        }
    };

    let socket = match web_sys::WebSocket::new(&ws_url) {
        Ok(socket) => socket,
        Err(error) => {
            preview.update(cx, |preview, cx| {
                preview.set_error(format!("open {ws_url}: {error:?}"), cx);
            });
            return;
        }
    };

    let open_app = Rc::clone(&app);
    let open_preview = preview.clone();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        update_preview(&open_app, &open_preview, |preview, cx| {
            preview.set_waiting(
                "Connected through daemon; attaching to target application…",
                cx,
            );
        });
    });
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let message_app = Rc::clone(&app);
    let message_preview = preview.clone();
    let on_message =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&text) else {
                return;
            };
            match payload.get("type").and_then(|value| value.as_str()) {
                Some("frame") => {
                    let Some(data) = payload.get("data").and_then(|value| value.as_str()) else {
                        return;
                    };
                    match base64::engine::general_purpose::STANDARD.decode(data) {
                        Ok(jpeg) => {
                            update_preview(&message_app, &message_preview, |preview, cx| {
                                preview.set_jpeg(jpeg, cx);
                            })
                        }
                        Err(error) => {
                            update_preview(&message_app, &message_preview, |preview, cx| {
                                preview.set_error(format!("invalid JPEG frame: {error}"), cx);
                            });
                        }
                    }
                }
                Some("frame_error") => {
                    let error = payload
                        .get("error")
                        .and_then(|value| value.as_str())
                        .unwrap_or("WinApp capture failed")
                        .to_string();
                    update_preview(&message_app, &message_preview, |preview, cx| {
                        preview.set_error(error, cx);
                    });
                }
                Some("response")
                    if payload.get("request_id").and_then(|value| value.as_str())
                        == Some("gpui-preview-attach") =>
                {
                    if payload.get("ok").and_then(|value| value.as_bool()) == Some(true) {
                        update_preview(&message_app, &message_preview, |preview, cx| {
                            preview.set_waiting("Attached; waiting for first frame…", cx);
                        });
                    } else {
                        let error = payload
                            .get("error")
                            .and_then(|value| value.as_str())
                            .unwrap_or("could not attach to target application")
                            .to_string();
                        update_preview(&message_app, &message_preview, |preview, cx| {
                            preview.set_error(error, cx);
                        });
                    }
                }
                _ => {}
            }
        });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let error_app = Rc::clone(&app);
    let error_preview = preview.clone();
    let on_error =
        Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
            let detail = if event.message().is_empty() {
                "browser rejected the WinApp preview WebSocket".to_string()
            } else {
                event.message()
            };
            update_preview(&error_app, &error_preview, |preview, cx| {
                preview.set_error(detail, cx);
            });
        });
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    let close_app = app;
    let close_preview = preview;
    let on_close =
        Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |event: web_sys::CloseEvent| {
            let detail = if event.reason().is_empty() {
                format!("preview WebSocket closed ({})", event.code())
            } else {
                format!("preview WebSocket closed: {}", event.reason())
            };
            update_preview(&close_app, &close_preview, |preview, cx| {
                preview.set_error(detail, cx);
            });
        });
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();
}

/// Same-origin sync XHR adapter for shared GPUI backend traits.
struct WasmBackend;

impl WasmBackend {
    fn xhr_json(method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
        let xhr = web_sys::XmlHttpRequest::new().map_err(|e| format!("{e:?}"))?;
        xhr.open_with_async(method, path, false)
            .map_err(|e| format!("open: {e:?}"))?;
        if body.is_some() {
            xhr.set_request_header("Content-Type", "application/json")
                .map_err(|e| format!("header: {e:?}"))?;
        }
        xhr.send_with_opt_str(body)
            .map_err(|e| format!("send: {e:?}"))?;
        let status = xhr.status().map_err(|e| format!("status: {e:?}"))?;
        let text = xhr
            .response_text()
            .map_err(|e| format!("response: {e:?}"))?
            .unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}: {text}"));
        }
        Ok(text)
    }
}

impl LlmConfigBackend for WasmBackend {
    fn get_llm_config(&self) -> Result<LlmConfigSnapshot, String> {
        let text = Self::xhr_json("GET", "/api/v1/llm/config", None)?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    fn set_llm_config(&self, update: LlmConfigUpdate) -> Result<(), String> {
        let body = serde_json::to_string(&update).map_err(|e| e.to_string())?;
        let _ = Self::xhr_json("PUT", "/api/v1/llm/config", Some(&body))?;
        Ok(())
    }

    fn list_profiles(&self) -> Result<ModelProfileListSnapshot, String> {
        let text = Self::xhr_json("GET", "/api/v1/llm/profiles", None)?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    fn get_profile(&self, id: &str) -> Result<ModelProfileSnapshot, String> {
        let path = format!("/api/v1/llm/profiles/{id}");
        let text = Self::xhr_json("GET", &path, None)?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    fn save_profile(&self, update: ModelProfileUpdate) -> Result<ModelProfileSnapshot, String> {
        let body = serde_json::to_string(&update).map_err(|e| e.to_string())?;
        let text = Self::xhr_json("PUT", "/api/v1/llm/profiles", Some(&body))?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    fn delete_profile(&self, id: &str) -> Result<(), String> {
        let path = format!("/api/v1/llm/profiles/{id}");
        let _ = Self::xhr_json("DELETE", &path, None)?;
        Ok(())
    }

    fn activate_profile(&self, id: &str) -> Result<(), String> {
        let path = format!("/api/v1/llm/profiles/{id}/activate");
        let _ = Self::xhr_json("POST", &path, None)?;
        Ok(())
    }
}

impl BrowserSessionsBackend for WasmBackend {
    fn start_browser_bridge(&self) -> Result<(), String> {
        let _ = Self::xhr_json(
            "POST",
            "/api/v1/browser/start",
            Some(r#"{"mode":"chrome"}"#),
        )?;
        Ok(())
    }

    fn list_browser_sessions(&self) -> Result<BrowserSessionListSnapshot, String> {
        let text = Self::xhr_json("GET", "/api/v1/browser/sessions", None)?;
        serde_json::from_str(&text).map_err(|error| format!("decode browser sessions: {error}"))
    }

    fn activate_browser_tab(&self, target: &BrowserTabTarget) -> Result<(), String> {
        let body = serde_json::to_string(target).map_err(|error| error.to_string())?;
        let _ = Self::xhr_json("POST", "/api/v1/browser/activate-tab", Some(&body))?;
        Ok(())
    }
}

/// Start the GPUI web shell and report async startup outcome to JavaScript.
///
/// GPU initialization is asynchronous. Call `on_ready` after the window opens
/// successfully, or `on_error` with a short English message if it fails.
/// Callback invocation failures are logged to the browser console.
#[wasm_bindgen]
pub fn run(on_ready: js_sys::Function, on_error: js_sys::Function) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    gpui_platform::web_init();

    let app = gpui_platform::single_threaded_web();

    // Keep the web Application's Rc alive for the page lifetime (upstream pattern).
    struct WasmApplication(Rc<gpui::AppCell>);
    let wasm_app = unsafe { std::mem::transmute::<gpui::Application, WasmApplication>(app) };
    let app_cell = wasm_app.0.clone();
    std::mem::forget(app_cell.clone());
    let app = unsafe { std::mem::transmute::<WasmApplication, gpui::Application>(wasm_app) };

    app.run(move |cx: &mut gpui::App| {
        bind_llm_config_keys(cx);
        let platform = Rc::new(WasmBackend);
        let llm_backend: Rc<dyn LlmConfigBackend> = platform.clone();
        let browser_backend: Rc<dyn BrowserSessionsBackend> = platform;
        let preview = cx.new(|_| WinAppPreview::new("target application"));
        match cx.open_window(gpui::WindowOptions::default(), |window, cx| {
            cx.new(|cx| {
                AppShell::new(
                    llm_backend.clone(),
                    browser_backend.clone(),
                    preview.clone(),
                    window,
                    cx,
                )
            })
        }) {
            Ok(_) => {
                cx.activate(true);
                // WinApp capture is opt-in. Starting it unconditionally would replace
                // an active Chrome bridge while the default Browser surface loads.
                if query_parameter("winapp_preview").is_some()
                    || query_parameter("winapp_ws").is_some()
                {
                    start_wasm_preview(preview, app_cell.clone(), cx);
                }
                if let Err(err) = on_ready.call0(&JsValue::NULL) {
                    web_sys::console::error_1(&err);
                }
            }
            Err(err) => {
                let message = format!("Failed to open window: {err:#}");
                if let Err(cb_err) = on_error.call1(&JsValue::NULL, &JsValue::from_str(&message)) {
                    web_sys::console::error_1(&cb_err);
                }
            }
        }
    });

    Ok(())
}
