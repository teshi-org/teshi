//! Native GPUI desktop shell for teshi.

use std::rc::Rc;

use gpui::{App, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};
use teshi_ui::{
    LlmConfigBackend, LlmConfigSnapshot, LlmConfigUpdate, LlmConfigView, bind_llm_config_keys,
};

struct NativeLlmBackend;

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
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        bind_llm_config_keys(cx);
        let bounds = Bounds::centered(None, size(px(720.0), px(560.0)), cx);
        let backend: Rc<dyn LlmConfigBackend> = Rc::new(NativeLlmBackend);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| LlmConfigView::new(backend.clone(), window, cx)),
        )
        .expect("open teshi-desktop window");
        cx.activate(true);
    });
}
