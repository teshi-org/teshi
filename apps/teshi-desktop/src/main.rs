//! Native GPUI desktop shell for teshi.

use std::rc::Rc;

use gpui::{App, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};
use teshi_engine::{ApiStyle, ModelProfile, ModelProfileList, ModelProfilePublic, PROVIDER_OPENAI};
use teshi_ui::{
    ApiStyleDto, AppShell, LlmConfigBackend, LlmConfigSnapshot, LlmConfigUpdate,
    ModelProfileListSnapshot, ModelProfileSnapshot, ModelProfileUpdate, bind_llm_config_keys,
};

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
        let bounds = Bounds::centered(None, size(px(720.0), px(560.0)), cx);
        let backend: Rc<dyn LlmConfigBackend> = Rc::new(NativeLlmBackend);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| AppShell::new(backend.clone(), window, cx)),
        )
        .expect("open teshi-desktop window");
        cx.activate(true);
    });
}
