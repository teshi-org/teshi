//! Durable, user-scoped trust for exact Chrome extension origins.
//!
//! Pairing records deliberately contain identity metadata only.  The
//! generation-bound bearer credential remains in `credential.json` and is
//! never part of this document.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::credential::{
    create_owner_only_state_dir, set_owner_only_file_permissions, valid_extension_origin,
    write_private_temp,
};
use crate::protocol::{
    BROWSER_BROKER_SCHEMA_VERSION, BrokerError, BrokerErrorCode, MAX_TRUSTED_EXTENSION_ORIGINS,
};

const PAIRING_FILE_NAME: &str = "pairing.json";
const MAX_PAIRING_FILE_BYTES: u64 = 64 * 1024;
const MAX_DISPLAY_NAME_BYTES: usize = 120;

/// A durable exact extension identity.  This record is not a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedExtensionOrigin {
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub created_at_ms: u64,
    pub last_seen_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingDocument {
    schema_version: u16,
    trusted_origins: Vec<TrustedExtensionOrigin>,
}

/// Result of an exact add/remove operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairingChange {
    pub changed: bool,
}

/// Atomic, bounded store beneath the per-user broker state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingStore {
    state_dir: PathBuf,
}

impl PairingStore {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn path(&self) -> PathBuf {
        self.state_dir.join(PAIRING_FILE_NAME)
    }

    /// Read the durable allowlist. Missing state means an unpaired broker;
    /// every other filesystem, schema, or validation error fails closed.
    pub fn load(&self) -> Result<Vec<TrustedExtensionOrigin>, BrokerError> {
        let path = self.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(store_error(format!(
                    "cannot inspect pairing store: {error}"
                )));
            }
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_PAIRING_FILE_BYTES
        {
            return Err(store_error("pairing store is not a bounded regular file"));
        }
        let file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|error| store_error(format!("cannot read pairing store: {error}")))?;
        let opened_metadata = file
            .metadata()
            .map_err(|error| store_error(format!("cannot inspect pairing store: {error}")))?;
        if !opened_metadata.is_file() || opened_metadata.len() > MAX_PAIRING_FILE_BYTES {
            return Err(store_error("pairing store is not a bounded regular file"));
        }
        let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
        file.take(MAX_PAIRING_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| store_error(format!("cannot read pairing store: {error}")))?;
        if bytes.len() as u64 > MAX_PAIRING_FILE_BYTES {
            return Err(store_error("pairing store exceeds its size limit"));
        }
        let document: PairingDocument = serde_json::from_slice(&bytes)
            .map_err(|_| store_error("pairing store is malformed"))?;
        validate_document(&document)
    }

    pub fn list(&self) -> Result<Vec<TrustedExtensionOrigin>, BrokerError> {
        self.load()
    }

    pub fn is_trusted(&self, origin: &str) -> Result<bool, BrokerError> {
        validate_origin(origin)?;
        Ok(self.load()?.iter().any(|record| record.origin == origin))
    }

    /// Add an exact origin without allowing a request body or environment
    /// override to write durable trust.
    pub fn add_exact(
        &self,
        origin: &str,
        display_name: Option<&str>,
    ) -> Result<PairingChange, BrokerError> {
        self.add_exact_at(origin, display_name, unix_ms())
    }

    pub fn add_exact_at(
        &self,
        origin: &str,
        display_name: Option<&str>,
        now_ms: u64,
    ) -> Result<PairingChange, BrokerError> {
        validate_origin(origin)?;
        let display_name = normalize_display_name(display_name)?;
        let mut records = self.load()?;
        if let Some(record) = records.iter_mut().find(|record| record.origin == origin) {
            let changed = record.display_name != display_name;
            record.display_name = display_name;
            record.last_seen_at_ms = now_ms;
            self.write(&records)?;
            return Ok(PairingChange { changed });
        }
        if records.len() >= MAX_TRUSTED_EXTENSION_ORIGINS {
            return Err(store_error(format!(
                "pairing store already contains the maximum of {MAX_TRUSTED_EXTENSION_ORIGINS} origins"
            )));
        }
        records.push(TrustedExtensionOrigin {
            origin: origin.to_owned(),
            display_name,
            created_at_ms: now_ms,
            last_seen_at_ms: now_ms,
        });
        records.sort_by(|left, right| left.origin.cmp(&right.origin));
        self.write(&records)?;
        Ok(PairingChange { changed: true })
    }

    pub fn remove_exact(&self, origin: &str) -> Result<PairingChange, BrokerError> {
        validate_origin(origin)?;
        let mut records = self.load()?;
        let before = records.len();
        records.retain(|record| record.origin != origin);
        if records.len() != before {
            self.write(&records)?;
        }
        Ok(PairingChange {
            changed: records.len() != before,
        })
    }

    /// Restore a previously loaded record set during a failed live update.
    pub(crate) fn replace(&self, records: &[TrustedExtensionOrigin]) -> Result<(), BrokerError> {
        let document = PairingDocument {
            schema_version: BROWSER_BROKER_SCHEMA_VERSION,
            trusted_origins: records.to_vec(),
        };
        validate_document(&document)?;
        self.write(records)
    }

    fn write(&self, records: &[TrustedExtensionOrigin]) -> Result<(), BrokerError> {
        let document = PairingDocument {
            schema_version: BROWSER_BROKER_SCHEMA_VERSION,
            trusted_origins: records.to_vec(),
        };
        validate_document(&document)?;
        ensure_private_state_dir(&self.state_dir)?;
        let path = self.path();
        let temp = self.state_dir.join(format!(
            ".{PAIRING_FILE_NAME}.{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let bytes = serde_json::to_vec(&document)
            .map_err(|_| store_error("cannot serialize pairing store"))?;
        let result = write_private_temp(&temp, &bytes).and_then(|()| {
            fs::rename(&temp, &path).map_err(|error| {
                store_error(format!("cannot atomically install pairing store: {error}"))
            })
        });
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        set_owner_only_file_permissions(&path)?;
        Ok(())
    }
}

pub(crate) fn validate_origin(origin: &str) -> Result<(), BrokerError> {
    if valid_extension_origin(origin) {
        Ok(())
    } else {
        Err(store_error(
            "pairing origin must be one exact chrome-extension:// origin",
        ))
    }
}

fn validate_document(
    document: &PairingDocument,
) -> Result<Vec<TrustedExtensionOrigin>, BrokerError> {
    if document.schema_version != BROWSER_BROKER_SCHEMA_VERSION
        || document.trusted_origins.len() > MAX_TRUSTED_EXTENSION_ORIGINS
    {
        return Err(store_error(
            "pairing store schema or entry limit is invalid",
        ));
    }
    let mut previous = None;
    for record in &document.trusted_origins {
        validate_origin(&record.origin)?;
        normalize_display_name(record.display_name.as_deref())?;
        if previous.is_some_and(|value: &str| value >= record.origin.as_str()) {
            return Err(store_error("pairing origins must be sorted and unique"));
        }
        previous = Some(record.origin.as_str());
    }
    Ok(document.trusted_origins.clone())
}

fn normalize_display_name(value: Option<&str>) -> Result<Option<String>, BrokerError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_DISPLAY_NAME_BYTES || value.chars().any(char::is_control) {
        return Err(store_error("pairing display name is invalid or too long"));
    }
    Ok(Some(value.to_owned()))
}

fn ensure_private_state_dir(path: &Path) -> Result<(), BrokerError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err(store_error(
            "pairing state path must be a real private directory",
        ));
    }
    create_owner_only_state_dir(path)
}

fn store_error(message: impl Into<String>) -> BrokerError {
    BrokerError::new(BrokerErrorCode::BrowserArtifactFailure, message)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SECOND: &str = "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn add_duplicate_remove_and_restart_persist_exact_identity_only() {
        let temp = tempfile::tempdir().unwrap();
        let store = PairingStore::new(temp.path());
        assert!(
            store
                .add_exact_at(FIRST, Some("QA account"), 10)
                .unwrap()
                .changed
        );
        assert!(
            !store
                .add_exact_at(FIRST, Some("QA account"), 11)
                .unwrap()
                .changed
        );
        assert_eq!(store.load().unwrap().len(), 1);
        let reloaded = PairingStore::new(temp.path());
        assert_eq!(reloaded.load().unwrap()[0].origin, FIRST);
        assert!(reloaded.remove_exact(FIRST).unwrap().changed);
        assert!(!reloaded.remove_exact(FIRST).unwrap().changed);
        let text = fs::read_to_string(reloaded.path()).unwrap();
        assert!(!text.contains("token"));
        assert!(!text.contains("project_root"));
    }

    #[test]
    fn invalid_origin_wildcard_and_wrong_scheme_fail_closed() {
        let store = PairingStore::new(tempfile::tempdir().unwrap().path());
        for origin in [
            "*",
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!",
            "https://example.test",
        ] {
            assert!(store.add_exact(origin, None).is_err(), "{origin}");
        }
        assert!(store.is_trusted("*").is_err());
    }

    #[test]
    fn over_limit_and_corrupt_store_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = PairingStore::new(temp.path());
        for index in 0..MAX_TRUSTED_EXTENSION_ORIGINS {
            let id = format!(
                "chrome-extension://{}",
                (index..index + 32)
                    .map(|value| char::from(b'a' + (value % 16) as u8))
                    .collect::<String>()
            );
            store.add_exact(&id, None).unwrap();
        }
        let extra = "chrome-extension://pppppppppppppppppppppppppppppppp";
        assert!(store.add_exact(extra, None).is_err());
        fs::write(store.path(), b"{not-json").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = PairingStore::new(temp.path());
        fs::create_dir_all(temp.path()).unwrap();
        fs::write(
            store.path(),
            serde_json::json!({
                "schema_version": 1,
                "trusted_origins": [],
                "token": "must-not-be-accepted"
            })
            .to_string(),
        )
        .unwrap();
        assert!(store.load().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_store_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let store = PairingStore::new(temp.path());
        let target = temp.path().join("target.json");
        fs::write(&target, b"keep").unwrap();
        symlink(&target, store.path()).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn display_name_is_bounded_and_control_free() {
        let store = PairingStore::new(tempfile::tempdir().unwrap().path());
        assert!(store.add_exact(FIRST, Some("\n"),).is_ok());
        assert!(
            store
                .add_exact(SECOND, Some(&"x".repeat(MAX_DISPLAY_NAME_BYTES + 1)))
                .is_err()
        );
    }
}
