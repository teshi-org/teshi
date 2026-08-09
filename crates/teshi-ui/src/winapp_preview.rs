//! Shared GPUI presentation for the native Windows application preview stream.

use std::sync::Arc;

use gpui::{
    Context, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, Render, SharedString,
    Styled, StyledImage, Window, div, img, prelude::FluentBuilder, px, rgb,
};

/// Current lifecycle state of the preview connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    /// The platform adapter is starting the sidecar or opening its WebSocket.
    Connecting,
    /// The socket is open and attachment/first frame is pending.
    Waiting,
    /// At least one frame has arrived.
    Streaming,
    /// Startup, attachment, or streaming failed.
    Failed,
}

#[derive(Clone)]
struct PreviewModel {
    status: PreviewStatus,
    target: SharedString,
    detail: SharedString,
    frame: Option<Arc<Image>>,
}

impl PreviewModel {
    fn new(target: impl Into<SharedString>) -> Self {
        Self {
            status: PreviewStatus::Connecting,
            target: target.into(),
            detail: "Starting WinApp preview…".into(),
            frame: None,
        }
    }

    fn waiting(&mut self, detail: impl Into<SharedString>) {
        self.status = PreviewStatus::Waiting;
        self.detail = detail.into();
    }

    fn set_jpeg(&mut self, jpeg: Vec<u8>) {
        self.frame = Some(Arc::new(Image::from_bytes(ImageFormat::Jpeg, jpeg)));
        self.status = PreviewStatus::Streaming;
        self.detail = "Live · prototype 8 FPS JPEG stream".into();
    }

    fn fail(&mut self, detail: impl Into<SharedString>) {
        self.status = PreviewStatus::Failed;
        self.detail = detail.into();
    }
}

/// Shared preview entity rendered by both native and WASM GPUI shells.
pub struct WinAppPreview {
    model: PreviewModel,
}

impl WinAppPreview {
    /// Create a preview initially waiting for its platform adapter to connect.
    pub fn new(target: impl Into<SharedString>) -> Self {
        Self {
            model: PreviewModel::new(target),
        }
    }

    /// Report that the WebSocket is open and the target/first frame is pending.
    pub fn set_waiting(&mut self, detail: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.model.waiting(detail);
        cx.notify();
    }

    /// Replace the previous image with the newest JPEG frame.
    pub fn set_jpeg(&mut self, jpeg: Vec<u8>, cx: &mut Context<Self>) {
        self.model.set_jpeg(jpeg);
        cx.notify();
    }

    /// Surface an adapter or sidecar failure while retaining the last good frame.
    pub fn set_error(&mut self, detail: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.model.fail(detail);
        cx.notify();
    }

    /// Current lifecycle state, primarily useful to hosts and tests.
    pub fn status(&self) -> PreviewStatus {
        self.model.status
    }

    fn status_label(&self) -> &'static str {
        match self.model.status {
            PreviewStatus::Connecting => "CONNECTING",
            PreviewStatus::Waiting => "WAITING",
            PreviewStatus::Streaming => "LIVE",
            PreviewStatus::Failed => "ERROR",
        }
    }
}

impl Render for WinAppPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let status_color = match self.model.status {
            PreviewStatus::Connecting | PreviewStatus::Waiting => rgb(0xf9e2af),
            PreviewStatus::Streaming => rgb(0xa6e3a1),
            PreviewStatus::Failed => rgb(0xf38ba8),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_color(rgb(0xcdd6f4))
                                    .child("WinApp window preview"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x6c7086))
                                    .child(self.model.target.clone()),
                            ),
                    )
                    .child(
                        div()
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(5.))
                            .bg(rgb(0x313244))
                            .text_sm()
                            .text_color(status_color)
                            .child(self.status_label()),
                    ),
            )
            .child(
                div()
                    .flex_grow(1.)
                    .w_full()
                    .min_h(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(0x313244))
                    .bg(rgb(0x11111b))
                    .when_some(self.model.frame.clone(), |this, frame| {
                        this.child(img(frame).size_full().object_fit(ObjectFit::Contain))
                    })
                    .when(self.model.frame.is_none(), |this| {
                        this.child(
                            div()
                                .text_color(rgb(0x6c7086))
                                .child("Waiting for the first captured frame…"),
                        )
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(status_color)
                    .child(self.model.detail.clone()),
            )
            .child(
                div().text_sm().text_color(rgb(0x585b70)).child(
                    "Prototype: keep the target window visible and unobscured while capturing.",
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_frame_replaces_previous_frame() {
        let mut model = PreviewModel::new("TargetApp.exe");
        model.set_jpeg(vec![1, 2, 3]);
        let first = model.frame.clone().unwrap();
        model.set_jpeg(vec![4, 5, 6]);

        assert_eq!(model.status, PreviewStatus::Streaming);
        assert_eq!(model.frame.as_ref().unwrap().bytes(), &[4, 5, 6]);
        assert!(!Arc::ptr_eq(&first, model.frame.as_ref().unwrap()));
    }

    #[test]
    fn stream_error_retains_last_good_frame() {
        let mut model = PreviewModel::new("TargetApp.exe");
        model.set_jpeg(vec![1, 2, 3]);
        let frame = model.frame.clone().unwrap();
        model.fail("window is minimized");

        assert_eq!(model.status, PreviewStatus::Failed);
        assert_eq!(model.detail, "window is minimized");
        assert!(Arc::ptr_eq(&frame, model.frame.as_ref().unwrap()));
    }

    #[test]
    fn waiting_state_names_target_progress() {
        let mut model = PreviewModel::new("TargetApp.exe");
        model.waiting("Attached; waiting for first frame");

        assert_eq!(model.status, PreviewStatus::Waiting);
        assert_eq!(model.target, "TargetApp.exe");
    }
}
