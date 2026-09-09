//! GPUI WASM shell hosted at teshi.org and connected to a local daemon.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use base64::Engine as _;
use futures_channel::oneshot;
use gpui::{AppCell, Entity, prelude::*};
use teshi_ui::{
    ApiRunBackend, ApiRunEventDto, ApiScenarioSnapshot, AppShell, BackendFuture,
    BrowserSessionListSnapshot, BrowserSessionsBackend, BrowserTabTarget, LlmConfigBackend,
    LlmConfigSnapshot, LlmConfigUpdate, ModelProfileListSnapshot, ModelProfileSnapshot,
    ModelProfileUpdate, WinAppPreview, bind_llm_config_keys,
};
use teshi_web_protocol::{
    CONTROL_PROTOCOL_VERSION, Channel, CliBuildIdentity, ClientHello, ClientMessage, ErrorCode,
    PREVIEW_PROTOCOL_VERSION, ServerMessage, UiCompatibility, UiManifest,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

mod e2e;

fn query_parameter(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get(name).filter(|value| !value.trim().is_empty())
}

#[derive(Debug, Clone)]
struct LaunchState {
    port: u16,
    token: String,
}

fn parse_launch_state() -> Result<LaunchState, String> {
    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_string())?;
    let location = window.location();
    let hash = location
        .hash()
        .map_err(|error| format!("read launch fragment: {error:?}"))?;
    let fragment = hash.strip_prefix('#').unwrap_or(hash.as_str());
    if fragment.trim().is_empty() {
        return Err("missing launch fragment; start the UI with `teshi web`".into());
    }
    let params = web_sys::UrlSearchParams::new_with_str(fragment)
        .map_err(|error| format!("parse launch fragment: {error:?}"))?;
    let raw_port = params
        .get("port")
        .ok_or_else(|| "launch fragment is missing port".to_string())?;
    let port = raw_port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| "launch fragment contains an invalid loopback port".to_string())?;
    let token = params
        .get("token")
        .filter(|token| token.len() >= 16 && token.len() <= 256)
        .ok_or_else(|| "launch fragment is missing a valid session token".to_string())?;
    if !token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("launch token contains unsupported characters".into());
    }

    // The fragment never needs to be sent back to teshi.org. Remove it before
    // any normal startup/fetch work so it cannot become a referrer or history
    // entry. Keep the parsed values only in process memory.
    let href = location
        .href()
        .map_err(|error| format!("read launch URL: {error:?}"))?;
    let clean_url = href.split('#').next().unwrap_or(&href);
    window
        .history()
        .map_err(|error| format!("read browser history: {error:?}"))?
        .replace_state_with_url(&JsValue::NULL, "", Some(clean_url))
        .map_err(|error| format!("remove launch fragment: {error:?}"))?;
    Ok(LaunchState { port, token })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Ready,
}

/// Single authenticated control connection shared by every hosted GPUI view.
///
/// All request futures are local futures: browser WebSocket callbacks resolve
/// their matching oneshot sender, while the bounded event queue preserves
/// runtime notifications without coupling view lifetimes to the socket.
struct ControlClient {
    launch: LaunchState,
    state: Cell<ConnectionState>,
    socket_generation: Cell<u64>,
    socket: RefCell<Option<web_sys::WebSocket>>,
    compatibility: RefCell<Option<UiCompatibility>>,
    pending: RefCell<HashMap<String, oneshot::Sender<Result<serde_json::Value, String>>>>,
    ready_waiters: RefCell<Vec<oneshot::Sender<Result<(), String>>>>,
    events: RefCell<VecDeque<(String, serde_json::Value)>>,
    next_id: Cell<u64>,
    reconnect_scheduled: Cell<bool>,
    permanent_error: RefCell<Option<String>>,
}

impl ControlClient {
    fn new(launch: LaunchState) -> Rc<Self> {
        Rc::new(Self {
            launch,
            state: Cell::new(ConnectionState::Disconnected),
            socket_generation: Cell::new(0),
            socket: RefCell::new(None),
            compatibility: RefCell::new(None),
            pending: RefCell::new(HashMap::new()),
            ready_waiters: RefCell::new(Vec::new()),
            events: RefCell::new(VecDeque::new()),
            next_id: Cell::new(1),
            reconnect_scheduled: Cell::new(false),
            permanent_error: RefCell::new(None),
        })
    }

    fn control_url(&self) -> String {
        format!("ws://127.0.0.1:{}/ws/control", self.launch.port)
    }

    fn preview_url(&self) -> String {
        format!("ws://127.0.0.1:{}/ws/preview", self.launch.port)
    }

    fn compatibility(&self) -> Option<UiCompatibility> {
        self.compatibility.borrow().clone()
    }

    async fn request(
        self: &Rc<Self>,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.ensure_ready().await?;
        let id = format!("rpc-{}", self.next_id.get());
        self.next_id.set(self.next_id.get().wrapping_add(1));
        let (sender, receiver) = oneshot::channel();
        self.pending.borrow_mut().insert(id.clone(), sender);
        let message = ClientMessage::Request(teshi_web_protocol::Request {
            id: id.clone(),
            method: method.to_string(),
            params,
        });
        let encoded = serde_json::to_string(&message).map_err(|error| error.to_string())?;
        let send_result = self
            .socket
            .borrow()
            .as_ref()
            .ok_or_else(|| "control WebSocket is not connected".to_string())?
            .send_with_str(&encoded)
            .map_err(|error| format!("send control request: {error:?}"));
        if let Err(error) = send_result {
            self.pending.borrow_mut().remove(&id);
            self.fail_connection(error.clone(), false);
            return Err(error);
        }
        receiver
            .await
            .map_err(|_| "control request was cancelled".to_string())?
    }

    async fn ensure_ready(self: &Rc<Self>) -> Result<(), String> {
        if let Some(error) = self.permanent_error.borrow().clone() {
            return Err(error);
        }
        if self.state.get() == ConnectionState::Ready {
            return Ok(());
        }
        let (sender, receiver) = oneshot::channel();
        let start = self.state.get() == ConnectionState::Disconnected;
        if start {
            self.state.set(ConnectionState::Connecting);
        }
        self.ready_waiters.borrow_mut().push(sender);
        if start {
            self.begin_connect();
        }
        receiver
            .await
            .map_err(|_| "control connection attempt was cancelled".to_string())?
    }

    fn begin_connect(self: &Rc<Self>) {
        let client = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let compatibility = match fetch_manifest().await {
                Ok(manifest) => UiCompatibility::from(manifest),
                Err(error) => {
                    client.fail_connection(format!("load hosted UI manifest: {error}"), false);
                    return;
                }
            };
            client.compatibility.replace(Some(compatibility.clone()));
            let socket = match web_sys::WebSocket::new(&client.control_url()) {
                Ok(socket) => socket,
                Err(error) => {
                    client.fail_connection(format!("open control WebSocket: {error:?}"), false);
                    return;
                }
            };
            client.install_socket(socket, compatibility);
        });
    }

    fn install_socket(self: &Rc<Self>, socket: web_sys::WebSocket, compatibility: UiCompatibility) {
        let generation = self.socket_generation.get().wrapping_add(1);
        self.socket_generation.set(generation);
        let open_client = self.clone();
        let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if open_client.socket_generation.get() != generation {
                return;
            }
            let hello = ClientMessage::ClientHello(ClientHello {
                token: open_client.launch.token.clone(),
                channel: Channel::Control,
                protocol_version: CONTROL_PROTOCOL_VERSION,
                ui: compatibility.clone(),
            });
            match serde_json::to_string(&hello) {
                Ok(encoded) => {
                    if let Some(socket) = open_client.socket.borrow().as_ref() {
                        if let Err(error) = socket.send_with_str(&encoded) {
                            open_client.fail_connection_if_current(
                                generation,
                                format!("send control handshake: {error:?}"),
                                false,
                            );
                        }
                    }
                }
                Err(error) => open_client.fail_connection_if_current(
                    generation,
                    format!("encode control handshake: {error}"),
                    true,
                ),
            }
        });
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        on_open.forget();

        let message_client = self.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                if message_client.socket_generation.get() != generation {
                    return;
                }
                let Some(text) = event.data().as_string() else {
                    message_client.fail_connection_if_current(
                        generation,
                        "control message was not text".into(),
                        true,
                    );
                    return;
                };
                let message = match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(message) => message,
                    Err(error) => {
                        message_client.fail_connection_if_current(
                            generation,
                            format!("decode control message: {error}"),
                            true,
                        );
                        return;
                    }
                };
                message_client.handle_server_message(message);
            },
        );
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();

        let error_client = self.clone();
        let on_error =
            Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
                if error_client.socket_generation.get() != generation {
                    return;
                }
                let detail = if event.message().is_empty() {
                    "control WebSocket failed".to_string()
                } else {
                    event.message()
                };
                error_client.fail_connection_if_current(generation, detail, false);
            });
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        let close_client = self.clone();
        let on_close =
            Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |event: web_sys::CloseEvent| {
                if close_client.socket_generation.get() != generation {
                    return;
                }
                let detail = if event.reason().is_empty() {
                    format!("control WebSocket closed ({})", event.code())
                } else {
                    format!("control WebSocket closed: {}", event.reason())
                };
                close_client.fail_connection_if_current(generation, detail, false);
            });
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();

        self.socket.replace(Some(socket));
    }

    fn handle_server_message(self: &Rc<Self>, message: ServerMessage) {
        match message {
            ServerMessage::ServerHello {
                daemon,
                channel,
                protocol_version,
                ..
            } => {
                let compatible = self
                    .compatibility()
                    .is_some_and(|compatibility| compatibility.supports(&daemon));
                if channel != Channel::Control || protocol_version != CONTROL_PROTOCOL_VERSION {
                    self.fail_connection("control protocol negotiation failed".into(), true);
                } else if !compatible {
                    let error = format!(
                        "this CLI build is incompatible with the hosted UI; upgrade nightly CLI from {}",
                        self.compatibility()
                            .map(|compatibility| compatibility.minimum_cli.semver)
                            .unwrap_or_else(|| "teshi.org".into())
                    );
                    self.permanent_error.replace(Some(error.clone()));
                    self.fail_connection(error, false);
                } else {
                    self.state.set(ConnectionState::Ready);
                    for waiter in self.ready_waiters.borrow_mut().drain(..) {
                        let _ = waiter.send(Ok(()));
                    }
                }
            }
            ServerMessage::Response {
                id,
                ok,
                result,
                error,
            } => {
                if let Some(sender) = self.pending.borrow_mut().remove(&id) {
                    let result = if ok {
                        result.ok_or_else(|| "control response omitted result".to_string())
                    } else {
                        Err(match error {
                            Some(error) => match error.details {
                                Some(details) => format!("{}: {details}", error.message),
                                None => error.message,
                            },
                            None => "control request failed".into(),
                        })
                    };
                    let _ = sender.send(result);
                }
            }
            ServerMessage::Event { event, payload } => {
                let mut events = self.events.borrow_mut();
                if events.len() >= 256 {
                    events.pop_front();
                }
                events.push_back((event, payload));
            }
            ServerMessage::Error(error) => {
                let permanent = matches!(
                    &error.code,
                    ErrorCode::IncompatibleCli
                        | ErrorCode::IncompatibleProtocol
                        | ErrorCode::InvalidToken
                );
                let message = if error.code == ErrorCode::IncompatibleCli {
                    "This hosted UI requires a newer compatible nightly CLI. Upgrade teshi and try again."
                        .to_string()
                } else {
                    error.message
                };
                self.fail_connection(message, permanent);
            }
        }
    }

    fn fail_connection(self: &Rc<Self>, error: String, permanent: bool) {
        if permanent {
            self.permanent_error.replace(Some(error.clone()));
        }
        self.state.set(ConnectionState::Disconnected);
        self.socket.borrow_mut().take();
        for (_, sender) in self.pending.borrow_mut().drain() {
            let _ = sender.send(Err(error.clone()));
        }
        for waiter in self.ready_waiters.borrow_mut().drain(..) {
            let _ = waiter.send(Err(error.clone()));
        }
        self.schedule_reconnect();
    }

    fn fail_connection_if_current(
        self: &Rc<Self>,
        generation: u64,
        error: String,
        permanent: bool,
    ) {
        if self.socket_generation.get() == generation {
            self.fail_connection(error, permanent);
        }
    }

    fn schedule_reconnect(self: &Rc<Self>) {
        if self.permanent_error.borrow().is_some() || self.reconnect_scheduled.replace(true) {
            return;
        }
        let client = self.clone();
        let Some(window) = web_sys::window() else {
            self.reconnect_scheduled.set(false);
            return;
        };
        let closure = Closure::wrap(Box::new(move || {
            client.reconnect_scheduled.set(false);
            if client.state.get() == ConnectionState::Disconnected {
                client.state.set(ConnectionState::Connecting);
                client.begin_connect();
            }
        }) as Box<dyn FnMut()>);
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            1_000,
        );
        closure.forget();
    }
}

async fn fetch_manifest() -> Result<UiManifest, String> {
    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_string())?;
    // The manifest is the compatibility gate; bypass intermediary caches so a
    // newly deployed UI cannot negotiate against stale minimum-build metadata.
    let manifest_url = format!("/app/ui-manifest.json?ts={}", js_sys::Date::now());
    let response = JsFuture::from(window.fetch_with_str(&manifest_url))
        .await
        .map_err(|error| format!("manifest request: {error:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|error| format!("manifest response: {error:?}"))?;
    if !response.ok() {
        return Err(format!("manifest HTTP {}", response.status()));
    }
    let json = JsFuture::from(
        response
            .json()
            .map_err(|error| format!("manifest JSON: {error:?}"))?,
    )
    .await
    .map_err(|error| format!("manifest JSON promise: {error:?}"))?;
    let text = js_sys::JSON::stringify(&json)
        .map_err(|error| format!("manifest stringify: {error:?}"))?
        .as_string()
        .ok_or_else(|| "manifest JSON is not a string".to_string())?;
    let manifest =
        serde_json::from_str(&text).map_err(|error| format!("decode manifest: {error}"))?;
    validate_manifest_against_bundle(manifest)
}

fn validate_manifest_against_bundle(manifest: UiManifest) -> Result<UiManifest, String> {
    let compiled_source = option_env!("TESHI_UI_SOURCE_SHA");
    let compiled_minimum = option_env!("TESHI_UI_MINIMUM_CLI_JSON");
    match (compiled_source, compiled_minimum) {
        (None, None) => {
            let production_host = web_sys::window()
                .and_then(|window| window.location().hostname().ok())
                .is_some_and(|hostname| hostname.eq_ignore_ascii_case("teshi.org"));
            if production_host {
                Err("hosted UI bundle is missing its compiled compatibility identity".into())
            } else {
                Ok(manifest)
            }
        }
        (Some(source), Some(minimum)) => {
            let minimum: CliBuildIdentity = serde_json::from_str(minimum)
                .map_err(|error| format!("decode compiled minimum CLI identity: {error}"))?;
            if manifest.ui_source_sha != source
                || manifest.minimum_cli != minimum
                || manifest.minimum_build_sequence != minimum.build_sequence
            {
                return Err(
                    "hosted UI manifest does not match the compatibility identity compiled into this bundle"
                        .into(),
                );
            }
            Ok(manifest)
        }
        _ => Err("hosted UI bundle has an incomplete compiled compatibility identity".into()),
    }
}

fn update_preview(
    app: &Rc<AppCell>,
    preview: &Entity<WinAppPreview>,
    update: impl FnOnce(&mut WinAppPreview, &mut gpui::Context<WinAppPreview>),
) {
    // Browser callbacks run on the same thread as GPUI. If GPUI is already in
    // an update, dropping this superseded event is preferable to re-entrant borrowing.
    if let Ok(mut cx) = app.try_borrow_mut() {
        let app: &mut gpui::App = std::ops::DerefMut::deref_mut(&mut cx);
        preview.update(app, update);
    }
}

fn schedule_preview_reconnect(
    preview: Entity<WinAppPreview>,
    app: Rc<AppCell>,
    client: Rc<ControlClient>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::wrap(Box::new(move || {
        start_wasm_preview(preview.clone(), app.clone(), client.clone(), false);
    }) as Box<dyn FnMut()>);
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        closure.as_ref().unchecked_ref(),
        2_000,
    );
    closure.forget();
}

fn start_wasm_preview(
    preview: Entity<WinAppPreview>,
    app: Rc<AppCell>,
    client: Rc<ControlClient>,
    start_winapp: bool,
) {
    let task_preview = preview.clone();
    let task_app = app.clone();
    let task_client = client.clone();
    wasm_bindgen_futures::spawn_local(async move {
        if start_winapp {
            if let Err(error) = task_client
                .request("browser.start", serde_json::json!({"mode": "winapp"}))
                .await
            {
                update_preview(&task_app, &task_preview, |preview, cx| {
                    preview.set_error(format!("start WinApp sidecar: {error}"), cx);
                });
                schedule_preview_reconnect(task_preview, task_app, task_client);
                return;
            }
        }
        if let Err(error) = task_client.ensure_ready().await {
            update_preview(&task_app, &task_preview, |preview, cx| {
                preview.set_error(format!("control connection unavailable: {error}"), cx);
            });
            schedule_preview_reconnect(task_preview, task_app, task_client);
            return;
        }
        let Some(compatibility) = task_client.compatibility() else {
            update_preview(&task_app, &task_preview, |preview, cx| {
                preview.set_error("hosted UI manifest is unavailable", cx);
            });
            schedule_preview_reconnect(task_preview, task_app, task_client);
            return;
        };
        open_wasm_preview_socket(task_preview, task_app, task_client, compatibility);
    });
}

fn open_wasm_preview_socket(
    preview: Entity<WinAppPreview>,
    app: Rc<AppCell>,
    client: Rc<ControlClient>,
    compatibility: UiCompatibility,
) {
    let ws_url = client.preview_url();

    let socket = match web_sys::WebSocket::new(&ws_url) {
        Ok(socket) => socket,
        Err(error) => {
            update_preview(&app, &preview, |preview, cx| {
                preview.set_error(format!("open {ws_url}: {error:?}"), cx);
            });
            schedule_preview_reconnect(preview, app, client);
            return;
        }
    };

    let open_app = Rc::clone(&app);
    let open_preview = preview.clone();
    let open_socket = socket.clone();
    let open_token = client.launch.token.clone();
    let open_compatibility = compatibility.clone();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        update_preview(&open_app, &open_preview, |preview, cx| {
            preview.set_waiting("Authenticating preview stream…", cx);
        });
        let hello = ClientMessage::ClientHello(ClientHello {
            token: open_token.clone(),
            channel: Channel::Preview,
            protocol_version: PREVIEW_PROTOCOL_VERSION,
            ui: open_compatibility.clone(),
        });
        if let Ok(encoded) = serde_json::to_string(&hello) {
            let _ = open_socket.send_with_str(&encoded);
        }
    });
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let message_app = Rc::clone(&app);
    let message_preview = preview.clone();
    let on_message =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&text) else {
                return;
            };
            if payload.get("type").and_then(|value| value.as_str()) == Some("server_hello") {
                update_preview(&message_app, &message_preview, |preview, cx| {
                    preview.set_waiting("Attached; waiting for first frame…", cx);
                });
                return;
            }
            match payload.get("type").and_then(|value| value.as_str()) {
                Some("frame") => {
                    let Some(data) = payload.get("data").and_then(|value| value.as_str()) else {
                        return;
                    };
                    match base64::engine::general_purpose::STANDARD.decode(data) {
                        Ok(jpeg) => {
                            let capture_backend = payload
                                .get("capture_backend")
                                .and_then(|value| value.as_str())
                                .map(str::to_owned);
                            let fallback_reason = payload
                                .get("capture_fallback_reason")
                                .and_then(|value| value.as_str())
                                .map(str::to_owned);
                            update_preview(&message_app, &message_preview, |preview, cx| {
                                preview.set_jpeg(
                                    jpeg,
                                    capture_backend.as_deref(),
                                    fallback_reason.as_deref(),
                                    cx,
                                );
                            })
                        }
                        Err(error) => {
                            update_preview(&message_app, &message_preview, |preview, cx| {
                                preview.set_error(format!("invalid JPEG frame: {error}"), cx);
                            });
                        }
                    }
                }
                Some("frame_error") => {
                    let error = payload
                        .get("error")
                        .and_then(|value| value.as_str())
                        .unwrap_or("screenshot stream failed")
                        .to_string();
                    update_preview(&message_app, &message_preview, |preview, cx| {
                        preview.set_error(error, cx);
                    });
                }
                Some("response")
                    if payload.get("request_id").and_then(|value| value.as_str())
                        == Some("gpui-preview-attach") =>
                {
                    if payload.get("ok").and_then(|value| value.as_bool()) == Some(true) {
                        update_preview(&message_app, &message_preview, |preview, cx| {
                            preview.set_waiting("Attached; waiting for first frame…", cx);
                        });
                    } else {
                        let error = payload
                            .get("error")
                            .and_then(|value| value.as_str())
                            .unwrap_or("could not attach to target application")
                            .to_string();
                        update_preview(&message_app, &message_preview, |preview, cx| {
                            preview.set_error(error, cx);
                        });
                    }
                }
                _ => {}
            }
        });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let error_app = Rc::clone(&app);
    let error_preview = preview.clone();
    let on_error =
        Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
            let detail = if event.message().is_empty() {
                "browser rejected the screenshot-stream WebSocket".to_string()
            } else {
                event.message()
            };
            update_preview(&error_app, &error_preview, |preview, cx| {
                preview.set_error(detail, cx);
            });
        });
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    let close_app = Rc::clone(&app);
    let close_preview = preview.clone();
    let reconnect_app = app;
    let reconnect_preview = preview;
    let reconnect_client = client;
    let on_close =
        Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |event: web_sys::CloseEvent| {
            let detail = if event.reason().is_empty() {
                format!("preview WebSocket closed ({})", event.code())
            } else {
                format!("preview WebSocket closed: {}", event.reason())
            };
            update_preview(&close_app, &close_preview, |preview, cx| {
                preview.set_error(detail, cx);
            });
            schedule_preview_reconnect(
                reconnect_preview.clone(),
                Rc::clone(&reconnect_app),
                Rc::clone(&reconnect_client),
            );
        });
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();
}

/// Async adapter for the shared GPUI backend traits.
#[derive(Clone)]
struct WasmBackend {
    control: Rc<ControlClient>,
}

impl LlmConfigBackend for WasmBackend {
    fn get_llm_config(&self) -> BackendFuture<LlmConfigSnapshot> {
        let control = self.control.clone();
        Box::pin(async move {
            serde_json::from_value(
                control
                    .request("llm.get_config", serde_json::json!({}))
                    .await?,
            )
            .map_err(|error| error.to_string())
        })
    }

    fn set_llm_config(&self, update: LlmConfigUpdate) -> BackendFuture<()> {
        let control = self.control.clone();
        Box::pin(async move {
            control
                .request(
                    "llm.set_config",
                    serde_json::to_value(update).map_err(|e| e.to_string())?,
                )
                .await?;
            Ok(())
        })
    }

    fn list_profiles(&self) -> BackendFuture<ModelProfileListSnapshot> {
        let control = self.control.clone();
        Box::pin(async move {
            serde_json::from_value(
                control
                    .request("llm.list_profiles", serde_json::json!({}))
                    .await?,
            )
            .map_err(|error| error.to_string())
        })
    }

    fn get_profile(&self, id: &str) -> BackendFuture<ModelProfileSnapshot> {
        let control = self.control.clone();
        let id = id.to_string();
        Box::pin(async move {
            serde_json::from_value(
                control
                    .request("llm.get_profile", serde_json::json!({"id": id}))
                    .await?,
            )
            .map_err(|error| error.to_string())
        })
    }

    fn save_profile(&self, update: ModelProfileUpdate) -> BackendFuture<ModelProfileSnapshot> {
        let control = self.control.clone();
        Box::pin(async move {
            serde_json::from_value(
                control
                    .request(
                        "llm.save_profile",
                        serde_json::to_value(update).map_err(|e| e.to_string())?,
                    )
                    .await?,
            )
            .map_err(|error| error.to_string())
        })
    }

    fn delete_profile(&self, id: &str) -> BackendFuture<()> {
        let control = self.control.clone();
        let id = id.to_string();
        Box::pin(async move {
            control
                .request("llm.delete_profile", serde_json::json!({"id": id}))
                .await?;
            Ok(())
        })
    }

    fn activate_profile(&self, id: &str) -> BackendFuture<()> {
        let control = self.control.clone();
        let id = id.to_string();
        Box::pin(async move {
            control
                .request("llm.activate_profile", serde_json::json!({"id": id}))
                .await?;
            Ok(())
        })
    }
}

impl BrowserSessionsBackend for WasmBackend {
    fn start_browser_bridge(&self) -> BackendFuture<()> {
        let control = self.control.clone();
        Box::pin(async move {
            control
                .request("browser.start", serde_json::json!({"mode": "chrome"}))
                .await?;
            Ok(())
        })
    }

    fn list_browser_sessions(&self) -> BackendFuture<BrowserSessionListSnapshot> {
        let control = self.control.clone();
        Box::pin(async move {
            serde_json::from_value(
                control
                    .request("browser.list_sessions", serde_json::json!({}))
                    .await?,
            )
            .map_err(|error| format!("decode browser sessions: {error}"))
        })
    }

    fn activate_browser_tab(&self, target: &BrowserTabTarget) -> BackendFuture<()> {
        let control = self.control.clone();
        let target = target.clone();
        Box::pin(async move {
            control
                .request(
                    "browser.activate_tab",
                    serde_json::to_value(target).map_err(|e| e.to_string())?,
                )
                .await?;
            Ok(())
        })
    }
}

impl ApiRunBackend for WasmBackend {
    fn list_scenarios(&self) -> BackendFuture<Vec<ApiScenarioSnapshot>> {
        let control = self.control.clone();
        Box::pin(async move {
            serde_json::from_value(
                control
                    .request("bdd.list_scenarios", serde_json::json!({}))
                    .await?,
            )
            .map_err(|error| error.to_string())
        })
    }

    fn start_run(&self, scenario_ids: &[String]) -> BackendFuture<Vec<ApiRunEventDto>> {
        let control = self.control.clone();
        let scenario_ids = scenario_ids.to_vec();
        Box::pin(async move {
            serde_json::from_value(
                control
                    .request(
                        "bdd.run",
                        serde_json::json!({ "scenario_ids": scenario_ids }),
                    )
                    .await?,
            )
            .map_err(|error| error.to_string())
        })
    }

    fn get_exchange(&self, exchange_id: &str, _redact: bool) -> BackendFuture<serde_json::Value> {
        let control = self.control.clone();
        let exchange_id = exchange_id.to_string();
        Box::pin(async move {
            control
                .request(
                    "api.get_exchange",
                    serde_json::json!({
                        "exchange_id": exchange_id,
                        // Hosted pages never receive plaintext HTTP
                        // credentials or bodies, even when an older UI asks
                        // to expand an exchange.
                        "redact": true,
                    }),
                )
                .await
        })
    }
}

/// Start the GPUI web shell and report async startup outcome to JavaScript.
///
/// GPU initialization is asynchronous. Call `on_ready` after the window opens
/// successfully, or `on_error` with a short English message if it fails.
/// Callback invocation failures are logged to the browser console.
#[wasm_bindgen]
pub fn run(on_ready: js_sys::Function, on_error: js_sys::Function) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let launch = parse_launch_state().map_err(|error| JsValue::from_str(&error))?;
    let control = ControlClient::new(launch);
    gpui_platform::web_init();

    let app = gpui_platform::single_threaded_web();

    // Keep the web Application's Rc alive for the page lifetime (upstream pattern).
    struct WasmApplication(Rc<gpui::AppCell>);
    let wasm_app = unsafe { std::mem::transmute::<gpui::Application, WasmApplication>(app) };
    let app_cell = wasm_app.0.clone();
    std::mem::forget(app_cell.clone());
    let app = unsafe { std::mem::transmute::<WasmApplication, gpui::Application>(wasm_app) };

    app.run(move |cx: &mut gpui::App| {
        bind_llm_config_keys(cx);
        let platform = Rc::new(WasmBackend {
            control: control.clone(),
        });
        let llm_backend: Rc<dyn LlmConfigBackend> = platform.clone();
        let browser_backend: Rc<dyn BrowserSessionsBackend> = platform.clone();
        let api_backend: Rc<dyn ApiRunBackend> = platform;
        let preview = cx.new(|_| WinAppPreview::new("browser tab"));
        let shell_slot: Rc<std::cell::RefCell<Option<gpui::Entity<AppShell>>>> =
            Rc::new(std::cell::RefCell::new(None));
        let shell_for_window = shell_slot.clone();
        let preview_for_window = preview.clone();
        match cx.open_window(gpui::WindowOptions::default(), move |window, cx| {
            let shell = cx.new(|cx| {
                AppShell::new(
                    llm_backend.clone(),
                    browser_backend.clone(),
                    api_backend.clone(),
                    preview_for_window.clone(),
                    window,
                    cx,
                )
            });
            *shell_for_window.borrow_mut() = Some(shell.clone());
            shell
        }) {
            Ok(_) => {
                if e2e::e2e_enabled() {
                    if let Some(shell) = shell_slot.borrow().clone() {
                        e2e::install(app_cell.clone(), shell);
                    }
                }
                cx.activate(true);
                // Connect to the daemon screenshot stream without starting WinApp.
                // Starting WinApp here would replace an active Chrome bridge.
                // `?winapp_preview=1` still starts the WinApp sidecar first.
                let start_winapp = query_parameter("winapp_preview").is_some();
                start_wasm_preview(preview, app_cell.clone(), control.clone(), start_winapp);
                // Do not report the page as ready until the hosted manifest
                // and control handshake have passed. In particular, an old
                // nightly must show the actionable upgrade state before any
                // view can issue business RPCs.
                let startup_control = control.clone();
                let ready_callback = on_ready.clone();
                let error_callback = on_error.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match startup_control.ensure_ready().await {
                        Ok(()) => {
                            if let Err(error) = ready_callback.call0(&JsValue::NULL) {
                                web_sys::console::error_1(&error);
                            }
                        }
                        Err(message) => {
                            if let Err(error) =
                                error_callback.call1(&JsValue::NULL, &JsValue::from_str(&message))
                            {
                                web_sys::console::error_1(&error);
                            }
                        }
                    }
                });
            }
            Err(err) => {
                let message = format!("Failed to open window: {err:#}");
                if let Err(cb_err) = on_error.call1(&JsValue::NULL, &JsValue::from_str(&message)) {
                    web_sys::console::error_1(&cb_err);
                }
            }
        }
    });

    Ok(())
}
