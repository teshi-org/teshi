//! Target-scoped screenshot/PDF evidence validation and managed storage.
//!
//! The extension remains the owner of Chrome/CDP capture.  This module is the
//! broker-side boundary for the untrusted bytes returned by that capture: it
//! binds a response to the request context, validates the actual payload, and
//! publishes a broker-generated artifact without replacing an existing file.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::protocol::{
    BrokerError, BrokerErrorCode, BrowserTarget, ExtensionResponse, MAX_NETWORK_PENDING_EVENTS,
    NetworkBatch,
};

/// Maximum decoded bytes retained or written for one browser artifact.
pub const MAX_ARTIFACT_BYTES: usize = 50 * 1024 * 1024;
/// Maximum encoded bytes accepted before base64 decoding allocates a buffer.
pub const MAX_ARTIFACT_BASE64_BYTES: usize = MAX_ARTIFACT_BYTES.div_ceil(3) * 4 + 4;
/// Maximum width or height of a decoded screenshot.
pub const MAX_IMAGE_DIMENSION: u32 = 16_384;
/// Maximum decoded screenshot pixels.
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;
/// Maximum number of prepared artifacts retained by one broker state owner.
pub const MAX_PREPARED_ARTIFACTS: usize = 128;
/// Maximum number of artifact names accepted by one cleanup operation.
pub const MAX_CLEANUP_ARTIFACTS: usize = 64;
/// Maximum bytes in a broker-generated artifact filename.
pub const MAX_ARTIFACT_FILENAME_BYTES: usize = 240;

const MANAGED_ARTIFACT_COMPONENTS: [&str; 3] = [".teshi", "artifacts", "browser"];

/// Default Console retention age, matching the Python broker during migration.
pub const DEFAULT_CONSOLE_MAX_AGE_MS: u64 = 300_000;
/// Default Console entry bound, matching the Python broker during migration.
pub const DEFAULT_CONSOLE_MAX_ENTRIES: usize = 500;
/// Default Console byte bound, matching the Python broker during migration.
pub const DEFAULT_CONSOLE_MAX_BYTES: usize = 1_048_576;
pub const MAX_CONSOLE_MAX_AGE_MS: u64 = 3_600_000;
pub const MAX_CONSOLE_MAX_ENTRIES: usize = 5_000;
pub const MAX_CONSOLE_MAX_BYTES: usize = 8 * 1024 * 1024;
/// A single event is bounded independently of the aggregate capture budget.
pub const MAX_CONSOLE_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_CONSOLE_EVENT_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_CONSOLE_EVENT_SOURCE_BYTES: usize = 120;
pub const MAX_CONSOLE_EVENT_URL_BYTES: usize = 4 * 1024;
pub const MAX_CONSOLE_CAPTURE_DIAGNOSTICS: usize = 64;
pub const MAX_ACTIVE_CONSOLE_CAPTURES: usize = 128;

const KNOWN_CONSOLE_LEVELS: [&str; 5] = ["debug", "log", "info", "warn", "error"];
const MAX_CONSOLE_LEVEL_FILTERS: usize = KNOWN_CONSOLE_LEVELS.len();
const MAX_CONSOLE_LEVEL_BYTES: usize = 32;
const MAX_CONSOLE_RAW_TEXT_BYTES: usize = MAX_CONSOLE_EVENT_TEXT_BYTES * 8;
const MAX_CONSOLE_RAW_URL_BYTES: usize = MAX_CONSOLE_EVENT_URL_BYTES * 2;
const DEFAULT_SENSITIVE_CONSOLE_FIELDS: [&str; 12] = [
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "token",
    "access-token",
    "refresh-token",
    "password",
    "passwd",
    "secret",
];
const REDACTION_MARKER: &str = "[REDACTED]";

/// Bounded target-scoped Console retention configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleCaptureConfig {
    #[serde(default = "default_console_capture_age_ms")]
    pub max_age_ms: u64,
    #[serde(default = "default_console_capture_entries")]
    pub max_entries: usize,
    #[serde(default = "default_console_capture_bytes")]
    pub max_bytes: usize,
}

impl Default for ConsoleCaptureConfig {
    fn default() -> Self {
        Self {
            max_age_ms: default_console_capture_age_ms(),
            max_entries: default_console_capture_entries(),
            max_bytes: default_console_capture_bytes(),
        }
    }
}

impl ConsoleCaptureConfig {
    fn from_values(
        max_age_ms: Option<&Value>,
        max_entries: Option<&Value>,
        max_bytes: Option<&Value>,
    ) -> Result<Self, BrokerError> {
        Ok(Self {
            max_age_ms: bounded_u64(
                max_age_ms,
                DEFAULT_CONSOLE_MAX_AGE_MS,
                1_000,
                MAX_CONSOLE_MAX_AGE_MS,
                "max_age_ms",
            )?,
            max_entries: bounded_usize(
                max_entries,
                DEFAULT_CONSOLE_MAX_ENTRIES,
                1,
                MAX_CONSOLE_MAX_ENTRIES,
                "max_entries",
            )?,
            max_bytes: bounded_usize(
                max_bytes,
                DEFAULT_CONSOLE_MAX_BYTES,
                1_024,
                MAX_CONSOLE_MAX_BYTES,
                "max_bytes",
            )?,
        })
    }
}

/// Bounded target-scoped Network capture configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkCaptureConfig {
    #[serde(default)]
    pub allowed_hostnames: Vec<String>,
    #[serde(default)]
    pub capture_request_bodies: bool,
    #[serde(default = "default_network_request_body_bytes")]
    pub max_request_body_bytes: usize,
    #[serde(default = "default_network_capture_age_ms")]
    pub max_age_ms: u64,
    #[serde(default = "default_network_capture_entries")]
    pub max_entries: usize,
    #[serde(default = "default_network_capture_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_network_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default)]
    pub sensitive_fields: BTreeSet<String>,
}

impl Default for NetworkCaptureConfig {
    fn default() -> Self {
        Self {
            allowed_hostnames: Vec::new(),
            capture_request_bodies: false,
            max_request_body_bytes: default_network_request_body_bytes(),
            max_age_ms: default_network_capture_age_ms(),
            max_entries: default_network_capture_entries(),
            max_bytes: default_network_capture_bytes(),
            max_body_bytes: default_network_body_bytes(),
            sensitive_fields: default_sensitive_fields(),
        }
    }
}

impl NetworkCaptureConfig {
    #[allow(clippy::too_many_arguments)]
    fn from_values(
        allowed_hostnames: Option<&Value>,
        capture_request_bodies: Option<&Value>,
        max_request_body_bytes: Option<&Value>,
        max_age_ms: Option<&Value>,
        max_entries: Option<&Value>,
        max_bytes: Option<&Value>,
        max_body_bytes: Option<&Value>,
        sensitive_fields: Option<&Value>,
    ) -> Result<Self, BrokerError> {
        let hostnames = normalize_allowed_hostnames(allowed_hostnames)?;
        let capture_request_bodies = match capture_request_bodies {
            None => false,
            Some(value) => value.as_bool().ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::InvalidBrowserOperation,
                    "capture_request_bodies must be a boolean",
                )
            })?,
        };
        Ok(Self {
            allowed_hostnames: hostnames.into_iter().collect(),
            capture_request_bodies,
            max_request_body_bytes: bounded_usize(
                max_request_body_bytes,
                default_network_request_body_bytes(),
                1,
                MAX_NETWORK_BODY_BYTES,
                "max_request_body_bytes",
            )?,
            max_age_ms: bounded_u64(
                max_age_ms,
                default_network_capture_age_ms(),
                1_000,
                MAX_NETWORK_CAPTURE_AGE_MS,
                "max_age_ms",
            )?,
            max_entries: bounded_usize(
                max_entries,
                default_network_capture_entries(),
                1,
                MAX_NETWORK_CAPTURE_ENTRIES,
                "max_entries",
            )?,
            max_bytes: bounded_usize(
                max_bytes,
                default_network_capture_bytes(),
                2_048,
                MAX_NETWORK_CAPTURE_BYTES,
                "max_bytes",
            )?,
            max_body_bytes: bounded_usize(
                max_body_bytes,
                default_network_body_bytes(),
                1_024,
                MAX_NETWORK_BODY_BYTES,
                "max_body_bytes",
            )?,
            sensitive_fields: normalize_sensitive_fields(sensitive_fields)?,
        })
    }
}

const fn default_network_capture_age_ms() -> u64 {
    300_000
}

const fn default_network_capture_entries() -> usize {
    1_000
}

const fn default_network_capture_bytes() -> usize {
    2 * 1024 * 1024
}

const fn default_network_request_body_bytes() -> usize {
    256 * 1024
}

const fn default_network_body_bytes() -> usize {
    256 * 1024
}

pub const MAX_NETWORK_CAPTURE_AGE_MS: u64 = 3_600_000;
pub const MAX_NETWORK_CAPTURE_ENTRIES: usize = 10_000;
pub const MAX_NETWORK_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_NETWORK_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_NETWORK_EVENT_BYTES: usize = 256 * 1024;
pub const MAX_NETWORK_PENDING_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_NETWORK_DIAGNOSTICS: usize = 64;
pub const MAX_NETWORK_BARRIER_RECORDS: usize = 64;
pub const MAX_NETWORK_GAP_DIAGNOSTICS: usize = 32;
pub const MAX_NETWORK_HOSTNAMES: usize = 64;
pub const MAX_NETWORK_HOSTNAME_BYTES: usize = 253;
pub const MAX_NETWORK_REQUEST_ID_BYTES: usize = 256;

/// Handle returned after the broker has reserved a Network capture ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkCaptureHandle {
    pub capture_id: String,
}

/// Explicit, lease-scoped authorization to fetch one bounded response body.
///
/// The access record is kept outside the retained Network request metadata so
/// a body request cannot be reconstructed after the capture is cleared or
/// terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkBodyAccess {
    pub capture_id: String,
    pub request_id: String,
    pub max_body_bytes: usize,
}

const fn default_console_capture_age_ms() -> u64 {
    DEFAULT_CONSOLE_MAX_AGE_MS
}

const fn default_console_capture_entries() -> usize {
    DEFAULT_CONSOLE_MAX_ENTRIES
}

const fn default_console_capture_bytes() -> usize {
    DEFAULT_CONSOLE_MAX_BYTES
}

/// Actual dimensions parsed from the image payload, never copied from JSON
/// metadata supplied by the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDimensions {
    pub width: u32,
    pub height: u32,
    pub pixels: u64,
}

/// Safe metadata returned after an artifact has been atomically published.
///
/// `path` is a broker-generated filename relative to the managed browser
/// artifact directory.  The absolute project path is intentionally not part
/// of this response or of diagnostic logging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedArtifact {
    pub path: String,
    pub size: u64,
    pub format: String,
    pub media_type: String,
    pub target: BrowserTarget,
    pub request_id: String,
    pub page_context_revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<ArtifactDimensions>,
    pub managed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    EvidenceJpeg,
    Screenshot,
    Pdf,
}

impl ArtifactKind {
    fn operation(self) -> &'static str {
        match self {
            Self::EvidenceJpeg => "capture_browser_evidence",
            Self::Screenshot => "capture_browser_screenshot",
            Self::Pdf => "generate_browser_pdf",
        }
    }

    fn default_format(self) -> &'static str {
        match self {
            Self::EvidenceJpeg => "jpeg",
            Self::Screenshot => "png",
            Self::Pdf => "pdf",
        }
    }

    fn response_data_key(self) -> &'static str {
        match self {
            Self::EvidenceJpeg => "screenshot",
            Self::Screenshot | Self::Pdf => "artifact_data",
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedRecord {
    kind: ArtifactKind,
    broker_start_id: String,
    project_root: PathBuf,
    caller_label: String,
    target: BrowserTarget,
    request_id: String,
    expected_page_context_revision: Option<String>,
    format: String,
    media_type: String,
    artifact_root: PathBuf,
    final_path: PathBuf,
    relative_path: String,
}

#[derive(Debug, Clone)]
struct ManagedRecord {
    artifact: PreparedArtifact,
    project_root: PathBuf,
    caller_label: String,
    final_path: PathBuf,
    content_digest: [u8; 32],
}

/// Handle returned to the broker state owner for one pending Console start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleCaptureHandle {
    pub capture_id: String,
}

#[derive(Debug, Clone)]
struct ConsoleEventRecord {
    value: Value,
    received_at: Instant,
    byte_size: usize,
}

#[derive(Debug, Clone)]
struct ConsoleCaptureRecord {
    capture_id: String,
    target: BrowserTarget,
    project_root: String,
    caller_label: String,
    broker_start_id: String,
    stream_generation: Option<u64>,
    config: ConsoleCaptureConfig,
    levels: BTreeSet<String>,
    sensitive_fields: BTreeSet<String>,
    events: VecDeque<ConsoleEventRecord>,
    retained_bytes: usize,
    evicted_age: u64,
    evicted_entries: u64,
    evicted_bytes: u64,
    rejected_events: u64,
    filtered_events: u64,
    truncated_events: u64,
}

#[derive(Debug, Clone)]
struct PendingConsoleCapture {
    request_id: String,
    capture_id: String,
    target: BrowserTarget,
    project_root: String,
    caller_label: String,
    broker_start_id: String,
    stream_generation: Option<u64>,
    config: ConsoleCaptureConfig,
    levels: BTreeSet<String>,
    sensitive_fields: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkRequestPhase {
    Requested,
    ResponseReceived,
    Finished,
    Failed,
}

#[derive(Debug, Clone)]
struct NetworkRequestRecord {
    value: Value,
    received_at: Instant,
    byte_size: usize,
    phase: NetworkRequestPhase,
}

#[derive(Debug, Clone)]
struct PendingNetworkCapture {
    request_id: String,
    capture_id: String,
    target: BrowserTarget,
    project_root: String,
    caller_label: String,
    broker_start_id: String,
    stream_generation: Option<u64>,
    config: NetworkCaptureConfig,
}

#[derive(Debug, Clone)]
struct NetworkCaptureRecord {
    capture_id: String,
    target: BrowserTarget,
    project_root: String,
    caller_label: String,
    broker_start_id: String,
    stream_generation: Option<u64>,
    config: NetworkCaptureConfig,
    requests: HashMap<String, NetworkRequestRecord>,
    request_order: VecDeque<String>,
    retained_bytes: usize,
    acknowledged_sequence: u64,
    highest_seen_sequence: u64,
    pending_events: HashMap<u64, Value>,
    pending_bytes: usize,
    clear_sequence: u64,
    dropped_events: u64,
    dropped_batches: u64,
    dropped_bytes: u64,
    rejected_events: u64,
    filtered_events: u64,
    duplicate_events: u64,
    gap_events: u64,
    loss_diagnostics: VecDeque<Value>,
    termination_reason: Option<String>,
    terminated_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct NetworkBarrier {
    target: BrowserTarget,
    capture_id: String,
    acknowledged_sequence: u64,
}

/// Single-owner evidence state.  `BrokerState` owns one instance and calls it
/// synchronously from the same event loop that owns requests and leases.
#[derive(Debug, Default)]
pub struct EvidenceStore {
    prepared: HashMap<String, PreparedRecord>,
    managed: HashMap<PathBuf, ManagedRecord>,
    console_captures: HashMap<BrowserTarget, ConsoleCaptureRecord>,
    pending_console_captures: HashMap<String, PendingConsoleCapture>,
    terminated_console_captures: VecDeque<Value>,
    network_captures: HashMap<BrowserTarget, NetworkCaptureRecord>,
    pending_network_captures: HashMap<String, PendingNetworkCapture>,
    network_barriers: HashMap<String, NetworkBarrier>,
    terminated_network_captures: VecDeque<Value>,
}

impl EvidenceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate and reserve a Console capture without replacing the currently
    /// active capture.  The replacement happens only after the extension has
    /// acknowledged the same capture ID, which makes start failure atomic.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_console_capture(
        &mut self,
        request_id: &str,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        target: &BrowserTarget,
        stream_generation: Option<u64>,
        levels: Option<&Value>,
        max_age_ms: Option<&Value>,
        max_entries: Option<&Value>,
        max_bytes: Option<&Value>,
        sensitive_fields: Option<&Value>,
    ) -> Result<ConsoleCaptureHandle, BrokerError> {
        validate_console_scope(
            request_id,
            broker_start_id,
            project_root,
            caller_label,
            target,
        )?;
        if self.pending_console_captures.contains_key(request_id) {
            return Err(BrokerError::new(
                BrokerErrorCode::DuplicateBrowserMutation,
                "console capture request_id is already reserved",
            ));
        }
        if self.pending_console_captures.len() >= MAX_PREPARED_ARTIFACTS {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker has too many pending console captures",
            ));
        }
        if !self.console_captures.contains_key(target)
            && self
                .console_captures
                .len()
                .saturating_add(self.pending_console_captures.len())
                >= MAX_ACTIVE_CONSOLE_CAPTURES
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker has too many active console captures",
            ));
        }
        let config = ConsoleCaptureConfig::from_values(max_age_ms, max_entries, max_bytes)?;
        let levels = normalize_console_levels(levels)?;
        let sensitive_fields = normalize_sensitive_fields(sensitive_fields)?;
        let capture_id = format!("console_{}", Uuid::new_v4().simple());
        self.pending_console_captures.insert(
            request_id.to_owned(),
            PendingConsoleCapture {
                request_id: request_id.to_owned(),
                capture_id: capture_id.clone(),
                target: target.clone(),
                project_root: project_root.to_owned(),
                caller_label: caller_label.to_owned(),
                broker_start_id: broker_start_id.to_owned(),
                stream_generation,
                config,
                levels,
                sensitive_fields,
            },
        );
        Ok(ConsoleCaptureHandle { capture_id })
    }

    /// Commit a prepared Console start after the extension echoed the capture
    /// ID.  A v1 extension that does not echo the additive field fails closed,
    /// because accepting ID-less events would allow an old capture to bleed
    /// into a replacement capture.
    pub fn commit_console_capture(
        &mut self,
        request_id: &str,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        target: &BrowserTarget,
        response: &ExtensionResponse,
    ) -> Result<Value, BrokerError> {
        let Some(pending) = self.pending_console_captures.remove(request_id) else {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "console capture response has no prepared request",
            ));
        };
        if pending.request_id != request_id
            || pending.broker_start_id != broker_start_id
            || pending.project_root != project_root
            || pending.caller_label != caller_label
            || pending.target != *target
            || !response.ok
        {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "console capture response does not match its prepared request",
            ));
        }
        let returned_capture_id = response
            .result
            .get("capture_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if returned_capture_id != Some(pending.capture_id.as_str()) {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityUnavailable,
                "extension must echo capture_id for target-scoped console capture",
            ));
        }
        if response.result.get("active").and_then(Value::as_bool) == Some(false) {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserOperationFailed,
                "extension did not activate console capture",
            ));
        }
        let record = ConsoleCaptureRecord {
            capture_id: pending.capture_id,
            target: pending.target,
            project_root: pending.project_root,
            caller_label: pending.caller_label,
            broker_start_id: pending.broker_start_id,
            stream_generation: pending.stream_generation,
            config: pending.config,
            levels: pending.levels,
            sensitive_fields: pending.sensitive_fields,
            events: VecDeque::new(),
            retained_bytes: 0,
            evicted_age: 0,
            evicted_entries: 0,
            evicted_bytes: 0,
            rejected_events: 0,
            filtered_events: 0,
            truncated_events: 0,
        };
        if let Some(previous) = self.console_captures.remove(target) {
            self.record_console_termination(&previous, "capture_replaced", None);
        }
        let summary = console_capture_summary(&record, true, None);
        self.console_captures.insert(target.clone(), record);
        Ok(summary)
    }

    pub fn abort_console_capture(&mut self, request_id: &str) {
        self.pending_console_captures.remove(request_id);
    }

    pub fn console_capture_id(&self, target: &BrowserTarget) -> Option<String> {
        self.console_captures
            .get(target)
            .map(|capture| capture.capture_id.clone())
    }

    pub fn console_capture_scope_matches(
        &self,
        target: &BrowserTarget,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
    ) -> bool {
        self.console_captures.get(target).is_some_and(|capture| {
            capture.broker_start_id == broker_start_id
                && capture.project_root == project_root
                && capture.caller_label == caller_label
        })
    }

    /// Move active captures to the current extension stream generation. Events
    /// carrying the previous generation remain rejectable after reconnect.
    pub fn update_console_stream_generation(
        &mut self,
        extension_instance_id: &str,
        generation: u64,
    ) {
        for capture in self.console_captures.values_mut() {
            if capture.target.extension_instance_id == extension_instance_id {
                capture.stream_generation = Some(generation);
            }
        }
        for capture in self.pending_console_captures.values_mut() {
            if capture.target.extension_instance_id == extension_instance_id {
                capture.stream_generation = Some(generation);
            }
        }
    }

    /// Retain a sanitized Console event only when its complete target,
    /// capture ID, and current extension stream generation all match.
    pub fn record_console_event(
        &mut self,
        extension_instance_id: &str,
        target: &BrowserTarget,
        capture_id: Option<&str>,
        stream_generation: Option<u64>,
        raw_event: Option<&Value>,
    ) -> bool {
        if target.extension_instance_id != extension_instance_id {
            return false;
        }
        let Some(capture) = self.console_captures.get_mut(target) else {
            return false;
        };
        if capture.capture_id != capture_id.unwrap_or_default()
            || capture.stream_generation != stream_generation
        {
            capture.rejected_events = capture.rejected_events.saturating_add(1);
            return false;
        }
        let Some((mut value, mut truncated)) =
            sanitize_console_event(raw_event, &capture.sensitive_fields)
        else {
            capture.rejected_events = capture.rejected_events.saturating_add(1);
            return false;
        };
        let level = value.get("level").and_then(Value::as_str).unwrap_or("log");
        if !capture.levels.contains(level) {
            capture.filtered_events = capture.filtered_events.saturating_add(1);
            return false;
        }
        let (byte_size, fit_truncated) = fit_console_event(
            &mut value,
            capture.config.max_bytes.min(MAX_CONSOLE_EVENT_BYTES),
        );
        truncated |= fit_truncated;
        if byte_size == 0 {
            capture.rejected_events = capture.rejected_events.saturating_add(1);
            return false;
        }
        if truncated {
            capture.truncated_events = capture.truncated_events.saturating_add(1);
        }
        let now = Instant::now();
        evict_console_events(capture, now);
        capture.events.push_back(ConsoleEventRecord {
            value,
            received_at: now,
            byte_size,
        });
        capture.retained_bytes = capture.retained_bytes.saturating_add(byte_size);
        evict_console_events(capture, now);
        true
    }

    pub fn list_console_events(
        &mut self,
        target: &BrowserTarget,
        levels: Option<&Value>,
        max_age_ms: Option<&Value>,
        max_entries: Option<&Value>,
        max_bytes: Option<&Value>,
    ) -> Result<Value, BrokerError> {
        let capture = self.console_captures.get_mut(target).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "console capture is not active for the selected target",
            )
        })?;
        let now = Instant::now();
        evict_console_events(capture, now);
        let selected_levels = levels
            .map(|value| normalize_console_levels(Some(value)))
            .transpose()?
            .unwrap_or_else(|| capture.levels.clone());
        if !selected_levels.is_subset(&capture.levels) {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "console list level filter cannot widen the active capture",
            ));
        }
        let age_limit = bounded_u64(
            max_age_ms,
            capture.config.max_age_ms,
            0,
            capture.config.max_age_ms,
            "max_age_ms",
        )?;
        let entry_limit = bounded_usize(
            max_entries,
            capture.config.max_entries,
            1,
            capture.config.max_entries,
            "max_entries",
        )?;
        let byte_limit = bounded_usize(
            max_bytes,
            capture.config.max_bytes,
            1,
            capture.config.max_bytes,
            "max_bytes",
        )?;
        let mut selected = Vec::new();
        let mut selected_bytes = 0usize;
        for event in capture.events.iter().rev() {
            if now.saturating_duration_since(event.received_at).as_millis() > u128::from(age_limit)
                || !event
                    .value
                    .get("level")
                    .and_then(Value::as_str)
                    .is_some_and(|level| selected_levels.contains(level))
            {
                continue;
            }
            if selected.len() >= entry_limit
                || selected_bytes.saturating_add(event.byte_size) > byte_limit
            {
                break;
            }
            selected.push(event.value.clone());
            selected_bytes = selected_bytes.saturating_add(event.byte_size);
        }
        selected.reverse();
        let mut summary = console_capture_summary(capture, true, None);
        if let Value::Object(object) = &mut summary {
            object.insert("events".into(), Value::Array(selected));
            object.insert(
                "returned_entries".into(),
                Value::from(
                    object
                        .get("events")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                ),
            );
            object.insert("returned_bytes".into(), Value::from(selected_bytes));
        }
        Ok(summary)
    }

    pub fn clear_console_capture(&mut self, target: &BrowserTarget) -> Result<Value, BrokerError> {
        let capture = self.console_captures.get_mut(target).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "console capture is not active for the selected target",
            )
        })?;
        evict_console_events(capture, Instant::now());
        let removed_entries = capture.events.len();
        let removed_bytes = capture.retained_bytes;
        capture.events.clear();
        capture.retained_bytes = 0;
        let mut summary = console_capture_summary(capture, true, None);
        if let Value::Object(object) = &mut summary {
            object.insert("removed_entries".into(), Value::from(removed_entries));
            object.insert("removed_bytes".into(), Value::from(removed_bytes));
        }
        Ok(summary)
    }

    pub fn stop_console_capture(
        &mut self,
        target: &BrowserTarget,
        reason: &str,
        detail: Option<&str>,
    ) -> Value {
        let Some(capture) = self.console_captures.remove(target) else {
            return json!({
                "target": target,
                "active": false,
                "capture_id": Value::Null,
                "removed_entries": 0,
                "removed_bytes": 0,
                "termination": {"reason": "already_stopped"},
            });
        };
        let removed_entries = capture.events.len();
        let removed_bytes = capture.retained_bytes;
        let termination = self.record_console_termination(&capture, reason, detail);
        json!({
            "target": capture.target,
            "active": false,
            "capture_id": capture.capture_id,
            "removed_entries": removed_entries,
            "removed_bytes": removed_bytes,
            "diagnostics": console_diagnostics(&capture),
            "termination": termination,
        })
    }

    pub fn terminate_console_target(
        &mut self,
        target: &BrowserTarget,
        reason: &str,
        detail: Option<&str>,
    ) -> bool {
        let Some(capture) = self.console_captures.remove(target) else {
            return false;
        };
        self.record_console_termination(&capture, reason, detail);
        true
    }

    pub fn terminate_console_session(
        &mut self,
        extension_instance_id: &str,
        reason: &str,
        detail: Option<&str>,
    ) -> usize {
        let targets = self
            .console_captures
            .keys()
            .filter(|target| target.extension_instance_id == extension_instance_id)
            .cloned()
            .collect::<Vec<_>>();
        let count = targets.len();
        for target in targets {
            self.terminate_console_target(&target, reason, detail);
        }
        count
    }

    pub fn console_capture_instance_ids(&self) -> Vec<String> {
        self.console_captures
            .keys()
            .map(|target| target.extension_instance_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn console_capture_handles_for_session(
        &self,
        extension_instance_id: &str,
    ) -> Vec<(BrowserTarget, String)> {
        self.console_captures
            .values()
            .filter(|capture| capture.target.extension_instance_id == extension_instance_id)
            .map(|capture| (capture.target.clone(), capture.capture_id.clone()))
            .collect()
    }

    /// Reserve a broker-owned Network capture ID.  The active capture is not
    /// replaced until the extension confirms the same ID, so a failed start
    /// cannot tear down a working capture.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_network_capture(
        &mut self,
        request_id: &str,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        target: &BrowserTarget,
        stream_generation: Option<u64>,
        allowed_hostnames: Option<&Value>,
        capture_request_bodies: Option<&Value>,
        max_request_body_bytes: Option<&Value>,
        max_age_ms: Option<&Value>,
        max_entries: Option<&Value>,
        max_bytes: Option<&Value>,
        max_body_bytes: Option<&Value>,
        sensitive_fields: Option<&Value>,
    ) -> Result<NetworkCaptureHandle, BrokerError> {
        validate_network_scope(
            request_id,
            broker_start_id,
            project_root,
            caller_label,
            target,
        )?;
        if self.pending_network_captures.contains_key(request_id) {
            return Err(BrokerError::new(
                BrokerErrorCode::DuplicateBrowserMutation,
                "network capture request_id is already reserved",
            ));
        }
        if self.pending_network_captures.len() >= MAX_PREPARED_ARTIFACTS {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker has too many pending network captures",
            ));
        }
        if !self.network_captures.contains_key(target)
            && self
                .network_captures
                .len()
                .saturating_add(self.pending_network_captures.len())
                >= MAX_ACTIVE_CONSOLE_CAPTURES
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker has too many active network captures",
            ));
        }
        let config = NetworkCaptureConfig::from_values(
            allowed_hostnames,
            capture_request_bodies,
            max_request_body_bytes,
            max_age_ms,
            max_entries,
            max_bytes,
            max_body_bytes,
            sensitive_fields,
        )?;
        let capture_id = format!("network_{}", Uuid::new_v4().simple());
        self.pending_network_captures.insert(
            request_id.to_owned(),
            PendingNetworkCapture {
                request_id: request_id.to_owned(),
                capture_id: capture_id.clone(),
                target: target.clone(),
                project_root: project_root.to_owned(),
                caller_label: caller_label.to_owned(),
                broker_start_id: broker_start_id.to_owned(),
                stream_generation,
                config,
            },
        );
        Ok(NetworkCaptureHandle { capture_id })
    }

    /// Commit a prepared Network capture only after the extension echoes the
    /// broker-owned capture ID and confirms activation.
    pub fn commit_network_capture(
        &mut self,
        request_id: &str,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        target: &BrowserTarget,
        response: &ExtensionResponse,
    ) -> Result<Value, BrokerError> {
        let Some(pending) = self.pending_network_captures.remove(request_id) else {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "network capture response has no prepared request",
            ));
        };
        if pending.request_id != request_id
            || pending.broker_start_id != broker_start_id
            || pending.project_root != project_root
            || pending.caller_label != caller_label
            || pending.target != *target
            || !response.ok
        {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "network capture response does not match its prepared request",
            ));
        }
        let returned_capture_id = response
            .result
            .get("capture_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if returned_capture_id != Some(pending.capture_id.as_str()) {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityUnavailable,
                "extension must echo capture_id for target-scoped network capture",
            ));
        }
        if response.result.get("active").and_then(Value::as_bool) == Some(false) {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserOperationFailed,
                "extension did not activate network capture",
            ));
        }
        let record = NetworkCaptureRecord {
            capture_id: pending.capture_id,
            target: pending.target,
            project_root: pending.project_root,
            caller_label: pending.caller_label,
            broker_start_id: pending.broker_start_id,
            stream_generation: pending.stream_generation,
            config: pending.config,
            requests: HashMap::new(),
            request_order: VecDeque::new(),
            retained_bytes: 0,
            acknowledged_sequence: 0,
            highest_seen_sequence: 0,
            pending_events: HashMap::new(),
            pending_bytes: 0,
            clear_sequence: 0,
            dropped_events: 0,
            dropped_batches: 0,
            dropped_bytes: 0,
            rejected_events: 0,
            filtered_events: 0,
            duplicate_events: 0,
            gap_events: 0,
            loss_diagnostics: VecDeque::new(),
            termination_reason: None,
            terminated_at_ms: None,
        };
        if let Some(previous) = self.network_captures.remove(target) {
            // A replacement confirms a new capture identity, but it does not
            // establish that unacknowledged old events were processed. Keep
            // the old cumulative ACK as the late-retransmit barrier.
            let barrier = previous.acknowledged_sequence;
            self.insert_network_barrier(&previous, barrier);
            self.record_network_termination(&previous, "capture_replaced", barrier, None);
        }
        let summary = network_capture_summary(&record, true, None);
        self.network_captures.insert(target.clone(), record);
        Ok(summary)
    }

    pub fn abort_network_capture(&mut self, request_id: &str) {
        self.pending_network_captures.remove(request_id);
    }

    pub fn network_capture_id(&self, target: &BrowserTarget) -> Option<String> {
        self.network_captures
            .get(target)
            .map(|capture| capture.capture_id.clone())
    }

    pub fn network_capture_scope_matches(
        &self,
        target: &BrowserTarget,
        capture_id: Option<&str>,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
    ) -> bool {
        self.network_captures.get(target).is_some_and(|capture| {
            capture.capture_id == capture_id.unwrap_or_default()
                && capture.broker_start_id == broker_start_id
                && capture.project_root == project_root
                && capture.caller_label == caller_label
        })
    }

    /// Move active captures to the current extension stream generation after
    /// reconnect.  A batch from the previous stream remains rejectable.
    pub fn update_network_stream_generation(
        &mut self,
        extension_instance_id: &str,
        generation: u64,
    ) {
        for capture in self.network_captures.values_mut() {
            if capture.target.extension_instance_id == extension_instance_id {
                capture.stream_generation = Some(generation);
            }
        }
        for capture in self.pending_network_captures.values_mut() {
            if capture.target.extension_instance_id == extension_instance_id {
                capture.stream_generation = Some(generation);
            }
        }
    }

    /// Accept one transport-authenticated batch and return the extension ACK.
    /// Only contiguous sequences that have reached a deterministic outcome
    /// advance the acknowledgement barrier.
    pub fn accept_network_batch(
        &mut self,
        extension_instance_id: &str,
        generation: u64,
        batch: &NetworkBatch,
    ) -> Value {
        let target = &batch.target;
        let capture_id = batch.capture_id.as_str();
        if target.extension_instance_id != extension_instance_id {
            return network_ack(capture_id, target, 0, false, Some("target_mismatch"));
        }
        if self
            .network_captures
            .get(target)
            .is_some_and(|capture| capture.capture_id != capture_id)
        {
            if let Some(barrier) = self
                .network_barriers
                .get(&network_capture_identity(target, capture_id))
            {
                return network_ack(
                    capture_id,
                    target,
                    barrier.acknowledged_sequence,
                    true,
                    None,
                );
            }
            return network_ack(capture_id, target, 0, false, Some("capture_mismatch"));
        }
        let Some(capture) = self.network_captures.get_mut(target) else {
            if let Some(barrier) = self
                .network_barriers
                .get(&network_capture_identity(target, capture_id))
            {
                return network_ack(
                    capture_id,
                    target,
                    barrier.acknowledged_sequence,
                    true,
                    None,
                );
            }
            return network_ack(capture_id, target, 0, false, Some("capture_mismatch"));
        };
        if capture.capture_id != capture_id {
            return network_ack(
                capture_id,
                target,
                capture.acknowledged_sequence,
                false,
                Some("capture_mismatch"),
            );
        }
        if capture.stream_generation != Some(generation) {
            return network_ack(
                capture_id,
                target,
                capture.acknowledged_sequence,
                false,
                Some("stream_generation_mismatch"),
            );
        }

        capture.dropped_events = capture
            .dropped_events
            .max(batch.dropped_events.max(batch.dropped_events_total));
        capture.dropped_bytes = capture
            .dropped_bytes
            .max(batch.dropped_bytes.max(batch.dropped_bytes_total));
        if let Some(diagnostics) = batch.diagnostics.as_ref().and_then(Value::as_object)
            && let Some(value) = diagnostics.get("dropped_batches").and_then(Value::as_u64)
        {
            capture.dropped_batches = capture.dropped_batches.max(value);
        }

        let already_terminated = capture.termination_reason.is_some();
        for event in &batch.events {
            capture.highest_seen_sequence = capture.highest_seen_sequence.max(event.seq);
            if event.seq <= capture.acknowledged_sequence
                || event.seq <= capture.clear_sequence
                || capture.pending_events.contains_key(&event.seq)
            {
                capture.duplicate_events = capture.duplicate_events.saturating_add(1);
                continue;
            }
            if already_terminated {
                capture.rejected_events = capture.rejected_events.saturating_add(1);
                continue;
            }
            if event.seq
                > capture
                    .acknowledged_sequence
                    .saturating_add(MAX_NETWORK_PENDING_EVENTS as u64)
            {
                capture.rejected_events = capture.rejected_events.saturating_add(1);
                continue;
            }
            let event_value = network_event_value(event);
            let event_bytes =
                serde_json::to_vec(&event_value).map_or(usize::MAX, |bytes| bytes.len());
            if event_bytes > MAX_NETWORK_EVENT_BYTES
                || capture.pending_bytes.saturating_add(event_bytes) > MAX_NETWORK_PENDING_BYTES
            {
                capture.rejected_events = capture.rejected_events.saturating_add(1);
                continue;
            }
            capture.pending_bytes = capture.pending_bytes.saturating_add(event_bytes);
            capture.pending_events.insert(event.seq, event_value);
        }

        while let Some(event) = capture
            .pending_events
            .remove(&capture.acknowledged_sequence.saturating_add(1))
        {
            let event_bytes = serde_json::to_vec(&event).map_or(0, |bytes| bytes.len());
            capture.pending_bytes = capture.pending_bytes.saturating_sub(event_bytes);
            capture.acknowledged_sequence = capture.acknowledged_sequence.saturating_add(1);
            match merge_network_event(capture, event) {
                NetworkMergeResult::Accepted => {}
                NetworkMergeResult::Filtered => {
                    capture.filtered_events = capture.filtered_events.saturating_add(1)
                }
                NetworkMergeResult::Rejected => {
                    capture.rejected_events = capture.rejected_events.saturating_add(1)
                }
            }
        }
        if let Some(&next) = capture.pending_events.keys().min()
            && next > capture.acknowledged_sequence.saturating_add(1)
        {
            record_network_gap(
                capture,
                capture.acknowledged_sequence.saturating_add(1),
                next - 1,
            );
        }

        if !already_terminated && let Some(reason) = batch.termination_reason.as_deref() {
            let barrier = batch
                .final_sequence
                .or(batch.last_seq)
                .unwrap_or(capture.acknowledged_sequence);
            advance_network_barrier(capture, barrier, "termination");
            capture.termination_reason = Some(truncate_utf8(reason, 256));
            capture.terminated_at_ms = Some(unix_ms());
            let summary = network_capture_summary(
                capture,
                false,
                Some(json!({
                    "reason": capture.termination_reason,
                    "detail": batch.termination_detail.as_deref().map(|detail| truncate_utf8(detail, 256)),
                    "at_ms": capture.terminated_at_ms,
                })),
            );
            self.terminated_network_captures.push_back(summary);
            while self.terminated_network_captures.len() > MAX_NETWORK_DIAGNOSTICS {
                self.terminated_network_captures.pop_front();
            }
        }

        network_ack(
            capture_id,
            target,
            capture.acknowledged_sequence,
            true,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_network_requests(
        &mut self,
        target: &BrowserTarget,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        max_age_ms: Option<&Value>,
        max_entries: Option<&Value>,
        max_bytes: Option<&Value>,
    ) -> Result<Value, BrokerError> {
        self.require_network_scope(target, broker_start_id, project_root, caller_label)?;
        let capture = self.network_captures.get_mut(target).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network capture is not active for the selected target",
            )
        })?;
        evict_network_requests(capture, Instant::now());
        let age_limit = bounded_u64(
            max_age_ms,
            capture.config.max_age_ms,
            0,
            capture.config.max_age_ms,
            "max_age_ms",
        )?;
        let entry_limit = bounded_usize(
            max_entries,
            capture.config.max_entries,
            1,
            capture.config.max_entries,
            "max_entries",
        )?;
        let byte_limit = bounded_usize(
            max_bytes,
            capture.config.max_bytes,
            1,
            capture.config.max_bytes,
            "max_bytes",
        )?;
        let now = Instant::now();
        let mut selected = Vec::new();
        let mut selected_bytes = 0usize;
        for request_id in capture.request_order.iter().rev() {
            let Some(record) = capture.requests.get(request_id) else {
                continue;
            };
            if now
                .saturating_duration_since(record.received_at)
                .as_millis()
                > u128::from(age_limit)
            {
                continue;
            }
            if selected.len() >= entry_limit
                || selected_bytes.saturating_add(record.byte_size) > byte_limit
            {
                break;
            }
            selected.push(network_request_summary(&record.value));
            selected_bytes = selected_bytes.saturating_add(record.byte_size);
        }
        selected.reverse();
        let mut summary =
            network_capture_summary(capture, capture.termination_reason.is_none(), None);
        if let Value::Object(object) = &mut summary {
            object.insert("requests".into(), Value::Array(selected));
            object.insert(
                "returned_entries".into(),
                Value::from(object["requests"].as_array().map_or(0, Vec::len)),
            );
            object.insert("returned_bytes".into(), Value::from(selected_bytes));
        }
        Ok(summary)
    }

    pub fn get_network_request_detail(
        &mut self,
        target: &BrowserTarget,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        request_id: &Value,
        include_body: bool,
    ) -> Result<Value, BrokerError> {
        self.require_network_scope(target, broker_start_id, project_root, caller_label)?;
        if include_body {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityUnavailable,
                "raw Network body access requires an explicit dispatched request",
            ));
        }
        let capture = self.network_captures.get_mut(target).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network capture is not active for the selected target",
            )
        })?;
        evict_network_requests(capture, Instant::now());
        let normalized = truncate_utf8(&value_text(Some(request_id)), MAX_NETWORK_REQUEST_ID_BYTES);
        let Some(record) = capture.requests.get(&normalized) else {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserTargetNotFound,
                "captured network request was not found or has expired",
            ));
        };
        Ok(json!({
            "capture_id": capture.capture_id,
            "target": capture.target,
            "active": capture.termination_reason.is_none(),
            "request": record.value,
        }))
    }

    /// Validate an explicit response-body request before dispatching to the
    /// extension.  The caller's lease and project scope are checked by the
    /// broker before reaching this store; this second check keeps the body
    /// access bound to the still-active capture and retained request.
    pub fn prepare_network_body_access(
        &mut self,
        target: &BrowserTarget,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        request_id: &Value,
        max_body_bytes: Option<&Value>,
    ) -> Result<NetworkBodyAccess, BrokerError> {
        self.require_network_scope(target, broker_start_id, project_root, caller_label)?;
        let capture = self.network_captures.get_mut(target).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network capture is not active for the selected target",
            )
        })?;
        evict_network_requests(capture, Instant::now());
        let normalized = truncate_utf8(&value_text(Some(request_id)), MAX_NETWORK_REQUEST_ID_BYTES);
        if !capture.requests.contains_key(&normalized) {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserTargetNotFound,
                "captured network request was not found or has expired",
            ));
        }
        let max_body_bytes = bounded_usize(
            max_body_bytes,
            capture.config.max_body_bytes,
            1,
            capture.config.max_body_bytes,
            "max_body_bytes",
        )?;
        Ok(NetworkBodyAccess {
            capture_id: capture.capture_id.clone(),
            request_id: normalized,
            max_body_bytes,
        })
    }

    /// Bound the response body returned by the extension and combine it with
    /// the already-retained, metadata-scoped request detail.  The raw body is
    /// never placed in capture summaries, diagnostics, or the retained event
    /// store.
    #[allow(clippy::too_many_arguments)]
    pub fn bound_network_body(
        &mut self,
        target: &BrowserTarget,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        access: &NetworkBodyAccess,
        body: Option<&Value>,
        base64_encoded: bool,
    ) -> Result<Value, BrokerError> {
        self.require_network_scope(target, broker_start_id, project_root, caller_label)?;
        let capture = self.network_captures.get_mut(target).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network capture is not active for the selected target",
            )
        })?;
        evict_network_requests(capture, Instant::now());
        if capture.capture_id != access.capture_id || capture.termination_reason.is_some() {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "network response-body access no longer matches the active capture",
            ));
        }
        let Some(record) = capture.requests.get(&access.request_id) else {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserTargetNotFound,
                "captured network request was not found or has expired",
            ));
        };
        let encoded_body = value_text(body);
        let (output, original_size, returned_size, truncated) = if base64_encoded {
            let raw = base64::engine::general_purpose::STANDARD
                .decode(encoded_body.as_bytes())
                .map_err(|_| {
                    BrokerError::new(
                        BrokerErrorCode::BrowserOperationFailed,
                        "browser returned an invalid base64 Network body",
                    )
                })?;
            let bounded = &raw[..raw.len().min(access.max_body_bytes)];
            (
                base64::engine::general_purpose::STANDARD.encode(bounded),
                raw.len(),
                bounded.len(),
                raw.len() > bounded.len(),
            )
        } else {
            let raw = encoded_body.as_bytes();
            let output = truncate_utf8(&encoded_body, access.max_body_bytes);
            (
                output.clone(),
                raw.len(),
                output.len(),
                raw.len() > output.len(),
            )
        };
        Ok(json!({
            "capture_id": capture.capture_id,
            "target": capture.target,
            "active": capture.termination_reason.is_none(),
            "request": record.value,
            "body": output,
            "base64_encoded": base64_encoded,
            "truncated": truncated,
            "original_size": original_size,
            "returned_size": returned_size,
        }))
    }

    pub fn clear_network_capture(
        &mut self,
        target: &BrowserTarget,
        capture_id: &str,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        sequence_barrier: u64,
    ) -> Result<Value, BrokerError> {
        self.require_network_scope(target, broker_start_id, project_root, caller_label)?;
        let capture = self.network_captures.get_mut(target).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network capture is not active for the selected target",
            )
        })?;
        if capture.capture_id != capture_id || capture.termination_reason.is_some() {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "network clear response does not match the active capture",
            ));
        }
        let removed_entries = capture.requests.len();
        let removed_bytes = capture.retained_bytes;
        capture.requests.clear();
        capture.request_order.clear();
        capture.retained_bytes = 0;
        advance_network_barrier(capture, sequence_barrier, "clear");
        let mut summary = network_capture_summary(capture, true, None);
        if let Value::Object(object) = &mut summary {
            object.insert("removed_entries".into(), Value::from(removed_entries));
            object.insert("removed_bytes".into(), Value::from(removed_bytes));
            object.insert("sequence_barrier".into(), Value::from(sequence_barrier));
        }
        Ok(summary)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stop_network_capture(
        &mut self,
        target: &BrowserTarget,
        capture_id: &str,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        sequence_barrier: u64,
        termination_reason: &str,
    ) -> Result<Value, BrokerError> {
        self.require_network_scope(target, broker_start_id, project_root, caller_label)?;
        let Some(mut capture) = self.network_captures.remove(target) else {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network capture is not active for the selected target",
            ));
        };
        if capture.capture_id != capture_id {
            self.network_captures.insert(target.clone(), capture);
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "network stop response does not match the active capture",
            ));
        }
        let removed_entries = capture.requests.len();
        let removed_bytes = capture.retained_bytes;
        advance_network_barrier(&mut capture, sequence_barrier, "stop");
        capture.termination_reason = Some(truncate_utf8(termination_reason, 256));
        capture.terminated_at_ms = Some(unix_ms());
        let barrier = NetworkBarrier {
            target: capture.target.clone(),
            capture_id: capture.capture_id.clone(),
            acknowledged_sequence: capture.acknowledged_sequence,
        };
        self.network_barriers.insert(
            network_capture_identity(&barrier.target, &barrier.capture_id),
            barrier,
        );
        self.trim_network_barriers();
        let summary = network_capture_summary(
            &capture,
            false,
            Some(json!({
                "reason": capture.termination_reason,
                "at_ms": capture.terminated_at_ms,
            })),
        );
        self.terminated_network_captures.push_back(summary.clone());
        while self.terminated_network_captures.len() > MAX_NETWORK_DIAGNOSTICS {
            self.terminated_network_captures.pop_front();
        }
        let mut response = summary;
        if let Value::Object(object) = &mut response {
            object.insert("removed_entries".into(), Value::from(removed_entries));
            object.insert("removed_bytes".into(), Value::from(removed_bytes));
        }
        Ok(response)
    }

    pub fn terminate_network_target(
        &mut self,
        target: &BrowserTarget,
        reason: &str,
        detail: Option<&str>,
    ) -> bool {
        let Some(mut capture) = self.network_captures.remove(target) else {
            return false;
        };
        let barrier = capture.acknowledged_sequence;
        advance_network_barrier(&mut capture, barrier, "termination");
        self.insert_network_barrier(&capture, barrier);
        self.record_network_termination(&capture, reason, barrier, detail);
        true
    }

    pub fn terminate_network_session(
        &mut self,
        extension_instance_id: &str,
        reason: &str,
        detail: Option<&str>,
    ) -> usize {
        let targets = self
            .network_captures
            .keys()
            .filter(|target| target.extension_instance_id == extension_instance_id)
            .cloned()
            .collect::<Vec<_>>();
        let count = targets.len();
        for target in targets {
            self.terminate_network_target(&target, reason, detail);
        }
        count
    }

    pub fn network_capture_instance_ids(&self) -> Vec<String> {
        self.network_captures
            .keys()
            .map(|target| target.extension_instance_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn network_capture_handles_for_session(
        &self,
        extension_instance_id: &str,
    ) -> Vec<(BrowserTarget, String)> {
        self.network_captures
            .values()
            .filter(|capture| capture.target.extension_instance_id == extension_instance_id)
            .map(|capture| (capture.target.clone(), capture.capture_id.clone()))
            .collect()
    }

    pub fn latest_network_termination(&self) -> Option<Value> {
        self.terminated_network_captures.back().cloned()
    }

    fn require_network_scope(
        &self,
        target: &BrowserTarget,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
    ) -> Result<(), BrokerError> {
        let Some(capture) = self.network_captures.get(target) else {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network capture is not active for the selected target",
            ));
        };
        if capture.broker_start_id != broker_start_id
            || capture.project_root != project_root
            || capture.caller_label != caller_label
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserLease,
                "network capture scope does not match this project and caller",
            ));
        }
        Ok(())
    }

    fn insert_network_barrier(
        &mut self,
        capture: &NetworkCaptureRecord,
        acknowledged_sequence: u64,
    ) {
        let barrier = NetworkBarrier {
            target: capture.target.clone(),
            capture_id: capture.capture_id.clone(),
            acknowledged_sequence,
        };
        self.network_barriers.insert(
            network_capture_identity(&barrier.target, &barrier.capture_id),
            barrier,
        );
        self.trim_network_barriers();
    }

    fn trim_network_barriers(&mut self) {
        while self.network_barriers.len() > MAX_NETWORK_BARRIER_RECORDS {
            let Some(oldest) = self.network_barriers.keys().next().cloned() else {
                break;
            };
            self.network_barriers.remove(&oldest);
        }
    }

    fn record_network_termination(
        &mut self,
        capture: &NetworkCaptureRecord,
        reason: &str,
        barrier: u64,
        detail: Option<&str>,
    ) {
        let mut summary = network_capture_summary(
            capture,
            false,
            Some(json!({
                "reason": truncate_utf8(reason, 256),
                "detail": detail.map(|value| truncate_utf8(&redact_urls_in_text(value, &capture.config.sensitive_fields), 256)),
                "barrier": barrier,
                "at_ms": unix_ms(),
            })),
        );
        if let Value::Object(object) = &mut summary {
            object.insert("retained_entries".into(), Value::from(0));
            object.insert("retained_bytes".into(), Value::from(0));
        }
        self.terminated_network_captures.push_back(summary);
        while self.terminated_network_captures.len() > MAX_NETWORK_DIAGNOSTICS {
            self.terminated_network_captures.pop_front();
        }
    }

    pub fn latest_console_termination(&self) -> Option<Value> {
        self.terminated_console_captures.back().cloned()
    }

    fn record_console_termination(
        &mut self,
        capture: &ConsoleCaptureRecord,
        reason: &str,
        detail: Option<&str>,
    ) -> Value {
        let termination = termination_value_with_fields(reason, detail, &capture.sensitive_fields);
        let mut summary = console_capture_summary(capture, false, Some(termination.clone()));
        if let Value::Object(object) = &mut summary {
            object.insert("retained_entries".into(), Value::from(0));
            object.insert("retained_bytes".into(), Value::from(0));
        }
        self.terminated_console_captures.push_back(summary);
        while self.terminated_console_captures.len() > MAX_CONSOLE_CAPTURE_DIAGNOSTICS {
            self.terminated_console_captures.pop_front();
        }
        termination
    }

    pub fn record_console_termination_event(
        &mut self,
        extension_instance_id: &str,
        target: &BrowserTarget,
        capture_id: Option<&str>,
        stream_generation: Option<u64>,
        reason: &str,
        detail: Option<&str>,
    ) -> bool {
        if target.extension_instance_id != extension_instance_id {
            return false;
        }
        let Some(capture) = self.console_captures.get(target) else {
            return false;
        };
        // A teardown notification can arrive over the authenticated HTTP
        // path after the extension WebSocket has already closed, so it may
        // legitimately carry no transport generation.  The capture ID and
        // complete target remain mandatory; ordinary Console events below
        // continue to require an exact generation.
        if capture.capture_id != capture_id.unwrap_or_default()
            || stream_generation
                .is_some_and(|generation| capture.stream_generation != Some(generation))
        {
            return false;
        }
        self.terminate_console_target(target, reason, detail)
    }

    #[cfg(test)]
    fn active_console_capture_count(&self) -> usize {
        self.console_captures.len()
    }

    #[cfg(test)]
    fn terminated_console_capture_count(&self) -> usize {
        self.terminated_console_captures.len()
    }

    /// Reserve a broker-generated artifact name after the request's target and
    /// lease have already been validated by `BrokerState`.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_request(
        &mut self,
        operation: &str,
        request_id: &str,
        broker_start_id: &str,
        project_root: &str,
        caller_label: &str,
        target: &BrowserTarget,
        expected_page_context_revision: Option<&str>,
        requested_format: Option<&str>,
    ) -> Result<(), BrokerError> {
        let kind = match operation {
            "capture_browser_evidence" => ArtifactKind::EvidenceJpeg,
            "capture_browser_screenshot" => ArtifactKind::Screenshot,
            "generate_browser_pdf" => ArtifactKind::Pdf,
            _ => return Ok(()),
        };
        if request_id.trim().is_empty() || request_id.len() > 256 {
            return Err(artifact_error("evidence request_id is invalid"));
        }
        if broker_start_id.trim().is_empty() || broker_start_id.len() > 256 {
            return Err(artifact_error("evidence broker generation is invalid"));
        }
        if caller_label.trim().is_empty() || caller_label.len() > 256 {
            return Err(artifact_error("evidence caller identity is invalid"));
        }
        if target.extension_instance_id.trim().is_empty()
            || target.window_id <= 0
            || target.tab_id <= 0
        {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "evidence target is invalid",
            ));
        }
        if self.prepared.contains_key(request_id)
            || self
                .managed
                .values()
                .any(|record| record.artifact.request_id == request_id)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::DuplicateBrowserMutation,
                "evidence request_id is already reserved",
            ));
        }
        if self.prepared.len() >= MAX_PREPARED_ARTIFACTS {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker has too many prepared browser artifacts",
            ));
        }

        let root = canonical_project_root(project_root)?;
        let artifact_root = inspect_artifact_root(&root, false)?.unwrap_or_else(|| {
            root.join(MANAGED_ARTIFACT_COMPONENTS[0])
                .join(MANAGED_ARTIFACT_COMPONENTS[1])
                .join(MANAGED_ARTIFACT_COMPONENTS[2])
        });
        let format = normalize_format(kind, requested_format)?;
        let filename = generated_filename(request_id, target, &format);
        let final_path = artifact_root.join(&filename);
        if final_path.exists() {
            return Err(artifact_error("generated browser artifact already exists"));
        }
        let expected_page_context_revision = expected_page_context_revision
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if expected_page_context_revision.is_some_and(|value| value.len() > 256) {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "page context revision exceeds the configured bound",
            ));
        }
        let expected_page_context_revision = expected_page_context_revision.map(str::to_owned);
        let media_type = match format.as_str() {
            "png" => "image/png",
            "jpeg" => "image/jpeg",
            "pdf" => "application/pdf",
            _ => return Err(artifact_error("unsupported browser artifact format")),
        }
        .to_owned();
        self.prepared.insert(
            request_id.to_owned(),
            PreparedRecord {
                kind,
                broker_start_id: broker_start_id.to_owned(),
                project_root: root,
                caller_label: caller_label.to_owned(),
                target: target.clone(),
                request_id: request_id.to_owned(),
                expected_page_context_revision,
                format,
                media_type,
                artifact_root,
                final_path,
                relative_path: filename,
            },
        );
        Ok(())
    }

    /// Validate and publish one matching extension response.  The pending
    /// record is consumed on both success and failure, so a duplicate response
    /// cannot publish a second file.
    #[allow(clippy::too_many_arguments)]
    pub fn commit_response(
        &mut self,
        request_id: &str,
        broker_start_id: &str,
        operation: &str,
        project_root: &str,
        caller_label: &str,
        target: &BrowserTarget,
        response: &ExtensionResponse,
    ) -> Result<PreparedArtifact, BrokerError> {
        let Some(prepared) = self.prepared.remove(request_id) else {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "evidence response has no prepared request",
            ));
        };
        self.commit_prepared(
            &prepared,
            broker_start_id,
            operation,
            project_root,
            caller_label,
            target,
            response,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_prepared(
        &mut self,
        prepared: &PreparedRecord,
        broker_start_id: &str,
        operation: &str,
        project_root: &str,
        caller_label: &str,
        target: &BrowserTarget,
        response: &ExtensionResponse,
    ) -> Result<PreparedArtifact, BrokerError> {
        if prepared.broker_start_id != broker_start_id
            || prepared.kind.operation() != operation
            || prepared.target != *target
            || response.request_id != prepared.request_id
            || response.target.as_ref() != Some(target)
            || prepared.caller_label != caller_label
        {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "evidence response does not match its prepared request",
            ));
        }
        let canonical_root = canonical_project_root(project_root)?;
        if canonical_root != prepared.project_root {
            return Err(artifact_error(
                "evidence project scope does not match the request",
            ));
        }
        if !response.ok {
            return Err(artifact_error("browser did not return successful evidence"));
        }

        let actual_revision = response
            .result
            .get("page_context_revision")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| artifact_error("browser evidence omitted page context revision"))?;
        if actual_revision.len() > 256 {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "page context revision exceeds the configured bound",
            ));
        }
        if prepared
            .expected_page_context_revision
            .as_deref()
            .is_some_and(|expected| expected != actual_revision)
        {
            let mut error = BrokerError::new(
                BrokerErrorCode::StaleBrowserTarget,
                "page changed before browser evidence was persisted",
            );
            error.recovery.insert(
                "page_context_revision".into(),
                Value::String(actual_revision.into()),
            );
            return Err(error);
        }

        if prepared.kind == ArtifactKind::Screenshot {
            let returned_format = response
                .result
                .get("format")
                .and_then(Value::as_str)
                .ok_or_else(|| artifact_error("browser screenshot omitted its format"))?;
            if normalize_format(prepared.kind, Some(returned_format))? != prepared.format {
                return Err(artifact_error(
                    "browser screenshot format does not match the request",
                ));
            }
        } else if prepared.kind == ArtifactKind::Pdf {
            let returned_format = response
                .result
                .get("format")
                .and_then(Value::as_str)
                .ok_or_else(|| artifact_error("browser PDF omitted its format"))?;
            if normalize_format(prepared.kind, Some(returned_format))? != prepared.format {
                return Err(artifact_error(
                    "browser PDF format does not match the request",
                ));
            }
        }

        let encoded = response
            .result
            .get(prepared.kind.response_data_key())
            .and_then(Value::as_str)
            .ok_or_else(|| artifact_error("browser evidence payload is missing"))?;
        let payload = decode_bounded_base64(encoded)?;
        let dimensions = validate_payload(&prepared.format, &payload)?;
        let artifact = PreparedArtifact {
            path: prepared.relative_path.clone(),
            size: payload.len() as u64,
            format: prepared.format.clone(),
            media_type: prepared.media_type.clone(),
            target: prepared.target.clone(),
            request_id: prepared.request_id.clone(),
            page_context_revision: actual_revision.to_owned(),
            dimensions,
            managed: true,
        };
        publish_no_clobber(prepared, &payload)?;
        let managed_path = fs::canonicalize(&prepared.final_path)
            .map_err(|_| artifact_error("published browser artifact path could not be verified"))?;
        self.managed.insert(
            managed_path.clone(),
            ManagedRecord {
                artifact: artifact.clone(),
                project_root: prepared.project_root.clone(),
                caller_label: prepared.caller_label.clone(),
                final_path: managed_path,
                content_digest: digest_payload(&payload),
            },
        );
        Ok(artifact)
    }

    /// Cancel, timeout, disconnect, or otherwise abandon one prepared request.
    pub fn abort_request(&mut self, request_id: &str) {
        self.prepared.remove(request_id);
    }

    /// Remove only artifacts that this store published for the same canonical
    /// project and caller.  Caller paths are broker-relative filenames; any
    /// absolute, parent, separator-containing, symlink, or unknown name fails
    /// closed without touching the filesystem.
    pub fn cleanup_managed(
        &mut self,
        project_root: &str,
        caller_label: &str,
        paths: &[String],
    ) -> Result<Value, BrokerError> {
        if paths.is_empty() || paths.len() > MAX_CLEANUP_ARTIFACTS {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "cleanup requires a bounded non-empty artifact list",
            ));
        }
        let root = canonical_project_root(project_root)?;
        let validated_paths = paths
            .iter()
            .map(|path| validate_relative_filename(path))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(artifact_root) = inspect_artifact_root(&root, false)? else {
            return Ok(json!({"removed": [], "missing": validated_paths}));
        };
        let mut removed = Vec::new();
        let mut missing = Vec::new();
        for relative in validated_paths {
            let final_path = artifact_root.join(&relative);
            let Some(record) = self.managed.get(&final_path).cloned() else {
                return Err(artifact_error("artifact is not managed by this broker"));
            };
            if record.project_root != root || record.caller_label != caller_label {
                return Err(artifact_error(
                    "artifact ownership does not match this request",
                ));
            }
            let metadata = match fs::symlink_metadata(&record.final_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.managed.remove(&record.final_path);
                    missing.push(relative);
                    continue;
                }
                Err(_) => {
                    return Err(artifact_error(
                        "managed artifact metadata could not be read",
                    ));
                }
            };
            if is_reparse_or_symlink(&metadata) || !metadata.is_file() {
                return Err(artifact_error("managed artifact is not a regular file"));
            }
            if metadata.len() != record.artifact.size {
                return Err(artifact_error("managed artifact changed before cleanup"));
            }
            let digest = digest_file(&record.final_path)?;
            if digest != record.content_digest {
                return Err(artifact_error("managed artifact changed before cleanup"));
            }
            let canonical = fs::canonicalize(&record.final_path)
                .map_err(|_| artifact_error("managed artifact path could not be verified"))?;
            ensure_within(&artifact_root, &canonical)?;
            fs::remove_file(&record.final_path)
                .map_err(|_| artifact_error("managed artifact could not be removed"))?;
            self.managed.remove(&record.final_path);
            removed.push(relative);
        }
        Ok(json!({"removed": removed, "missing": missing}))
    }

    #[cfg(test)]
    fn prepared_path(&self, request_id: &str) -> &Path {
        &self.prepared[request_id].final_path
    }

    #[cfg(test)]
    fn managed_count(&self) -> usize {
        self.managed.len()
    }
}

fn validate_console_scope(
    request_id: &str,
    broker_start_id: &str,
    project_root: &str,
    caller_label: &str,
    target: &BrowserTarget,
) -> Result<(), BrokerError> {
    if request_id.trim().is_empty() || request_id.len() > 256 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "console capture request_id must contain 1 to 256 bytes",
        ));
    }
    if broker_start_id.trim().is_empty() || broker_start_id.len() > 256 {
        return Err(BrokerError::new(
            BrokerErrorCode::MismatchedBrowserResponse,
            "console capture broker generation is invalid",
        ));
    }
    if project_root.trim().is_empty() || project_root.len() > 4096 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "console capture project scope is invalid",
        ));
    }
    if caller_label.trim().is_empty() || caller_label.len() > 120 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "console capture caller scope is invalid",
        ));
    }
    if target.extension_instance_id.trim().is_empty()
        || target.extension_instance_id.len() > 128
        || target.window_id <= 0
        || target.tab_id <= 0
    {
        return Err(BrokerError::new(
            BrokerErrorCode::MismatchedBrowserResponse,
            "console capture target is invalid",
        ));
    }
    Ok(())
}

fn validate_network_scope(
    request_id: &str,
    broker_start_id: &str,
    project_root: &str,
    caller_label: &str,
    target: &BrowserTarget,
) -> Result<(), BrokerError> {
    if request_id.trim().is_empty() || request_id.len() > 256 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "network capture request_id must contain 1 to 256 bytes",
        ));
    }
    if broker_start_id.trim().is_empty() || broker_start_id.len() > 256 {
        return Err(BrokerError::new(
            BrokerErrorCode::MismatchedBrowserResponse,
            "network capture broker generation is invalid",
        ));
    }
    if project_root.trim().is_empty() || project_root.len() > 4096 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "network capture project scope is invalid",
        ));
    }
    if caller_label.trim().is_empty() || caller_label.len() > 120 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "network capture caller scope is invalid",
        ));
    }
    if target.extension_instance_id.trim().is_empty()
        || target.extension_instance_id.len() > 128
        || target.window_id <= 0
        || target.tab_id <= 0
    {
        return Err(BrokerError::new(
            BrokerErrorCode::MismatchedBrowserResponse,
            "network capture target is invalid",
        ));
    }
    Ok(())
}

fn normalize_allowed_hostnames(value: Option<&Value>) -> Result<BTreeSet<String>, BrokerError> {
    let Some(value) = value else {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "start_network_capture requires at least one exact hostname",
        ));
    };
    let values = if let Some(value) = value.as_array() {
        value.clone()
    } else if value.is_string() {
        vec![value.clone()]
    } else {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "allowed_hostnames must be an array of exact hostnames",
        ));
    };
    if values.len() > MAX_NETWORK_HOSTNAMES {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "allowed_hostnames exceeds the capture limit",
        ));
    }
    let mut normalized = BTreeSet::new();
    for value in values {
        let Some(raw_value) = value.as_str() else {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "allowed_hostnames must contain strings",
            ));
        };
        let raw = raw_value.trim().to_ascii_lowercase();
        if raw.is_empty()
            || raw.len() > MAX_NETWORK_HOSTNAME_BYTES
            || raw.chars().any(char::is_whitespace)
            || ["://", "/", "\\", "@", "*", "?", "#"]
                .iter()
                .any(|marker| raw.contains(marker))
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "allowed_hostnames must contain exact hostnames without URL components or wildcards",
            ));
        }
        let parsed = Url::parse(&format!("http://{raw}")).map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "allowed_hostnames contains an invalid hostname",
            )
        })?;
        if parsed.port().is_some()
            || parsed.username() != ""
            || parsed.password().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "allowed_hostnames must contain exact hostnames without URL components",
            ));
        }
        let hostname = parsed
            .domain()
            .or_else(|| parsed.host_str())
            .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
            .filter(|host| !host.is_empty())
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::InvalidBrowserOperation,
                    "allowed_hostnames contains an invalid hostname",
                )
            })?;
        if hostname.len() > MAX_NETWORK_HOSTNAME_BYTES
            || hostname.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "allowed_hostnames contains an invalid exact hostname",
            ));
        }
        normalized.insert(hostname);
    }
    if normalized.is_empty() {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "start_network_capture requires at least one exact hostname",
        ));
    }
    Ok(normalized)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkMergeResult {
    Accepted,
    Filtered,
    Rejected,
}

fn bounded_request_body(value: Option<&Value>, limit: usize) -> Value {
    let Some(value) = value.and_then(Value::as_object) else {
        return json!({
            "encoding": "utf8",
            "body": "",
            "captured_size": 0,
            "original_size": Value::Null,
            "truncated": false,
            "unavailable_reason": "invalid_request_body",
        });
    };
    let mut encoding = value_text(value.get("encoding")).to_ascii_lowercase();
    if encoding.is_empty() {
        encoding = if value
            .get("base64_encoded")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "base64".into()
        } else {
            "utf8".into()
        };
    }
    let body_text = value
        .get("body")
        .or_else(|| value.get("data"))
        .map(|body| value_text(Some(body)))
        .unwrap_or_default();
    let mut unavailable_reason = value
        .get("unavailable_reason")
        .and_then(Value::as_str)
        .or_else(|| value.get("unavailable").and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned();
    unavailable_reason = truncate_utf8(&unavailable_reason, 256);
    let (encoding, output, raw_size, captured_size) = if encoding == "base64" {
        match base64::engine::general_purpose::STANDARD.decode(body_text.as_bytes()) {
            Ok(raw) => {
                let bounded = &raw[..raw.len().min(limit)];
                (
                    "base64",
                    base64::engine::general_purpose::STANDARD.encode(bounded),
                    raw.len(),
                    bounded.len(),
                )
            }
            Err(_) => {
                if unavailable_reason.is_empty() {
                    unavailable_reason = "invalid_base64".into();
                }
                ("base64", String::new(), 0, 0)
            }
        }
    } else {
        let raw = body_text.as_bytes();
        let output = truncate_utf8(&body_text, limit);
        let captured_size = output.len();
        ("utf8", output, raw.len(), captured_size)
    };
    let original_size = value.get("original_size").and_then(|size| {
        size.as_u64()
            .map(|size| usize::try_from(size).unwrap_or(usize::MAX))
            .or_else(|| {
                size.as_i64()
                    .map(|size| usize::try_from(size.max(0)).unwrap_or(usize::MAX))
            })
    });
    let truncated = value
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || raw_size > limit
        || original_size.is_some_and(|size| size > captured_size);
    json!({
        "encoding": encoding,
        "body": output,
        "captured_size": captured_size,
        "original_size": original_size,
        "truncated": truncated,
        "unavailable_reason": (!unavailable_reason.is_empty()).then_some(unavailable_reason),
    })
}

fn network_event_value(event: &crate::protocol::NetworkEvent) -> Value {
    let mut object = event
        .data
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<String, Value>>();
    if let Some(Value::Object(nested)) = object.remove("event") {
        Value::Object(nested)
    } else {
        Value::Object(object)
    }
}

fn merge_network_event(capture: &mut NetworkCaptureRecord, raw_event: Value) -> NetworkMergeResult {
    let Some(raw) = raw_event.as_object() else {
        return NetworkMergeResult::Rejected;
    };
    evict_network_requests(capture, Instant::now());
    let request_id = truncate_utf8(
        &value_text(raw.get("request_id")),
        MAX_NETWORK_REQUEST_ID_BYTES,
    );
    let event_type = value_text(raw.get("event_type")).to_ascii_lowercase();
    if request_id.is_empty()
        || !matches!(
            event_type.as_str(),
            "request" | "response" | "finished" | "failed"
        )
    {
        return NetworkMergeResult::Rejected;
    }
    let previous = capture.requests.get(&request_id).cloned();
    if event_type == "request" {
        let raw_url = truncate_utf8(&value_text(raw.get("url")), 8_192);
        if !url_matches_allowed_hostname(&raw_url, &capture.config.allowed_hostnames) {
            return NetworkMergeResult::Filtered;
        }
        if previous.as_ref().is_some_and(|record| {
            matches!(
                record.phase,
                NetworkRequestPhase::Finished | NetworkRequestPhase::Failed
            )
        }) {
            return NetworkMergeResult::Rejected;
        }
        let mut value = Map::new();
        value.insert("request_id".into(), Value::String(request_id.clone()));
        value.insert(
            "timestamp_ms".into(),
            Value::from(network_timestamp(raw.get("timestamp_ms"))),
        );
        value.insert(
            "url".into(),
            Value::String(redact_network_url(
                &raw_url,
                &capture.config.sensitive_fields,
            )),
        );
        value.insert(
            "method".into(),
            Value::String(truncate_utf8(&value_text(raw.get("method")), 32)),
        );
        value.insert(
            "resource_type".into(),
            Value::String(truncate_utf8(&value_text(raw.get("resource_type")), 64)),
        );
        if raw.get("headers").is_some() || raw.get("request_headers").is_some() {
            value.insert(
                "request_headers".into(),
                redact_network_mapping(
                    raw.get("headers").or_else(|| raw.get("request_headers")),
                    &capture.config.sensitive_fields,
                ),
            );
        }
        if capture.config.capture_request_bodies && raw.get("request_body").is_some() {
            let request_body = raw.get("request_body").map(|request_body| {
                if request_body.is_object() {
                    request_body.clone()
                } else {
                    json!({
                        "body": request_body,
                        "encoding": raw.get("request_body_encoding"),
                        "base64_encoded": raw.get("request_body_base64_encoded"),
                        "original_size": raw.get("request_body_original_size"),
                        "truncated": raw.get("request_body_truncated"),
                        "unavailable_reason": raw.get("request_body_unavailable_reason"),
                    })
                }
            });
            value.insert(
                "request_body".into(),
                bounded_request_body(request_body.as_ref(), capture.config.max_request_body_bytes),
            );
            value.insert("has_request_body".into(), Value::Bool(true));
        }
        return store_network_request(
            capture,
            request_id,
            Value::Object(value),
            NetworkRequestPhase::Requested,
        );
    }

    let Some(previous) = previous else {
        return NetworkMergeResult::Rejected;
    };
    if matches!(
        previous.phase,
        NetworkRequestPhase::Finished | NetworkRequestPhase::Failed
    ) {
        return NetworkMergeResult::Rejected;
    }
    let mut value = previous.value;
    let Some(object) = value.as_object_mut() else {
        return NetworkMergeResult::Rejected;
    };
    match event_type.as_str() {
        "response" => {
            object.insert(
                "status".into(),
                Value::from(raw.get("status").and_then(Value::as_i64).unwrap_or(0)),
            );
            object.insert(
                "status_text".into(),
                Value::String(truncate_utf8(&value_text(raw.get("status_text")), 256)),
            );
            object.insert(
                "mime_type".into(),
                Value::String(truncate_utf8(&value_text(raw.get("mime_type")), 256)),
            );
            object.insert(
                "protocol".into(),
                Value::String(truncate_utf8(&value_text(raw.get("protocol")), 64)),
            );
            object.insert(
                "from_cache".into(),
                Value::Bool(
                    raw.get("from_cache")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ),
            );
            if raw.get("headers").is_some() || raw.get("response_headers").is_some() {
                object.insert(
                    "response_headers".into(),
                    redact_network_mapping(
                        raw.get("headers").or_else(|| raw.get("response_headers")),
                        &capture.config.sensitive_fields,
                    ),
                );
            }
            store_network_request(
                capture,
                request_id,
                Value::Object(object.clone()),
                NetworkRequestPhase::ResponseReceived,
            )
        }
        "finished" => {
            object.insert("finished".into(), Value::Bool(true));
            object.insert(
                "encoded_data_length".into(),
                Value::from(
                    raw.get("encoded_data_length")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        .max(0),
                ),
            );
            store_network_request(
                capture,
                request_id,
                Value::Object(object.clone()),
                NetworkRequestPhase::Finished,
            )
        }
        "failed" => {
            object.insert("finished".into(), Value::Bool(true));
            object.insert("failed".into(), Value::Bool(true));
            object.insert(
                "error_text".into(),
                Value::String(truncate_utf8(
                    &redact_urls_in_text(
                        &value_text(raw.get("error_text")),
                        &capture.config.sensitive_fields,
                    ),
                    1_024,
                )),
            );
            object.insert(
                "canceled".into(),
                Value::Bool(
                    raw.get("canceled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ),
            );
            store_network_request(
                capture,
                request_id,
                Value::Object(object.clone()),
                NetworkRequestPhase::Failed,
            )
        }
        _ => NetworkMergeResult::Rejected,
    }
}

fn store_network_request(
    capture: &mut NetworkCaptureRecord,
    request_id: String,
    value: Value,
    phase: NetworkRequestPhase,
) -> NetworkMergeResult {
    let Ok(encoded) = serde_json::to_vec(&value) else {
        return NetworkMergeResult::Rejected;
    };
    let byte_size = encoded.len();
    if byte_size > capture.config.max_bytes.min(MAX_NETWORK_EVENT_BYTES) {
        return NetworkMergeResult::Rejected;
    }
    if let Some(previous) = capture.requests.remove(&request_id) {
        capture.retained_bytes = capture.retained_bytes.saturating_sub(previous.byte_size);
        capture.request_order.retain(|key| key != &request_id);
    }
    capture.request_order.push_back(request_id.clone());
    capture.requests.insert(
        request_id,
        NetworkRequestRecord {
            value,
            received_at: Instant::now(),
            byte_size,
            phase,
        },
    );
    capture.retained_bytes = capture.retained_bytes.saturating_add(byte_size);
    evict_network_requests(capture, Instant::now());
    NetworkMergeResult::Accepted
}

fn evict_network_requests(capture: &mut NetworkCaptureRecord, now: Instant) {
    while let Some(request_id) = capture.request_order.front().cloned() {
        let Some(record) = capture.requests.get(&request_id) else {
            capture.request_order.pop_front();
            continue;
        };
        if now
            .saturating_duration_since(record.received_at)
            .as_millis()
            <= u128::from(capture.config.max_age_ms)
            && capture.requests.len() <= capture.config.max_entries
            && capture.retained_bytes <= capture.config.max_bytes
        {
            break;
        }
        capture.request_order.pop_front();
        if let Some(removed) = capture.requests.remove(&request_id) {
            capture.retained_bytes = capture.retained_bytes.saturating_sub(removed.byte_size);
        }
    }
}

fn network_timestamp(value: Option<&Value>) -> u64 {
    let now = unix_ms();
    value
        .and_then(Value::as_u64)
        .filter(|timestamp| *timestamp <= now.saturating_add(86_400_000))
        .unwrap_or(now)
}

fn url_matches_allowed_hostname(value: &str, allowed_hostnames: &[String]) -> bool {
    let Ok(parsed) = Url::parse(value) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(hostname) = parsed
        .domain()
        .or_else(|| parsed.host_str())
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
    else {
        return false;
    };
    allowed_hostnames.iter().any(|allowed| allowed == &hostname)
}

fn redact_network_url(value: &str, fields: &BTreeSet<String>) -> String {
    let Ok(mut parsed) = Url::parse(value) else {
        return REDACTION_MARKER.to_owned();
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return REDACTION_MARKER.to_owned();
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    let query = parsed.query().map(|_| {
        parsed
            .query_pairs()
            .map(|(name, value)| {
                let name = name.into_owned();
                (
                    name.clone(),
                    if is_sensitive_field(&name, fields) {
                        REDACTION_MARKER.to_owned()
                    } else {
                        value.into_owned()
                    },
                )
            })
            .collect::<Vec<_>>()
    });
    parsed.set_fragment(None);
    if let Some(pairs) = query {
        let mut serializer = parsed.query_pairs_mut();
        serializer.clear();
        for (name, value) in pairs {
            serializer.append_pair(&name, &value);
        }
    }
    parsed.into()
}

fn redact_network_mapping(value: Option<&Value>, fields: &BTreeSet<String>) -> Value {
    redact_network_value(value.unwrap_or(&Value::Null), fields, 0)
}

fn redact_network_value(value: &Value, fields: &BTreeSet<String>, depth: usize) -> Value {
    if depth > 4 {
        return Value::String("[TRUNCATED]".into());
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .take(256)
                .map(|(key, value)| {
                    (
                        truncate_utf8(key, 256),
                        if is_sensitive_field(key, fields) {
                            Value::String(REDACTION_MARKER.into())
                        } else {
                            redact_network_value(value, fields, depth + 1)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(128)
                .map(|value| redact_network_value(value, fields, depth + 1))
                .collect(),
        ),
        Value::String(value) => {
            Value::String(truncate_utf8(&redact_urls_in_text(value, fields), 4_096))
        }
        _ => value.clone(),
    }
}

fn network_request_summary(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return json!({});
    };
    let fields = [
        "request_id",
        "timestamp_ms",
        "url",
        "method",
        "resource_type",
        "status",
        "status_text",
        "mime_type",
        "protocol",
        "from_cache",
        "finished",
        "failed",
        "error_text",
        "canceled",
        "encoded_data_length",
        "has_request_body",
    ];
    Value::Object(
        fields
            .iter()
            .filter_map(|field| {
                object
                    .get(*field)
                    .cloned()
                    .map(|value| ((*field).into(), value))
            })
            .collect(),
    )
}

fn network_capture_identity(target: &BrowserTarget, capture_id: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        target.extension_instance_id, target.window_id, target.tab_id, capture_id
    )
}

fn network_ack(
    capture_id: &str,
    target: &BrowserTarget,
    acknowledged_sequence: u64,
    accepted: bool,
    reason: Option<&str>,
) -> Value {
    let mut value = json!({
        "type": "network_ack",
        "capture_id": capture_id,
        "target": target,
        "ack_seq": acknowledged_sequence,
        "acknowledged_sequence": acknowledged_sequence,
        "accepted": accepted,
    });
    if let Some(reason) = reason {
        value["reason"] = Value::String(reason.into());
    }
    value
}

fn record_network_gap(capture: &mut NetworkCaptureRecord, from: u64, to: u64) {
    if from > to {
        return;
    }
    capture.gap_events = capture.gap_events.saturating_add(1);
    capture.loss_diagnostics.push_back(json!({
        "kind": "sequence_gap",
        "from": from,
        "to": to,
        "at_ms": unix_ms(),
    }));
    while capture.loss_diagnostics.len() > MAX_NETWORK_GAP_DIAGNOSTICS {
        capture.loss_diagnostics.pop_front();
    }
}

fn advance_network_barrier(capture: &mut NetworkCaptureRecord, barrier: u64, source: &str) {
    let barrier = barrier.max(capture.acknowledged_sequence);
    if barrier > capture.acknowledged_sequence.saturating_add(1) {
        record_network_gap(capture, capture.acknowledged_sequence + 1, barrier - 1);
    }
    capture
        .pending_events
        .retain(|sequence, _| *sequence > barrier);
    if source == "termination" {
        capture.rejected_events = capture
            .rejected_events
            .saturating_add(capture.pending_events.len() as u64);
        capture.pending_events.clear();
    }
    capture.pending_bytes = capture
        .pending_events
        .values()
        .map(|event| serde_json::to_vec(event).map_or(0, |bytes| bytes.len()))
        .sum();
    capture.acknowledged_sequence = barrier;
    capture.highest_seen_sequence = capture.highest_seen_sequence.max(barrier);
    if matches!(source, "clear" | "stop" | "termination") {
        capture.clear_sequence = capture.clear_sequence.max(barrier);
    }
}

fn network_capture_summary(
    capture: &NetworkCaptureRecord,
    active: bool,
    termination: Option<Value>,
) -> Value {
    json!({
        "target": capture.target,
        "capture_id": capture.capture_id,
        "active": active,
        "allowed_hostnames": capture.config.allowed_hostnames,
        "capture_request_bodies": capture.config.capture_request_bodies,
        "retention": {
            "max_age_ms": capture.config.max_age_ms,
            "max_entries": capture.config.max_entries,
            "max_bytes": capture.config.max_bytes,
            "max_body_bytes": capture.config.max_body_bytes,
            "max_request_body_bytes": capture.config.max_request_body_bytes,
        },
        "retained_entries": capture.requests.len(),
        "retained_bytes": capture.retained_bytes,
        "delivery": {
            "acknowledged_sequence": capture.acknowledged_sequence,
            "highest_seen_sequence": capture.highest_seen_sequence,
            "clear_sequence": capture.clear_sequence,
            "pending_sequences": capture.pending_events.len(),
            "pending_bytes": capture.pending_bytes,
            "dropped_events": capture.dropped_events,
            "dropped_batches": capture.dropped_batches,
            "dropped_bytes": capture.dropped_bytes,
            "rejected_events": capture.rejected_events,
            "filtered_events": capture.filtered_events,
            "duplicate_events": capture.duplicate_events,
            "gap_events": capture.gap_events,
            "loss_diagnostics": capture.loss_diagnostics,
        },
        "termination": termination.or_else(|| {
            capture.termination_reason.as_ref().map(|reason| json!({
                "reason": reason,
                "at_ms": capture.terminated_at_ms,
            }))
        }),
    })
}

fn bounded_u64(
    value: Option<&Value>,
    default: u64,
    minimum: u64,
    maximum: u64,
    field_name: &str,
) -> Result<u64, BrokerError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value.as_u64().ok_or_else(|| {
        BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            format!("{field_name} must be an integer between {minimum} and {maximum}"),
        )
    })?;
    if parsed < minimum || parsed > maximum {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            format!("{field_name} must be between {minimum} and {maximum}"),
        ));
    }
    Ok(parsed)
}

fn bounded_usize(
    value: Option<&Value>,
    default: usize,
    minimum: usize,
    maximum: usize,
    field_name: &str,
) -> Result<usize, BrokerError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value.as_u64().and_then(|value| usize::try_from(value).ok());
    let Some(parsed) = parsed else {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            format!("{field_name} must be an integer between {minimum} and {maximum}"),
        ));
    };
    if parsed < minimum || parsed > maximum {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            format!("{field_name} must be between {minimum} and {maximum}"),
        ));
    }
    Ok(parsed)
}

fn normalize_console_levels(value: Option<&Value>) -> Result<BTreeSet<String>, BrokerError> {
    let Some(value) = value else {
        return Ok(KNOWN_CONSOLE_LEVELS
            .iter()
            .map(|level| (*level).into())
            .collect());
    };
    let Some(values) = value.as_array() else {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "levels must be an array",
        ));
    };
    if values.len() > MAX_CONSOLE_LEVEL_FILTERS {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "levels must contain at most five entries",
        ));
    }
    let levels = values
        .iter()
        .map(|item| {
            let value = value_text(Some(item));
            if value.len() > MAX_CONSOLE_LEVEL_BYTES {
                return Err(BrokerError::new(
                    BrokerErrorCode::InvalidBrowserOperation,
                    "console level value is too long",
                ));
            }
            Ok(value.to_ascii_lowercase())
        })
        .collect::<Result<BTreeSet<_>, BrokerError>>()?;
    let unknown = levels
        .iter()
        .filter(|level| !KNOWN_CONSOLE_LEVELS.contains(&level.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        let mut error = BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "unsupported console level filter",
        );
        error.recovery.insert(
            "unsupported_levels".into(),
            Value::Array(unknown.into_iter().map(Value::String).collect()),
        );
        error.recovery.insert(
            "supported_levels".into(),
            Value::Array(
                KNOWN_CONSOLE_LEVELS
                    .iter()
                    .map(|level| Value::String((*level).into()))
                    .collect(),
            ),
        );
        return Err(error);
    }
    if levels.is_empty() {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "levels must not be empty",
        ));
    }
    Ok(levels)
}

fn normalize_sensitive_fields(value: Option<&Value>) -> Result<BTreeSet<String>, BrokerError> {
    let mut fields = DEFAULT_SENSITIVE_CONSOLE_FIELDS
        .iter()
        .map(|field| (*field).into())
        .collect::<BTreeSet<String>>();
    let Some(value) = value else {
        return Ok(fields);
    };
    let Some(values) = value.as_array() else {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "sensitive_fields must be an array",
        ));
    };
    for item in values.iter().take(128) {
        let field = value_text(Some(item)).trim().to_ascii_lowercase();
        if !field.is_empty() {
            fields.insert(truncate_utf8(&field, 128));
        }
    }
    Ok(fields)
}

fn value_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn is_sensitive_field(name: &str, fields: &BTreeSet<String>) -> bool {
    let normalized = name.to_ascii_lowercase().replace('_', "-");
    fields.contains(&normalized)
        || ["token", "password", "passwd", "secret"]
            .iter()
            .any(|marker| normalized.contains(marker))
}

fn redact_field_values(value: &str, fields: &BTreeSet<String>) -> String {
    let mut output = value.to_owned();
    for field in fields {
        if !is_sensitive_field(field, fields) {
            continue;
        }
        let needle = field.to_ascii_lowercase();
        if needle.is_empty() {
            continue;
        }
        let mut search_from = 0usize;
        loop {
            let lower = output.to_ascii_lowercase();
            let Some(relative) = lower[search_from..].find(&needle) else {
                break;
            };
            let start = search_from + relative;
            let before_ok = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric();
            let field_end = start + needle.len();
            if !before_ok {
                search_from = field_end;
                continue;
            }
            let mut value_start = field_end;
            while value_start < output.len()
                && matches!(output.as_bytes()[value_start], b'"' | b'\'' | b' ' | b'\t')
            {
                value_start += 1;
            }
            if value_start >= output.len() || !matches!(output.as_bytes()[value_start], b':' | b'=')
            {
                search_from = field_end;
                continue;
            }
            value_start += 1;
            while value_start < output.len()
                && matches!(output.as_bytes()[value_start], b' ' | b'\t')
            {
                value_start += 1;
            }
            let quote = output
                .as_bytes()
                .get(value_start)
                .copied()
                .filter(|byte| matches!(byte, b'"' | b'\''));
            if quote.is_some() {
                value_start += 1;
            }
            let mut value_end = value_start;
            while value_end < output.len() {
                let byte = output.as_bytes()[value_end];
                if quote == Some(byte) {
                    break;
                }
                if quote.is_none() && matches!(byte, b',' | b';' | b'\n' | b'\r' | b'}' | b']') {
                    break;
                }
                if byte == b'\\'
                    && quote.is_some()
                    && output
                        .as_bytes()
                        .get(value_end + 1)
                        .is_some_and(|next| *next == quote.unwrap())
                {
                    value_end = value_end.saturating_add(2);
                    continue;
                }
                value_end += 1;
            }
            if value_end > value_start {
                output.replace_range(value_start..value_end, REDACTION_MARKER);
                search_from = value_start + REDACTION_MARKER.len();
            } else {
                search_from = field_end;
            }
        }
    }
    output
}

fn redact_console_url(value: &str, fields: &BTreeSet<String>) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return REDACTION_MARKER.to_owned();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return REDACTION_MARKER.to_owned();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    redact_field_values(url.as_ref(), fields)
}

fn redact_urls_in_text(value: &str, fields: &BTreeSet<String>) -> String {
    let mut output = redact_field_values(value, fields);
    let mut search_from = 0usize;
    loop {
        let lower = output.to_ascii_lowercase();
        let remaining = &lower[search_from..];
        let http = remaining.find("http://");
        let https = remaining.find("https://");
        let Some(start) = (match (http, https) {
            (Some(left), Some(right)) => Some(search_from + left.min(right)),
            (Some(index), None) | (None, Some(index)) => Some(search_from + index),
            (None, None) => None,
        }) else {
            return output;
        };
        let mut end = start;
        while end < output.len()
            && !matches!(
                output.as_bytes()[end],
                b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' | b',' | b';' | b')' | b']' | b'}'
            )
        {
            end += 1;
        }
        let raw_url = output[start..end].to_owned();
        let safe_url = redact_console_url(&raw_url, fields);
        if safe_url == raw_url {
            search_from = end;
        } else {
            output.replace_range(start..end, &safe_url);
            search_from = start.saturating_add(safe_url.len());
        }
    }
}

fn sanitize_console_event(
    raw: Option<&Value>,
    sensitive_fields: &BTreeSet<String>,
) -> Option<(Value, bool)> {
    let object = raw?.as_object()?;
    let mut truncated = false;
    let raw_level = value_text(object.get("level"));
    let bounded_level = truncate_utf8(&raw_level, MAX_CONSOLE_LEVEL_BYTES);
    let mut level = bounded_level.to_ascii_lowercase();
    if level == "warning" {
        level = "warn".into();
    }
    if !KNOWN_CONSOLE_LEVELS.contains(&level.as_str()) {
        level = "log".into();
    }
    let now_ms = unix_ms();
    let timestamp_ms = object
        .get("timestamp_ms")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .map(|value| value as u64)
        .filter(|value| *value <= now_ms.saturating_add(86_400_000))
        .unwrap_or(now_ms);
    let raw_text = value_text(object.get("text"));
    let bounded_raw_text = truncate_utf8(&raw_text, MAX_CONSOLE_RAW_TEXT_BYTES);
    truncated |= bounded_raw_text.len() < raw_text.len();
    let redacted_text = redact_urls_in_text(&bounded_raw_text, sensitive_fields);
    let text = truncate_utf8(&redacted_text, MAX_CONSOLE_EVENT_TEXT_BYTES);
    truncated |= text.len() < redacted_text.len();

    let raw_source = value_text(object.get("source"));
    let bounded_raw_source = truncate_utf8(&raw_source, MAX_CONSOLE_EVENT_SOURCE_BYTES * 2);
    truncated |= bounded_raw_source.len() < raw_source.len();
    let redacted_source = redact_urls_in_text(&bounded_raw_source, sensitive_fields);
    let source = truncate_utf8(&redacted_source, MAX_CONSOLE_EVENT_SOURCE_BYTES);
    truncated |= source.len() < redacted_source.len();

    let raw_url = value_text(object.get("url"));
    let bounded_raw_url = truncate_utf8(&raw_url, MAX_CONSOLE_RAW_URL_BYTES);
    truncated |= bounded_raw_url.len() < raw_url.len();
    let redacted_url = if bounded_raw_url.is_empty() {
        String::new()
    } else {
        redact_console_url(&bounded_raw_url, sensitive_fields)
    };
    let url = truncate_utf8(&redacted_url, MAX_CONSOLE_EVENT_URL_BYTES);
    truncated |= url.len() < redacted_url.len();

    let mut event = Map::from_iter([
        ("timestamp_ms".into(), Value::from(timestamp_ms)),
        ("level".into(), Value::String(level)),
        ("text".into(), Value::String(text)),
    ]);
    if !source.is_empty() {
        event.insert("source".into(), Value::String(source));
    }
    if !url.is_empty() {
        event.insert("url".into(), Value::String(url));
    }
    if let Some(line_number) = object
        .get("line_number")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
    {
        event.insert("line_number".into(), Value::from(line_number));
    }
    if truncated {
        event.insert("truncated".into(), Value::Bool(true));
    }
    Some((Value::Object(event), truncated))
}

fn fit_console_event(value: &mut Value, max_bytes: usize) -> (usize, bool) {
    let mut truncated = value
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    loop {
        let encoded_len = serde_json::to_vec(value).map_or(0, |encoded| encoded.len());
        if encoded_len <= max_bytes {
            return (encoded_len, truncated);
        }
        truncated = true;
        let Some(object) = value.as_object_mut() else {
            return (0, true);
        };
        object.insert("truncated".into(), Value::Bool(true));
        let overflow = encoded_len.saturating_sub(max_bytes).saturating_add(8);
        let mut changed = false;
        for field in ["text", "url", "source"] {
            let Some(current) = object.get(field).and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            if current.is_empty() {
                continue;
            }
            let target = current.len().saturating_sub(overflow);
            object.insert(field.into(), Value::String(truncate_utf8(&current, target)));
            changed = true;
            break;
        }
        if !changed {
            for field in ["line_number", "url", "source"] {
                if object.remove(field).is_some() {
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            return (0, true);
        }
    }
}

fn evict_console_events(capture: &mut ConsoleCaptureRecord, now: Instant) {
    while let Some(front) = capture.events.front() {
        let reason = if now.saturating_duration_since(front.received_at).as_millis()
            > u128::from(capture.config.max_age_ms)
        {
            Some("age")
        } else if capture.events.len() > capture.config.max_entries {
            Some("entries")
        } else if capture.retained_bytes > capture.config.max_bytes {
            Some("bytes")
        } else {
            None
        };
        let Some(reason) = reason else {
            break;
        };
        let removed = capture
            .events
            .pop_front()
            .expect("console event exists while evicting");
        capture.retained_bytes = capture.retained_bytes.saturating_sub(removed.byte_size);
        match reason {
            "age" => capture.evicted_age = capture.evicted_age.saturating_add(1),
            "entries" => capture.evicted_entries = capture.evicted_entries.saturating_add(1),
            "bytes" => {
                capture.evicted_bytes = capture
                    .evicted_bytes
                    .saturating_add(removed.byte_size as u64);
            }
            _ => {}
        }
    }
}

fn console_diagnostics(capture: &ConsoleCaptureRecord) -> Value {
    json!({
        "evicted_age": capture.evicted_age,
        "evicted_entries": capture.evicted_entries,
        "evicted_bytes": capture.evicted_bytes,
        "rejected_events": capture.rejected_events,
        "filtered_events": capture.filtered_events,
        "truncated_events": capture.truncated_events,
    })
}

fn console_capture_summary(
    capture: &ConsoleCaptureRecord,
    active: bool,
    termination: Option<Value>,
) -> Value {
    json!({
        "target": capture.target,
        "capture_id": capture.capture_id,
        "active": active,
        "levels": capture.levels,
        "retention": {
            "max_age_ms": capture.config.max_age_ms,
            "max_entries": capture.config.max_entries,
            "max_bytes": capture.config.max_bytes,
            "max_event_bytes": MAX_CONSOLE_EVENT_BYTES.min(capture.config.max_bytes),
        },
        "retained_entries": capture.events.len(),
        "retained_bytes": capture.retained_bytes,
        "diagnostics": console_diagnostics(capture),
        "termination": termination,
    })
}

fn normalize_console_termination_reason(reason: &str) -> &'static str {
    match reason {
        "explicit_stop" => "explicit_stop",
        "capture_replaced" => "capture_replaced",
        "target_closed" => "target_closed",
        "debugger_detached" => "debugger_detached",
        "session_disconnected" => "session_disconnected",
        "lease_expired" => "lease_expired",
        "lease_released" => "lease_released",
        "already_stopped" => "already_stopped",
        "navigation" => "navigation",
        "broker_restart" => "broker_restart",
        "stream_disconnected" => "stream_disconnected",
        "extension_reported" => "extension_reported",
        _ => "extension_reported",
    }
}

#[cfg(test)]
fn termination_value(reason: &str, detail: Option<&str>) -> Value {
    termination_value_with_fields(reason, detail, &default_sensitive_fields())
}

fn termination_value_with_fields(
    reason: &str,
    detail: Option<&str>,
    sensitive_fields: &BTreeSet<String>,
) -> Value {
    let safe_detail = detail
        .map(|value| {
            let bounded = truncate_utf8(value, MAX_CONSOLE_EVENT_URL_BYTES);
            redact_urls_in_text(&bounded, sensitive_fields)
        })
        .map(|value| truncate_utf8(&value, 256));
    json!({
        "reason": normalize_console_termination_reason(reason),
        "detail": safe_detail,
        "at_ms": unix_ms(),
    })
}

fn default_sensitive_fields() -> BTreeSet<String> {
    DEFAULT_SENSITIVE_CONSOLE_FIELDS
        .iter()
        .map(|field| (*field).into())
        .collect()
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn artifact_error(message: &str) -> BrokerError {
    BrokerError::new(BrokerErrorCode::BrowserArtifactFailure, message)
}

fn canonical_project_root(value: &str) -> Result<PathBuf, BrokerError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4096 {
        return Err(artifact_error("project root is invalid"));
    }
    let path = Path::new(value);
    let canonical =
        fs::canonicalize(path).map_err(|_| artifact_error("project root could not be verified"))?;
    let metadata = fs::metadata(&canonical)
        .map_err(|_| artifact_error("project root metadata could not be read"))?;
    if !metadata.is_dir() {
        return Err(artifact_error("project root is not a directory"));
    }
    Ok(canonical)
}

fn inspect_artifact_root(
    project_root: &Path,
    create: bool,
) -> Result<Option<PathBuf>, BrokerError> {
    let mut current = project_root.to_path_buf();
    for component in MANAGED_ARTIFACT_COMPONENTS {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
                    return Err(artifact_error("managed artifact directory is not safe"));
                }
                let canonical = fs::canonicalize(&current).map_err(|_| {
                    artifact_error("managed artifact directory could not be verified")
                })?;
                ensure_within(project_root, &canonical)?;
                current = canonical;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                fs::create_dir(&current).map_err(|_| {
                    artifact_error("managed artifact directory could not be created")
                })?;
                let metadata = fs::symlink_metadata(&current).map_err(|_| {
                    artifact_error("managed artifact directory could not be verified")
                })?;
                if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
                    return Err(artifact_error("managed artifact directory is not safe"));
                }
                let canonical = fs::canonicalize(&current).map_err(|_| {
                    artifact_error("managed artifact directory could not be verified")
                })?;
                ensure_within(project_root, &canonical)?;
                current = canonical;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(artifact_error(
                    "managed artifact directory could not be inspected",
                ));
            }
        }
    }
    Ok(Some(current))
}

fn ensure_within(root: &Path, candidate: &Path) -> Result<(), BrokerError> {
    if candidate.strip_prefix(root).is_err() {
        return Err(artifact_error("managed artifact path escaped the project"));
    }
    Ok(())
}

fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || metadata.file_type().is_symlink()
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn generated_filename(request_id: &str, target: &BrowserTarget, format: &str) -> String {
    let request = sanitize_component(request_id, "request");
    let profile = sanitize_component(&target.extension_instance_id, "profile");
    let suffix = Uuid::new_v4().simple().to_string();
    let mut filename = format!(
        "teshi-{request}-{profile}-w{}-t{}-{suffix}.{format}",
        target.window_id, target.tab_id
    );
    if filename.len() > MAX_ARTIFACT_FILENAME_BYTES {
        filename.truncate(MAX_ARTIFACT_FILENAME_BYTES.saturating_sub(format.len() + 1));
        filename.push('.');
        filename.push_str(format);
    }
    filename
}

fn sanitize_component(value: &str, fallback: &str) -> String {
    let mut result = String::with_capacity(value.len().min(64));
    for character in value.chars() {
        if result.len() >= 64 {
            break;
        }
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            result.push(character);
        } else {
            result.push('-');
        }
    }
    let result = result.trim_matches(['.', '-', '_']).to_owned();
    if result.is_empty() {
        fallback.to_owned()
    } else {
        result
    }
}

fn normalize_format(kind: ArtifactKind, value: Option<&str>) -> Result<String, BrokerError> {
    let value = value.unwrap_or(kind.default_format()).trim();
    if value.len() > 16 {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "browser artifact format exceeds the configured bound",
        ));
    }
    let value = value.to_ascii_lowercase();
    let normalized = match value.as_str() {
        "jpg" => "jpeg",
        "png" | "jpeg" | "pdf" => value.as_str(),
        _ => {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "browser artifact format is unsupported",
            ));
        }
    };
    let valid = match kind {
        ArtifactKind::EvidenceJpeg => normalized == "jpeg",
        ArtifactKind::Screenshot => matches!(normalized, "png" | "jpeg"),
        ArtifactKind::Pdf => normalized == "pdf",
    };
    if !valid {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "browser artifact format does not match the operation",
        ));
    }
    Ok(normalized.to_owned())
}

fn decode_bounded_base64(value: &str) -> Result<Vec<u8>, BrokerError> {
    if value.is_empty() || value.len() > MAX_ARTIFACT_BASE64_BYTES || !value.is_ascii() {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "browser artifact payload exceeds the encoded byte bound",
        ));
    }
    let estimated = value.len().saturating_div(4).saturating_mul(3);
    if estimated > MAX_ARTIFACT_BYTES.saturating_add(3) {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "browser artifact payload exceeds the decoded byte bound",
        ));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| artifact_error("browser artifact payload is not valid base64"))?;
    if decoded.is_empty() {
        return Err(artifact_error("browser artifact payload is empty"));
    }
    if decoded.len() > MAX_ARTIFACT_BYTES {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "browser artifact exceeds the decoded byte bound",
        ));
    }
    Ok(decoded)
}

fn validate_payload(
    format: &str,
    payload: &[u8],
) -> Result<Option<ArtifactDimensions>, BrokerError> {
    match format {
        "png" => parse_png_dimensions(payload).map(Some),
        "jpeg" => parse_jpeg_dimensions(payload).map(Some),
        "pdf" => {
            if payload.len() < 5 || &payload[..5] != b"%PDF-" {
                return Err(artifact_error(
                    "browser PDF payload has an invalid signature",
                ));
            }
            if !payload.windows(5).any(|window| window == b"%%EOF") {
                return Err(artifact_error("browser PDF payload has no EOF marker"));
            }
            Ok(None)
        }
        _ => Err(artifact_error("browser artifact format is unsupported")),
    }
}

fn parse_png_dimensions(payload: &[u8]) -> Result<ArtifactDimensions, BrokerError> {
    if payload.len() < 24 || &payload[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(artifact_error(
            "browser PNG payload has an invalid signature",
        ));
    }
    let chunk_len = u32::from_be_bytes(payload[8..12].try_into().unwrap());
    if chunk_len != 13 || &payload[12..16] != b"IHDR" {
        return Err(artifact_error("browser PNG payload has no valid IHDR"));
    }
    let width = u32::from_be_bytes(payload[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(payload[20..24].try_into().unwrap());
    checked_dimensions(width, height)
}

fn parse_jpeg_dimensions(payload: &[u8]) -> Result<ArtifactDimensions, BrokerError> {
    if payload.len() < 4
        || payload[..2] != [0xff, 0xd8]
        || payload[payload.len() - 2..] != [0xff, 0xd9]
    {
        return Err(artifact_error(
            "browser JPEG payload has an invalid signature",
        ));
    }
    let mut cursor = 2usize;
    while cursor < payload.len() {
        if payload[cursor] != 0xff {
            return Err(artifact_error("browser JPEG marker is malformed"));
        }
        while cursor < payload.len() && payload[cursor] == 0xff {
            cursor += 1;
        }
        let marker = *payload
            .get(cursor)
            .ok_or_else(|| artifact_error("browser JPEG marker is truncated"))?;
        cursor += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length_end = cursor
            .checked_add(2)
            .ok_or_else(|| artifact_error("browser JPEG segment length overflowed"))?;
        let length_bytes = payload
            .get(cursor..length_end)
            .ok_or_else(|| artifact_error("browser JPEG segment length is truncated"))?;
        let segment_len = u16::from_be_bytes([length_bytes[0], length_bytes[1]]) as usize;
        if segment_len < 2 {
            return Err(artifact_error("browser JPEG segment length is invalid"));
        }
        let segment_end = cursor
            .checked_add(segment_len)
            .ok_or_else(|| artifact_error("browser JPEG segment length overflowed"))?;
        if segment_end > payload.len() {
            return Err(artifact_error("browser JPEG segment is truncated"));
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if segment_len < 7 {
                return Err(artifact_error("browser JPEG dimensions are truncated"));
            }
            let height = u16::from_be_bytes([payload[cursor + 3], payload[cursor + 4]]) as u32;
            let width = u16::from_be_bytes([payload[cursor + 5], payload[cursor + 6]]) as u32;
            return checked_dimensions(width, height);
        }
        cursor = segment_end;
    }
    Err(artifact_error(
        "browser JPEG has no supported dimension marker",
    ))
}

fn checked_dimensions(width: u32, height: u32) -> Result<ArtifactDimensions, BrokerError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "browser screenshot pixel count overflowed",
            )
        })?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "browser screenshot dimensions exceed the configured bound",
        ));
    }
    Ok(ArtifactDimensions {
        width,
        height,
        pixels,
    })
}

fn publish_no_clobber(prepared: &PreparedRecord, payload: &[u8]) -> Result<(), BrokerError> {
    let artifact_root = inspect_artifact_root(&prepared.project_root, true)?
        .ok_or_else(|| artifact_error("managed artifact directory is unavailable"))?;
    ensure_within(&prepared.project_root, &artifact_root)?;
    let expected_root = fs::canonicalize(&prepared.artifact_root)
        .map_err(|_| artifact_error("managed artifact directory could not be verified"))?;
    if artifact_root != expected_root {
        return Err(artifact_error(
            "managed artifact directory changed before publication",
        ));
    }
    let final_parent = prepared
        .final_path
        .parent()
        .ok_or_else(|| artifact_error("browser artifact path is invalid"))?;
    let final_parent = fs::canonicalize(final_parent)
        .map_err(|_| artifact_error("browser artifact directory could not be verified"))?;
    if final_parent != artifact_root {
        return Err(artifact_error(
            "browser artifact path escaped its managed directory",
        ));
    }
    if fs::symlink_metadata(&prepared.final_path).is_ok() {
        return Err(artifact_error(
            "refusing to replace an existing browser artifact",
        ));
    }

    let temp_path = artifact_root.join(format!(
        ".{}.tmp-{}",
        prepared.relative_path,
        Uuid::new_v4().simple()
    ));
    let temp = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
    {
        Ok(file) => file,
        Err(_) => {
            return Err(artifact_error(
                "browser artifact temporary file could not be created",
            ));
        }
    };
    let result = write_and_publish(temp, &temp_path, &prepared.final_path, payload);
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn write_and_publish(
    mut temp: File,
    temp_path: &Path,
    final_path: &Path,
    payload: &[u8],
) -> Result<(), BrokerError> {
    temp.write_all(payload)
        .map_err(|_| artifact_error("browser artifact temporary write failed"))?;
    temp.sync_all()
        .map_err(|_| artifact_error("browser artifact temporary flush failed"))?;
    drop(temp);
    let temp_meta = fs::symlink_metadata(temp_path)
        .map_err(|_| artifact_error("browser artifact temporary file could not be verified"))?;
    if is_reparse_or_symlink(&temp_meta)
        || !temp_meta.is_file()
        || temp_meta.len() != payload.len() as u64
    {
        return Err(artifact_error(
            "browser artifact temporary file failed validation",
        ));
    }
    let temp_canonical = fs::canonicalize(temp_path)
        .map_err(|_| artifact_error("browser artifact temporary path could not be verified"))?;
    let root = temp_path
        .parent()
        .ok_or_else(|| artifact_error("browser artifact temporary path is invalid"))?;
    ensure_within(root, &temp_canonical)?;

    // A same-directory hard-link creates the final directory entry without
    // replacing an existing entry.  It is intentionally preferred to a plain
    // rename, whose overwrite behavior differs between Unix and Windows.
    fs::hard_link(temp_path, final_path)
        .map_err(|_| artifact_error("browser artifact could not be atomically published"))?;
    let final_meta = match fs::symlink_metadata(final_path) {
        Ok(metadata) => metadata,
        Err(_) => {
            let _ = fs::remove_file(final_path);
            return Err(artifact_error(
                "published browser artifact could not be verified",
            ));
        }
    };
    if is_reparse_or_symlink(&final_meta)
        || !final_meta.is_file()
        || final_meta.len() != payload.len() as u64
    {
        let _ = fs::remove_file(final_path);
        return Err(artifact_error(
            "published browser artifact failed validation",
        ));
    }
    let final_canonical = match fs::canonicalize(final_path) {
        Ok(path) => path,
        Err(_) => {
            let _ = fs::remove_file(final_path);
            return Err(artifact_error(
                "published browser artifact path could not be verified",
            ));
        }
    };
    if let Err(error) = ensure_within(root, &final_canonical) {
        let _ = fs::remove_file(final_path);
        return Err(error);
    }
    if fs::remove_file(temp_path).is_err() {
        let _ = fs::remove_file(final_path);
        return Err(artifact_error(
            "browser artifact temporary file could not be cleaned",
        ));
    }
    Ok(())
}

fn digest_payload(payload: &[u8]) -> [u8; 32] {
    Sha256::digest(payload).into()
}

fn digest_file(path: &Path) -> Result<[u8; 32], BrokerError> {
    let payload = fs::read(path)
        .map_err(|_| artifact_error("managed artifact could not be read for verification"))?;
    if payload.len() > MAX_ARTIFACT_BYTES {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "managed artifact exceeds the configured bound",
        ));
    }
    Ok(digest_payload(&payload))
}

fn validate_relative_filename(value: &str) -> Result<String, BrokerError> {
    if value.trim().is_empty() || value.len() > MAX_ARTIFACT_FILENAME_BYTES {
        return Err(artifact_error("managed artifact filename is invalid"));
    }
    let bytes = value.as_bytes();
    let windows_drive_prefix =
        bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if value.contains(['/', '\\']) || windows_drive_prefix {
        return Err(artifact_error("managed artifact filename must be relative"));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(artifact_error("managed artifact filename must be relative"));
    }
    let mut components = path.components();
    let Some(Component::Normal(name)) = components.next() else {
        return Err(artifact_error("managed artifact filename is invalid"));
    };
    if components.next().is_some() || name.to_string_lossy() != value {
        return Err(artifact_error(
            "managed artifact filename must be one generated component",
        ));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use super::*;
    use serde_json::json;

    const BROKER_GENERATION: &str = "broker-generation-a";
    const CALLER: &str = "caller-a";

    fn target() -> BrowserTarget {
        BrowserTarget {
            extension_instance_id: "profile-a".into(),
            window_id: 7,
            tab_id: 42,
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 0x08];
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[1, 1, 0x11, 0, 0xff, 0xd9]);
        bytes
    }

    fn response(
        request_id: &str,
        operation: &str,
        target: BrowserTarget,
        format: Option<&str>,
        key: &str,
        payload: &[u8],
    ) -> ExtensionResponse {
        let mut result = BTreeMap::new();
        if let Some(format) = format {
            result.insert("format".into(), Value::String(format.into()));
        }
        result.insert(
            "page_context_revision".into(),
            Value::String("revision-1".into()),
        );
        result.insert(
            key.into(),
            Value::String(base64::engine::general_purpose::STANDARD.encode(payload)),
        );
        ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(1),
            protocol_version: Some(1),
            request_id: request_id.into(),
            operation: operation.into(),
            extension_instance_id: Some(target.extension_instance_id.clone()),
            target: Some(target),
            ok: true,
            code: None,
            error: None,
            result,
        }
    }

    fn console_target(profile: &str, window_id: i64, tab_id: i64) -> BrowserTarget {
        BrowserTarget {
            extension_instance_id: profile.into(),
            window_id,
            tab_id,
        }
    }

    fn console_response(
        request_id: &str,
        target: &BrowserTarget,
        capture_id: &str,
        active: bool,
    ) -> ExtensionResponse {
        ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(1),
            protocol_version: Some(1),
            request_id: request_id.into(),
            operation: "start_console_capture".into(),
            extension_instance_id: Some(target.extension_instance_id.clone()),
            target: Some(target.clone()),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::from([
                ("active".into(), Value::Bool(active)),
                ("capture_id".into(), Value::String(capture_id.into())),
            ]),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_console(
        store: &mut EvidenceStore,
        request_id: &str,
        target: &BrowserTarget,
        generation: Option<u64>,
        levels: Option<Value>,
        max_age_ms: Option<Value>,
        max_entries: Option<Value>,
        max_bytes: Option<Value>,
    ) -> String {
        let handle = store
            .prepare_console_capture(
                request_id,
                BROKER_GENERATION,
                "project-a",
                CALLER,
                target,
                generation,
                levels.as_ref(),
                max_age_ms.as_ref(),
                max_entries.as_ref(),
                max_bytes.as_ref(),
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id.clone();
        store
            .commit_console_capture(
                request_id,
                BROKER_GENERATION,
                "project-a",
                CALLER,
                target,
                &console_response(request_id, target, &capture_id, true),
            )
            .unwrap();
        capture_id
    }

    fn prepare(
        store: &mut EvidenceStore,
        project: &Path,
        request_id: &str,
        operation: &str,
        format: Option<&str>,
    ) {
        store
            .prepare_request(
                operation,
                request_id,
                BROKER_GENERATION,
                &project.to_string_lossy(),
                CALLER,
                &target(),
                Some("revision-1"),
                format,
            )
            .unwrap();
    }

    #[test]
    fn screenshot_and_pdf_are_validated_and_published_with_complete_bytes() {
        let project = tempfile::tempdir().unwrap();
        let mut store = EvidenceStore::new();
        prepare(
            &mut store,
            project.path(),
            "screenshot-1",
            "capture_browser_screenshot",
            Some("png"),
        );
        let png_payload = png(3, 2);
        let screenshot = store
            .commit_response(
                "screenshot-1",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "screenshot-1",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &png_payload,
                ),
            )
            .unwrap();
        let screenshot_path = project
            .path()
            .join(MANAGED_ARTIFACT_COMPONENTS[0])
            .join(MANAGED_ARTIFACT_COMPONENTS[1])
            .join(MANAGED_ARTIFACT_COMPONENTS[2])
            .join(&screenshot.path);
        assert_eq!(fs::read(&screenshot_path).unwrap(), png_payload);
        assert_eq!(screenshot.size, png_payload.len() as u64);
        assert_eq!(screenshot.dimensions.unwrap().pixels, 6);

        prepare(
            &mut store,
            project.path(),
            "pdf-1",
            "generate_browser_pdf",
            None,
        );
        let pdf_payload = b"%PDF-1.7\nfixture\n%%EOF".to_vec();
        let pdf = store
            .commit_response(
                "pdf-1",
                BROKER_GENERATION,
                "generate_browser_pdf",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "pdf-1",
                    "generate_browser_pdf",
                    target(),
                    Some("pdf"),
                    "artifact_data",
                    &pdf_payload,
                ),
            )
            .unwrap();
        let pdf_path = project
            .path()
            .join(MANAGED_ARTIFACT_COMPONENTS[0])
            .join(MANAGED_ARTIFACT_COMPONENTS[1])
            .join(MANAGED_ARTIFACT_COMPONENTS[2])
            .join(&pdf.path);
        assert_eq!(fs::read(pdf_path).unwrap(), pdf_payload);
        assert!(pdf.dimensions.is_none());
        assert_eq!(store.managed_count(), 2);
    }

    #[test]
    fn payload_format_dimensions_and_pixel_bounds_are_fail_closed() {
        let project = tempfile::tempdir().unwrap();
        let mut store = EvidenceStore::new();
        prepare(
            &mut store,
            project.path(),
            "bad-format",
            "capture_browser_screenshot",
            Some("png"),
        );
        let error = store
            .commit_response(
                "bad-format",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "bad-format",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &jpeg(3, 2),
                ),
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserArtifactFailure);
        assert!(!project.path().join(".teshi").exists());

        prepare(
            &mut store,
            project.path(),
            "too-large",
            "capture_browser_screenshot",
            Some("png"),
        );
        let error = store
            .commit_response(
                "too-large",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "too-large",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &png(MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION),
                ),
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserResourceLimit);
        assert!(!project.path().join(".teshi").exists());
    }

    #[test]
    fn cleanup_requires_a_broker_managed_relative_name_and_preserves_sentinels() {
        let project = tempfile::tempdir().unwrap();
        let mut store = EvidenceStore::new();
        prepare(
            &mut store,
            project.path(),
            "cleanup-1",
            "capture_browser_screenshot",
            Some("png"),
        );
        let payload = png(1, 1);
        let artifact = store
            .commit_response(
                "cleanup-1",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "cleanup-1",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &payload,
                ),
            )
            .unwrap();
        let root = project
            .path()
            .join(MANAGED_ARTIFACT_COMPONENTS[0])
            .join(MANAGED_ARTIFACT_COMPONENTS[1])
            .join(MANAGED_ARTIFACT_COMPONENTS[2]);
        let sentinel = project.path().join("keep.bin");
        fs::write(&sentinel, b"keep").unwrap();
        let absolute = root.join(&artifact.path).to_string_lossy().into_owned();
        assert_eq!(
            store
                .cleanup_managed(&project.path().to_string_lossy(), CALLER, &[absolute])
                .unwrap_err()
                .code,
            BrokerErrorCode::BrowserArtifactFailure
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert!(root.join(&artifact.path).exists());

        assert_eq!(
            store
                .cleanup_managed(
                    &project.path().to_string_lossy(),
                    CALLER,
                    &[format!("../{}", artifact.path)],
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::BrowserArtifactFailure
        );
        for invalid in [
            format!(r"..\{}", artifact.path),
            format!(r"C:\{}", artifact.path),
            format!(r"\\server\share\{}", artifact.path),
        ] {
            assert_eq!(
                store
                    .cleanup_managed(&project.path().to_string_lossy(), CALLER, &[invalid])
                    .unwrap_err()
                    .code,
                BrokerErrorCode::BrowserArtifactFailure
            );
        }
        let cleaned = store
            .cleanup_managed(
                &project.path().to_string_lossy(),
                CALLER,
                std::slice::from_ref(&artifact.path),
            )
            .unwrap();
        assert_eq!(cleaned["removed"][0], artifact.path);
        assert!(!root.join(&artifact.path).exists());
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
    }

    #[test]
    fn cross_project_and_existing_file_are_rejected_without_clobbering() {
        let project = tempfile::tempdir().unwrap();
        let other_project = tempfile::tempdir().unwrap();
        let mut store = EvidenceStore::new();
        prepare(
            &mut store,
            project.path(),
            "cross-project",
            "capture_browser_screenshot",
            Some("png"),
        );
        let cross_project = store
            .commit_response(
                "cross-project",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &other_project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "cross-project",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &png(1, 1),
                ),
            )
            .unwrap_err();
        assert_eq!(cross_project.code, BrokerErrorCode::BrowserArtifactFailure);
        assert!(!project.path().join(".teshi").exists());

        prepare(
            &mut store,
            project.path(),
            "existing-file",
            "capture_browser_screenshot",
            Some("png"),
        );
        let final_path = store.prepared_path("existing-file").to_owned();
        fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        fs::write(&final_path, b"user-sentinel").unwrap();
        let error = store
            .commit_response(
                "existing-file",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "existing-file",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &png(1, 1),
                ),
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserArtifactFailure);
        assert_eq!(fs::read(final_path).unwrap(), b"user-sentinel");
        assert_eq!(store.managed_count(), 0);
    }

    #[test]
    fn encoded_payload_limit_is_checked_before_decoding() {
        let project = tempfile::tempdir().unwrap();
        let mut store = EvidenceStore::new();
        prepare(
            &mut store,
            project.path(),
            "encoded-too-large",
            "generate_browser_pdf",
            None,
        );
        let oversized = "A".repeat(MAX_ARTIFACT_BASE64_BYTES + 1);
        let mut response = response(
            "encoded-too-large",
            "generate_browser_pdf",
            target(),
            Some("pdf"),
            "artifact_data",
            b"x",
        );
        response
            .result
            .insert("artifact_data".into(), Value::String(oversized));
        let error = store
            .commit_response(
                "encoded-too-large",
                BROKER_GENERATION,
                "generate_browser_pdf",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response,
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserResourceLimit);
        assert!(!project.path().join(".teshi").exists());
    }

    #[test]
    fn managed_root_symlink_is_rejected_before_file_access() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = project.path().join(".teshi");
        let link_result = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(outside.path(), &link)
            }
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(outside.path(), &link)
            }
        };
        if link_result.is_err() {
            // Some Windows hosts deny unprivileged symlink creation.  The
            // runtime path still checks reparse metadata when the OS permits it.
            return;
        }
        let mut store = EvidenceStore::new();
        let error = store
            .prepare_request(
                "capture_browser_screenshot",
                "symlink-root",
                BROKER_GENERATION,
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                Some("revision-1"),
                Some("png"),
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserArtifactFailure);
        assert!(!outside.path().join("artifacts").exists());
    }

    #[test]
    fn cleanup_rejects_a_changed_file_with_the_same_size() {
        let project = tempfile::tempdir().unwrap();
        let mut store = EvidenceStore::new();
        prepare(
            &mut store,
            project.path(),
            "changed-file",
            "capture_browser_screenshot",
            Some("png"),
        );
        let artifact = store
            .commit_response(
                "changed-file",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "changed-file",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &png(1, 1),
                ),
            )
            .unwrap();
        let path = project
            .path()
            .join(MANAGED_ARTIFACT_COMPONENTS[0])
            .join(MANAGED_ARTIFACT_COMPONENTS[1])
            .join(MANAGED_ARTIFACT_COMPONENTS[2])
            .join(&artifact.path);
        let mut changed = fs::read(&path).unwrap();
        let last = changed.len() - 1;
        changed[last] ^= 0xff;
        fs::write(&path, changed).unwrap();
        let error = store
            .cleanup_managed(
                &project.path().to_string_lossy(),
                CALLER,
                std::slice::from_ref(&artifact.path),
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserArtifactFailure);
        assert!(path.exists());
    }

    #[test]
    fn abort_and_generation_or_revision_mismatch_leave_no_artifact() {
        let project = tempfile::tempdir().unwrap();
        let mut store = EvidenceStore::new();
        prepare(
            &mut store,
            project.path(),
            "aborted",
            "capture_browser_screenshot",
            Some("png"),
        );
        let pending_path = store.prepared_path("aborted").to_owned();
        store.abort_request("aborted");
        assert!(!pending_path.exists());

        prepare(
            &mut store,
            project.path(),
            "stale",
            "capture_browser_screenshot",
            Some("png"),
        );
        let error = store
            .commit_response(
                "stale",
                "old-generation",
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response(
                    "stale",
                    "capture_browser_screenshot",
                    target(),
                    Some("png"),
                    "artifact_data",
                    &png(1, 1),
                ),
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::MismatchedBrowserResponse);

        prepare(
            &mut store,
            project.path(),
            "navigation",
            "capture_browser_screenshot",
            Some("png"),
        );
        let mut response = response(
            "navigation",
            "capture_browser_screenshot",
            target(),
            Some("png"),
            "artifact_data",
            &png(1, 1),
        );
        response.result.insert(
            "page_context_revision".into(),
            Value::String("revision-2".into()),
        );
        let error = store
            .commit_response(
                "navigation",
                BROKER_GENERATION,
                "capture_browser_screenshot",
                &project.path().to_string_lossy(),
                CALLER,
                &target(),
                &response,
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::StaleBrowserTarget);
        assert!(!project.path().join(".teshi").exists());
    }

    #[test]
    fn console_config_scope_and_capture_generation_are_fail_closed() {
        let target_a = console_target("profile-a", 7, 42);
        let target_b = console_target("profile-b", 8, 52);
        let mut store = EvidenceStore::new();
        let invalid_level = store
            .prepare_console_capture(
                "invalid-level",
                BROKER_GENERATION,
                "project-a",
                CALLER,
                &target_a,
                Some(7),
                Some(&json!(["trace"])),
                None,
                None,
                None,
                None,
            )
            .unwrap_err();
        assert_eq!(invalid_level.code, BrokerErrorCode::InvalidBrowserOperation);
        for (request_id, field, value) in [
            (
                "age-too-large",
                "max_age_ms",
                json!(MAX_CONSOLE_MAX_AGE_MS + 1),
            ),
            (
                "entries-too-large",
                "max_entries",
                json!(MAX_CONSOLE_MAX_ENTRIES + 1),
            ),
            (
                "bytes-too-large",
                "max_bytes",
                json!(MAX_CONSOLE_MAX_BYTES + 1),
            ),
        ] {
            let error = store
                .prepare_console_capture(
                    request_id,
                    BROKER_GENERATION,
                    "project-a",
                    CALLER,
                    &target_a,
                    Some(7),
                    None,
                    (field == "max_age_ms").then_some(&value),
                    (field == "max_entries").then_some(&value),
                    (field == "max_bytes").then_some(&value),
                    None,
                )
                .unwrap_err();
            assert_eq!(error.code, BrokerErrorCode::InvalidBrowserOperation);
        }

        let capture_a = start_console(
            &mut store,
            "capture-a",
            &target_a,
            Some(7),
            Some(json!(["error"])),
            None,
            None,
            None,
        );
        assert!(store.console_capture_scope_matches(
            &target_a,
            BROKER_GENERATION,
            "project-a",
            CALLER
        ));
        assert!(!store.console_capture_scope_matches(
            &target_a,
            BROKER_GENERATION,
            "project-b",
            CALLER
        ));
        assert!(!store.record_console_event(
            "profile-b",
            &target_b,
            Some(&capture_a),
            Some(7),
            Some(&json!({"level": "error", "text": "cross-profile"})),
        ));
        assert!(!store.record_console_event(
            "profile-a",
            &target_a,
            Some(&capture_a),
            Some(6),
            Some(&json!({"level": "error", "text": "old-generation"})),
        ));
        assert!(store.record_console_event(
            "profile-a",
            &target_a,
            Some(&capture_a),
            Some(7),
            Some(&json!({"level": "error", "text": "kept"})),
        ));
        assert_eq!(
            store
                .list_console_events(&target_a, Some(&json!(["info"])), None, None, None)
                .unwrap_err()
                .code,
            BrokerErrorCode::InvalidBrowserOperation
        );

        let capture_b = start_console(
            &mut store,
            "capture-b",
            &target_a,
            Some(8),
            Some(json!(["error"])),
            None,
            None,
            None,
        );
        assert_ne!(capture_a, capture_b);
        assert_eq!(store.terminated_console_capture_count(), 1);
        assert!(!store.record_console_event(
            "profile-a",
            &target_a,
            Some(&capture_a),
            Some(7),
            Some(&json!({"level": "error", "text": "late-old-capture"})),
        ));
        assert!(store.record_console_event(
            "profile-a",
            &target_a,
            Some(&capture_b),
            Some(8),
            Some(&json!({"level": "error", "text": "new-capture"})),
        ));
    }

    #[test]
    fn console_events_are_redacted_truncated_filtered_and_bounded() {
        let target = target();
        let mut store = EvidenceStore::new();
        let capture_id = start_console(
            &mut store,
            "bounded",
            &target,
            Some(11),
            Some(json!(["error"])),
            Some(json!(1_000)),
            Some(json!(2)),
            Some(json!(1_024)),
        );
        let sensitive = json!({
            "timestamp_ms": i64::MAX,
            "level": "error",
            "text": format!(
                "token: \"page-secret\" Authorization: Bearer real-secret url=https://user:pass@example.test/path?token=abc#frag {}",
                "x".repeat(MAX_CONSOLE_EVENT_TEXT_BYTES + 100)
            ),
            "source": "sensitive-source",
            "url": "https://user:pass@example.test/path?token=abc#frag",
            "line_number": 7,
        });
        assert!(store.record_console_event(
            "profile-a",
            &target,
            Some(&capture_id),
            Some(11),
            Some(&sensitive),
        ));
        let redacted = store
            .list_console_events(&target, None, None, None, None)
            .unwrap();
        let redacted_event = &redacted["events"][0];
        assert!(!redacted_event.to_string().contains("page-secret"));
        assert!(!redacted_event.to_string().contains("real-secret"));
        assert!(!redacted_event.to_string().contains("token=abc"));
        assert!(!redacted_event.to_string().contains("user:pass"));
        assert!(!store.record_console_event(
            "profile-a",
            &target,
            Some(&capture_id),
            Some(11),
            Some(&json!({"level": "info", "text": "filtered"})),
        ));
        for index in 0..3 {
            assert!(store.record_console_event(
                "profile-a",
                &target,
                Some(&capture_id),
                Some(11),
                Some(&json!({
                    "timestamp_ms": -1,
                    "level": "error",
                    "text": format!(
                        "event-{index}-{}",
                        "y".repeat(if index == 0 { 2_000 } else { 700 })
                    ),
                })),
            ));
        }
        let listed = store
            .list_console_events(&target, None, None, None, None)
            .unwrap();
        assert!(listed["retained_entries"].as_u64().unwrap() <= 2);
        assert!(listed["retained_bytes"].as_u64().unwrap() <= 1_024);
        assert!(listed["diagnostics"]["evicted_bytes"].as_u64().unwrap() > 0);
        assert!(listed["diagnostics"]["truncated_events"].as_u64().unwrap() > 0);
        assert!(listed["events"].as_array().unwrap().iter().all(|event| {
            let encoded = serde_json::to_vec(event).unwrap();
            encoded.len() <= MAX_CONSOLE_EVENT_BYTES.min(1_024)
                && event["timestamp_ms"].as_u64().unwrap() <= unix_ms()
                && !event.to_string().contains("page-secret")
                && !event.to_string().contains("real-secret")
                && !event.to_string().contains("token=abc")
                && !event.to_string().contains("user:pass")
        }));

        let count_target = console_target("profile-count", 9, 98);
        let count_capture = start_console(
            &mut store,
            "count",
            &count_target,
            Some(13),
            None,
            None,
            Some(json!(2)),
            Some(json!(MAX_CONSOLE_MAX_BYTES)),
        );
        for index in 0..3 {
            assert!(store.record_console_event(
                "profile-count",
                &count_target,
                Some(&count_capture),
                Some(13),
                Some(&json!({"level": "log", "text": format!("count-{index}")})),
            ));
        }
        let count_list = store
            .list_console_events(&count_target, None, None, None, None)
            .unwrap();
        assert_eq!(count_list["retained_entries"], 2);
        assert!(
            count_list["diagnostics"]["evicted_entries"]
                .as_u64()
                .unwrap()
                > 0
        );

        let age_capture = start_console(
            &mut store,
            "age",
            &console_target("profile-age", 9, 99),
            Some(12),
            None,
            Some(json!(1_000)),
            None,
            None,
        );
        let age_target = console_target("profile-age", 9, 99);
        assert!(store.record_console_event(
            "profile-age",
            &age_target,
            Some(&age_capture),
            Some(12),
            Some(&json!({"level": "log", "text": "old"})),
        ));
        store
            .console_captures
            .get_mut(&age_target)
            .unwrap()
            .events
            .front_mut()
            .unwrap()
            .received_at = Instant::now() - Duration::from_secs(2);
        let age_list = store
            .list_console_events(&age_target, None, None, None, None)
            .unwrap();
        assert_eq!(age_list["retained_entries"], 0);
        assert!(age_list["diagnostics"]["evicted_age"].as_u64().unwrap() > 0);
    }

    #[test]
    fn console_termination_diagnostics_are_structured_bounded_and_redacted() {
        let target = target();
        let mut store = EvidenceStore::new();
        let capture_id = start_console(
            &mut store,
            "termination",
            &target,
            Some(21),
            None,
            None,
            None,
            None,
        );
        assert!(store.record_console_termination_event(
            "profile-a",
            &target,
            Some(&capture_id),
            Some(21),
            "debugger_detached",
            Some("token=secret https://example.test/?password=hidden"),
        ));
        assert_eq!(store.active_console_capture_count(), 0);
        assert_eq!(store.terminated_console_capture_count(), 1);

        let reconnect_target = console_target("profile-reconnect", 9, 97);
        let reconnect_capture = start_console(
            &mut store,
            "reconnect",
            &reconnect_target,
            Some(22),
            None,
            None,
            None,
            None,
        );
        assert!(store.record_console_termination_event(
            "profile-reconnect",
            &reconnect_target,
            Some(&reconnect_capture),
            None,
            "stream_disconnected",
            Some("transport closed"),
        ));

        for reason in [
            "explicit_stop",
            "capture_replaced",
            "target_closed",
            "debugger_detached",
            "session_disconnected",
            "lease_expired",
            "lease_released",
            "already_stopped",
            "navigation",
            "broker_restart",
            "stream_disconnected",
            "extension_reported",
        ] {
            assert_eq!(
                normalize_console_termination_reason(reason),
                if reason == "extension_reported" {
                    "extension_reported"
                } else {
                    reason
                }
            );
        }
        let safe = termination_value(
            "debugger_detached",
            Some(&format!("{} token=secret", "z".repeat(10_000))),
        );
        assert!(safe["detail"].as_str().unwrap().len() <= 256);
        assert!(!safe.to_string().contains("token=secret"));
    }
}
