use std::{fmt, io, path::PathBuf, time::Duration};

use thiserror::Error;

pub type AcpResult<T> = Result<T, AcpError>;

#[derive(Debug, Error)]
pub enum AcpError {
    #[error("ACP executable not found: {program}")]
    ExecutableNotFound { program: String },
    #[error("configured ACP executable does not exist: {path}")]
    ConfiguredExecutableMissing { path: PathBuf },
    #[error("failed to spawn ACP agent `{program}`: {source}")]
    SpawnFailed { program: String, source: io::Error },
    #[error("ACP protocol decode failed: {0}")]
    ProtocolDecode(String),
    #[error("ACP protocol violation: {0}")]
    ProtocolViolation(String),
    #[error("unsupported ACP protocol version: {0}")]
    UnsupportedProtocolVersion(u32),
    #[error("ACP initialize failed: {0}")]
    InitializeFailed(String),
    #[error("ACP authentication is required; select an agent-advertised authentication method")]
    AuthenticationRequired,
    #[error("ACP authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("ACP capability unsupported: {0}")]
    CapabilityUnsupported(&'static str),
    #[error("ACP session failed: {0}")]
    SessionFailed(String),
    #[error("ACP permission failed: {0}")]
    PermissionFailed(String),
    #[error("ACP agent process exited")]
    ProcessExited,
    #[error("ACP operation cancelled")]
    Cancelled,
    #[error("ACP operation timed out after {0:?}")]
    Timeout(Duration),
    #[error("ACP prompt already running for this session")]
    PromptAlreadyRunning,
    #[error("ACP registry fetch failed: {0}")]
    RegistryFetch(String),
    #[error("ACP registry parse failed: {0}")]
    RegistryParse(String),
    #[error("ACP registry platform unsupported: {0}")]
    UnsupportedPlatform(String),
    #[error("ACP shutdown failed: {0}")]
    Shutdown(String),
    #[error("ACP invalid lifecycle transition: {from:?} -> {to:?}")]
    InvalidState {
        from: crate::client::AcpLifecycle,
        to: &'static str,
    },
}

/// Redacts values that are commonly used for ACP/Cursor secrets before they are
/// inserted into Display/Debug/error strings controlled by Teshi.
pub fn redact_secret(input: &str) -> String {
    let upper = input.to_ascii_uppercase();
    if upper.contains("CURSOR_API_KEY")
        || upper.contains("CURSOR_AUTH_TOKEN")
        || upper.contains("API_KEY=")
        || upper.contains("AUTH_TOKEN=")
        || upper.contains("TOKEN=")
    {
        return "[REDACTED]".to_string();
    }
    input.to_string()
}

#[derive(Clone, PartialEq, Eq)]
pub struct Redacted(pub String);

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&redact_secret(&self.0))
    }
}

impl fmt::Display for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&redact_secret(&self.0))
    }
}
