//! Multi-profile LLM configuration UI hosted under the settings surface.

use std::collections::HashMap;

use gpui::{
    App, Context, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    KeyDownEvent, MouseButton, ParentElement, Render, SharedString, Styled, Window, actions, div,
    px, rgb,
};
use serde_json::Value;

use crate::backend::{ApiStyleDto, ModelProfileSnapshot, ModelProfileUpdate, SharedLlmBackend};

actions!(
    llm_config,
    [
        Backspace,
        FocusNext,
        FocusPrev,
        SaveConfig,
        NewProfile,
        CloneProfile,
        DeleteProfile,
        ActivateProfile,
    ]
);

const PROVIDERS: &[&str] = &["openai", "anthropic", "deepseek-openai"];

fn default_base_url(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "https://api.anthropic.com",
        "deepseek-openai" => "https://api.deepseek.com",
        _ => "https://api.openai.com/v1",
    }
}

fn provider_label(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "Anthropic",
        "deepseek-openai" => "DeepSeek (OpenAI-compatible)",
        _ => "OpenAI",
    }
}

fn is_sensitive_http_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "api-key"
    ) || name.ends_with("-api-key")
        || name.ends_with("-auth-token")
        || name.ends_with("-access-token")
}

fn prepare_clone_draft(draft: &mut ModelProfileUpdate) {
    draft.id.clear();
    draft.name = format!("{} (copy)", draft.name);
    draft.api_key.clear();
    draft
        .http_headers
        .retain(|name, value| !is_sensitive_http_header(name) && !value.starts_with('…'));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Provider,
    ApiStyle,
    ModelId,
    MaxContext,
    MaxOutput,
    BaseUrl,
    ApiKey,
    Stream,
    HeadersJson,
    ChatOptionsJson,
}

impl Field {
    fn all(provider: &str) -> Vec<Self> {
        let mut fields = vec![Self::Name, Self::Provider];
        if provider == "openai" {
            fields.push(Self::ApiStyle);
        }
        fields.extend([
            Self::ModelId,
            Self::MaxContext,
            Self::MaxOutput,
            Self::BaseUrl,
            Self::ApiKey,
            Self::Stream,
            Self::HeadersJson,
            Self::ChatOptionsJson,
        ]);
        fields
    }

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Provider => "Provider",
            Self::ApiStyle => "API Style",
            Self::ModelId => "Model",
            Self::MaxContext => "Max Context Tokens",
            Self::MaxOutput => "Max Output Tokens",
            Self::BaseUrl => "Base URL",
            Self::ApiKey => "API Key",
            Self::Stream => "Streaming",
            Self::HeadersJson => "HTTP Headers (JSON object)",
            Self::ChatOptionsJson => "Chat Options (JSON object)",
        }
    }

    fn is_select(self) -> bool {
        matches!(self, Self::Provider | Self::ApiStyle | Self::Stream)
    }
}

/// Settings-hosted multi-profile LLM editor (list + form).
///
/// Key context remains `LlmConfigView` so [`bind_llm_config_keys`] still applies
/// when this view is nested under [`crate::AppShell`] settings.
pub struct LlmConfigView {
    backend: SharedLlmBackend,
    focus_handle: FocusHandle,
    profiles: Vec<ModelProfileSnapshot>,
    active_id: Option<String>,
    selected_index: usize,
    field: Field,
    /// Draft being edited (not yet saved).
    draft: ModelProfileUpdate,
    api_key_configured: bool,
    api_key_masked: String,
    headers_json: String,
    chat_options_json: String,
    status: SharedString,
}

impl LlmConfigView {
    /// Create the view and load profiles from `backend`.
    pub fn new(backend: SharedLlmBackend, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let mut view = Self {
            backend,
            focus_handle,
            profiles: Vec::new(),
            active_id: None,
            selected_index: 0,
            field: Field::Name,
            draft: ModelProfileUpdate::default(),
            api_key_configured: false,
            api_key_masked: String::new(),
            headers_json: "{}".into(),
            chat_options_json: "{}".into(),
            status: "Loading…".into(),
        };
        view.reload_list();
        view
    }

    fn reload_list(&mut self) {
        match self.backend.list_profiles() {
            Ok(list) => {
                self.profiles = list.profiles;
                self.active_id = list.active_id;
                if self.profiles.is_empty() {
                    self.start_new_draft();
                    self.status = "No profiles — fill fields and Save".into();
                } else {
                    if self.selected_index >= self.profiles.len() {
                        self.selected_index = 0;
                    }
                    let id = self.profiles[self.selected_index].id.clone();
                    self.load_profile_into_draft(&id);
                    self.status = format!("{} profile(s)", self.profiles.len()).into();
                }
            }
            Err(err) => {
                self.status = format!("Load failed: {err}").into();
            }
        }
    }

    fn load_profile_into_draft(&mut self, id: &str) {
        match self.backend.get_profile(id) {
            Ok(snap) => self.apply_snapshot(snap),
            Err(err) => {
                self.status = format!("Load profile failed: {err}").into();
            }
        }
    }

    fn apply_snapshot(&mut self, snap: ModelProfileSnapshot) {
        self.draft = ModelProfileUpdate {
            id: snap.id,
            name: snap.name,
            provider: snap.provider,
            api_style: snap.api_style,
            model_id: snap.model_id,
            max_context_tokens: snap.max_context_tokens,
            max_output_tokens: snap.max_output_tokens,
            base_url: snap.base_url,
            api_key: String::new(),
            stream: snap.stream,
            http_headers: snap.http_headers,
            chat_options: snap.chat_options,
        };
        self.api_key_configured = snap.api_key_configured;
        self.api_key_masked = snap.api_key_masked;
        self.headers_json =
            serde_json::to_string_pretty(&self.draft.http_headers).unwrap_or_else(|_| "{}".into());
        self.chat_options_json =
            serde_json::to_string_pretty(&self.draft.chat_options).unwrap_or_else(|_| "{}".into());
        if self.field == Field::ApiStyle && self.draft.provider != "openai" {
            self.field = Field::Name;
        }
    }

    fn start_new_draft(&mut self) {
        self.draft = ModelProfileUpdate {
            id: String::new(),
            name: "New Profile".into(),
            provider: "openai".into(),
            api_style: ApiStyleDto::ChatCompletions,
            model_id: "gpt-4o-mini".into(),
            max_context_tokens: None,
            max_output_tokens: 1024,
            base_url: default_base_url("openai").into(),
            api_key: String::new(),
            stream: true,
            http_headers: HashMap::new(),
            chat_options: HashMap::new(),
        };
        self.api_key_configured = false;
        self.api_key_masked.clear();
        self.headers_json = "{}".into();
        self.chat_options_json = "{}".into();
        self.field = Field::Name;
    }

    fn start_clone_draft(&mut self) {
        prepare_clone_draft(&mut self.draft);
        self.api_key_configured = false;
        self.api_key_masked.clear();
        self.headers_json =
            serde_json::to_string_pretty(&self.draft.http_headers).unwrap_or_else(|_| "{}".into());
        self.status = "Cloned draft — Save to persist".into();
    }

    fn set_provider(&mut self, next: &str) {
        let prev = self.draft.provider.clone();
        let prev_default = default_base_url(&prev);
        let custom = !self.draft.base_url.is_empty() && self.draft.base_url != prev_default;
        self.draft.provider = next.to_string();
        if !custom {
            self.draft.base_url = default_base_url(next).into();
        }
        if next != "openai" {
            self.draft.api_style = ApiStyleDto::ChatCompletions;
            if self.field == Field::ApiStyle {
                self.field = Field::ModelId;
            }
        }
    }

    fn cycle_select(&mut self) {
        match self.field {
            Field::Provider => {
                let idx = PROVIDERS
                    .iter()
                    .position(|p| *p == self.draft.provider.as_str())
                    .unwrap_or(0);
                let next = PROVIDERS[(idx + 1) % PROVIDERS.len()];
                self.set_provider(next);
            }
            Field::ApiStyle => {
                self.draft.api_style = match self.draft.api_style {
                    ApiStyleDto::ChatCompletions => ApiStyleDto::Responses,
                    ApiStyleDto::Responses => ApiStyleDto::ChatCompletions,
                };
            }
            Field::Stream => {
                self.draft.stream = !self.draft.stream;
            }
            _ => {}
        }
    }

    fn active_buffer_mut(&mut self) -> Option<&mut String> {
        match self.field {
            Field::Name => Some(&mut self.draft.name),
            Field::ModelId => Some(&mut self.draft.model_id),
            Field::BaseUrl => Some(&mut self.draft.base_url),
            Field::ApiKey => Some(&mut self.draft.api_key),
            Field::HeadersJson => Some(&mut self.headers_json),
            Field::ChatOptionsJson => Some(&mut self.chat_options_json),
            Field::MaxContext => None,
            Field::MaxOutput => None,
            Field::Provider | Field::ApiStyle | Field::Stream => None,
        }
    }

    fn push_digit_field(&mut self, ch: char) {
        if !ch.is_ascii_digit() {
            return;
        }
        match self.field {
            Field::MaxContext => {
                let mut s = self
                    .draft
                    .max_context_tokens
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                s.push(ch);
                self.draft.max_context_tokens = s.parse().ok();
            }
            Field::MaxOutput => {
                let mut s = self.draft.max_output_tokens.to_string();
                if self.draft.max_output_tokens == 0 {
                    s.clear();
                }
                s.push(ch);
                if let Ok(n) = s.parse() {
                    self.draft.max_output_tokens = n;
                }
            }
            _ => {}
        }
    }

    fn pop_digit_field(&mut self) {
        match self.field {
            Field::MaxContext => {
                if let Some(n) = self.draft.max_context_tokens {
                    let mut s = n.to_string();
                    s.pop();
                    self.draft.max_context_tokens =
                        if s.is_empty() { None } else { s.parse().ok() };
                }
            }
            Field::MaxOutput => {
                let mut s = self.draft.max_output_tokens.to_string();
                s.pop();
                self.draft.max_output_tokens = if s.is_empty() {
                    0
                } else {
                    s.parse().unwrap_or(0)
                };
            }
            _ => {}
        }
    }

    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.field, Field::MaxContext | Field::MaxOutput) {
            self.pop_digit_field();
        } else if let Some(buf) = self.active_buffer_mut() {
            buf.pop();
        }
        cx.notify();
    }

    fn on_focus_next(&mut self, _: &FocusNext, _: &mut Window, cx: &mut Context<Self>) {
        let fields = Field::all(&self.draft.provider);
        let idx = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        self.field = fields[(idx + 1) % fields.len()];
        cx.notify();
    }

    fn on_focus_prev(&mut self, _: &FocusPrev, _: &mut Window, cx: &mut Context<Self>) {
        let fields = Field::all(&self.draft.provider);
        let idx = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        self.field = fields[(idx + fields.len() - 1) % fields.len()];
        cx.notify();
    }

    fn on_save(&mut self, _: &SaveConfig, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn on_new(&mut self, _: &NewProfile, _: &mut Window, cx: &mut Context<Self>) {
        self.start_new_draft();
        self.status = "New profile draft — Save to persist".into();
        cx.notify();
    }

    fn on_clone(&mut self, _: &CloneProfile, _: &mut Window, cx: &mut Context<Self>) {
        self.start_clone_draft();
        cx.notify();
    }

    fn on_delete(&mut self, _: &DeleteProfile, _: &mut Window, cx: &mut Context<Self>) {
        if self.draft.id.is_empty() {
            self.status = "Nothing to delete".into();
            cx.notify();
            return;
        }
        let id = self.draft.id.clone();
        match self.backend.delete_profile(&id) {
            Ok(()) => {
                self.reload_list();
                self.status = "Deleted".into();
            }
            Err(err) => {
                self.status = format!("Delete failed: {err}").into();
            }
        }
        cx.notify();
    }

    fn on_activate(&mut self, _: &ActivateProfile, _: &mut Window, cx: &mut Context<Self>) {
        if self.draft.id.is_empty() {
            self.status = "Save the profile before activating".into();
            cx.notify();
            return;
        }
        let id = self.draft.id.clone();
        match self.backend.activate_profile(&id) {
            Ok(()) => {
                self.reload_list();
                self.status = "Activated".into();
            }
            Err(err) => {
                self.status = format!("Activate failed: {err}").into();
            }
        }
        cx.notify();
    }

    fn parse_extras(&mut self) -> Result<(), String> {
        let headers: HashMap<String, String> = serde_json::from_str(&self.headers_json)
            .map_err(|e| format!("HTTP headers JSON: {e}"))?;
        let options: HashMap<String, Value> = serde_json::from_str(&self.chat_options_json)
            .map_err(|e| format!("Chat options JSON: {e}"))?;
        self.draft.http_headers = headers;
        self.draft.chat_options = options;
        Ok(())
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = self.parse_extras() {
            self.status = format!("Save failed: {err}").into();
            cx.notify();
            return;
        }
        match self.backend.save_profile(self.draft.clone()) {
            Ok(snap) => {
                let id = snap.id.clone();
                self.apply_snapshot(snap);
                self.reload_list();
                if let Some(idx) = self.profiles.iter().position(|p| p.id == id) {
                    self.selected_index = idx;
                }
                self.load_profile_into_draft(&id);
                self.status = "Saved".into();
            }
            Err(err) => {
                self.status = format!("Save failed: {err}").into();
            }
        }
        cx.notify();
    }

    fn select_profile(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.profiles.len() {
            return;
        }
        self.selected_index = index;
        let id = self.profiles[index].id.clone();
        self.load_profile_into_draft(&id);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ch) = event.keystroke.key_char.as_ref() {
            if ch == " " && self.field.is_select() {
                self.cycle_select();
                cx.notify();
                return;
            }
            if matches!(self.field, Field::MaxContext | Field::MaxOutput) {
                for c in ch.chars() {
                    self.push_digit_field(c);
                }
                cx.notify();
                return;
            }
            if ch.chars().all(|c| !c.is_control())
                && let Some(buf) = self.active_buffer_mut()
            {
                buf.push_str(ch);
                cx.notify();
            }
        }
    }

    fn field_display_value(&self, field: Field) -> SharedString {
        match field {
            Field::Name => self.draft.name.clone().into(),
            Field::Provider => provider_label(&self.draft.provider).into(),
            Field::ApiStyle => match self.draft.api_style {
                ApiStyleDto::ChatCompletions => "Chat Completions".into(),
                ApiStyleDto::Responses => "Responses".into(),
            },
            Field::ModelId => self.draft.model_id.clone().into(),
            Field::MaxContext => self
                .draft
                .max_context_tokens
                .map(|n| n.to_string())
                .unwrap_or_else(|| "(unset)".into())
                .into(),
            Field::MaxOutput => self.draft.max_output_tokens.to_string().into(),
            Field::BaseUrl => {
                if self.draft.base_url.is_empty() {
                    format!("(default: {})", default_base_url(&self.draft.provider)).into()
                } else {
                    self.draft.base_url.clone().into()
                }
            }
            Field::ApiKey => {
                if self.draft.api_key.is_empty() {
                    if self.api_key_configured {
                        format!(
                            "(stored {} — type to replace)",
                            if self.api_key_masked.is_empty() {
                                "key".into()
                            } else {
                                self.api_key_masked.clone()
                            }
                        )
                        .into()
                    } else {
                        "…".into()
                    }
                } else {
                    SharedString::from("*".repeat(self.draft.api_key.chars().count().min(32)))
                }
            }
            Field::Stream => if self.draft.stream { "On" } else { "Off" }.into(),
            Field::HeadersJson => self.headers_json.clone().into(),
            Field::ChatOptionsJson => self.chat_options_json.clone().into(),
        }
    }

    fn field_row(&self, field: Field, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.field == field;
        let border = if selected {
            rgb(0x89b4fa)
        } else {
            rgb(0x45475a)
        };
        let hint = if field.is_select() && selected {
            " · Space to cycle"
        } else {
            ""
        };
        div()
            .id(SharedString::from(format!("field-{}", field.label())))
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xa6adc8))
                    .child(format!("{}{hint}", field.label())),
            )
            .child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(border)
                    .bg(rgb(0x313244))
                    .text_color(rgb(0xcdd6f4))
                    .child(self.field_display_value(field))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.field = field;
                            window.focus(&this.focus_handle, cx);
                            cx.notify();
                        }),
                    ),
            )
    }

    fn action_button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(id))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(rgb(0x313244))
            .text_color(rgb(0xcdd6f4))
            .text_sm()
            .cursor_pointer()
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| on_click(this, cx)),
            )
    }

    fn render_profile_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap(px(4.)).w(px(200.));
        for (i, profile) in self.profiles.iter().enumerate() {
            let selected = i == self.selected_index;
            let active = Some(profile.id.as_str()) == self.active_id.as_deref();
            let label = if active {
                format!("★ {}", profile.name)
            } else {
                profile.name.clone()
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("profile-{i}")))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .bg(if selected {
                        rgb(0x45475a)
                    } else {
                        rgb(0x313244)
                    })
                    .border_1()
                    .border_color(if selected {
                        rgb(0x89b4fa)
                    } else {
                        rgb(0x313244)
                    })
                    .text_color(rgb(0xcdd6f4))
                    .text_sm()
                    .cursor_pointer()
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| this.select_profile(i, cx)),
                    ),
            );
        }
        if self.profiles.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(0x6c7086))
                    .child("No profiles yet"),
            );
        }
        list
    }
}

impl Focusable for LlmConfigView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LlmConfigView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fields = Field::all(&self.draft.provider);
        let mut editor = div().flex().flex_col().gap(px(10.)).flex_1();
        for field in fields {
            editor = editor.child(self.field_row(field, cx));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .p(px(16.))
            .gap(px(12.))
            .text_color(rgb(0xcdd6f4))
            .track_focus(&self.focus_handle(cx))
            .key_context("LlmConfigView")
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_focus_prev))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_new))
            .on_action(cx.listener(Self::on_clone))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_activate))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Model Configuration"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.))
                            .child(self.action_button("btn-new", "New", cx, |this, cx| {
                                this.start_new_draft();
                                this.status = "New profile draft — Save to persist".into();
                                cx.notify();
                            }))
                            .child(self.action_button("btn-clone", "Clone", cx, |this, cx| {
                                this.start_clone_draft();
                                cx.notify();
                            }))
                            .child(self.action_button("btn-delete", "Delete", cx, |this, cx| {
                                if this.draft.id.is_empty() {
                                    this.status = "Nothing to delete".into();
                                    cx.notify();
                                    return;
                                }
                                let id = this.draft.id.clone();
                                match this.backend.delete_profile(&id) {
                                    Ok(()) => {
                                        this.reload_list();
                                        this.status = "Deleted".into();
                                    }
                                    Err(err) => {
                                        this.status = format!("Delete failed: {err}").into();
                                    }
                                }
                                cx.notify();
                            }))
                            .child(self.action_button("btn-activate", "Activate", cx, |this, cx| {
                                if this.draft.id.is_empty() {
                                    this.status = "Save the profile before activating".into();
                                    cx.notify();
                                    return;
                                }
                                let id = this.draft.id.clone();
                                match this.backend.activate_profile(&id) {
                                    Ok(()) => {
                                        this.reload_list();
                                        this.status = "Activated".into();
                                    }
                                    Err(err) => {
                                        this.status = format!("Activate failed: {err}").into();
                                    }
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .id("btn-save")
                                    .px(px(12.))
                                    .py(px(6.))
                                    .rounded(px(6.))
                                    .bg(rgb(0x89b4fa))
                                    .text_color(rgb(0x1e1e2e))
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .cursor_pointer()
                                    .child("Save")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| this.save(cx)),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x6c7086))
                    .child(
                        "Tab/Shift+Tab: fields · Space: cycle select · Enter: save · buttons: New/Clone/Delete/Activate",
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(16.))
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xa6adc8))
                                    .child("Profiles"),
                            )
                            .child(self.render_profile_list(cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xa6adc8))
                                    .child("Model Options / Extra Options"),
                            )
                            .child(editor),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xa6adc8))
                    .child(self.status.clone()),
            )
    }
}

/// Register keybindings used by [`LlmConfigView`]. Call once during app startup.
pub fn bind_llm_config_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("LlmConfigView")),
        KeyBinding::new("tab", FocusNext, Some("LlmConfigView")),
        KeyBinding::new("shift-tab", FocusPrev, Some("LlmConfigView")),
        KeyBinding::new("enter", SaveConfig, Some("LlmConfigView")),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_draft_clears_masked_and_known_credential_headers() {
        let mut draft = ModelProfileUpdate {
            id: "source".into(),
            name: "Source".into(),
            api_key: "masked-or-raw".into(),
            ..ModelProfileUpdate::default()
        };
        draft
            .http_headers
            .insert("Authorization".into(), "…cret".into());
        draft.http_headers.insert("api-key".into(), "…1234".into());
        draft
            .http_headers
            .insert("X-Future-Credential".into(), "…5678".into());
        draft
            .http_headers
            .insert("X-Region".into(), "us-east".into());

        prepare_clone_draft(&mut draft);

        assert!(draft.id.is_empty());
        assert_eq!(draft.name, "Source (copy)");
        assert!(draft.api_key.is_empty());
        assert_eq!(draft.http_headers.len(), 1);
        assert_eq!(draft.http_headers["X-Region"], "us-east");
    }

    #[test]
    fn clone_draft_clears_unmasked_sensitive_headers_too() {
        let mut draft = ModelProfileUpdate::default();
        draft
            .http_headers
            .insert("X-Auth-Token".into(), "raw-token".into());

        prepare_clone_draft(&mut draft);

        assert!(draft.http_headers.is_empty());
    }
}
