//! Versioned wire types for the hosted Teshi Web UI.
//!
//! The daemon and WASM shell intentionally share only serialization contracts
//! here. Domain handlers and authorization remain owned by their respective
//! hosts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MANIFEST_SCHEMA_VERSION: u16 = 1;
pub const CONTROL_PROTOCOL_VERSION: u16 = 1;
pub const PREVIEW_PROTOCOL_VERSION: u16 = 1;

/// The application channel authenticated by a WebSocket hello.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Control,
    Preview,
}

/// Product identity sent by the daemon during protocol negotiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliBuildIdentity {
    pub semver: String,
    pub channel: String,
    pub git_sha: String,
    pub build_timestamp: String,
    pub build_sequence: u64,
}

/// Compatibility data carried by the UI hello and persisted in its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiCompatibility {
    pub manifest_schema: u16,
    pub ui_source_sha: String,
    pub minimum_cli: CliBuildIdentity,
    pub minimum_build_sequence: u64,
    pub control_protocol: u16,
    pub preview_protocol: u16,
}

/// First message sent by a browser WebSocket before any business message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientHello {
    pub token: String,
    pub channel: Channel,
    pub protocol_version: u16,
    pub ui: UiCompatibility,
}

/// Request envelope for the multiplexed control channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Structured error shared by handshake and request responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Stable machine-readable protocol failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidMessage,
    HandshakeRequired,
    HandshakeTimeout,
    InvalidToken,
    InvalidOrigin,
    WrongChannel,
    IncompatibleProtocol,
    IncompatibleCli,
    Forbidden,
    UnknownMethod,
    RequestFailed,
    SessionExpired,
    PreviewUnavailable,
}

/// Client-to-daemon messages. Tagged envelopes keep the first-message gate
/// explicit and leave room for future protocol message kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    ClientHello(ClientHello),
    Request(Request),
    Close,
}

/// Server-to-client messages for control and preview channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    ServerHello {
        daemon: CliBuildIdentity,
        channel: Channel,
        protocol_version: u16,
        session_id: String,
        capabilities: Vec<HostedCapability>,
    },
    Response {
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ProtocolError>,
    },
    Event {
        event: String,
        payload: Value,
    },
    Error(ProtocolError),
}

/// Explicit business operations available to the hosted UI.
///
/// This is deliberately not the daemon's `Admin` role. New operations must be
/// added here and reviewed before becoming reachable from the hosted origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedCapability {
    Project,
    Filesystem,
    Gherkin,
    BddRun,
    ApiExchange,
    Locator,
    Steps,
    LlmConfig,
    BrowserSessions,
    Terminal,
    Agent,
    RuntimeEvents,
}

/// Versioned manifest emitted alongside `/app/` by the Pages build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiManifest {
    pub manifest_schema: u16,
    pub ui_source_sha: String,
    pub deployment_id: String,
    pub minimum_cli: CliBuildIdentity,
    pub minimum_build_sequence: u64,
    pub control_protocol: u16,
    pub preview_protocol: u16,
    pub supported_channel: String,
    pub nightly_upgrade_url: String,
}

/// Release channel of source builds produced from a local checkout.
const LOCAL_CHANNEL: &str = "local";

impl UiCompatibility {
    /// Whether this UI may talk to a daemon with the given build identity.
    ///
    /// Schema and protocol versions must always match exactly. Builds on the
    /// manifest's channel must reach `minimum_build_sequence`. Local source
    /// builds are also accepted, but their sequence is a local Unix time that
    /// cannot be ordered against CI run identities, so they are gated on a
    /// build timestamp no older than the minimum CLI's. Any other channel
    /// (`stable`, `dev`) is rejected.
    pub fn supports(&self, daemon: &CliBuildIdentity) -> bool {
        let protocol_matches = self.manifest_schema == MANIFEST_SCHEMA_VERSION
            && self.control_protocol == CONTROL_PROTOCOL_VERSION
            && self.preview_protocol == PREVIEW_PROTOCOL_VERSION;
        if !protocol_matches || self.minimum_build_sequence == 0 || daemon.build_sequence == 0 {
            return false;
        }
        if daemon.channel == LOCAL_CHANNEL {
            return match (
                utc_timestamp(&daemon.build_timestamp),
                utc_timestamp(&self.minimum_cli.build_timestamp),
            ) {
                (Some(built), Some(minimum)) => built >= minimum,
                _ => false,
            };
        }
        daemon.build_sequence >= self.minimum_build_sequence
            && daemon.channel == self.minimum_cli.channel
    }
}

/// Accept only canonical `YYYY-MM-DDTHH:MM:SSZ` timestamps.
///
/// Both CI and local builds emit this exact shape, which makes lexical
/// comparison equivalent to chronological comparison.
fn utc_timestamp(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    let canonical = bytes.len() == 20
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        });
    canonical.then_some(value)
}

impl From<UiManifest> for UiCompatibility {
    fn from(manifest: UiManifest) -> Self {
        Self {
            manifest_schema: manifest.manifest_schema,
            ui_source_sha: manifest.ui_source_sha,
            minimum_cli: manifest.minimum_cli,
            minimum_build_sequence: manifest.minimum_build_sequence,
            control_protocol: manifest.control_protocol,
            preview_protocol: manifest.preview_protocol,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(sequence: u64) -> CliBuildIdentity {
        CliBuildIdentity {
            semver: "0.7.10".into(),
            channel: "nightly".into(),
            git_sha: "a".repeat(40),
            build_timestamp: "2026-09-09T00:00:00Z".into(),
            build_sequence: sequence,
        }
    }

    fn compatibility(minimum: u64) -> UiCompatibility {
        UiCompatibility {
            manifest_schema: MANIFEST_SCHEMA_VERSION,
            ui_source_sha: "b".repeat(40),
            minimum_cli: identity(minimum),
            minimum_build_sequence: minimum,
            control_protocol: CONTROL_PROTOCOL_VERSION,
            preview_protocol: PREVIEW_PROTOCOL_VERSION,
        }
    }

    #[test]
    fn hello_round_trips_as_tagged_json() {
        let message = ClientMessage::ClientHello(ClientHello {
            token: "tk_test".into(),
            channel: Channel::Control,
            protocol_version: CONTROL_PROTOCOL_VERSION,
            ui: compatibility(10),
        });
        let encoded = serde_json::to_string(&message).unwrap();
        assert!(encoded.contains(r#""type":"client_hello""#));
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&encoded).unwrap(),
            message
        );
    }

    #[test]
    fn compatibility_requires_supported_channel_and_nonzero_sequence() {
        assert!(compatibility(10).supports(&identity(10)));
        assert!(compatibility(10).supports(&identity(11)));
        assert!(!compatibility(0).supports(&identity(1)));
        assert!(!compatibility(10).supports(&identity(9)));
        assert!(!compatibility(10).supports(&identity(0)));
        let mut stable = identity(10);
        stable.channel = "stable".into();
        assert!(!compatibility(10).supports(&stable));
        let mut dev = identity(10);
        dev.channel = "dev".into();
        assert!(!compatibility(10).supports(&dev));
    }

    fn local(timestamp: &str, sequence: u64) -> CliBuildIdentity {
        CliBuildIdentity {
            channel: "local".into(),
            build_timestamp: timestamp.into(),
            ..identity(sequence)
        }
    }

    #[test]
    fn test_local_build_compatibility_uses_build_timestamp() {
        // The minimum CLI was built at 2026-09-09T00:00:00Z with sequence 10^10;
        // local sequences are Unix seconds and therefore always smaller.
        let ui = compatibility(10_000_000_000);
        let cases = [
            (
                "2026-11-11T07:52:00Z",
                1_794_383_520,
                true,
                "newer local build",
            ),
            ("2026-09-09T00:00:00Z", 1_788_912_000, true, "same instant"),
            (
                "2026-09-08T23:59:59Z",
                1_788_911_999,
                false,
                "older local build",
            ),
            ("2026-11-11T07:52:00Z", 0, false, "incomplete identity"),
            (
                "2026-11-11T15:52:00+08:00",
                1_794_383_520,
                false,
                "non-UTC form",
            ),
            ("", 1_794_383_520, false, "missing timestamp"),
        ];
        for (timestamp, sequence, expected, label) in cases {
            assert_eq!(
                ui.supports(&local(timestamp, sequence)),
                expected,
                "{label}: {timestamp}"
            );
        }
    }

    #[test]
    fn test_local_build_still_requires_matching_protocol() {
        let mut ui = compatibility(10);
        ui.control_protocol = CONTROL_PROTOCOL_VERSION + 1;
        assert!(!ui.supports(&local("2026-11-11T07:52:00Z", 1_794_383_520)));
    }

    #[test]
    fn response_preserves_request_id_and_error_shape() {
        let response = ServerMessage::Response {
            id: "r1".into(),
            ok: false,
            result: None,
            error: Some(ProtocolError {
                code: ErrorCode::Forbidden,
                message: "not allowed".into(),
                details: None,
            }),
        };
        let decoded: ServerMessage =
            serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn manifest_round_trips_into_the_hello_compatibility_contract() {
        let manifest = UiManifest {
            manifest_schema: MANIFEST_SCHEMA_VERSION,
            ui_source_sha: "b".repeat(40),
            deployment_id: "pages-42".into(),
            minimum_cli: identity(42),
            minimum_build_sequence: 42,
            control_protocol: CONTROL_PROTOCOL_VERSION,
            preview_protocol: PREVIEW_PROTOCOL_VERSION,
            supported_channel: "nightly".into(),
            nightly_upgrade_url: "https://github.com/teshi-org/teshi/releases".into(),
        };
        let decoded: UiManifest =
            serde_json::from_str(&serde_json::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(decoded, manifest);
        assert!(UiCompatibility::from(decoded).supports(&identity(42)));
    }
}
