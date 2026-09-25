//! User-scoped Rust transport and state owner for the Chrome extension bridge.
//!
//! The crate intentionally does not depend on `teshi-engine` or UI crates. CLI and
//! daemon entry points host it; protocol records here are the server-side wire
//! boundary shared with the extension and existing typed Rust client.

pub mod credential;
pub mod protocol;
pub mod server;
pub mod session;
pub mod state;

pub use credential::{PrivateBrokerCredential, PrivateCredentialStore};
pub use protocol::{
    BROWSER_BROKER_IDENTITY_CHALLENGE_PATH, BROWSER_BROKER_PROTOCOL_VERSION,
    BROWSER_BROKER_SCHEMA_VERSION, BrokerErrorCode, BrokerIdentityChallenge, BrokerIdentityProof,
    BrowserTarget, CHROME_DISCOVERY_PORT, DiscoveryResponse, EndpointRecord, ExtensionHeartbeat,
    ExtensionResponse, MAX_TRUSTED_EXTENSION_ORIGINS,
};
pub use server::{BrokerEvent, BrokerPublication, BrokerRuntime, BrokerServerConfig};
pub use session::{
    BrowserSessionRecord, DEFAULT_HEARTBEAT_TTL, DISCONNECTED_RETENTION, ELEMENT_REFERENCE_TTL,
    ElementReferenceRecord, LEGACY_INSTANCE_ID, MAX_ELEMENT_REFERENCES, PreviewFrameRecord,
    SessionHealth, SessionRegistry,
};
pub use state::BrokerState;
