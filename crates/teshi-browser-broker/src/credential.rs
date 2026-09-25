//! Private per-user bearer credential storage.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::protocol::{
    BROWSER_BROKER_SCHEMA_VERSION, BrokerError, BrokerErrorCode, BrokerIdentityChallenge,
    BrokerIdentityProof, EndpointRecord, MAX_TRUSTED_EXTENSION_ORIGINS,
};

const CREDENTIAL_FILE_NAME: &str = "credential.json";
const MAX_CREDENTIAL_FILE_BYTES: u64 = 8 * 1024;
const IDENTITY_PROOF_DOMAIN: &[u8] = b"teshi-browser-broker-identity-proof-v1\0";
const IDENTITY_NONCE_BYTES: usize = 32;
type HmacSha256 = Hmac<Sha256>;

/// Private record binding the secret to one exact broker generation.
///
/// Deliberately does not implement `Debug`; callers should not accidentally log
/// the bearer token while reporting broker state.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateBrokerCredential {
    pub schema_version: u16,
    pub protocol_version: u16,
    pub broker_pid: u32,
    pub broker_start_id: String,
    trusted_extension_origins: Vec<String>,
    token: String,
}

impl fmt::Debug for PrivateBrokerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateBrokerCredential")
            .field("schema_version", &self.schema_version)
            .field("protocol_version", &self.protocol_version)
            .field("broker_pid", &self.broker_pid)
            .field("broker_start_id", &self.broker_start_id)
            .field("trusted_extension_origins", &self.trusted_extension_origins)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl PrivateBrokerCredential {
    /// Bind one server-generated token to the broker's public generation record.
    pub fn for_endpoint(
        endpoint: &EndpointRecord,
        token: impl Into<String>,
        mut trusted_extension_origins: Vec<String>,
    ) -> Result<Self, BrokerError> {
        trusted_extension_origins.sort();
        let credential = Self {
            schema_version: endpoint.schema_version,
            protocol_version: endpoint.protocol_version,
            broker_pid: endpoint.broker_pid,
            broker_start_id: endpoint.broker_start_id.clone(),
            trusted_extension_origins,
            token: token.into(),
        };
        credential.validate()?;
        Ok(credential)
    }

    /// Return the secret only to the authenticated local Teshi client path.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Return the exact extension origin authorized to receive the token.
    pub fn trusted_extension_origins(&self) -> &[String] {
        &self.trusted_extension_origins
    }

    /// Require the private credential and public endpoint to identify one exact
    /// schema, protocol, PID, and start generation before client authentication.
    pub fn validate_endpoint(&self, endpoint: &EndpointRecord) -> Result<(), BrokerError> {
        self.validate()?;
        if self.schema_version != endpoint.schema_version
            || self.protocol_version != endpoint.protocol_version
            || self.broker_pid != endpoint.broker_pid
            || self.broker_start_id != endpoint.broker_start_id
        {
            return Err(BrokerError::new(
                BrokerErrorCode::IncompatibleBrowserSession,
                "private broker credential belongs to a different process generation",
            ));
        }
        Ok(())
    }

    /// Verify a one-shot proof without disclosing the bearer token to the listener.
    pub fn verify_identity_proof(
        &self,
        challenge: &BrokerIdentityChallenge,
        response: &BrokerIdentityProof,
        endpoint: &EndpointRecord,
    ) -> Result<(), BrokerError> {
        self.validate_endpoint(endpoint)?;
        if response.schema_version != endpoint.schema_version
            || response.protocol_version != endpoint.protocol_version
            || response.broker_pid != endpoint.broker_pid
            || response.broker_start_id != endpoint.broker_start_id
            || response.nonce != challenge.nonce
            || response.proof.len() > 128
        {
            return Err(identity_authentication_error());
        }
        let nonce = decode_identity_nonce(&challenge.nonce)?;
        let proof = URL_SAFE_NO_PAD
            .decode(&response.proof)
            .map_err(|_| identity_authentication_error())?;
        let mac = identity_mac(
            &self.token,
            endpoint.schema_version,
            endpoint.protocol_version,
            endpoint.broker_pid,
            &endpoint.broker_start_id,
            &nonce,
        );
        mac.verify_slice(&proof)
            .map_err(|_| identity_authentication_error())
    }

    fn validate(&self) -> Result<(), BrokerError> {
        let token_is_url_safe = self.token.len() >= 32
            && self
                .token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
        if self.schema_version != BROWSER_BROKER_SCHEMA_VERSION
            || self.protocol_version == 0
            || self.protocol_version > crate::protocol::BROWSER_BROKER_PROTOCOL_VERSION
            || self.broker_pid == 0
            || self.broker_start_id.is_empty()
            || self.broker_start_id.len() > 128
            || self.trusted_extension_origins.is_empty()
            || self.trusted_extension_origins.len() > MAX_TRUSTED_EXTENSION_ORIGINS
            || self
                .trusted_extension_origins
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .trusted_extension_origins
                .iter()
                .any(|origin| !valid_extension_origin(origin))
            || !token_is_url_safe
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "private broker credential record is malformed",
            ));
        }
        Ok(())
    }
}

/// Create the nonce-bound response served by the local identity endpoint.
pub(crate) fn create_identity_proof(
    token: &str,
    schema_version: u16,
    protocol_version: u16,
    broker_pid: u32,
    broker_start_id: &str,
    nonce: &str,
) -> Result<BrokerIdentityProof, BrokerError> {
    if schema_version == 0
        || protocol_version == 0
        || broker_pid == 0
        || broker_start_id.is_empty()
        || broker_start_id.len() > 128
    {
        return Err(identity_authentication_error());
    }
    let nonce_bytes = decode_identity_nonce(nonce)?;
    let mac = identity_mac(
        token,
        schema_version,
        protocol_version,
        broker_pid,
        broker_start_id,
        &nonce_bytes,
    );
    let proof = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(BrokerIdentityProof {
        schema_version,
        protocol_version,
        broker_pid,
        broker_start_id: broker_start_id.to_owned(),
        nonce: nonce.to_owned(),
        proof,
    })
}

fn decode_identity_nonce(nonce: &str) -> Result<Vec<u8>, BrokerError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(nonce)
        .map_err(|_| identity_authentication_error())?;
    if decoded.len() != IDENTITY_NONCE_BYTES || URL_SAFE_NO_PAD.encode(&decoded) != nonce {
        return Err(identity_authentication_error());
    }
    Ok(decoded)
}

fn identity_mac(
    token: &str,
    schema_version: u16,
    protocol_version: u16,
    broker_pid: u32,
    broker_start_id: &str,
    nonce: &[u8],
) -> HmacSha256 {
    let mut mac =
        HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(IDENTITY_PROOF_DOMAIN);
    mac.update(&schema_version.to_be_bytes());
    mac.update(&protocol_version.to_be_bytes());
    mac.update(&broker_pid.to_be_bytes());
    mac.update(&(broker_start_id.len() as u16).to_be_bytes());
    mac.update(broker_start_id.as_bytes());
    mac.update(nonce);
    mac
}

fn identity_authentication_error() -> BrokerError {
    BrokerError::new(
        BrokerErrorCode::BrokerAuthenticationFailed,
        "local listener did not prove the private broker identity",
    )
}

fn valid_extension_origin(origin: &str) -> bool {
    let Some(id) = origin.strip_prefix("chrome-extension://") else {
        return false;
    };
    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
}

/// Credential file beneath the current OS user's Teshi browser-broker state dir.
/// On Unix the directory and file are owner-only. On Windows the file inherits
/// the ACL of the user's Local AppData directory.
#[derive(Debug, Clone)]
pub struct PrivateCredentialStore {
    state_dir: PathBuf,
}

impl PrivateCredentialStore {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.state_dir.join(CREDENTIAL_FILE_NAME)
    }

    /// Persist a token through a same-directory atomic rename without writing it
    /// into a project endpoint. A random temporary name avoids cross-process
    /// collisions during broker startup races.
    pub fn write(&self, credential: &PrivateBrokerCredential) -> Result<(), BrokerError> {
        credential.validate()?;
        create_owner_only_state_dir(&self.state_dir)?;

        let path = self.path();
        let temp = self.state_dir.join(format!(
            ".{CREDENTIAL_FILE_NAME}.{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let bytes = serde_json::to_vec(credential).map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "cannot serialize private broker credential",
            )
        })?;
        let result = write_private_temp(&temp, &bytes).and_then(|()| {
            fs::rename(&temp, &path).map_err(|error| {
                BrokerError::new(
                    BrokerErrorCode::BrowserArtifactFailure,
                    format!("cannot atomically install private broker credential: {error}"),
                )
            })
        });
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        set_owner_only_file_permissions(&path)?;
        Ok(())
    }

    /// Load a bounded credential and require it to belong to this exact endpoint.
    pub fn read_for_endpoint(
        &self,
        endpoint: &EndpointRecord,
    ) -> Result<PrivateBrokerCredential, BrokerError> {
        let path = self.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            BrokerError::new(
                BrokerErrorCode::BrokerAuthenticationFailed,
                format!("private broker credential is unavailable: {error}"),
            )
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_CREDENTIAL_FILE_BYTES
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerAuthenticationFailed,
                "private broker credential is not a bounded regular file",
            ));
        }
        let file = OpenOptions::new().read(true).open(&path).map_err(|error| {
            BrokerError::new(
                BrokerErrorCode::BrokerAuthenticationFailed,
                format!("cannot read private broker credential: {error}"),
            )
        })?;
        let opened_metadata = file.metadata().map_err(|error| {
            BrokerError::new(
                BrokerErrorCode::BrokerAuthenticationFailed,
                format!("cannot inspect private broker credential: {error}"),
            )
        })?;
        if !opened_metadata.is_file() || opened_metadata.len() > MAX_CREDENTIAL_FILE_BYTES {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerAuthenticationFailed,
                "private broker credential is not a bounded regular file",
            ));
        }
        let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
        file.take(MAX_CREDENTIAL_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                BrokerError::new(
                    BrokerErrorCode::BrokerAuthenticationFailed,
                    format!("cannot read private broker credential: {error}"),
                )
            })?;
        if bytes.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerAuthenticationFailed,
                "private broker credential exceeds the file size limit",
            ));
        }
        let credential: PrivateBrokerCredential = serde_json::from_slice(&bytes).map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::BrokerAuthenticationFailed,
                "private broker credential is malformed",
            )
        })?;
        credential.validate_endpoint(endpoint)?;
        Ok(credential)
    }
}

fn write_private_temp(path: &Path, bytes: &[u8]) -> Result<(), BrokerError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        BrokerError::new(
            BrokerErrorCode::BrowserArtifactFailure,
            format!("cannot create private broker credential temp file: {error}"),
        )
    })?;
    file.write_all(bytes).map_err(|error| {
        BrokerError::new(
            BrokerErrorCode::BrowserArtifactFailure,
            format!("cannot write private broker credential: {error}"),
        )
    })?;
    file.sync_all().map_err(|error| {
        BrokerError::new(
            BrokerErrorCode::BrowserArtifactFailure,
            format!("cannot flush private broker credential: {error}"),
        )
    })
}

#[cfg(unix)]
fn create_owner_only_state_dir(path: &Path) -> Result<(), BrokerError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).recursive(true);
    builder.create(path).map_err(|error| {
        BrokerError::new(
            BrokerErrorCode::BrowserArtifactFailure,
            format!("cannot create private broker state directory: {error}"),
        )
    })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        BrokerError::new(
            BrokerErrorCode::BrowserArtifactFailure,
            format!("cannot restrict private broker state directory: {error}"),
        )
    })
}

#[cfg(not(unix))]
fn create_owner_only_state_dir(path: &Path) -> Result<(), BrokerError> {
    fs::create_dir_all(path).map_err(|error| {
        BrokerError::new(
            BrokerErrorCode::BrowserArtifactFailure,
            format!("cannot create private broker state directory: {error}"),
        )
    })
}

#[cfg(unix)]
fn set_owner_only_file_permissions(path: &Path) -> Result<(), BrokerError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        BrokerError::new(
            BrokerErrorCode::BrowserArtifactFailure,
            format!("cannot restrict private broker credential file: {error}"),
        )
    })
}

#[cfg(not(unix))]
fn set_owner_only_file_permissions(_path: &Path) -> Result<(), BrokerError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> EndpointRecord {
        EndpointRecord {
            schema_version: BROWSER_BROKER_SCHEMA_VERSION,
            protocol_version: crate::protocol::BROWSER_BROKER_PROTOCOL_VERSION,
            mode: "chrome".into(),
            ws_url: "ws://127.0.0.1:43123/".into(),
            discovery_url: "http://127.0.0.1:17373/v1/bridge".into(),
            extension_frame_ws_url: "ws://127.0.0.1:43123/extension/frames".into(),
            broker_pid: 42,
            broker_start_id: "broker-generation-1".into(),
            broker_features: Vec::new(),
            bridge: "rust".into(),
        }
    }

    #[test]
    fn credential_is_private_and_bound_to_endpoint_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = PrivateCredentialStore::new(temp.path().join("teshi").join("browser-broker"));
        let endpoint = endpoint();
        let credential = PrivateBrokerCredential::for_endpoint(
            &endpoint,
            "A".repeat(43),
            vec!["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()],
        )
        .unwrap();
        store.write(&credential).unwrap();

        let loaded = store.read_for_endpoint(&endpoint).unwrap();
        assert_eq!(loaded.token(), "A".repeat(43));
        let mut stale = endpoint.clone();
        stale.broker_start_id = "broker-generation-2".into();
        assert_eq!(
            store.read_for_endpoint(&stale).unwrap_err().code,
            BrokerErrorCode::IncompatibleBrowserSession
        );
        let mut reused_pid = endpoint.clone();
        reused_pid.broker_pid += 1;
        assert_eq!(
            store.read_for_endpoint(&reused_pid).unwrap_err().code,
            BrokerErrorCode::IncompatibleBrowserSession
        );

        let public_record = serde_json::to_string(&endpoint).unwrap();
        assert!(!public_record.contains(loaded.token()));
        assert!(!store.path().to_string_lossy().contains("project"));
    }

    #[test]
    fn credential_store_rejects_oversized_and_symlink_files() {
        let temp = tempfile::tempdir().unwrap();
        let store = PrivateCredentialStore::new(temp.path().join("state"));
        fs::create_dir_all(temp.path().join("state")).unwrap();
        fs::write(
            store.path(),
            vec![b'x'; MAX_CREDENTIAL_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert_eq!(
            store.read_for_endpoint(&endpoint()).unwrap_err().code,
            BrokerErrorCode::BrokerAuthenticationFailed
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = temp.path().join("credential-target.json");
            fs::write(&target, b"{}").unwrap();
            fs::remove_file(store.path()).unwrap();
            symlink(&target, store.path()).unwrap();
            assert_eq!(
                store.read_for_endpoint(&endpoint()).unwrap_err().code,
                BrokerErrorCode::BrokerAuthenticationFailed
            );
        }
    }

    #[test]
    fn trusted_origins_are_canonical_bounded_and_unique() {
        let endpoint = endpoint();
        let first = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let second = "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let credential = PrivateBrokerCredential::for_endpoint(
            &endpoint,
            "C".repeat(43),
            vec![second.into(), first.into()],
        )
        .unwrap();
        assert_eq!(
            credential.trusted_extension_origins(),
            &[first.to_owned(), second.to_owned()]
        );

        assert!(
            PrivateBrokerCredential::for_endpoint(
                &endpoint,
                "C".repeat(43),
                vec![first.into(), first.into()],
            )
            .is_err()
        );
        assert!(
            PrivateBrokerCredential::for_endpoint(
                &endpoint,
                "C".repeat(43),
                (0..=MAX_TRUSTED_EXTENSION_ORIGINS)
                    .map(|index| {
                        let first = char::from(b'a' + (index / 16) as u8);
                        let second = char::from(b'a' + (index % 16) as u8);
                        format!("chrome-extension://{first}{second}{}", "a".repeat(30))
                    })
                    .collect(),
            )
            .is_err()
        );
    }

    #[test]
    fn broker_identity_proof_is_nonce_and_generation_bound() {
        let endpoint = endpoint();
        let trusted_origins = vec!["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()];
        let credential =
            PrivateBrokerCredential::for_endpoint(&endpoint, "D".repeat(43), trusted_origins)
                .unwrap();
        let challenge = BrokerIdentityChallenge {
            nonce: URL_SAFE_NO_PAD.encode([9u8; IDENTITY_NONCE_BYTES]),
        };
        let proof = create_identity_proof(
            credential.token(),
            endpoint.schema_version,
            endpoint.protocol_version,
            endpoint.broker_pid,
            &endpoint.broker_start_id,
            &challenge.nonce,
        )
        .unwrap();
        credential
            .verify_identity_proof(&challenge, &proof, &endpoint)
            .unwrap();

        let mut changed_nonce = challenge.clone();
        changed_nonce.nonce = URL_SAFE_NO_PAD.encode([8u8; IDENTITY_NONCE_BYTES]);
        assert!(
            credential
                .verify_identity_proof(&changed_nonce, &proof, &endpoint)
                .is_err()
        );

        let mut changed_generation = endpoint.clone();
        changed_generation.broker_start_id.push_str("-old");
        assert!(
            credential
                .verify_identity_proof(&challenge, &proof, &changed_generation)
                .is_err()
        );

        let wrong_key_proof = create_identity_proof(
            &"E".repeat(43),
            endpoint.schema_version,
            endpoint.protocol_version,
            endpoint.broker_pid,
            &endpoint.broker_start_id,
            &challenge.nonce,
        )
        .unwrap();
        assert!(
            credential
                .verify_identity_proof(&challenge, &wrong_key_proof, &endpoint)
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn credential_directory_and_file_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let store = PrivateCredentialStore::new(temp.path().join("private-state"));
        let endpoint = endpoint();
        let credential = PrivateBrokerCredential::for_endpoint(
            &endpoint,
            "B".repeat(43),
            vec!["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()],
        )
        .unwrap();
        store.write(&credential).unwrap();
        assert_eq!(
            fs::metadata(&temp.path().join("private-state"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
