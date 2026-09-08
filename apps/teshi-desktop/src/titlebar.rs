//! Custom GPUI titlebar for the native desktop shell.
//!
//! Replaces the platform titlebar with a Zed-style bar that draws the app title,
//! a draggable region, and platform-aware minimize / maximize / close controls.

use gpui::{
    App, Context as GpuiContext, Decorations, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, Styled, Window, WindowControlArea, div,
    prelude::FluentBuilder, px, rgb,
};

const TITLEBAR_HEIGHT_PX: f32 = 32.0;
const MACOS_TRAFFIC_LIGHT_PAD_PX: f32 = 80.0;
const TITLEBAR_BG: u32 = 0x1e1e2e;
const TITLEBAR_BORDER: u32 = 0x313244;
const TITLEBAR_TEXT: u32 = 0xcdd6f4;
const BUTTON_HOVER_BG: u32 = 0x45475a;
const CLOSE_BUTTON_HOVER_BG: u32 = 0xe64553;

/// Custom titlebar rendered at the top of the desktop window.
#[derive(Default)]
pub struct Titlebar;

impl Titlebar {
    /// Create a new titlebar view.
    pub fn new() -> Self {
        Self
    }
}

impl Render for Titlebar {
    fn render(&mut self, window: &mut Window, cx: &mut GpuiContext<Self>) -> impl IntoElement {
        let controls = window.window_controls();
        let decorations = window.window_decorations();
        // On Windows we always request `WindowDecorations::Client` (see main.rs), so the
        // native titlebar is hidden and we must draw our own controls regardless of what
        // `window_decorations()` reports (the Windows platform backend keeps returning
        // `Decorations::Server`). On Linux we only draw them under client-side decorations.
        let draw_custom_controls = if cfg!(target_os = "windows") {
            true
        } else {
            !matches!(decorations, Decorations::Server)
        };
        let any_controls = controls.minimize || controls.maximize || controls.fullscreen;

        // On macOS the traffic lights occupy the top-left corner; leave room for them.
        let left_pad = if cfg!(target_os = "macos") {
            px(MACOS_TRAFFIC_LIGHT_PAD_PX)
        } else {
            px(12.)
        };

        div()
            .w_full()
            .h(px(TITLEBAR_HEIGHT_PX))
            .bg(rgb(TITLEBAR_BG))
            .border_b_1()
            .border_color(rgb(TITLEBAR_BORDER))
            // The whole bar is a draggable region for moving the window.
            .window_control_area(WindowControlArea::Drag)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .pl(left_pad)
            .pr(px(8.))
            // Double-click the titlebar to toggle maximize/zoom, like Zed or a native bar.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, event: &gpui::MouseDownEvent, window, _| {
                    if event.click_count == 2 {
                        window.zoom_window();
                    }
                }),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TITLEBAR_TEXT))
                    .child("teshi"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.))
                    .when(draw_custom_controls && controls.minimize, |this| {
                        this.child(control_button(
                            "—",
                            cx.listener(|_, _, window, cx| {
                                cx.stop_propagation();
                                window.minimize_window();
                            }),
                            false,
                            WindowControlArea::Min,
                        ))
                    })
                    .when(
                        draw_custom_controls && (controls.maximize || controls.fullscreen),
                        |this| {
                            this.child(control_button(
                                "□",
                                cx.listener(|_, _, window, cx| {
                                    cx.stop_propagation();
                                    window.zoom_window();
                                }),
                                false,
                                WindowControlArea::Max,
                            ))
                        },
                    )
                    .when(draw_custom_controls && any_controls, |this| {
                        this.child(control_button(
                            "×",
                            cx.listener(|_, _, window, cx| {
                                cx.stop_propagation();
                                window.remove_window();
                            }),
                            true,
                            WindowControlArea::Close,
                        ))
                    }),
            )
    }
}

/// A single window-control button with a hover background.
fn control_button(
    label: &'static str,
    on_click: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) + 'static,
    is_close: bool,
    control_area: WindowControlArea,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(30.))
        .h_full()
        .text_color(rgb(TITLEBAR_TEXT))
        .text_sm()
        .cursor_pointer()
        .window_control_area(control_area)
        .hover(|mut style| {
            style.background = Some(
                rgb(if is_close {
                    CLOSE_BUTTON_HOVER_BG
                } else {
                    BUTTON_HOVER_BG
                })
                .into(),
            );
            style
        })
        .child(label)
        .on_mouse_down(MouseButton::Left, on_click)
}
