//! Target-scoped screenshot/PDF evidence validation and managed storage.
//!
//! The extension remains the owner of Chrome/CDP capture.  This module is the
//! broker-side boundary for the untrusted bytes returned by that capture: it
//! binds a response to the request context, validates the actual payload, and
//! publishes a broker-generated artifact without replacing an existing file.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::protocol::{BrokerError, BrokerErrorCode, BrowserTarget, ExtensionResponse};

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

/// Configuration shape reserved for the later Console capture stage.
///
/// Defining the DTO here keeps the single evidence ownership boundary without
/// advertising or implementing the 5.3 capture capability in this stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleCaptureConfig {
    #[serde(default = "default_capture_age_ms")]
    pub max_age_ms: u64,
    #[serde(default = "default_capture_entries")]
    pub max_entries: usize,
    #[serde(default = "default_capture_bytes")]
    pub max_bytes: usize,
}

impl Default for ConsoleCaptureConfig {
    fn default() -> Self {
        Self {
            max_age_ms: default_capture_age_ms(),
            max_entries: default_capture_entries(),
            max_bytes: default_capture_bytes(),
        }
    }
}

/// Configuration shape reserved for the later Network capture stage.
///
/// The network store and acknowledgement protocol remain outside 5.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkCaptureConfig {
    #[serde(default = "default_capture_age_ms")]
    pub max_age_ms: u64,
    #[serde(default = "default_capture_entries")]
    pub max_entries: usize,
    #[serde(default = "default_capture_bytes")]
    pub max_bytes: usize,
}

impl Default for NetworkCaptureConfig {
    fn default() -> Self {
        Self {
            max_age_ms: default_capture_age_ms(),
            max_entries: default_capture_entries(),
            max_bytes: default_capture_bytes(),
        }
    }
}

const fn default_capture_age_ms() -> u64 {
    300_000
}

const fn default_capture_entries() -> usize {
    1_000
}

const fn default_capture_bytes() -> usize {
    8 * 1024 * 1024
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

/// Single-owner evidence state.  `BrokerState` owns one instance and calls it
/// synchronously from the same event loop that owns requests and leases.
#[derive(Debug, Default)]
pub struct EvidenceStore {
    prepared: HashMap<String, PreparedRecord>,
    managed: HashMap<PathBuf, ManagedRecord>,
}

impl EvidenceStore {
    pub fn new() -> Self {
        Self::default()
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

    use super::*;

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
}
