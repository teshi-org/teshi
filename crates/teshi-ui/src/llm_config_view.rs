//! Single-screen LLM configuration UI for the GPUI spike.

use gpui::{
    App, Context, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    KeyDownEvent, MouseButton, ParentElement, Render, SharedString, Styled, Window, actions, div,
    px, rgb,
};

use crate::backend::{LlmConfigSnapshot, LlmConfigUpdate, SharedLlmBackend};

actions!(llm_config, [Backspace, FocusNext, SaveConfig]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    BaseUrl,
    Model,
    ApiKey,
}

impl Field {
    fn next(self) -> Self {
        match self {
            Self::BaseUrl => Self::Model,
            Self::Model => Self::ApiKey,
            Self::ApiKey => Self::BaseUrl,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::BaseUrl => "Base URL",
            Self::Model => "Model",
            Self::ApiKey => "API Key",
        }
    }
}

/// Root view: base URL, model, and API key with Save.
pub struct LlmConfigView {
    backend: SharedLlmBackend,
    focus_handle: FocusHandle,
    active: Field,
    base_url: String,
    model: String,
    api_key: String,
    api_key_configured: bool,
    status: SharedString,
}

impl LlmConfigView {
    /// Create the view and load the initial snapshot from `backend`.
    pub fn new(backend: SharedLlmBackend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let mut view = Self {
            backend,
            focus_handle,
            active: Field::BaseUrl,
            base_url: String::new(),
            model: String::new(),
            api_key: String::new(),
            api_key_configured: false,
            status: "Loading…".into(),
        };
        view.reload_from_backend();
        view
    }

    fn reload_from_backend(&mut self) {
        match self.backend.get_llm_config() {
            Ok(snap) => self.apply_snapshot(snap),
            Err(err) => {
                self.status = format!("Load failed: {err}").into();
            }
        }
    }

    fn apply_snapshot(&mut self, snap: LlmConfigSnapshot) {
        self.base_url = snap.base_url;
        self.model = snap.model;
        self.api_key_configured = snap.api_key_configured;
        self.status = if snap.api_key_configured {
            format!(
                "Configured ({})",
                if snap.api_key_masked.is_empty() {
                    "key set".to_string()
                } else {
                    snap.api_key_masked
                }
            )
            .into()
        } else {
            "Not configured".into()
        };
    }

    fn active_buffer_mut(&mut self) -> &mut String {
        match self.active {
            Field::BaseUrl => &mut self.base_url,
            Field::Model => &mut self.model,
            Field::ApiKey => &mut self.api_key,
        }
    }

    fn on_backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.active_buffer_mut().pop();
        cx.notify();
    }

    fn on_focus_next(&mut self, _: &FocusNext, _: &mut Window, cx: &mut Context<Self>) {
        self.active = self.active.next();
        cx.notify();
    }

    fn on_save(&mut self, _: &SaveConfig, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let update = LlmConfigUpdate {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key: self.api_key.clone(),
        };
        match self.backend.set_llm_config(update) {
            Ok(()) => {
                self.api_key.clear();
                self.reload_from_backend();
                if !self.status.to_string().starts_with("Load failed") {
                    self.status = "Saved".into();
                }
            }
            Err(err) => {
                self.status = format!("Save failed: {err}").into();
            }
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ch) = event.keystroke.key_char.as_ref() {
            if ch.chars().all(|c| !c.is_control()) {
                self.active_buffer_mut().push_str(ch);
                cx.notify();
            }
        }
    }

    fn field_row(&self, field: Field, value: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.active == field;
        let border = if selected {
            rgb(0x89b4fa)
        } else {
            rgb(0x45475a)
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
                    .child(field.label()),
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
                    .child(if value.is_empty() {
                        SharedString::from(match field {
                            Field::ApiKey if self.api_key_configured => {
                                "(stored — type to replace)".to_string()
                            }
                            _ => "…".to_string(),
                        })
                    } else if field == Field::ApiKey {
                        SharedString::from("*".repeat(value.chars().count().min(32)))
                    } else {
                        SharedString::from(value.to_string())
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.active = field;
                            window.focus(&this.focus_handle, cx);
                            cx.notify();
                        }),
                    ),
            )
    }
}

impl Focusable for LlmConfigView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LlmConfigView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .track_focus(&self.focus_handle(cx))
            .key_context("LlmConfigView")
            .on_action(cx.listener(Self::on_backspace))
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_save))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .w(px(480.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .p(px(24.))
                    .rounded(px(12.))
                    .bg(rgb(0x181825))
                    .border_1()
                    .border_color(rgb(0x313244))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::MEDIUM)
                            .child("LLM Configuration"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x6c7086))
                            .child("Tab: next field · Enter: save · Backspace: delete"),
                    )
                    .child(self.field_row(Field::BaseUrl, &self.base_url, cx))
                    .child(self.field_row(Field::Model, &self.model, cx))
                    .child(self.field_row(Field::ApiKey, &self.api_key, cx))
                    .child(
                        div()
                            .id("save-btn")
                            .px(px(16.))
                            .py(px(10.))
                            .rounded(px(6.))
                            .bg(rgb(0x89b4fa))
                            .text_color(rgb(0x1e1e2e))
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .child("Save")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.save(cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xa6adc8))
                            .child(self.status.clone()),
                    ),
            )
    }
}

/// Register keybindings used by [`LlmConfigView`]. Call once during app startup.
pub fn bind_llm_config_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("LlmConfigView")),
        KeyBinding::new("tab", FocusNext, Some("LlmConfigView")),
        KeyBinding::new("enter", SaveConfig, Some("LlmConfigView")),
    ]);
}
