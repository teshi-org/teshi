//! Shared root shell: WinApp preview main surface plus settings host.

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::FluentBuilder, px, rgb,
};

use crate::backend::SharedLlmBackend;
use crate::llm_config_view::LlmConfigView;
use crate::winapp_preview::WinAppPreview;

/// Which primary surface the shell is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellSurface {
    /// Default landing surface (WinApp preview).
    #[default]
    Main,
    /// Settings host (LLM config and future panels).
    Settings,
}

/// Root GPUI view for desktop and web: WinApp preview + settings navigation.
///
/// Construct with a [`SharedLlmBackend`]; the shell injects it into the
/// settings-hosted [`LlmConfigView`]. Default surface is [`ShellSurface::Main`].
pub struct AppShell {
    surface: ShellSurface,
    focus_handle: FocusHandle,
    llm_config: Entity<LlmConfigView>,
    winapp_preview: Entity<WinAppPreview>,
}

impl AppShell {
    /// Create the root shell and an embedded LLM config view (shown under settings).
    ///
    /// Does not focus the LLM form until the user opens settings, so the main
    /// surface remains the initial keyboard target.
    pub fn new(
        backend: SharedLlmBackend,
        winapp_preview: Entity<WinAppPreview>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let llm_config = cx.new(|cx| LlmConfigView::new(backend, window, cx));
        Self {
            surface: ShellSurface::Main,
            focus_handle,
            llm_config,
            winapp_preview,
        }
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.surface = ShellSurface::Settings;
        let handle = self.llm_config.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.surface = ShellSurface::Main;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match self.surface {
            ShellSurface::Main => "Teshi",
            ShellSurface::Settings => "Settings",
        };
        let (btn_id, btn_label) = match self.surface {
            ShellSurface::Main => ("open-settings", "Settings"),
            ShellSurface::Settings => ("close-settings", "Back"),
        };
        let is_main = self.surface == ShellSurface::Main;

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(16.))
            .py(px(12.))
            .border_b_1()
            .border_color(rgb(0x313244))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(0xcdd6f4))
                    .child(title),
            )
            .child(
                div()
                    .id(SharedString::from(btn_id))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .bg(rgb(0x313244))
                    .text_color(rgb(0xcdd6f4))
                    .text_sm()
                    .cursor_pointer()
                    .child(btn_label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if is_main {
                                this.open_settings(window, cx);
                            } else {
                                this.close_settings(window, cx);
                            }
                        }),
                    ),
            )
    }

    fn render_main(&self) -> impl IntoElement {
        div().size_full().child(self.winapp_preview.clone())
    }
}

impl Focusable for AppShell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .track_focus(&self.focus_handle(cx))
            .child(self.render_header(cx))
            .when(self.surface == ShellSurface::Main, |this| {
                this.child(self.render_main())
            })
            .when(self.surface == ShellSurface::Settings, |this| {
                this.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(self.llm_config.clone()),
                )
            })
    }
}
