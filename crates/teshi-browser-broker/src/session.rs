//! Typed browser Profile/session registry used by the broker state owner.
//!
//! The transport only authenticates and delivers messages.  This module owns
//! the profile-scoped identity and liveness rules that must remain independent
//! of whichever Teshi surface started the user broker.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::protocol::{
    BROWSER_BROKER_PROTOCOL_VERSION, BROWSER_BROKER_SCHEMA_VERSION, BrokerError, BrokerErrorCode,
    BrowserTarget, ExtensionHeartbeat, ExtensionTab, ExtensionWindow,
};

/// Compatibility identity used by pre-v1 extensions that do not advertise a
/// persistent Profile instance ID.
pub const LEGACY_INSTANCE_ID: &str = "legacy-single-session";
/// Heartbeats older than this are no longer considered live.
pub const DEFAULT_HEARTBEAT_TTL: Duration = Duration::from_secs(8);
/// Disconnected records remain visible long enough for diagnostics and recovery.
pub const DISCONNECTED_RETENTION: Duration = Duration::from_secs(60);
/// Bound one extension session's queued heartbeat commands.
pub const MAX_SESSION_COMMAND_QUEUE: usize = 256;
/// Bound retained latest frames/subscriptions for one Profile.
pub const MAX_TARGET_FRAME_RECORDS: usize = 64;
/// Keep snapshot-local element aliases only for a bounded amount of time.
pub const ELEMENT_REFERENCE_TTL: Duration = Duration::from_secs(120);
/// Bound snapshot-local element aliases retained by one Profile.
pub const MAX_ELEMENT_REFERENCES: usize = 512;

/// Public liveness state for one extension Profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionHealth {
    Ready,
    Stale,
    Disconnected,
    Incompatible,
    DebuggerConflict,
}

impl SessionHealth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Stale => "stale",
            Self::Disconnected => "disconnected",
            Self::Incompatible => "incompatible",
            Self::DebuggerConflict => "debugger_conflict",
        }
    }
}

/// One target-scoped TSH1 frame after the transport has validated its envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrameRecord {
    pub target: BrowserTarget,
    pub seq: u64,
    pub url: String,
    pub jpeg: Vec<u8>,
    pub captured_at: Instant,
}

/// Broker-owned element alias bound to one target, page revision and request scope.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementReferenceRecord {
    pub alias: String,
    pub target: BrowserTarget,
    pub snapshot_id: String,
    pub page_context_revision: String,
    pub project_root: String,
    pub caller_label: String,
    pub context: Value,
    pub element: Value,
    pub created_at: Instant,
}

/// Mutable state for one extension installation/Profile identity.
#[derive(Debug, Clone)]
pub struct BrowserSessionRecord {
    extension_instance_id: String,
    profile_label: String,
    profile_label_managed: bool,
    extension_version: String,
    protocol_version: u16,
    browser: BTreeMap<String, Value>,
    features: Vec<crate::protocol::FeatureAvailability>,
    supported_actions: Vec<String>,
    supported_operations: Vec<String>,
    optional_permissions: BTreeMap<String, bool>,
    windows: Vec<ExtensionWindow>,
    active_window_id: Option<i64>,
    active_tab_id: Option<i64>,
    page_url: String,
    page_title: String,
    last_heartbeat: Instant,
    disconnected_since: Option<Instant>,
    last_frame_at: Option<Instant>,
    last_frame_error: String,
    stream_generation: Option<u64>,
    command_queue: VecDeque<Value>,
    frames: HashMap<BrowserTarget, PreviewFrameRecord>,
    subscriptions: BTreeSet<BrowserTarget>,
    element_references: HashMap<String, ElementReferenceRecord>,
}

impl BrowserSessionRecord {
    fn new(extension_instance_id: String, now: Instant) -> Self {
        Self {
            extension_instance_id,
            profile_label: String::new(),
            profile_label_managed: false,
            extension_version: "legacy".into(),
            protocol_version: 0,
            browser: BTreeMap::new(),
            features: Vec::new(),
            supported_actions: Vec::new(),
            supported_operations: Vec::new(),
            optional_permissions: BTreeMap::new(),
            windows: Vec::new(),
            active_window_id: None,
            active_tab_id: None,
            page_url: String::new(),
            page_title: String::new(),
            last_heartbeat: now,
            disconnected_since: None,
            last_frame_at: None,
            last_frame_error: String::new(),
            stream_generation: None,
            command_queue: VecDeque::new(),
            frames: HashMap::new(),
            subscriptions: BTreeSet::new(),
            element_references: HashMap::new(),
        }
    }

    pub fn extension_instance_id(&self) -> &str {
        &self.extension_instance_id
    }

    pub fn profile_label(&self) -> &str {
        &self.profile_label
    }

    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn features(&self) -> &[crate::protocol::FeatureAvailability] {
        &self.features
    }

    pub fn supports_feature(&self, required: &str) -> bool {
        self.features
            .iter()
            .any(|feature| feature.feature == required && feature.available)
    }

    pub fn supported_operations(&self) -> &[String] {
        &self.supported_operations
    }

    pub fn windows(&self) -> &[ExtensionWindow] {
        &self.windows
    }

    pub fn supported_actions(&self) -> &[String] {
        &self.supported_actions
    }

    pub fn optional_permissions(&self) -> &BTreeMap<String, bool> {
        &self.optional_permissions
    }

    pub fn is_legacy(&self) -> bool {
        self.protocol_version == 0
    }

    pub fn compatible(&self) -> bool {
        self.is_legacy() || self.protocol_version == BROWSER_BROKER_PROTOCOL_VERSION
    }

    pub fn alive_at(&self, now: Instant, heartbeat_ttl: Duration) -> bool {
        now.saturating_duration_since(self.last_heartbeat) < heartbeat_ttl
    }

    pub fn health_at(&self, now: Instant, heartbeat_ttl: Duration) -> SessionHealth {
        if !self.compatible() {
            return SessionHealth::Incompatible;
        }
        if !self.alive_at(now, heartbeat_ttl) {
            return SessionHealth::Disconnected;
        }
        if self
            .last_frame_error
            .to_ascii_lowercase()
            .contains("debugger")
        {
            return SessionHealth::DebuggerConflict;
        }
        if now.saturating_duration_since(self.last_heartbeat) >= heartbeat_ttl.mul_f32(0.75) {
            return SessionHealth::Stale;
        }
        SessionHealth::Ready
    }

    pub fn current_stream_generation(&self) -> Option<u64> {
        self.stream_generation
    }

    pub fn current_active_target(&self) -> Option<BrowserTarget> {
        let active = self
            .iter_tabs()
            .into_iter()
            .find(|tab| tab.debuggable && (Some(tab.id) == self.active_tab_id || tab.active))?;
        Some(BrowserTarget {
            extension_instance_id: self.extension_instance_id.clone(),
            window_id: if active.window_id > 0 {
                active.window_id
            } else {
                self.active_window_id.unwrap_or_default()
            },
            tab_id: active.id,
        })
    }

    pub fn iter_tabs(&self) -> Vec<ExtensionTab> {
        self.windows
            .iter()
            .flat_map(|window| {
                window.tabs.iter().map(|tab| {
                    let mut tab = tab.clone();
                    if tab.window_id <= 0 {
                        tab.window_id = window.id;
                    }
                    tab
                })
            })
            .collect()
    }

    pub fn pop_command(&mut self) -> Option<Value> {
        self.command_queue.pop_front()
    }

    pub fn queued_command_count(&self) -> usize {
        self.command_queue.len()
    }

    pub fn remove_queued_command(&mut self, request_id: &str) -> bool {
        let before = self.command_queue.len();
        self.command_queue.retain(|command| {
            command.get("request_id").and_then(Value::as_str) != Some(request_id)
        });
        before != self.command_queue.len()
    }

    pub fn queue_command(&mut self, command: Value) -> Result<(), BrokerError> {
        if self.command_queue.len() >= MAX_SESSION_COMMAND_QUEUE {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserSessionBusy,
                "selected browser session command queue is full; retry later",
            ));
        }
        self.command_queue.push_back(command);
        Ok(())
    }

    pub fn restore_command_front(&mut self, command: Value) -> Result<(), BrokerError> {
        if self.command_queue.len() >= MAX_SESSION_COMMAND_QUEUE {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserSessionBusy,
                "selected browser session command queue is full; retry later",
            ));
        }
        self.command_queue.push_front(command);
        Ok(())
    }

    pub fn subscribe_target(&mut self, target: BrowserTarget) -> Result<(), BrokerError> {
        self.require_target(&target)?;
        self.subscriptions.insert(target);
        Ok(())
    }

    pub fn unsubscribe_target(&mut self, target: &BrowserTarget) {
        self.subscriptions.remove(target);
    }

    pub fn is_subscribed(&self, target: &BrowserTarget) -> bool {
        self.subscriptions.contains(target)
    }

    pub fn latest_frame(&self, target: &BrowserTarget) -> Option<&PreviewFrameRecord> {
        self.frames.get(target)
    }

    pub fn element_reference_count(&self) -> usize {
        self.element_references.len()
    }

    pub fn cache_snapshot_references(
        &mut self,
        target: BrowserTarget,
        response: &mut Value,
        fallback_snapshot_id: &str,
        project_root: &str,
        caller_label: &str,
        now: Instant,
    ) -> Result<(), BrokerError> {
        self.require_target(&target)?;
        let Some(elements) = response
            .get("interactive_elements")
            .and_then(Value::as_array)
            .cloned()
        else {
            return Ok(());
        };
        let snapshot_id = response
            .get("snapshot_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(fallback_snapshot_id)
            .chars()
            .take(256)
            .collect::<String>();
        let page_context_revision = response
            .get("page_context_revision")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(256)
            .collect::<String>();
        self.clear_element_references(Some(&target));
        let mut published = Vec::new();
        for element in elements.into_iter().take(MAX_ELEMENT_REFERENCES) {
            let Value::Object(mut object) = element else {
                continue;
            };
            let alias = format!("@e{}", published.len() + 1);
            let original = Value::Object(object.clone());
            let context = object.get("context").cloned().unwrap_or(Value::Null);
            object.insert("ref".into(), Value::String(alias.clone()));
            object.insert("snapshot_id".into(), Value::String(snapshot_id.clone()));
            object.insert(
                "page_context_revision".into(),
                Value::String(page_context_revision.clone()),
            );
            let published_element = Value::Object(object);
            self.element_references.insert(
                element_reference_key(&target, &alias),
                ElementReferenceRecord {
                    alias,
                    target: target.clone(),
                    snapshot_id: snapshot_id.clone(),
                    page_context_revision: page_context_revision.clone(),
                    project_root: project_root.to_owned(),
                    caller_label: caller_label.to_owned(),
                    context,
                    element: original,
                    created_at: now,
                },
            );
            published.push(published_element);
        }
        response["snapshot_id"] = Value::String(snapshot_id);
        response["interactive_elements"] = Value::Array(published);
        self.evict_element_references(now);
        Ok(())
    }

    pub fn resolve_element_reference(
        &mut self,
        target: &BrowserTarget,
        alias: &str,
        page_context_revision: Option<&str>,
        snapshot_id: Option<&str>,
        project_root: &str,
        caller_label: &str,
        now: Instant,
    ) -> Result<ElementReferenceRecord, BrokerError> {
        self.evict_element_references(now);
        let key = element_reference_key(target, alias);
        let Some(reference) = self.element_references.get(&key) else {
            return Err(stale_element_reference(target, alias));
        };
        let revision_matches = page_context_revision
            .filter(|value| !value.is_empty())
            .is_none_or(|value| value == reference.page_context_revision);
        let snapshot_matches = snapshot_id
            .filter(|value| !value.is_empty())
            .is_none_or(|value| value == reference.snapshot_id);
        if reference.target != *target
            || reference.project_root != project_root
            || reference.caller_label != caller_label
            || !revision_matches
            || !snapshot_matches
        {
            return Err(stale_element_reference(target, alias));
        }
        Ok(reference.clone())
    }

    pub fn clear_element_references(&mut self, target: Option<&BrowserTarget>) {
        let Some(target) = target else {
            self.element_references.clear();
            return;
        };
        let prefix = element_reference_target_prefix(target);
        self.element_references
            .retain(|key, _| !key.starts_with(&prefix));
    }

    fn evict_element_references(&mut self, now: Instant) {
        self.element_references.retain(|_, reference| {
            now.saturating_duration_since(reference.created_at) <= ELEMENT_REFERENCE_TTL
        });
        if self.element_references.len() > MAX_ELEMENT_REFERENCES {
            let mut retained = self
                .element_references
                .drain()
                .collect::<Vec<(String, ElementReferenceRecord)>>();
            retained.sort_by_key(|(_, reference)| reference.created_at);
            let drop_count = retained.len() - MAX_ELEMENT_REFERENCES;
            self.element_references
                .extend(retained.into_iter().skip(drop_count));
        }
    }

    pub(crate) fn require_target(&self, target: &BrowserTarget) -> Result<(), BrokerError> {
        if target.extension_instance_id != self.extension_instance_id {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "browser target does not match its extension session",
            ));
        }
        let tab = self
            .iter_tabs()
            .into_iter()
            .find(|tab| tab.id == target.tab_id && tab.window_id == target.window_id)
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserTargetNotFound,
                    "selected browser window/tab is no longer available",
                )
            })?;
        if !tab.debuggable {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserTargetNotFound,
                "selected tab cannot be debugged; choose an http(s) or file page",
            ));
        }
        Ok(())
    }

    fn update_from_heartbeat(&mut self, heartbeat: ExtensionHeartbeat, now: Instant) {
        let previous_tabs = self.iter_tabs();
        let windows = normalize_windows(&heartbeat);
        if !self.profile_label_managed {
            self.profile_label = heartbeat.profile_label.trim().chars().take(120).collect();
        }
        self.extension_version = heartbeat
            .extension_version
            .trim()
            .chars()
            .take(64)
            .collect();
        if self.extension_version.is_empty() {
            self.extension_version = "legacy".into();
        }
        self.protocol_version = heartbeat.protocol_version.unwrap_or_default();
        self.browser = heartbeat.browser;
        self.features = heartbeat.features;
        self.supported_actions = heartbeat.supported_actions;
        self.supported_operations = heartbeat.supported_operations;
        self.optional_permissions = heartbeat.optional_permissions;
        self.windows = windows;
        self.active_window_id = heartbeat.active_window_id;
        self.active_tab_id = heartbeat.active_tab_id;
        self.page_url = heartbeat.url.chars().take(4096).collect();
        self.page_title = heartbeat.title.chars().take(500).collect();
        if !heartbeat.frame_error.trim().is_empty() {
            self.last_frame_error = heartbeat.frame_error.chars().take(1000).collect();
        }
        self.clear_changed_target_state(&previous_tabs);
        self.last_heartbeat = now;
        self.disconnected_since = None;
    }

    fn clear_changed_target_state(&mut self, previous_tabs: &[ExtensionTab]) {
        for previous in previous_tabs {
            let target = BrowserTarget {
                extension_instance_id: self.extension_instance_id.clone(),
                window_id: previous.window_id,
                tab_id: previous.id,
            };
            let current = self
                .iter_tabs()
                .into_iter()
                .find(|tab| tab.window_id == previous.window_id && tab.id == previous.id);
            if current.is_none_or(|tab| {
                !previous.url.is_empty() && !tab.url.is_empty() && previous.url != tab.url
            }) {
                self.frames.remove(&target);
                self.subscriptions.remove(&target);
                self.clear_element_references(Some(&target));
            }
        }
    }

    fn mark_disconnected(&mut self, now: Instant) {
        if self.disconnected_since.is_none() {
            self.disconnected_since = Some(now);
        }
        self.stream_generation = None;
        self.command_queue.clear();
        self.frames.clear();
        self.subscriptions.clear();
        self.element_references.clear();
    }

    fn attach_stream(&mut self, generation: u64) -> Result<Option<u64>, BrokerError> {
        if self
            .stream_generation
            .is_some_and(|current| generation <= current)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::IncompatibleBrowserSession,
                "extension stream generation is stale",
            ));
        }
        let previous = self.stream_generation.replace(generation);
        Ok(previous)
    }

    fn detach_stream(&mut self, generation: u64) -> bool {
        if self.stream_generation == Some(generation) {
            self.stream_generation = None;
            true
        } else {
            false
        }
    }

    fn update_frame(
        &mut self,
        target: BrowserTarget,
        seq: u64,
        url: String,
        jpeg: Vec<u8>,
        now: Instant,
    ) -> Result<(), BrokerError> {
        self.require_target(&target)?;
        if self
            .frames
            .get(&target)
            .is_some_and(|previous| seq <= previous.seq)
        {
            return Ok(());
        }
        if self.frames.len() >= MAX_TARGET_FRAME_RECORDS && !self.frames.contains_key(&target) {
            if let Some(oldest) = self
                .frames
                .iter()
                .min_by_key(|(_, frame)| frame.captured_at)
                .map(|(target, _)| target.clone())
            {
                self.frames.remove(&oldest);
            }
        }
        self.frames.insert(
            target.clone(),
            PreviewFrameRecord {
                target,
                seq,
                url: url.chars().take(16 * 1024).collect(),
                jpeg,
                captured_at: now,
            },
        );
        self.last_frame_at = Some(now);
        self.last_frame_error.clear();
        Ok(())
    }

    pub fn public_contract(&self, now: Instant, heartbeat_ttl: Duration) -> Value {
        let browser_name = self
            .browser
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Chromium");
        let browser_version = self
            .browser
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let platform = self.browser.get("platform").cloned().unwrap_or(Value::Null);
        let age_ms = now
            .saturating_duration_since(self.last_heartbeat)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        json!({
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "identity": {
                "extension_instance_id": self.extension_instance_id,
                "profile_label": (!self.profile_label.is_empty()).then(|| self.profile_label.clone()),
                "extension_version": self.extension_version,
                "protocol_version": self.protocol_version,
            },
            "browser": {
                "name": browser_name,
                "version": browser_version,
                "platform": platform,
            },
            "health": self.health_at(now, heartbeat_ttl).as_str(),
            "last_heartbeat_age_ms": age_ms,
            "windows": self.windows,
            "capabilities": {
                "features": self.features,
                "supported_actions": self.supported_actions,
                "supported_operations": self.supported_operations,
                "optional_permissions": self.optional_permissions,
            },
        })
    }
}

fn normalize_windows(heartbeat: &ExtensionHeartbeat) -> Vec<ExtensionWindow> {
    if !heartbeat.windows.is_empty() {
        return heartbeat
            .windows
            .iter()
            .map(|window| ExtensionWindow {
                id: window.id,
                focused: window.focused,
                tabs: window
                    .tabs
                    .iter()
                    .map(|tab| {
                        let mut tab = tab.clone();
                        if tab.window_id <= 0 {
                            tab.window_id = window.id;
                        }
                        tab
                    })
                    .collect(),
            })
            .collect();
    }

    let fallback_window_id = heartbeat.active_window_id.unwrap_or_default();
    let mut grouped = BTreeMap::<i64, Vec<ExtensionTab>>::new();
    for raw_tab in &heartbeat.tabs {
        let mut tab = raw_tab.clone();
        if tab.window_id <= 0 {
            tab.window_id = fallback_window_id;
        }
        grouped.entry(tab.window_id).or_default().push(tab);
    }
    grouped
        .into_iter()
        .map(|(id, tabs)| ExtensionWindow {
            id,
            focused: Some(id) == heartbeat.active_window_id,
            tabs,
        })
        .collect()
}

fn element_reference_target_prefix(target: &BrowserTarget) -> String {
    format!(
        "{}:{}:{}:",
        target.extension_instance_id, target.window_id, target.tab_id
    )
}

fn element_reference_key(target: &BrowserTarget, alias: &str) -> String {
    format!("{}{}", element_reference_target_prefix(target), alias)
}

fn stale_element_reference(target: &BrowserTarget, alias: &str) -> BrokerError {
    let mut error = BrokerError::new(
        BrokerErrorCode::StaleElementReference,
        format!(
            "element reference {} is stale or belongs to another target",
            if alias.trim().is_empty() {
                "<empty>"
            } else {
                alias
            }
        ),
    );
    error.recovery.insert(
        "extension_instance_id".into(),
        Value::String(target.extension_instance_id.clone()),
    );
    error.recovery.insert(
        "retry".into(),
        Value::String("request a new snapshot and use its revision-bound reference".into()),
    );
    error
}

/// Profile/session registry owned by the broker state loop.
#[derive(Debug)]
pub struct SessionRegistry {
    heartbeat_ttl: Duration,
    disconnected_retention: Duration,
    sessions: BTreeMap<String, BrowserSessionRecord>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_HEARTBEAT_TTL, DISCONNECTED_RETENTION)
    }
}

impl SessionRegistry {
    pub fn new(heartbeat_ttl: Duration, disconnected_retention: Duration) -> Self {
        Self {
            heartbeat_ttl,
            disconnected_retention,
            sessions: BTreeMap::new(),
        }
    }

    pub fn heartbeat_ttl(&self) -> Duration {
        self.heartbeat_ttl
    }

    pub fn get(&self, extension_instance_id: &str) -> Option<&BrowserSessionRecord> {
        self.sessions.get(extension_instance_id)
    }

    pub fn get_mut(&mut self, extension_instance_id: &str) -> Option<&mut BrowserSessionRecord> {
        self.sessions.get_mut(extension_instance_id)
    }

    pub fn ensure_stream_session(
        &mut self,
        extension_instance_id: &str,
        protocol_version: u16,
        now: Instant,
    ) -> Result<(), BrokerError> {
        if extension_instance_id.trim().is_empty() || extension_instance_id.len() > 128 {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "extension instance identity is invalid",
            ));
        }
        let record = self
            .sessions
            .entry(extension_instance_id.to_owned())
            .or_insert_with(|| BrowserSessionRecord::new(extension_instance_id.to_owned(), now));
        if record.protocol_version == 0 {
            record.protocol_version = protocol_version;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn register_heartbeat(
        &mut self,
        heartbeat: ExtensionHeartbeat,
        now: Instant,
    ) -> Result<String, BrokerError> {
        self.expire_stale(now);
        let instance_id = heartbeat
            .extension_instance_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| LEGACY_INSTANCE_ID.to_owned());
        if instance_id.len() > 128 {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "extension instance identity exceeds its size limit",
            ));
        }
        let record = self
            .sessions
            .entry(instance_id.clone())
            .or_insert_with(|| BrowserSessionRecord::new(instance_id.clone(), now));
        record.update_from_heartbeat(heartbeat, now);
        Ok(instance_id)
    }

    pub fn heartbeat_response(&mut self, extension_instance_id: &str, now: Instant) -> Value {
        let Some(record) = self.sessions.get_mut(extension_instance_id) else {
            return json!({
                "ok": false,
                "code": BrokerErrorCode::BrowserSessionDisconnected.as_str(),
                "error": "browser extension session is not registered",
            });
        };
        let command = record.pop_command();
        json!({
            "ok": true,
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
            "extension_instance_id": record.extension_instance_id,
            "compatible": record.compatible(),
            "required_protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
            "command_transports": ["direct-ws", "heartbeat"],
            "accepted_features": record.features,
            "cmd": command,
            "health": record.health_at(now, self.heartbeat_ttl).as_str(),
        })
    }

    pub fn list_public(&mut self, now: Instant) -> Vec<Value> {
        self.expire_stale(now);
        self.sessions
            .values()
            .map(|record| record.public_contract(now, self.heartbeat_ttl))
            .collect()
    }

    pub fn require_live(
        &mut self,
        extension_instance_id: &str,
        now: Instant,
    ) -> Result<&mut BrowserSessionRecord, BrokerError> {
        self.expire_stale(now);
        let record = self
            .sessions
            .get_mut(extension_instance_id)
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserTargetNotFound,
                    "browser session was not found",
                )
            })?;
        if !record.compatible() {
            return Err(BrokerError::new(
                BrokerErrorCode::IncompatibleBrowserSession,
                "browser extension protocol is incompatible with this broker",
            ));
        }
        if !record.alive_at(now, self.heartbeat_ttl) {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserSessionDisconnected,
                "browser extension session is disconnected",
            ));
        }
        Ok(record)
    }

    pub fn resolve_target(
        &mut self,
        target: Option<&BrowserTarget>,
        now: Instant,
    ) -> Result<(String, BrowserTarget, bool), BrokerError> {
        self.expire_stale(now);
        if let Some(target) = target {
            let record = self.require_live(&target.extension_instance_id, now)?;
            record.require_target(target)?;
            return Ok((target.extension_instance_id.clone(), target.clone(), true));
        }

        let candidates: Vec<(String, BrowserTarget)> = self
            .sessions
            .values()
            .filter(|record| record.compatible() && record.alive_at(now, self.heartbeat_ttl))
            .filter_map(|record| {
                record
                    .current_active_target()
                    .map(|target| (record.extension_instance_id.clone(), target))
            })
            .collect();
        match candidates.as_slice() {
            [] => Err(BrokerError::new(
                BrokerErrorCode::BrowserUnavailable,
                "no live debuggable browser extension target is available",
            )),
            [(instance_id, target)] => Ok((instance_id.clone(), target.clone(), false)),
            _ => {
                let candidates = candidates
                    .iter()
                    .map(|(instance_id, target)| {
                        json!({
                            "extension_instance_id": instance_id,
                            "window_id": target.window_id,
                            "tab_id": target.tab_id,
                        })
                    })
                    .collect::<Vec<_>>();
                let mut error = BrokerError::new(
                    BrokerErrorCode::AmbiguousBrowserTarget,
                    "multiple browser profiles are available; select an explicit target",
                );
                error
                    .recovery
                    .insert("candidates".into(), Value::Array(candidates));
                Err(error)
            }
        }
    }

    pub fn attach_stream(
        &mut self,
        extension_instance_id: &str,
        generation: u64,
        now: Instant,
    ) -> Result<Option<u64>, BrokerError> {
        let record = self.require_live(extension_instance_id, now)?;
        record.attach_stream(generation)
    }

    pub fn set_profile_label(
        &mut self,
        extension_instance_id: &str,
        label: &str,
        now: Instant,
    ) -> Result<String, BrokerError> {
        let normalized: String = label.trim().chars().take(120).collect();
        if normalized.is_empty() {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "profile label is required",
            ));
        }
        let duplicate = self.sessions.values().any(|record| {
            record.extension_instance_id() != extension_instance_id
                && record.alive_at(now, self.heartbeat_ttl)
                && record.profile_label.eq_ignore_ascii_case(&normalized)
        });
        if duplicate {
            return Err(BrokerError::new(
                BrokerErrorCode::AmbiguousBrowserTarget,
                "profile label is already used by another live session",
            ));
        }
        let record = self.require_live(extension_instance_id, now)?;
        record.profile_label = normalized.clone();
        record.profile_label_managed = true;
        Ok(normalized)
    }

    pub fn clear_profile_label(
        &mut self,
        extension_instance_id: &str,
        now: Instant,
    ) -> Result<(), BrokerError> {
        let record = self.require_live(extension_instance_id, now)?;
        record.profile_label.clear();
        record.profile_label_managed = true;
        Ok(())
    }

    pub fn list_tabs(
        &mut self,
        extension_instance_id: &str,
        now: Instant,
    ) -> Result<Value, BrokerError> {
        let record = self.require_live(extension_instance_id, now)?;
        Ok(json!({
            "extension_instance_id": extension_instance_id,
            "windows": record.windows,
        }))
    }

    pub fn mark_frame_error(&mut self, extension_instance_id: &str, error: &str) {
        if let Some(record) = self.sessions.get_mut(extension_instance_id) {
            record.last_frame_error = error.chars().take(1000).collect();
        }
    }

    pub fn clear_element_references(&mut self, extension_instance_id: &str) {
        if let Some(record) = self.sessions.get_mut(extension_instance_id) {
            record.clear_element_references(None);
        }
    }

    pub fn resolve_element_reference(
        &mut self,
        target: &BrowserTarget,
        alias: &str,
        page_context_revision: Option<&str>,
        snapshot_id: Option<&str>,
        project_root: &str,
        caller_label: &str,
        now: Instant,
    ) -> Result<ElementReferenceRecord, BrokerError> {
        let record = self.require_live(&target.extension_instance_id, now)?;
        record.resolve_element_reference(
            target,
            alias,
            page_context_revision,
            snapshot_id,
            project_root,
            caller_label,
            now,
        )
    }

    pub fn detach_stream(&mut self, extension_instance_id: &str, generation: u64) -> bool {
        self.sessions
            .get_mut(extension_instance_id)
            .is_some_and(|record| record.detach_stream(generation))
    }

    pub fn update_frame(
        &mut self,
        target: BrowserTarget,
        seq: u64,
        url: String,
        jpeg: Vec<u8>,
        now: Instant,
    ) -> Result<(), BrokerError> {
        let record = self.require_live(&target.extension_instance_id, now)?;
        record.update_frame(target, seq, url, jpeg, now)
    }

    pub fn subscribe_target(
        &mut self,
        target: BrowserTarget,
        now: Instant,
    ) -> Result<(), BrokerError> {
        let record = self.require_live(&target.extension_instance_id, now)?;
        record.subscribe_target(target)
    }

    /// Expire heartbeat liveness and remove records past the diagnostic window.
    /// Returns identities that were removed so pending state can fail them.
    pub fn expire_stale(&mut self, now: Instant) -> Vec<String> {
        let mut removed = Vec::new();
        for record in self.sessions.values_mut() {
            if !record.alive_at(now, self.heartbeat_ttl) {
                record.mark_disconnected(now);
            }
        }
        self.sessions.retain(|instance_id, record| {
            let keep = record.disconnected_since.is_none_or(|since| {
                now.saturating_duration_since(since) <= self.disconnected_retention
            });
            if !keep {
                removed.push(instance_id.clone());
            }
            keep
        });
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ExtensionTab, ExtensionWindow, FeatureAvailability};

    fn heartbeat(instance_id: Option<&str>, url: &str) -> ExtensionHeartbeat {
        ExtensionHeartbeat {
            schema_version: instance_id
                .map(|_| Some(BROWSER_BROKER_SCHEMA_VERSION))
                .flatten(),
            protocol_version: instance_id
                .map(|_| Some(BROWSER_BROKER_PROTOCOL_VERSION))
                .flatten(),
            extension_instance_id: instance_id.map(str::to_owned),
            profile_label: instance_id.unwrap_or("legacy").into(),
            extension_version: "test".into(),
            features: vec![FeatureAvailability {
                feature: "p0.control".into(),
                available: true,
                reason: None,
            }],
            supported_actions: vec!["click".into()],
            supported_operations: vec!["get_page_snapshot".into()],
            optional_permissions: BTreeMap::new(),
            browser: BTreeMap::from([("name".into(), json!("Chromium"))]),
            project_root: Some("/ignored/project".into()),
            url: url.into(),
            title: "Test".into(),
            active_window_id: Some(7),
            active_tab_id: Some(42),
            tabs: vec![],
            windows: vec![ExtensionWindow {
                id: 7,
                focused: true,
                tabs: vec![ExtensionTab {
                    id: 42,
                    window_id: 7,
                    title: "Test".into(),
                    url: url.into(),
                    active: true,
                    favicon_url: String::new(),
                    debuggable: true,
                }],
            }],
            frame_error: String::new(),
        }
    }

    fn target(instance_id: &str) -> BrowserTarget {
        BrowserTarget {
            extension_instance_id: instance_id.into(),
            window_id: 7,
            tab_id: 42,
        }
    }

    #[test]
    fn heartbeat_reconnect_refreshes_one_profile_without_project_ownership() {
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        registry
            .register_heartbeat(heartbeat(Some("profile-a"), "https://before.test"), start)
            .unwrap();
        registry
            .register_heartbeat(
                heartbeat(Some("profile-a"), "https://after.test"),
                start + Duration::from_secs(1),
            )
            .unwrap();

        assert_eq!(registry.len(), 1);
        let session = registry.get("profile-a").unwrap();
        assert_eq!(session.extension_instance_id(), "profile-a");
        assert_eq!(session.windows()[0].tabs[0].url, "https://after.test");
        assert_eq!(
            session.health_at(start + Duration::from_secs(1), DEFAULT_HEARTBEAT_TTL),
            SessionHealth::Ready
        );
    }

    #[test]
    fn legacy_and_future_protocols_have_explicit_compatibility_health() {
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        registry
            .register_heartbeat(heartbeat(None, "https://legacy.test"), start)
            .unwrap();
        assert!(registry.get(LEGACY_INSTANCE_ID).unwrap().is_legacy());

        let mut future = heartbeat(Some("future"), "https://future.test");
        future.protocol_version = Some(BROWSER_BROKER_PROTOCOL_VERSION + 1);
        registry.register_heartbeat(future, start).unwrap();
        assert_eq!(
            registry
                .get("future")
                .unwrap()
                .health_at(start, DEFAULT_HEARTBEAT_TTL),
            SessionHealth::Incompatible
        );
    }

    #[test]
    fn stale_profiles_clear_stream_commands_frames_and_subscriptions_then_expire() {
        let start = Instant::now();
        let mut registry = SessionRegistry::new(Duration::from_secs(2), Duration::from_secs(5));
        registry
            .register_heartbeat(heartbeat(Some("profile-a"), "https://test"), start)
            .unwrap();
        registry.attach_stream("profile-a", 1, start).unwrap();
        registry
            .get_mut("profile-a")
            .unwrap()
            .queue_command(json!({"cmd":"click"}))
            .unwrap();
        registry
            .subscribe_target(target("profile-a"), start)
            .unwrap();
        registry
            .update_frame(
                target("profile-a"),
                1,
                "https://test".into(),
                vec![0xff, 0xd8, 0xff, 0xd9],
                start,
            )
            .unwrap();

        registry.expire_stale(start + Duration::from_secs(3));
        let session = registry.get("profile-a").unwrap();
        assert_eq!(
            session.health_at(start + Duration::from_secs(3), Duration::from_secs(2)),
            SessionHealth::Disconnected
        );
        assert_eq!(session.current_stream_generation(), None);
        assert_eq!(session.queued_command_count(), 0);
        assert!(session.latest_frame(&target("profile-a")).is_none());
        assert!(!session.is_subscribed(&target("profile-a")));

        let removed = registry.expire_stale(start + Duration::from_secs(9));
        assert_eq!(removed, vec!["profile-a"]);
        assert!(registry.is_empty());
    }

    #[test]
    fn explicit_target_isolated_and_implicit_target_rejects_ambiguity() {
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        registry
            .register_heartbeat(heartbeat(Some("profile-a"), "https://a.test"), start)
            .unwrap();
        registry
            .register_heartbeat(heartbeat(Some("profile-b"), "https://b.test"), start)
            .unwrap();

        let error = registry.resolve_target(None, start).unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::AmbiguousBrowserTarget);
        assert_eq!(error.recovery["candidates"].as_array().unwrap().len(), 2);

        let (instance_id, resolved, explicit) = registry
            .resolve_target(Some(&target("profile-a")), start)
            .unwrap();
        assert_eq!(instance_id, "profile-a");
        assert_eq!(resolved, target("profile-a"));
        assert!(explicit);

        let mismatch = registry
            .resolve_target(
                Some(&BrowserTarget {
                    extension_instance_id: "profile-b".into(),
                    window_id: 7,
                    tab_id: 999,
                }),
                start,
            )
            .unwrap_err();
        assert_eq!(mismatch.code, BrokerErrorCode::BrowserTargetNotFound);
    }

    #[test]
    fn shared_ambiguous_target_fixture_has_no_dispatch_side_effect() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../resources/browser_contract_fixtures.json"
        ))
        .unwrap();
        let scenario = &fixture["stateful"]["ambiguous_implicit_target"];
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        for item in scenario["targets"].as_array().unwrap() {
            let instance_id = item["extension_instance_id"].as_str().unwrap();
            registry
                .register_heartbeat(
                    heartbeat(Some(instance_id), &format!("https://{instance_id}.test")),
                    start,
                )
                .unwrap();
        }

        let error = registry.resolve_target(None, start).unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::AmbiguousBrowserTarget);
        assert_eq!(
            registry
                .sessions
                .values()
                .map(BrowserSessionRecord::queued_command_count)
                .sum::<usize>(),
            scenario["expected_dispatched_commands"].as_u64().unwrap() as usize
        );
    }

    #[test]
    fn stream_generation_replacement_does_not_allow_old_disconnect_to_remove_new_stream() {
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        registry
            .register_heartbeat(heartbeat(Some("profile-a"), "https://test"), start)
            .unwrap();
        assert_eq!(
            registry.attach_stream("profile-a", 10, start).unwrap(),
            None
        );
        assert_eq!(
            registry.attach_stream("profile-a", 11, start).unwrap(),
            Some(10)
        );
        assert!(!registry.detach_stream("profile-a", 10));
        assert_eq!(
            registry
                .get("profile-a")
                .unwrap()
                .current_stream_generation(),
            Some(11)
        );
        assert!(registry.detach_stream("profile-a", 11));
    }

    #[test]
    fn legacy_tab_inventory_is_normalized_and_navigation_clears_target_state() {
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        let mut first = heartbeat(None, "https://before.test");
        first.windows.clear();
        first.tabs = vec![ExtensionTab {
            id: 42,
            window_id: 9,
            title: "Legacy".into(),
            url: "https://before.test".into(),
            active: true,
            favicon_url: String::new(),
            debuggable: true,
        }];
        first.active_window_id = Some(9);
        registry.register_heartbeat(first, start).unwrap();
        assert_eq!(registry.get(LEGACY_INSTANCE_ID).unwrap().windows()[0].id, 9);

        registry
            .attach_stream(LEGACY_INSTANCE_ID, 1, start)
            .unwrap();
        registry
            .subscribe_target(
                BrowserTarget {
                    extension_instance_id: LEGACY_INSTANCE_ID.into(),
                    window_id: 9,
                    tab_id: 42,
                },
                start,
            )
            .unwrap();
        registry
            .update_frame(
                BrowserTarget {
                    extension_instance_id: LEGACY_INSTANCE_ID.into(),
                    window_id: 9,
                    tab_id: 42,
                },
                1,
                "https://before.test".into(),
                vec![0xff, 0xd8, 0xff, 0xd9],
                start,
            )
            .unwrap();

        let mut navigated = heartbeat(None, "https://after.test");
        navigated.windows.clear();
        navigated.tabs = vec![ExtensionTab {
            id: 42,
            window_id: 9,
            title: "Legacy".into(),
            url: "https://after.test".into(),
            active: true,
            favicon_url: String::new(),
            debuggable: true,
        }];
        navigated.active_window_id = Some(9);
        registry
            .register_heartbeat(navigated, start + Duration::from_secs(1))
            .unwrap();

        let target = BrowserTarget {
            extension_instance_id: LEGACY_INSTANCE_ID.into(),
            window_id: 9,
            tab_id: 42,
        };
        let session = registry.get(LEGACY_INSTANCE_ID).unwrap();
        assert!(session.latest_frame(&target).is_none());
        assert!(!session.is_subscribed(&target));
    }

    #[test]
    fn preview_frames_keep_the_newest_sequence_for_each_target() {
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        registry
            .register_heartbeat(heartbeat(Some("profile-a"), "https://test"), start)
            .unwrap();
        let target = target("profile-a");
        registry
            .update_frame(
                target.clone(),
                2,
                "https://new.test".into(),
                vec![0xff, 0xd8, 0xff, 0xd9],
                start + Duration::from_secs(2),
            )
            .unwrap();
        registry
            .update_frame(
                target.clone(),
                1,
                "https://old.test".into(),
                vec![0xff, 0xd8, 0xff, 0xd9],
                start + Duration::from_secs(3),
            )
            .unwrap();
        let frame = registry
            .get("profile-a")
            .unwrap()
            .latest_frame(&target)
            .unwrap();
        assert_eq!(frame.seq, 2);
        assert_eq!(frame.url, "https://new.test");
    }

    #[test]
    fn snapshot_references_are_bound_to_target_revision_and_request_scope() {
        let start = Instant::now();
        let mut registry = SessionRegistry::default();
        registry
            .register_heartbeat(heartbeat(Some("profile-a"), "https://test"), start)
            .unwrap();
        let target = target("profile-a");
        let record = registry.get_mut("profile-a").unwrap();
        let mut response = json!({
            "snapshot_id": "snapshot-1",
            "page_context_revision": "revision-1",
            "interactive_elements": [{
                "tag": "button",
                "role": "button",
                "context": {"frame": "main"}
            }]
        });
        record
            .cache_snapshot_references(
                target.clone(),
                &mut response,
                "request-1",
                "C:/project-a",
                "caller-a",
                start,
            )
            .unwrap();
        assert_eq!(response["interactive_elements"][0]["ref"], "@e1");
        assert_eq!(record.element_reference_count(), 1);

        let resolved = record
            .resolve_element_reference(
                &target,
                "@e1",
                Some("revision-1"),
                Some("snapshot-1"),
                "C:/project-a",
                "caller-a",
                start,
            )
            .unwrap();
        assert_eq!(resolved.element["tag"], "button");
        assert_eq!(
            record
                .resolve_element_reference(
                    &target,
                    "@e1",
                    Some("revision-1"),
                    Some("snapshot-1"),
                    "C:/project-b",
                    "caller-a",
                    start,
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::StaleElementReference
        );
        assert_eq!(
            record
                .resolve_element_reference(
                    &BrowserTarget {
                        extension_instance_id: "profile-b".into(),
                        window_id: 7,
                        tab_id: 42,
                    },
                    "@e1",
                    Some("revision-1"),
                    Some("snapshot-1"),
                    "C:/project-a",
                    "caller-a",
                    start,
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::StaleElementReference
        );
        assert_eq!(
            record
                .resolve_element_reference(
                    &target,
                    "@e1",
                    Some("revision-2"),
                    Some("snapshot-1"),
                    "C:/project-a",
                    "caller-a",
                    start,
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::StaleElementReference
        );

        let mut next_response = json!({
            "page_context_revision": "revision-2",
            "interactive_elements": [{"tag": "input"}]
        });
        record
            .cache_snapshot_references(
                target.clone(),
                &mut next_response,
                "request-2",
                "C:/project-a",
                "caller-a",
                start + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(record.element_reference_count(), 1);
        assert!(
            record
                .resolve_element_reference(
                    &target,
                    "@e1",
                    Some("revision-1"),
                    None,
                    "C:/project-a",
                    "caller-a",
                    start + Duration::from_secs(1),
                )
                .is_err()
        );
        assert_eq!(
            record
                .resolve_element_reference(
                    &target,
                    "@e1",
                    Some("revision-2"),
                    None,
                    "C:/project-a",
                    "caller-a",
                    start + ELEMENT_REFERENCE_TTL + Duration::from_secs(2),
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::StaleElementReference
        );
    }
}
