//! GPUI WASM shell for teshi (Path 1: served by teshi-daemon).

use std::rc::Rc;

use gpui::prelude::*;
use teshi_ui::{
    LlmConfigBackend, LlmConfigSnapshot, LlmConfigUpdate, LlmConfigView, bind_llm_config_keys,
};
use wasm_bindgen::prelude::*;

/// Sync XHR backend so [`LlmConfigBackend`] matches the native sync trait.
struct WasmLlmBackend;

impl WasmLlmBackend {
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

impl LlmConfigBackend for WasmLlmBackend {
    fn get_llm_config(&self) -> Result<LlmConfigSnapshot, String> {
        let text = Self::xhr_json("GET", "/api/v1/llm/config", None)?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    fn set_llm_config(&self, update: LlmConfigUpdate) -> Result<(), String> {
        let body = serde_json::to_string(&update).map_err(|e| e.to_string())?;
        let _ = Self::xhr_json("PUT", "/api/v1/llm/config", Some(&body))?;
        Ok(())
    }
}

#[wasm_bindgen]
pub fn run() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    gpui_platform::web_init();

    let app = gpui_platform::single_threaded_web();

    // Keep the web Application's Rc alive for the page lifetime (upstream pattern).
    struct WasmApplication(Rc<gpui::AppCell>);
    let wasm_app = unsafe { std::mem::transmute::<gpui::Application, WasmApplication>(app) };
    std::mem::forget(wasm_app.0.clone());
    let app = unsafe { std::mem::transmute::<WasmApplication, gpui::Application>(wasm_app) };

    app.run(|cx: &mut gpui::App| {
        bind_llm_config_keys(cx);
        let backend: Rc<dyn LlmConfigBackend> = Rc::new(WasmLlmBackend);
        cx.open_window(gpui::WindowOptions::default(), |window, cx| {
            cx.new(|cx| LlmConfigView::new(backend.clone(), window, cx))
        })
        .expect("open teshi-web window");
        cx.activate(true);
    });

    Ok(())
}
