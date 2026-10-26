//! GPUI run inspection with project-scoped step screenshots and replay history.

use std::sync::Arc;
use std::time::Duration;

use gpui::StatefulInteractiveElement;
use gpui::{
    App, Context, FocusHandle, Focusable, Image, ImageFormat, InteractiveElement, IntoElement,
    MouseButton, ObjectFit, ParentElement, Render, SharedString, Styled, StyledImage, Window, div,
    img, prelude::FluentBuilder, px, rgb,
};
use serde_json::Value;

use crate::backend::{ApiRunEventDto, ApiScenarioSnapshot, SharedApiRunBackend};

/// Lists project scenarios, starts a run, and shows step/exchange trees.
pub struct ApiRunView {
    backend: SharedApiRunBackend,
    focus_handle: FocusHandle,
    scenarios: Vec<ApiScenarioSnapshot>,
    selected: usize,
    events: Vec<ApiRunEventDto>,
    status: SharedString,
    expand_plaintext: bool,
    expanded_exchange: Option<Value>,
    history: Vec<Value>,
    active_run: Option<String>,
    selected_step: Option<String>,
    screenshot: Option<Arc<Image>>,
    screenshot_detail: String,
    enlarged: bool,
    revision: u64,
}

impl ApiRunView {
    /// Create an empty Run surface and load the scenario list.
    pub fn new(backend: SharedApiRunBackend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let mut view = Self {
            backend,
            focus_handle,
            scenarios: Vec::new(),
            selected: 0,
            events: Vec::new(),
            status: "Select a scenario and press Run.".into(),
            expand_plaintext: false,
            expanded_exchange: None,
            history: Vec::new(),
            active_run: None,
            selected_step: None,
            screenshot: None,
            screenshot_detail: "Select a step to view its screenshot.".into(),
            enlarged: false,
            revision: 0,
        };
        view.reload_scenarios(cx);
        view
    }

    /// Reload the scenario list from the host backend.
    pub fn reload_scenarios_public(&mut self, cx: &mut Context<Self>) {
        self.reload_scenarios(cx);
    }

    /// Start the currently selected scenario.
    pub fn run_selected_public(&mut self, cx: &mut Context<Self>) {
        self.run_selected(cx);
    }

    /// Open a history entry through the same path as its rendered control.
    pub fn open_history_public(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(id) = self
            .history
            .get(index)
            .and_then(|run| run["id"].as_str())
            .map(str::to_owned)
        {
            self.open_run(id, cx);
        }
    }

    /// Open a known replay identity, for example from an external host link.
    pub fn open_replay_public(&mut self, id: String, cx: &mut Context<Self>) {
        self.open_run(id, cx);
    }

    /// Number of screenshot outcomes available for step selection.
    pub fn capture_count(&self) -> usize {
        self.events
            .iter()
            .filter(|event| event.type_name == "step_screenshot")
            .count()
    }

    /// Select a screenshot event through the same path as its rendered row.
    pub fn select_capture_public(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(event) = self
            .events
            .iter()
            .filter(|event| event.type_name == "step_screenshot")
            .nth(index)
            .map(|event| event.payload.clone())
        {
            self.select_screenshot(event, cx);
        }
    }

    /// Toggle image enlargement without changing the selected run or step.
    pub fn toggle_image_public(&mut self, cx: &mut Context<Self>) {
        if self.screenshot.is_some() {
            self.enlarged = !self.enlarged;
            cx.notify();
        }
    }

    /// Observable screenshot state mirrored by the local E2E dock.
    pub fn screenshot_state(&self) -> Value {
        let selected = self.events.iter().find(|event| {
            event.type_name == "step_screenshot"
                && event.payload["step_id"].as_str() == self.selected_step.as_deref()
        });
        serde_json::json!({"run_id":self.active_run,"step_id":self.selected_step,
            "image_ready":self.screenshot.is_some(),"enlarged":self.enlarged,
            "detail":self.screenshot_detail,"history_count":self.history.len(),
            "step_status":selected.map(|event| &event.payload["status"]),
            "image_file":selected.map(|event| &event.payload["screenshot_file"])})
    }

    /// Toggle expansion of the latest HTTP exchange.
    pub fn toggle_expand_public(&mut self, cx: &mut Context<Self>) {
        self.toggle_expand(cx);
    }

    /// Status line shown on the Run surface.
    pub fn status_text(&self) -> String {
        self.status.to_string()
    }

    /// One-line summary of listed scenarios for the e2e DOM bridge.
    pub fn scenario_list_text(&self) -> String {
        if self.scenarios.is_empty() {
            return String::new();
        }
        self.scenarios
            .iter()
            .map(|scenario| format!("[{}] {}", scenario.engine_mode, scenario.name))
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// One-line summary of inspect events for the e2e DOM bridge.
    pub fn events_text(&self) -> String {
        self.events
            .iter()
            .map(Self::render_event_summary)
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Whether the inspect surface is showing expanded exchange details.
    pub fn secrets_expanded(&self) -> bool {
        self.expand_plaintext
    }

    fn reload_scenarios(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        self.active_run = None;
        self.events.clear();
        self.history.clear();
        self.selected_step = None;
        self.screenshot = None;
        self.enlarged = false;
        self.screenshot_detail = "Select a step to view its screenshot.".into();
        let backend = self.backend.clone();
        self.status = "Loading scenarios…".into();
        cx.spawn(async move |this, cx| {
            let result = backend.list_scenarios().await;
            let history = backend.replay_history().await;
            let _ = this.update(cx, |view, cx| {
                if view.revision != revision {
                    return;
                }
                match history {
                    Ok(history) => view.history = history,
                    Err(error) => {
                        view.screenshot_detail = format!("Cannot load replay history: {error}")
                    }
                }
                match result {
                    Ok(list) => {
                        view.scenarios = list;
                        if view.selected >= view.scenarios.len() {
                            view.selected = view.scenarios.len().saturating_sub(1);
                        }
                        view.status = format!("{} scenarios", view.scenarios.len()).into();
                    }
                    Err(error) => {
                        view.status = format!("Failed to list scenarios: {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_selected(&mut self, cx: &mut Context<Self>) {
        let Some(scenario) = self.scenarios.get(self.selected) else {
            self.status = "No scenario selected".into();
            cx.notify();
            return;
        };
        let id = scenario.id.clone();
        if self.backend.supports_replay_history() {
            self.revision += 1;
            let revision = self.revision;
            self.events.clear();
            self.active_run = None;
            self.selected_step = None;
            self.screenshot = None;
            self.enlarged = false;
            self.screenshot_detail = "Waiting for step screenshots…".into();
            self.status = format!("Starting {}…", scenario.name).into();
            let backend = self.backend.clone();
            cx.spawn(async move |this, cx| {
                let result = backend.begin_replay(&[id]).await;
                let _ = this.update(cx, |view, cx| {
                    if view.revision != revision {
                        return;
                    }
                    match result {
                        Ok(id) => view.open_run(id, cx),
                        Err(error) => view.status = format!("Cannot start run: {error}").into(),
                    }
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        let backend = self.backend.clone();
        self.status = format!("Running {}…", scenario.name).into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = backend.start_run(&[id]).await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(events) => {
                        view.events = events;
                        view.expanded_exchange = None;
                        view.status = format!("Run finished ({} events)", view.events.len()).into();
                    }
                    Err(error) => view.status = format!("Run failed: {error}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_run(&mut self, id: String, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        self.active_run = Some(id.clone());
        self.events.clear();
        self.expanded_exchange = None;
        self.selected_step = None;
        self.screenshot = None;
        self.enlarged = false;
        self.screenshot_detail = "Select a step to view its screenshot.".into();
        let backend = self.backend.clone();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                let result = backend.replay_snapshot(&id).await;
                let history = backend.replay_history().await;
                let keep_polling = this
                    .update(cx, |view, cx| {
                        if view.revision != revision {
                            return false;
                        }
                        if let Ok(history) = history {
                            view.history = history;
                        }
                        match result {
                            Ok(snapshot) => {
                                view.events = snapshot["events"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .map(|payload| ApiRunEventDto {
                                        type_name: payload["type"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .into(),
                                        payload: payload.clone(),
                                    })
                                    .collect();
                                let status = snapshot["status"].as_str().unwrap_or("unknown");
                                view.status = format!("Run {status} · {id}").into();
                                cx.notify();
                                status == "running"
                            }
                            Err(error) => {
                                view.screenshot = None;
                                view.selected_step = None;
                                view.enlarged = false;
                                view.events.clear();
                                view.screenshot_detail =
                                    "This run is unavailable in the current project.".into();
                                view.status = format!("Cannot load run: {error}").into();
                                cx.notify();
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
                executor.timer(Duration::from_millis(500)).await;
            }
        })
        .detach();
        cx.notify();
    }

    fn select_screenshot(&mut self, event: Value, cx: &mut Context<Self>) {
        let Some(run_id) = self.active_run.clone() else {
            return;
        };
        let Some(step_id) = event["step_id"].as_str().map(str::to_owned) else {
            return;
        };
        self.selected_step = Some(step_id.clone());
        self.screenshot = None;
        self.enlarged = false;
        self.screenshot_detail = match event["capture_status"].as_str() {
            Some("available") => "Loading screenshot…".into(),
            Some("unsupported") => "This backend does not support screenshots for this run.".into(),
            Some("failed") => format!(
                "Screenshot capture failed: {}",
                event["message"].as_str().unwrap_or("Capture unavailable")
            ),
            _ => "No screenshot is available for this step.".into(),
        };
        let Some(filename) = event["screenshot_file"].as_str().map(str::to_owned) else {
            cx.notify();
            return;
        };
        let backend = self.backend.clone();
        let revision = self.revision;
        cx.spawn(async move |this, cx| {
            let result = backend.replay_image(&run_id, &filename).await;
            let _ = this.update(cx, |view, cx| {
                // Discard images arriving after a different step, run, or project was selected.
                if view.revision != revision || view.selected_step.as_deref() != Some(&step_id) {
                    return;
                }
                match result {
                    Ok(bytes) => {
                        let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                            ImageFormat::Png
                        } else {
                            ImageFormat::Jpeg
                        };
                        view.screenshot = Some(Arc::new(Image::from_bytes(format, bytes)));
                        let scenario = event["scenario"]
                            .as_str()
                            .or_else(|| {
                                view.events
                                    .iter()
                                    .find(|candidate| {
                                        candidate.type_name == "start_case"
                                            && candidate.payload["case_id"] == event["case_id"]
                                    })
                                    .and_then(|candidate| candidate.payload["name"].as_str())
                            })
                            .unwrap_or("");
                        view.screenshot_detail = format!(
                            "{} · {}\n{} {} · {}\n{}",
                            event["feature"].as_str().unwrap_or(""),
                            scenario,
                            event["step_keyword"].as_str().unwrap_or(""),
                            event["step_text"].as_str().unwrap_or(""),
                            event["status"].as_str().unwrap_or(""),
                            event["captured_at"].as_str().unwrap_or("")
                        );
                    }
                    Err(error) => {
                        view.screenshot_detail = format!("Saved image cannot be loaded: {error}")
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_expand(&mut self, cx: &mut Context<Self>) {
        self.expand_plaintext = !self.expand_plaintext;
        if !self.expand_plaintext {
            self.expanded_exchange = None;
            self.status = "Showing redacted exchanges".into();
            cx.notify();
            return;
        }
        let Some(id) = self.events.iter().rev().find_map(|event| {
            if event.type_name == "http_exchange" {
                event
                    .payload
                    .get("exchange_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                None
            }
        }) else {
            self.status = "No HTTP exchange to expand".into();
            self.expand_plaintext = false;
            cx.notify();
            return;
        };
        let backend = self.backend.clone();
        self.status = "Loading exchange details…".into();
        cx.spawn(async move |this, cx| {
            let result = backend.get_exchange(&id, false).await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(value) => {
                        view.expanded_exchange = Some(value);
                        let redacted = view
                            .expanded_exchange
                            .as_ref()
                            .and_then(|value| value.get("redacted"))
                            .and_then(Value::as_bool)
                            .unwrap_or(true);
                        view.status = if redacted {
                            "Showing redacted details for latest exchange".into()
                        } else {
                            "Showing plaintext for latest exchange".into()
                        };
                    }
                    Err(error) => {
                        view.expand_plaintext = false;
                        view.status = format!("Expand failed: {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_event_summary(event: &ApiRunEventDto) -> String {
        match event.type_name.as_str() {
            "start_case" => format!(
                "case start {}",
                event
                    .payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ),
            "start_step" => format!(
                "  step {}",
                event
                    .payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ),
            "end_step" => format!(
                "  step {}",
                event
                    .payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("end")
            ),
            "step_screenshot" => format!(
                "  {} {} · {} · screenshot {}",
                event.payload["step_keyword"].as_str().unwrap_or(""),
                event.payload["step_text"].as_str().unwrap_or(""),
                event.payload["status"].as_str().unwrap_or(""),
                event.payload["capture_status"]
                    .as_str()
                    .unwrap_or("unavailable")
            ),
            "http_exchange" => {
                let method = event
                    .payload
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let url = event
                    .payload
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let redacted = event
                    .payload
                    .get("redacted")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                format!("    {method} {url}  redacted={redacted}")
            }
            "case_passed" => "case passed".into(),
            "case_failed" => format!(
                "case failed {}",
                event
                    .payload
                    .get("error")
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ),
            other => other.to_string(),
        }
    }
}

impl Focusable for ApiRunView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ApiRunView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.enlarged {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .p(px(16.))
                .gap(px(8.))
                .bg(rgb(0x11111b))
                .text_color(rgb(0xcdd6f4))
                .child(
                    div()
                        .id("replay-image-close")
                        .cursor_pointer()
                        .child("Close enlarged preview")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.toggle_image_public(cx);
                            }),
                        ),
                )
                .child(div().text_sm().child(self.screenshot_detail.clone()))
                .child(div().flex_1().min_h(px(0.)).w_full().when_some(
                    self.screenshot.clone(),
                    |element, image| {
                        element.child(img(image).size_full().object_fit(ObjectFit::Contain))
                    },
                ))
                .into_any_element();
        }
        let selected = self.selected;
        div()
            .size_full()
            .flex()
            .flex_row()
            .gap(px(12.))
            .p(px(16.))
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .w(px(320.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id("run-reload")
                                    .px(px(10.))
                                    .py(px(6.))
                                    .rounded(px(6.))
                                    .bg(rgb(0x313244))
                                    .cursor_pointer()
                                    .child("Refresh")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.reload_scenarios(cx);
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("run-start")
                                    .px(px(10.))
                                    .py(px(6.))
                                    .rounded(px(6.))
                                    .bg(rgb(0x313244))
                                    .cursor_pointer()
                                    .child("Run")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.run_selected(cx);
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("run-expand")
                                    .px(px(10.))
                                    .py(px(6.))
                                    .rounded(px(6.))
                                    .bg(rgb(0x313244))
                                    .cursor_pointer()
                                    .child("Expand secrets")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.toggle_expand(cx);
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .children(self.scenarios.iter().enumerate().map(
                                |(index, scenario)| {
                                    let selected = index == selected;
                                    div()
                                        .id(("api-scenario", index))
                                        .px(px(8.))
                                        .py(px(6.))
                                        .rounded(px(4.))
                                        .bg(if selected {
                                            rgb(0x45475a)
                                        } else {
                                            rgb(0x313244)
                                        })
                                        .child(format!(
                                            "[{}] {}",
                                            scenario.engine_mode, scenario.name
                                        ))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, _, cx| {
                                                this.selected = index;
                                                cx.notify();
                                            }),
                                        )
                                },
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xa6adc8))
                            .child(self.status.clone()),
                    )
                    .child(div().text_sm().child("Run history"))
                    .child(
                        div()
                            .id("replay-history")
                            .max_h(px(220.))
                            .overflow_y_scroll()
                            .children(self.history.iter().enumerate().map(|(index, run)| {
                                let id = run["id"].as_str().unwrap_or_default().to_string();
                                div()
                                    .id(("replay-run", index))
                                    .p(px(6.))
                                    .cursor_pointer()
                                    .bg(if self.active_run.as_deref() == Some(&id) {
                                        rgb(0x45475a)
                                    } else {
                                        rgb(0x313244)
                                    })
                                    .child(format!(
                                        "{} · {}",
                                        run["started_at"].as_str().unwrap_or(""),
                                        run["status"].as_str().unwrap_or("")
                                    ))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.open_run(id.clone(), cx)
                                        }),
                                    )
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xa6adc8))
                            .child("Events (read-only inspect — no Gherkin editor)"),
                    )
                    .child(
                        div()
                            .id("replay-events")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .children(self.events.iter().enumerate().map(|(index, event)| {
                                let payload = event.payload.clone();
                                div()
                                    .id(("replay-event", index))
                                    .text_sm()
                                    .p(px(4.))
                                    .child(Self::render_event_summary(event))
                                    .when(event.type_name == "step_screenshot", |element| {
                                        element
                                            .cursor_pointer()
                                            .bg(
                                                if self.selected_step.as_deref()
                                                    == event.payload["step_id"].as_str()
                                                {
                                                    rgb(0x45475a)
                                                } else {
                                                    rgb(0x313244)
                                                },
                                            )
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(move |this, _, _, cx| {
                                                    this.select_screenshot(payload.clone(), cx)
                                                }),
                                            )
                                    })
                            })),
                    )
                    .when_some(self.expanded_exchange.clone(), |this, value| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(rgb(0xf38ba8))
                                .child(value.to_string()),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(if self.enlarged { 760. } else { 360. }))
                    .min_w(px(200.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(div().text_sm().child("Step screenshot"))
                    .child(div().text_sm().child(self.screenshot_detail.clone()))
                    .when_some(self.screenshot.clone(), |panel, image| {
                        panel
                            .child(
                                div()
                                    .id("replay-image-toggle")
                                    .cursor_pointer()
                                    .child(if self.enlarged {
                                        "Close enlarged preview"
                                    } else {
                                        "Enlarge image"
                                    })
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.toggle_image_public(cx);
                                        }),
                                    ),
                            )
                            .child(
                                img(image)
                                    .w_full()
                                    .h(px(if self.enlarged { 650. } else { 280. }))
                                    .object_fit(ObjectFit::Contain),
                            )
                    }),
            )
            .into_any_element()
    }
}

/// Redacted inspector smoke: never prints live secret material from a fixture.
#[cfg(test)]
mod tests {
    use super::ApiRunView;
    use crate::backend::ApiRunEventDto;
    use serde_json::json;

    #[test]
    fn screenshot_controls_preserve_selection_and_clear_project_state() {
        use crate::replay_fixture::{ReplayFixtureBackend, ReplayFixtureMode};
        use gpui::TestAppContext;
        use std::rc::Rc;
        let mut cx = TestAppContext::single();
        cx.skip_drawing();
        let backend = ReplayFixtureBackend::default();
        backend.reset(ReplayFixtureMode::Failed);
        let hosted = backend.clone();
        let window = cx.add_window(move |window, cx| ApiRunView::new(Rc::new(hosted), window, cx));
        let view = window.root(&mut cx).unwrap();
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.run_selected_public(cx));
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.select_capture_public(1, cx));
        cx.run_until_parked();
        let selected = cx.read(|cx| view.read(cx).screenshot_state());
        assert_eq!(selected["image_ready"], true);
        assert_eq!(selected["step_status"], "failed");
        view.update(&mut cx, |view, cx| view.toggle_image_public(cx));
        assert_eq!(
            cx.read(|cx| view.read(cx).screenshot_state())["enlarged"],
            true
        );
        view.update(&mut cx, |view, cx| view.toggle_image_public(cx));
        assert_eq!(cx.read(|cx| view.read(cx).screenshot_state()), selected);
        backend.switch_project();
        view.update(&mut cx, |view, cx| view.reload_scenarios_public(cx));
        cx.run_until_parked();
        let cleared = cx.read(|cx| view.read(cx).screenshot_state());
        assert_eq!(cleared["image_ready"], false);
        assert!(cleared["run_id"].is_null());
        assert!(cleared["step_id"].is_null());
        assert_eq!(cleared["history_count"], 0);
        cx.quit();
    }

    #[test]
    fn missing_image_and_capture_failure_do_not_display_previous_step_image() {
        use crate::replay_fixture::{ReplayFixtureBackend, ReplayFixtureMode};
        use gpui::TestAppContext;
        use std::rc::Rc;
        for mode in [
            ReplayFixtureMode::Missing,
            ReplayFixtureMode::CaptureFailed,
            ReplayFixtureMode::Unsupported,
        ] {
            let mut cx = TestAppContext::single();
            cx.skip_drawing();
            let backend = ReplayFixtureBackend::default();
            backend.reset(mode);
            let window =
                cx.add_window(move |window, cx| ApiRunView::new(Rc::new(backend), window, cx));
            let view = window.root(&mut cx).unwrap();
            cx.run_until_parked();
            view.update(&mut cx, |view, cx| view.run_selected_public(cx));
            cx.run_until_parked();
            if mode == ReplayFixtureMode::Missing {
                view.update(&mut cx, |view, cx| view.select_capture_public(1, cx));
                cx.run_until_parked();
                assert_eq!(
                    cx.read(|cx| view.read(cx).screenshot_state())["image_ready"],
                    true
                );
            }
            view.update(&mut cx, |view, cx| view.select_capture_public(0, cx));
            cx.run_until_parked();
            let state = cx.read(|cx| view.read(cx).screenshot_state());
            assert_eq!(state["image_ready"], false);
            assert_eq!(state["step_status"], "passed");
            let detail = state["detail"].as_str().unwrap();
            assert!(
                detail.contains(match mode {
                    ReplayFixtureMode::Missing => "cannot be loaded",
                    ReplayFixtureMode::CaptureFailed => "capture failed",
                    _ => "does not support",
                }),
                "{detail}"
            );
            cx.quit();
        }
    }

    #[test]
    fn replay_history_restores_original_images_after_reopening() {
        use crate::replay_fixture::{ReplayFixtureBackend, ReplayFixtureMode};
        use gpui::TestAppContext;
        use std::rc::Rc;
        let mut cx = TestAppContext::single();
        cx.skip_drawing();
        let backend = ReplayFixtureBackend::default();
        backend.reset(ReplayFixtureMode::Success);
        let earlier = backend.seed_run();
        let later = backend.seed_run();
        let hosted = backend.clone();
        let window = cx.add_window(move |window, cx| ApiRunView::new(Rc::new(hosted), window, cx));
        let view = window.root(&mut cx).unwrap();
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.open_history_public(1, cx));
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.select_capture_public(0, cx));
        cx.run_until_parked();
        let original = cx.read(|cx| view.read(cx).screenshot_state());
        assert_eq!(original["run_id"], earlier);
        assert!(original["image_file"].as_str().unwrap().contains(&earlier));
        view.update(&mut cx, |view, cx| view.open_history_public(0, cx));
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.select_capture_public(0, cx));
        cx.run_until_parked();
        let current = cx.read(|cx| view.read(cx).screenshot_state());
        assert_eq!(current["run_id"], later);
        assert_ne!(current["image_file"], original["image_file"]);
        backend.set_closed(true);
        view.update(&mut cx, |view, cx| view.reload_scenarios_public(cx));
        cx.run_until_parked();
        backend.set_closed(false);
        view.update(&mut cx, |view, cx| view.reload_scenarios_public(cx));
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.open_history_public(1, cx));
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.select_capture_public(0, cx));
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| view.read(cx).screenshot_state()), original);
        cx.quit();
    }

    #[test]
    fn active_replay_shows_earlier_image_and_discards_late_project_response() {
        use crate::replay_fixture::{ReplayFixtureBackend, ReplayFixtureMode};
        use gpui::TestAppContext;
        use std::rc::Rc;
        let mut cx = TestAppContext::single();
        cx.skip_drawing();
        let backend = ReplayFixtureBackend::default();
        backend.reset(ReplayFixtureMode::Running);
        let hosted = backend.clone();
        let window = cx.add_window(move |window, cx| ApiRunView::new(Rc::new(hosted), window, cx));
        let view = window.root(&mut cx).unwrap();
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.run_selected_public(cx));
        cx.run_until_parked();
        view.update(&mut cx, |view, cx| view.select_capture_public(0, cx));
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| view.read(cx).screenshot_state())["image_ready"],
            true
        );
        assert!(
            cx.read(|cx| view.read(cx).status_text())
                .contains("running")
        );
        view.update(&mut cx, |view, cx| view.select_capture_public(0, cx));
        // The image request is pending on the foreground executor at this point.
        backend.switch_project();
        view.update(&mut cx, |view, cx| view.reload_scenarios_public(cx));
        cx.run_until_parked();
        let state = cx.read(|cx| view.read(cx).screenshot_state());
        assert_eq!(state["image_ready"], false);
        assert!(state["run_id"].is_null());
        assert!(state["step_id"].is_null());
        cx.quit();
    }

    #[test]
    fn event_summary_keeps_redacted_flag() {
        let event = ApiRunEventDto {
            type_name: "http_exchange".into(),
            payload: json!({
                "method": "POST",
                "url": "https://example.test/users",
                "redacted": true,
                "request_headers": {"Authorization": "***"}
            }),
        };
        let summary = ApiRunView::render_event_summary(&event);
        assert!(summary.contains("redacted=true"));
        assert!(!summary.to_lowercase().contains("bearer"));
    }
}
