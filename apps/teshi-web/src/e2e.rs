//! DOM bridge so teshi browser locators can drive the GPUI WASM canvas.
//!
//! GPUI paints to `<canvas>`, which has no Playwright-accessible controls.
//! When the page is opened with `?e2e=1`, this module mounts a visible dock of
//! `data-testid` buttons and status nodes that call into [`AppShell`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{AppCell, Entity};
use serde_json::json;
use teshi_ui::{
    ApiRunBackend, ApiRunEventDto, ApiScenarioSnapshot, AppShell, BackendFuture,
    BrowserMetadataSnapshot, BrowserSessionIdentitySnapshot, BrowserSessionListSnapshot,
    BrowserSessionSnapshot, BrowserSessionsBackend, BrowserTabSnapshot, BrowserTabTarget,
    BrowserWindowSnapshot, ShellSurface,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Document, HtmlElement};

struct E2eHost {
    app: Rc<AppCell>,
    shell: Entity<AppShell>,
}

thread_local! {
    static E2E: RefCell<Option<E2eHost>> = const { RefCell::new(None) };
    static BRIDGE_ATTEMPTED: Cell<bool> = const { Cell::new(false) };
}

/// Whether the page requested the locator-friendly e2e dock.
pub fn e2e_enabled() -> bool {
    super::query_parameter("e2e").as_deref() == Some("1")
}

/// Return the deterministic browser backend used only by the local E2E dock.
///
/// The production Web shell uses the daemon-backed implementation.  The
/// explicit `?e2e=1` harness needs repeatable profile and bridge states so its
/// business Features can be replayed without depending on a user's Chrome
/// profiles or extension installation.
pub fn browser_backend() -> Rc<dyn BrowserSessionsBackend> {
    Rc::new(E2eBrowserSessionsBackend)
}

/// Return the deterministic Run backend used only by the local E2E dock.
pub fn api_run_backend() -> Rc<dyn ApiRunBackend> {
    Rc::new(E2eApiRunBackend)
}

struct E2eBrowserSessionsBackend;

impl BrowserSessionsBackend for E2eBrowserSessionsBackend {
    fn start_browser_bridge(&self) -> BackendFuture<()> {
        mark_bridge_attempted();
        Box::pin(async { Err("E2E fixture: Chrome bridge unavailable".to_string()) })
    }

    fn list_browser_sessions(&self) -> BackendFuture<BrowserSessionListSnapshot> {
        Box::pin(async {
            Ok(BrowserSessionListSnapshot {
                extension_connected: true,
                ambiguous_browser_target: true,
                sessions: vec![
                    fixture_browser_session("e2e-profile-a", "E2E Profile A", 1, 101),
                    fixture_browser_session("e2e-profile-b", "E2E Profile B", 2, 201),
                ],
            })
        })
    }

    fn activate_browser_tab(&self, _target: &BrowserTabTarget) -> BackendFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

fn fixture_browser_session(
    extension_instance_id: &str,
    profile_label: &str,
    window_id: i64,
    tab_id: i64,
) -> BrowserSessionSnapshot {
    BrowserSessionSnapshot {
        identity: BrowserSessionIdentitySnapshot {
            extension_instance_id: extension_instance_id.to_string(),
            profile_label: Some(profile_label.to_string()),
            extension_version: "e2e-fixture".to_string(),
            protocol_version: 1,
        },
        browser: BrowserMetadataSnapshot {
            name: "Chrome".to_string(),
            version: "e2e-fixture".to_string(),
            platform: Some("Windows".to_string()),
        },
        health: "ready".to_string(),
        last_heartbeat_age_ms: 0,
        windows: vec![BrowserWindowSnapshot {
            id: window_id,
            focused: true,
            tabs: vec![BrowserTabSnapshot {
                id: tab_id,
                window_id: Some(window_id),
                title: "Teshi E2E fixture".to_string(),
                url: "about:blank".to_string(),
                active: true,
                debuggable: true,
            }],
        }],
        lease: None,
    }
}

struct E2eApiRunBackend;

impl ApiRunBackend for E2eApiRunBackend {
    fn list_scenarios(&self) -> BackendFuture<Vec<ApiScenarioSnapshot>> {
        Box::pin(async {
            Ok(vec![ApiScenarioSnapshot {
                id: "e2e-api-scenario".to_string(),
                feature_path: "features/e2e-fixture.feature".to_string(),
                name: "Create a user then fetch by extracted id".to_string(),
                tags: vec!["@api".to_string()],
                engine_mode: "api".to_string(),
            }])
        })
    }

    fn start_run(&self, _scenario_ids: &[String]) -> BackendFuture<Vec<ApiRunEventDto>> {
        Box::pin(async {
            Ok(vec![
                ApiRunEventDto {
                    type_name: "start_case".to_string(),
                    payload: json!({
                        "type": "start_case",
                        "name": "Create a user then fetch by extracted id"
                    }),
                },
                ApiRunEventDto {
                    type_name: "start_step".to_string(),
                    payload: json!({
                        "type": "start_step",
                        "text": "the API fixture is available"
                    }),
                },
                ApiRunEventDto {
                    type_name: "http_exchange".to_string(),
                    payload: json!({
                        "type": "http_exchange",
                        "exchange_id": "e2e-exchange",
                        "method": "GET",
                        "url": "https://e2e.fixture.test/users/1",
                        "redacted": true
                    }),
                },
                ApiRunEventDto {
                    type_name: "end_step".to_string(),
                    payload: json!({ "type": "end_step", "status": "passed" }),
                },
                ApiRunEventDto {
                    type_name: "case_passed".to_string(),
                    payload: json!({ "type": "case_passed" }),
                },
            ])
        })
    }

    fn get_exchange(&self, exchange_id: &str, _redact: bool) -> BackendFuture<serde_json::Value> {
        let exchange_id = exchange_id.to_string();
        Box::pin(async move {
            if exchange_id != "e2e-exchange" {
                return Err(format!("E2E fixture exchange not found: {exchange_id}"));
            }
            Ok(json!({
                "exchange_id": "e2e-exchange",
                "redacted": false,
                "fixture_value": "plaintext fixture value"
            }))
        })
    }
}

fn mark_bridge_attempted() {
    BRIDGE_ATTEMPTED.with(|attempted| attempted.set(true));
}

/// Mount the e2e dock and bind it to the live [`AppShell`].
pub fn install(app: Rc<AppCell>, shell: Entity<AppShell>) {
    BRIDGE_ATTEMPTED.with(|attempted| attempted.set(false));
    E2E.with(|slot| {
        *slot.borrow_mut() = Some(E2eHost {
            app: app.clone(),
            shell: shell.clone(),
        });
    });
    if let Err(error) = mount_dom() {
        web_sys::console::error_1(&JsValue::from_str(&format!("e2e dock: {error}")));
        return;
    }
    // AppCell is still borrowed during window setup; sync after the stack unwinds.
    if let Some(window) = web_sys::window() {
        let closure = Closure::wrap(Box::new(sync_status) as Box<dyn FnMut()>);
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            0,
        );
        closure.forget();

        // AppShell updates arrive asynchronously through the control channel.
        // Keep the locator-facing DOM in sync after those responses arrive,
        // rather than only refreshing it immediately after a button click.
        let closure = Closure::wrap(Box::new(sync_status) as Box<dyn FnMut()>);
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            250,
        );
        closure.forget();
    }
}

fn document() -> Result<Document, String> {
    web_sys::window()
        .ok_or_else(|| "window unavailable".to_string())?
        .document()
        .ok_or_else(|| "document unavailable".to_string())
}

fn mount_dom() -> Result<(), String> {
    let document = document()?;
    if document.get_element_by_id("e2e-root").is_some() {
        bind_buttons()?;
        return Ok(());
    }
    let root = document
        .create_element("div")
        .map_err(|e| format!("create e2e-root: {e:?}"))?;
    root.set_id("e2e-root");
    root.set_attribute("data-testid", "e2e-root")
        .map_err(|e| format!("testid e2e-root: {e:?}"))?;
    root.set_inner_html(
        r#"
<button type="button" data-testid="open-browser-sessions">Browser</button>
<button type="button" data-testid="open-winapp-preview">Preview</button>
<button type="button" data-testid="open-api-run">Run</button>
<button type="button" data-testid="open-settings">Settings</button>
<button type="button" data-testid="refresh-browser-sessions">Refresh profiles</button>
<button type="button" data-testid="start-browser-bridge">Connect Chrome</button>
<button type="button" data-testid="select-first-browser-profile">Select first profile</button>
<button type="button" data-testid="run-reload">Refresh scenarios</button>
<button type="button" data-testid="run-start">Run scenario</button>
<button type="button" data-testid="run-expand">Expand secrets</button>
<div data-testid="e2e-surface" role="status">browser</div>
<div data-testid="e2e-browser-status" role="status"></div>
<div data-testid="e2e-run-status" role="status"></div>
<div data-testid="e2e-scenarios" role="status"></div>
<div data-testid="e2e-events" role="status"></div>
<div data-testid="e2e-editor" role="status">none</div>
<div data-testid="e2e-profile-count" role="status">0</div>
<div data-testid="e2e-auto-selected" role="status">false</div>
<div data-testid="e2e-profile-selected" role="status">false</div>
<div data-testid="e2e-bridge-unavailable" role="status">true</div>
<div data-testid="e2e-bridge-attempted" role="status">false</div>
"#,
    );
    let body = document
        .body()
        .ok_or_else(|| "document body unavailable".to_string())?;
    body.append_child(&root)
        .map_err(|e| format!("append e2e-root: {e:?}"))?;
    bind_buttons()
}

fn bind_buttons() -> Result<(), String> {
    bind_click("open-browser-sessions", || {
        with_shell(|shell, cx| shell.show_surface(ShellSurface::Browser, None, cx))
    })?;
    bind_click("open-winapp-preview", || {
        with_shell(|shell, cx| shell.show_surface(ShellSurface::WinApp, None, cx))
    })?;
    bind_click("open-api-run", || {
        with_shell(|shell, cx| shell.show_surface(ShellSurface::Run, None, cx))
    })?;
    bind_click("open-settings", || {
        with_shell(|shell, cx| shell.show_surface(ShellSurface::Settings, None, cx))
    })?;
    bind_click("refresh-browser-sessions", || {
        with_shell(|shell, cx| {
            shell
                .browser_sessions()
                .update(cx, |view, cx| view.refresh_public(cx));
        })
    })?;
    bind_click("start-browser-bridge", || {
        mark_bridge_attempted();
        with_shell(|shell, cx| {
            shell
                .browser_sessions()
                .update(cx, |view, cx| view.start_bridge_public(cx));
        })
    })?;
    bind_click("select-first-browser-profile", || {
        with_shell(|shell, cx| {
            shell
                .browser_sessions()
                .update(cx, |view, cx| view.select_first_eligible(cx));
        })
    })?;
    bind_click("run-reload", || {
        with_shell(|shell, cx| {
            shell
                .api_run()
                .update(cx, |view, cx| view.reload_scenarios_public(cx));
        })
    })?;
    bind_click("run-start", || {
        with_shell(|shell, cx| {
            shell
                .api_run()
                .update(cx, |view, cx| view.run_selected_public(cx));
        })
    })?;
    bind_click("run-expand", || {
        with_shell(|shell, cx| {
            shell
                .api_run()
                .update(cx, |view, cx| view.toggle_expand_public(cx));
        })
    })?;
    Ok(())
}

fn bind_click(test_id: &'static str, on_click: impl Fn() + 'static) -> Result<(), String> {
    let document = document()?;
    let selector = format!("[data-testid=\"{test_id}\"]");
    let element = document
        .query_selector(&selector)
        .map_err(|e| format!("query {test_id}: {e:?}"))?
        .ok_or_else(|| format!("missing {test_id}"))?;
    let html: HtmlElement = element
        .dyn_into()
        .map_err(|_| format!("{test_id} is not an HTMLElement"))?;
    let closure = Closure::wrap(Box::new(move || {
        on_click();
        sync_status();
    }) as Box<dyn FnMut()>);
    html.set_onclick(Some(closure.as_ref().unchecked_ref()));
    closure.forget();
    Ok(())
}

fn with_shell(update: impl FnOnce(&mut AppShell, &mut gpui::Context<AppShell>)) {
    E2E.with(|slot| {
        let host_slot = slot.borrow();
        let Some(host) = host_slot.as_ref() else {
            return;
        };
        if let Ok(mut cx) = host.app.try_borrow_mut() {
            let app: &mut gpui::App = std::ops::DerefMut::deref_mut(&mut cx);
            host.shell.update(app, |shell, cx| update(shell, cx));
        }
    });
}

fn sync_status() {
    with_shell(|shell, cx| {
        let surface = shell.surface().as_str().to_string();
        let browser = shell.browser_sessions().read(cx);
        let browser_status = browser.status_text();
        let profile_count = browser.profile_count().to_string();
        let auto_selected = if browser.auto_selected() {
            "true"
        } else {
            "false"
        }
        .to_string();
        let profile_selected = if browser.explicitly_selected() {
            "true"
        } else {
            "false"
        }
        .to_string();
        let run = shell.api_run().read(cx);
        let run_status = run.status_text();
        let scenarios = run.scenario_list_text();
        let events = run.events_text();
        let _ = run;
        let bridge_attempted =
            BRIDGE_ATTEMPTED.with(|attempted| if attempted.get() { "true" } else { "false" });
        set_text("e2e-surface", &surface);
        set_text("e2e-browser-status", &browser_status);
        set_text("e2e-run-status", &run_status);
        set_text("e2e-scenarios", &scenarios);
        set_text("e2e-events", &events);
        set_text("e2e-profile-count", &profile_count);
        set_text("e2e-auto-selected", &auto_selected);
        set_text("e2e-profile-selected", &profile_selected);
        set_text("e2e-bridge-unavailable", "true");
        set_text("e2e-bridge-attempted", bridge_attempted);
    });
}

fn set_text(test_id: &str, value: &str) {
    let Ok(document) = document() else {
        return;
    };
    let Ok(Some(element)) = document.query_selector(&format!("[data-testid=\"{test_id}\"]")) else {
        return;
    };
    element.set_text_content(Some(value));
}
