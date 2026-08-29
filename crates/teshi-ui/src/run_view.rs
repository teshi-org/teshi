//! GPUI Run/API inspect surface (no Gherkin editor).

use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, SharedString, Styled, Window, div, prelude::FluentBuilder, px, rgb,
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
        };
        view.reload_scenarios();
        view
    }

    /// Reload the scenario list from the host backend.
    pub fn reload_scenarios_public(&mut self, cx: &mut Context<Self>) {
        self.reload_scenarios();
        cx.notify();
    }

    /// Start the currently selected scenario.
    pub fn run_selected_public(&mut self, cx: &mut Context<Self>) {
        self.run_selected(cx);
    }

    /// Toggle plaintext expansion of the latest HTTP exchange.
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

    /// Whether the inspect surface is showing plaintext secrets.
    pub fn secrets_expanded(&self) -> bool {
        self.expand_plaintext
    }

    fn reload_scenarios(&mut self) {
        match self.backend.list_scenarios() {
            Ok(list) => {
                self.scenarios = list;
                if self.selected >= self.scenarios.len() {
                    self.selected = self.scenarios.len().saturating_sub(1);
                }
                self.status = format!("{} scenarios", self.scenarios.len()).into();
            }
            Err(err) => {
                self.status = format!("Failed to list scenarios: {err}").into();
            }
        }
    }

    fn run_selected(&mut self, cx: &mut Context<Self>) {
        let Some(scenario) = self.scenarios.get(self.selected) else {
            self.status = "No scenario selected".into();
            cx.notify();
            return;
        };
        let id = scenario.id.clone();
        self.status = format!("Running {}…", scenario.name).into();
        cx.notify();
        match self.backend.start_run(&[id]) {
            Ok(events) => {
                self.events = events;
                self.expanded_exchange = None;
                self.status = format!("Run finished ({} events)", self.events.len()).into();
            }
            Err(err) => {
                self.status = format!("Run failed: {err}").into();
            }
        }
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
        match self.backend.get_exchange(&id, false) {
            Ok(value) => {
                self.expanded_exchange = Some(value);
                self.status = "Showing plaintext for latest exchange".into();
            }
            Err(err) => {
                self.expand_plaintext = false;
                self.status = format!("Expand failed: {err}").into();
            }
        }
        cx.notify();
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
                                            this.reload_scenarios();
                                            cx.notify();
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
                    .children(
                        self.events
                            .iter()
                            .map(|event| div().text_sm().child(Self::render_event_summary(event))),
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
    }
}

/// Redacted inspector smoke: never prints live secret material from a fixture.
#[cfg(test)]
mod tests {
    use super::ApiRunView;
    use crate::backend::ApiRunEventDto;
    use serde_json::json;

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
