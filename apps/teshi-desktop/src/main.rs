use gpui::{
    App, Application, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    rgb, size,
};

struct RootView;

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .justify_center()
            .items_center()
            .gap_4()
            .child(
                div()
                    .text_xl()
                    .text_color(rgb(0xcdd6f4))
                    .child("Hello, GPUI!"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x585b70))
                    .child("teshi desktop shell (GPUI)"),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1024.0), px(768.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| RootView),
        )
        .unwrap();
        cx.activate(true);
    });
}
