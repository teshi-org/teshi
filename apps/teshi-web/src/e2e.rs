//! DOM bridge so teshi browser locators can drive the GPUI WASM canvas.
//!
//! GPUI paints to `<canvas>`, which has no Playwright-accessible controls.
//! When the page is opened with `?e2e=1`, this module mounts a visible dock of
//! `data-testid` buttons and status nodes that call into [`AppShell`].

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{AppCell, Entity};
use teshi_ui::{AppShell, ShellSurface};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Document, HtmlElement};

struct E2eHost {
    app: Rc<AppCell>,
    shell: Entity<AppShell>,
}

thread_local! {
    static E2E: RefCell<Option<E2eHost>> = const { RefCell::new(None) };
}

/// Whether the page requested the locator-friendly e2e dock.
pub fn e2e_enabled() -> bool {
    super::query_parameter("e2e").as_deref() == Some("1")
}

/// Mount the e2e dock and bind it to the live [`AppShell`].
pub fn install(app: Rc<AppCell>, shell: Entity<AppShell>) {
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
        drop(run);
        set_text("e2e-surface", &surface);
        set_text("e2e-browser-status", &browser_status);
        set_text("e2e-run-status", &run_status);
        set_text("e2e-scenarios", &scenarios);
        set_text("e2e-events", &events);
        set_text("e2e-profile-count", &profile_count);
        set_text("e2e-auto-selected", &auto_selected);
        set_text("e2e-profile-selected", &profile_selected);
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
