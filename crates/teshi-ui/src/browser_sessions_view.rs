//! Shared GPUI browser-profile discovery and explicit tab-selection surface.

use gpui::{
    App, Context, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, SharedString, Styled, Window, div, px, rgb,
};

use crate::backend::{
    BrowserSessionListSnapshot, BrowserSessionSnapshot, BrowserTabTarget,
    SharedBrowserSessionsBackend,
};

#[derive(Debug, Clone, Default)]
struct BrowserSessionSelection {
    sessions: Vec<BrowserSessionSnapshot>,
    selected_id: Option<String>,
    explicitly_selected: bool,
}

impl BrowserSessionSelection {
    fn replace(&mut self, sessions: Vec<BrowserSessionSnapshot>) {
        self.sessions = sessions;
        if self.explicitly_selected {
            return;
        }

        let eligible = self
            .sessions
            .iter()
            .filter(|session| session.is_eligible())
            .collect::<Vec<_>>();
        self.selected_id =
            (eligible.len() == 1).then(|| eligible[0].identity.extension_instance_id.clone());
    }

    fn select(&mut self, extension_instance_id: String) {
        self.selected_id = Some(extension_instance_id);
        self.explicitly_selected = true;
    }

    fn selected(&self) -> Option<&BrowserSessionSnapshot> {
        let selected = self.selected_id.as_deref()?;
        self.sessions
            .iter()
            .find(|session| session.identity.extension_instance_id == selected)
    }

    fn has_missing_explicit_selection(&self) -> bool {
        self.explicitly_selected && self.selected_id.is_some() && self.selected().is_none()
    }
}

/// Shared browser-profile view rendered by native Desktop and GPUI WASM Web.
pub struct BrowserSessionsView {
    backend: SharedBrowserSessionsBackend,
    focus_handle: FocusHandle,
    model: BrowserSessionSelection,
    status: SharedString,
}

impl BrowserSessionsView {
    /// Create the view and perform one initial broker refresh.
    pub fn new(
        backend: SharedBrowserSessionsBackend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let mut view = Self {
            backend,
            focus_handle,
            model: BrowserSessionSelection::default(),
            status: "Loading browser sessions…".into(),
        };
        view.refresh();
        view
    }

    /// Reload sessions from the host backend.
    pub fn refresh_public(&mut self, cx: &mut Context<Self>) {
        self.refresh();
        cx.notify();
    }

    /// Start the Chrome bridge from the Browser Profiles surface.
    pub fn start_bridge_public(&mut self, cx: &mut Context<Self>) {
        self.start_bridge(cx);
    }

    /// Select the first eligible connected profile (explicit user choice).
    pub fn select_first_eligible(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .model
            .sessions
            .iter()
            .find(|session| session.is_eligible())
            .map(|session| session.identity.extension_instance_id.clone())
        else {
            self.status = "No eligible browser profile to select.".into();
            cx.notify();
            return;
        };
        self.select_session(id, cx);
    }

    /// Status line shown on Browser Profiles.
    pub fn status_text(&self) -> String {
        self.status.to_string()
    }

    /// Number of connected browser profiles.
    pub fn profile_count(&self) -> usize {
        self.model.sessions.len()
    }

    /// Whether a profile is selected without an explicit user click.
    pub fn auto_selected(&self) -> bool {
        self.model.selected_id.is_some() && !self.model.explicitly_selected
    }

    /// Whether the user has explicitly selected a profile.
    pub fn explicitly_selected(&self) -> bool {
        self.model.explicitly_selected && self.model.selected().is_some()
    }

    fn refresh(&mut self) {
        match self.backend.list_browser_sessions() {
            Ok(BrowserSessionListSnapshot { sessions, .. }) => {
                self.model.replace(sessions);
                self.status = match self.model.sessions.len() {
                    0 => "No browser extension sessions. Start the Chrome bridge, then reload the extension.".into(),
                    1 => "One browser profile connected.".into(),
                    count if self.model.selected_id.is_none() => format!(
                        "{count} browser profiles connected. Select one explicitly before viewing tabs."
                    )
                    .into(),
                    count => format!("{count} browser profiles connected.").into(),
                };
                if self.model.has_missing_explicit_selection() {
                    self.status = "The selected browser profile disconnected. It was not replaced automatically.".into();
                }
            }
            Err(error) => {
                self.model.replace(Vec::new());
                self.status = format!("Browser bridge unavailable: {error}").into();
            }
        }
    }

    fn start_bridge(&mut self, cx: &mut Context<Self>) {
        self.status = "Starting Chrome bridge…".into();
        match self.backend.start_browser_bridge() {
            Ok(()) => self.refresh(),
            Err(error) => self.status = format!("Start Chrome bridge failed: {error}").into(),
        }
        cx.notify();
    }

    fn select_session(&mut self, extension_instance_id: String, cx: &mut Context<Self>) {
        self.model.select(extension_instance_id);
        self.status = "Browser profile selected explicitly.".into();
        cx.notify();
    }

    fn activate_tab(&mut self, target: BrowserTabTarget, cx: &mut Context<Self>) {
        match self.backend.activate_browser_tab(&target) {
            Ok(()) => {
                self.status =
                    "Tab activation queued; refresh after the next extension heartbeat.".into();
                self.refresh();
            }
            Err(error) => self.status = format!("Tab activation failed: {error}").into(),
        }
        cx.notify();
    }

    fn action_button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
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

    fn session_label(session: &BrowserSessionSnapshot) -> String {
        session
            .identity
            .profile_label
            .as_deref()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or("Unnamed profile")
            .to_string()
    }

    fn short_id(id: &str) -> String {
        id.chars().take(8).collect()
    }

    fn health_color(health: &str) -> gpui::Rgba {
        match health {
            "ready" => rgb(0xa6e3a1),
            "stale" | "debugger_conflict" => rgb(0xf9e2af),
            _ => rgb(0xf38ba8),
        }
    }

    fn render_session_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().w(px(280.)).flex().flex_col().gap(px(8.));
        for session in &self.model.sessions {
            let id = session.identity.extension_instance_id.clone();
            let click_id = id.clone();
            let selected = self.model.selected_id.as_deref() == Some(id.as_str());
            let lease = session
                .lease
                .as_ref()
                .map(|lease| format!("Leased by {}", lease.owner_label))
                .unwrap_or_else(|| "Available".into());
            list = list.child(
                div()
                    .id(SharedString::from(format!("browser-session-{id}")))
                    .p(px(10.))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(if selected {
                        rgb(0x89b4fa)
                    } else {
                        rgb(0x45475a)
                    })
                    .bg(if selected {
                        rgb(0x313244)
                    } else {
                        rgb(0x181825)
                    })
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0xcdd6f4))
                                    .child(Self::session_label(session)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(Self::health_color(&session.health))
                                    .child(session.health.to_uppercase()),
                            ),
                    )
                    .child(div().text_sm().text_color(rgb(0xa6adc8)).child(format!(
                        "{} · {} {}",
                        Self::short_id(&id),
                        session.browser.name,
                        session.browser.version
                    )))
                    .child(div().text_sm().text_color(rgb(0x6c7086)).child(lease))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.select_session(click_id.clone(), cx);
                        }),
                    ),
            );
        }
        if self.model.sessions.is_empty() {
            list = list.child(
                div()
                    .p(px(12.))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(rgb(0x45475a))
                    .text_sm()
                    .text_color(rgb(0x6c7086))
                    .child("No extension heartbeat received."),
            );
        }
        list
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div().flex_1().min_w(px(0.)).flex().flex_col().gap(px(8.));
        let Some(selected_id) = self.model.selected_id.as_deref() else {
            return panel.child(
                div()
                    .p(px(16.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(0x45475a))
                    .text_color(rgb(0xf9e2af))
                    .child("Select a browser profile. No profile is chosen automatically while multiple profiles are available."),
            );
        };
        let Some(session) = self.model.selected() else {
            return panel.child(
                div()
                    .p(px(16.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(0xf38ba8))
                    .text_color(rgb(0xf38ba8))
                    .child(format!(
                        "Selected profile {} is disconnected. Refresh or reconnect that same profile.",
                        Self::short_id(selected_id)
                    )),
            );
        };

        panel = panel.child(
            div()
                .text_lg()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(0xcdd6f4))
                .child(format!("Tabs in {}", Self::session_label(session))),
        );
        let extension_instance_id = session.identity.extension_instance_id.clone();
        let mut tab_count = 0usize;
        for window_snapshot in &session.windows {
            for tab in &window_snapshot.tabs {
                tab_count += 1;
                let target = BrowserTabTarget {
                    extension_instance_id: extension_instance_id.clone(),
                    window_id: tab.window_id.unwrap_or(window_snapshot.id),
                    tab_id: tab.id,
                };
                let active = tab.active;
                let can_activate = tab.debuggable && session.health == "ready";
                let title = if tab.title.trim().is_empty() {
                    "Untitled tab".to_string()
                } else {
                    tab.title.clone()
                };
                panel = panel.child(
                    div()
                        .p(px(10.))
                        .rounded(px(7.))
                        .border_1()
                        .border_color(if active { rgb(0xa6e3a1) } else { rgb(0x45475a) })
                        .bg(rgb(0x181825))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(10.))
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(div().text_color(rgb(0xcdd6f4)).child(if active {
                                    format!("● {title}")
                                } else {
                                    title
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(0x6c7086))
                                        .child(tab.url.clone()),
                                ),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "activate-tab-{}-{}",
                                    target.window_id, target.tab_id
                                )))
                                .px(px(10.))
                                .py(px(6.))
                                .rounded(px(6.))
                                .bg(if can_activate {
                                    rgb(0x313244)
                                } else {
                                    rgb(0x242434)
                                })
                                .text_color(if can_activate {
                                    rgb(0xcdd6f4)
                                } else {
                                    rgb(0x585b70)
                                })
                                .text_sm()
                                .cursor_pointer()
                                .child(if active {
                                    "Active"
                                } else if tab.debuggable {
                                    "Activate"
                                } else {
                                    "Unavailable"
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        if can_activate && !active {
                                            this.activate_tab(target.clone(), cx);
                                        }
                                    }),
                                ),
                        ),
                );
            }
        }
        if tab_count == 0 {
            panel = panel.child(
                div()
                    .text_color(rgb(0x6c7086))
                    .child("This profile reports no tabs."),
            );
        }
        panel
    }
}

impl Focusable for BrowserSessionsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for BrowserSessionsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .track_focus(&self.focus_handle(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0xcdd6f4))
                                    .child("Browser Profiles"),
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
                            .flex()
                            .gap(px(8.))
                            .child(self.action_button(
                                "start-browser-bridge",
                                "Connect Chrome",
                                cx,
                                |this, cx| {
                                    this.start_bridge(cx);
                                },
                            ))
                            .child(self.action_button(
                                "refresh-browser-sessions",
                                "Refresh",
                                cx,
                                |this, cx| {
                                    this.refresh();
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .gap(px(14.))
                    .child(self.render_session_list(cx))
                    .child(self.render_tabs(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        BrowserMetadataSnapshot, BrowserSessionIdentitySnapshot, BrowserTabSnapshot,
        BrowserWindowSnapshot,
    };

    fn session(id: &str, health: &str) -> BrowserSessionSnapshot {
        BrowserSessionSnapshot {
            identity: BrowserSessionIdentitySnapshot {
                extension_instance_id: id.into(),
                profile_label: Some(format!("profile-{id}")),
                extension_version: "0.7.9".into(),
                protocol_version: 1,
            },
            browser: BrowserMetadataSnapshot {
                name: "Chrome".into(),
                version: "151".into(),
                platform: Some("Win32".into()),
            },
            health: health.into(),
            windows: vec![BrowserWindowSnapshot {
                id: 1,
                tabs: vec![BrowserTabSnapshot {
                    id: 2,
                    window_id: Some(1),
                    title: "Example".into(),
                    url: "https://example.com".into(),
                    active: true,
                    debuggable: true,
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn one_eligible_session_is_selected_for_compatibility() {
        let mut model = BrowserSessionSelection::default();
        model.replace(vec![session("a", "ready")]);
        assert_eq!(model.selected_id.as_deref(), Some("a"));
        assert!(!model.explicitly_selected);
    }

    #[test]
    fn multiple_sessions_are_never_implicitly_selected() {
        let mut model = BrowserSessionSelection::default();
        model.replace(vec![session("a", "ready"), session("b", "ready")]);
        assert_eq!(model.selected_id, None);
    }

    #[test]
    fn adding_a_second_session_clears_compatibility_selection() {
        let mut model = BrowserSessionSelection::default();
        model.replace(vec![session("a", "ready")]);
        model.replace(vec![session("a", "ready"), session("b", "ready")]);
        assert_eq!(model.selected_id, None);
    }

    #[test]
    fn explicit_selection_is_retained_when_profile_disconnects() {
        let mut model = BrowserSessionSelection::default();
        model.replace(vec![session("a", "ready"), session("b", "ready")]);
        model.select("b".into());
        model.replace(vec![session("a", "ready")]);
        assert_eq!(model.selected_id.as_deref(), Some("b"));
        assert!(model.has_missing_explicit_selection());
    }
}
