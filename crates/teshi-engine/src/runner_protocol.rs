//! Runner protocol v1: the handshake and capability contract between Teshi and
//! an executor.
//!
//! Teshi writes one request line carrying [`RUNNER_PROTOCOL_VERSION`] to the
//! runner's stdin. The runner's first protocol event on stdout must be a
//! `start_run` that reports the same `protocol_version` and the
//! `capabilities` it uses. Events outside the declared capabilities are
//! protocol errors. There is no fallback for runners that skip the handshake.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::{json, Value};

/// The only runner protocol version this build speaks.
pub const RUNNER_PROTOCOL_VERSION: u64 = 1;

/// Optional event families a runner may emit after declaring them.
///
/// Case-level events (`start_case`, `case_passed`, `case_failed`,
/// `case_skipped`), `log` and `end_run` are always allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunnerCapability {
    /// `start_step` and `end_step`.
    Steps,
    /// `artifact`.
    Artifacts,
    /// `case_blocked`, for scenarios whose external target is unavailable.
    Blocked,
    /// `http_exchange`.
    HttpExchange,
}

impl RunnerCapability {
    /// Wire name used in the `capabilities` array.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steps => "steps",
            Self::Artifacts => "artifacts",
            Self::Blocked => "blocked",
            Self::HttpExchange => "http_exchange",
        }
    }

    /// Parses a wire name; unknown names return `None` so newer runners can
    /// advertise capabilities this build does not use.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "steps" => Some(Self::Steps),
            "artifacts" => Some(Self::Artifacts),
            "blocked" => Some(Self::Blocked),
            "http_exchange" => Some(Self::HttpExchange),
            _ => None,
        }
    }

    /// Capability an event type requires, or `None` for always-allowed and
    /// unknown event types.
    #[must_use]
    pub fn required_by(event_type: &str) -> Option<Self> {
        match event_type {
            "start_step" | "end_step" => Some(Self::Steps),
            "artifact" => Some(Self::Artifacts),
            "case_blocked" => Some(Self::Blocked),
            "http_exchange" => Some(Self::HttpExchange),
            _ => None,
        }
    }
}

/// Why a runner's output violates protocol v1.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunnerProtocolError {
    /// The first protocol event was not `start_run`.
    #[error(
        "runner did not complete the v1 handshake: first event was `{found}`, expected `start_run`"
    )]
    FirstEventNotStartRun {
        /// Type of the event that arrived first.
        found: String,
    },
    /// `start_run` omitted `protocol_version`.
    #[error("runner did not complete the v1 handshake: `start_run` has no `protocol_version`")]
    MissingVersion,
    /// `start_run` reported a version this build does not speak.
    #[error("unsupported runner protocol version {0}; this Teshi build speaks version {RUNNER_PROTOCOL_VERSION}")]
    UnsupportedVersion(String),
    /// `start_run` omitted the `capabilities` array.
    #[error("runner did not complete the v1 handshake: `start_run` has no `capabilities` array")]
    MissingCapabilities,
    /// The runner emitted an event family it did not declare.
    #[error("runner emitted `{event}` without declaring the `{}` capability", capability.as_str())]
    UndeclaredCapability {
        /// Offending event type.
        event: String,
        /// Capability the event requires.
        capability: RunnerCapability,
    },
}

/// A runner's accepted v1 handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerHandshake {
    capabilities: BTreeSet<RunnerCapability>,
}

impl RunnerHandshake {
    /// Validates the runner's first protocol event.
    ///
    /// # Errors
    /// Returns [`RunnerProtocolError`] when the event is not a v1 `start_run`
    /// with a `capabilities` array.
    pub fn from_first_event(event: &Value) -> Result<Self, RunnerProtocolError> {
        let kind = event["type"].as_str().unwrap_or_default();
        if kind != "start_run" {
            return Err(RunnerProtocolError::FirstEventNotStartRun {
                found: kind.to_string(),
            });
        }
        match event.get("protocol_version") {
            None | Some(Value::Null) => return Err(RunnerProtocolError::MissingVersion),
            Some(version) if version.as_u64() == Some(RUNNER_PROTOCOL_VERSION) => {}
            Some(version) => {
                return Err(RunnerProtocolError::UnsupportedVersion(version.to_string()))
            }
        }
        let names = event["capabilities"]
            .as_array()
            .ok_or(RunnerProtocolError::MissingCapabilities)?;
        let capabilities = names
            .iter()
            .filter_map(Value::as_str)
            .filter_map(RunnerCapability::parse)
            .collect();
        Ok(Self { capabilities })
    }

    /// Whether the runner declared `capability`.
    #[must_use]
    pub fn has(&self, capability: RunnerCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Declared capabilities in wire form, for evidence and diagnostics.
    #[must_use]
    pub fn capability_names(&self) -> Vec<&'static str> {
        self.capabilities.iter().map(|c| c.as_str()).collect()
    }

    /// Checks an event that follows the handshake.
    ///
    /// # Errors
    /// Returns [`RunnerProtocolError::UndeclaredCapability`] when the event
    /// needs a capability the runner did not declare.
    pub fn check_event(&self, event: &Value) -> Result<(), RunnerProtocolError> {
        let kind = event["type"].as_str().unwrap_or_default();
        match RunnerCapability::required_by(kind) {
            Some(capability) if !self.has(capability) => {
                Err(RunnerProtocolError::UndeclaredCapability {
                    event: kind.to_string(),
                    capability,
                })
            }
            _ => Ok(()),
        }
    }
}

impl fmt::Display for RunnerHandshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "protocol v{RUNNER_PROTOCOL_VERSION} [{}]",
            self.capability_names().join(", ")
        )
    }
}

/// Builds the handshake `start_run` event a v1 runner emits first.
#[must_use]
pub fn start_run_event(total: usize, capabilities: &[RunnerCapability]) -> Value {
    json!({
        "type": "start_run",
        "total": total,
        "protocol_version": RUNNER_PROTOCOL_VERSION,
        "capabilities": capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake_accepts_v1_start_run_and_ignores_unknown_capabilities() {
        let handshake = RunnerHandshake::from_first_event(&json!({
            "type": "start_run", "protocol_version": 1, "capabilities": ["steps", "future"]
        }))
        .expect("valid handshake");
        assert!(handshake.has(RunnerCapability::Steps));
        assert_eq!(handshake.capability_names(), vec!["steps"]);
    }

    #[test]
    fn test_handshake_rejections_name_the_violation() {
        let cases = [
            (
                json!({"type": "start_case", "case_id": "c"}),
                RunnerProtocolError::FirstEventNotStartRun {
                    found: "start_case".into(),
                },
            ),
            (
                json!({"type": "start_run", "capabilities": []}),
                RunnerProtocolError::MissingVersion,
            ),
            (
                json!({"type": "start_run", "protocol_version": 2, "capabilities": []}),
                RunnerProtocolError::UnsupportedVersion("2".into()),
            ),
            (
                json!({"type": "start_run", "protocol_version": "1", "capabilities": []}),
                RunnerProtocolError::UnsupportedVersion("\"1\"".into()),
            ),
            (
                json!({"type": "start_run", "protocol_version": 1}),
                RunnerProtocolError::MissingCapabilities,
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(
                RunnerHandshake::from_first_event(&event),
                Err(expected.clone()),
                "event {event}"
            );
        }
    }

    #[test]
    fn test_check_event_enforces_declared_capabilities() {
        let handshake =
            RunnerHandshake::from_first_event(&start_run_event(1, &[RunnerCapability::Steps]))
                .expect("helper emits a valid handshake");
        for allowed in [
            "start_case",
            "case_passed",
            "log",
            "end_run",
            "end_step",
            "planned_step",
        ] {
            assert!(
                handshake.check_event(&json!({"type": allowed})).is_ok(),
                "{allowed} should be allowed"
            );
        }
        assert_eq!(
            handshake.check_event(&json!({"type": "case_blocked"})),
            Err(RunnerProtocolError::UndeclaredCapability {
                event: "case_blocked".into(),
                capability: RunnerCapability::Blocked,
            })
        );
    }
}
