//! Shared root shell: browser sessions, WinApp preview, and settings host.

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, Styled, Window, div, prelude::FluentBuilder,
    px, rgb,
};

use crate::backend::{SharedBrowserSessionsBackend, SharedLlmBackend};
use crate::browser_sessions_view::BrowserSessionsView;
use crate::llm_config_view::LlmConfigView;
use crate::winapp_preview::WinAppPreview;

/// Which primary surface the shell is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellSurface {
    /// Default browser-profile discovery and selection surface.
    #[default]
    Browser,
    /// Native Windows application preview.
    WinApp,
    /// Settings host (LLM config and future panels).
    Settings,
}

/// Root GPUI view for desktop and web: browser sessions, WinApp preview, and settings.
///
/// Construct with platform backends; the shell injects them into the shared
/// child views. Default surface is [`ShellSurface::Browser`].
pub struct AppShell {
    surface: ShellSurface,
    focus_handle: FocusHandle,
    llm_config: Entity<LlmConfigView>,
    browser_sessions: Entity<BrowserSessionsView>,
    winapp_preview: Entity<WinAppPreview>,
}

impl AppShell {
    /// Create the root shell and an embedded LLM config view (shown under settings).
    ///
    /// Does not focus the LLM form until the user opens settings, so the main
    /// surface remains the initial keyboard target.
    pub fn new(
        llm_backend: SharedLlmBackend,
        browser_backend: SharedBrowserSessionsBackend,
        winapp_preview: Entity<WinAppPreview>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let llm_config = cx.new(|cx| LlmConfigView::new(llm_backend, window, cx));
        let browser_sessions = cx.new(|cx| BrowserSessionsView::new(browser_backend, window, cx));
        Self {
            surface: ShellSurface::Browser,
            focus_handle,
            llm_config,
            browser_sessions,
            winapp_preview,
        }
    }

    fn set_surface(&mut self, surface: ShellSurface, window: &mut Window, cx: &mut Context<Self>) {
        self.surface = surface;
        let handle = match surface {
            ShellSurface::Browser => self.browser_sessions.read(cx).focus_handle(cx),
            ShellSurface::Settings => self.llm_config.read(cx).focus_handle(cx),
            ShellSurface::WinApp => self.focus_handle.clone(),
        };
        window.focus(&handle, cx);
        cx.notify();
    }

    fn nav_button(
        &self,
        id: &'static str,
        label: &'static str,
        surface: ShellSurface,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.surface == surface;
        div()
            .id(id)
            .px(px(12.))
            .py(px(6.))
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
                cx.listener(move |this, _, window, cx| {
                    this.set_surface(surface, window, cx);
                }),
            )
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match self.surface {
            ShellSurface::Browser => "Teshi · Browser Profiles",
            ShellSurface::WinApp => "Teshi · WinApp Preview",
            ShellSurface::Settings => "Settings",
        };

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
                    .flex()
                    .gap(px(8.))
                    .child(self.nav_button(
                        "open-browser-sessions",
                        "Browser",
                        ShellSurface::Browser,
                        cx,
                    ))
                    .child(self.nav_button(
                        "open-winapp-preview",
                        "WinApp",
                        ShellSurface::WinApp,
                        cx,
                    ))
                    .child(self.nav_button(
                        "open-settings",
                        "Settings",
                        ShellSurface::Settings,
                        cx,
                    )),
            )
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
            .when(self.surface == ShellSurface::Browser, |this| {
                this.child(div().size_full().child(self.browser_sessions.clone()))
            })
            .when(self.surface == ShellSurface::WinApp, |this| {
                this.child(div().size_full().child(self.winapp_preview.clone()))
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
