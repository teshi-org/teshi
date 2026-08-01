use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// A single credential entry stored in `auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialEntry {
    #[serde(rename = "type")]
    pub credential_type: String,
    pub key: String,
}

/// Legacy credential storage at `<config_dir>/teshi/auth.json`.
///
/// Runtime LLM credentials now live on shared engine model profiles.
/// This manager remains only so `${auth:provider}` placeholders in older
/// `config.toml` files can still resolve during layered config load, and so
/// one-time import can read existing keys.
///
/// Handles read/write with `0600` file permissions on Unix.
/// Reading a file with unsafe permissions produces a warning but does not fail.
pub struct CredentialManager {
    path: PathBuf,
}

impl CredentialManager {
    /// Creates a new manager backed by the default path:
    /// `<config_dir>/teshi/auth.json` (e.g. `~/.config/teshi/auth.json` on Linux).
    pub fn new() -> Result<Self> {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("teshi");
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create config directory {}", dir.display()))?;
        let path = dir.join("auth.json");
        Ok(Self { path })
    }

    /// Loads all credentials from the store.
    ///
    /// Returns an empty map if the file does not exist.
    /// Warns about unsafe file permissions on Unix.
    pub fn load(&self) -> Result<HashMap<String, CredentialEntry>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        self.warn_if_permissions_unsafe()?;
        let data = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        let map: HashMap<String, CredentialEntry> = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse {}", self.path.display()))?;
        Ok(map)
    }

    /// Saves the credential map to the store atomically using a temp file.
    ///
    /// On Unix, sets file permissions to `0o600` after writing.
    /// On other platforms, no permission changes are applied.
    #[allow(dead_code)] // retained for tests and rare legacy repair paths
    pub fn save(&self, credentials: &HashMap<String, CredentialEntry>) -> Result<()> {
        let dir = self
            .path
            .parent()
            .expect("auth.json must have a parent directory");
        fs::create_dir_all(dir)?;

        let json =
            serde_json::to_string_pretty(credentials).context("failed to serialize credentials")?;

        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, &json).with_context(|| format!("failed to write {}", tmp.display()))?;

        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&tmp)
                .with_context(|| format!("failed to stat {}", tmp.display()))?
                .permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&tmp, perms)
                .with_context(|| format!("failed to set permissions on {}", tmp.display()))?;
        }

        fs::rename(&tmp, &self.path).with_context(|| {
            format!(
                "failed to rename {} → {}",
                tmp.display(),
                self.path.display()
            )
        })?;

        Ok(())
    }

    /// Returns the path to the auth store.
    #[allow(dead_code)] // retained for tests and status helpers
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Masks an API key for display: shows first 4 and last 4 characters.
    #[allow(dead_code)] // retained for tests
    pub fn mask_key(key: &str) -> String {
        if key.len() <= 8 {
            return "*".repeat(key.len());
        }
        format!("{}****{}", &key[..4], &key[key.len() - 4..])
    }

    /// On Unix, warns via eprintln if the file has permissive permissions.
    fn warn_if_permissions_unsafe(&self) -> Result<()> {
        let meta = fs::metadata(&self.path)
            .with_context(|| format!("failed to stat {}", self.path.display()))?;

        #[cfg(unix)]
        {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 && mode != 0o400 {
                eprintln!(
                    "warning: {} has unsafe permissions 0o{:03o}; consider running: chmod 600 {}",
                    self.path.display(),
                    mode,
                    self.path.display()
                );
            }
        }
        #[cfg(not(unix))]
        let _ = meta;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_manager() -> CredentialManager {
        let tmp = std::env::temp_dir().join("teshi-auth-test");
        let teshi_dir = tmp.join("teshi");
        fs::create_dir_all(&teshi_dir).unwrap();
        let path = teshi_dir.join("auth.json");
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        let mut map: HashMap<String, CredentialEntry> = HashMap::new();
        map.insert(
            "openai".into(),
            CredentialEntry {
                credential_type: "api_key".into(),
                key: "sk-test12345678".to_string(),
            },
        );
        let json = serde_json::to_string_pretty(&map).unwrap();
        fs::write(&path, &json).unwrap();
        CredentialManager { path }
    }

    #[test]
    fn test_load_credentials() {
        let mgr = test_manager();
        let creds = mgr.load().unwrap();
        assert_eq!(creds.len(), 1);
        let entry = creds.get("openai").unwrap();
        assert_eq!(entry.credential_type, "api_key");
        assert_eq!(entry.key, "sk-test12345678");
        let _ = fs::remove_file(mgr.path);
    }

    #[test]
    fn test_mask_key() {
        let key = "sk-test12345678";
        assert_eq!(CredentialManager::mask_key(key), "sk-t****5678");
    }

    #[test]
    fn test_mask_key_short() {
        let key = "short";
        assert_eq!(CredentialManager::mask_key(key), "*****");
    }

    #[test]
    fn test_save_and_reload() {
        let tmp = std::env::temp_dir().join("teshi-auth-test-save");
        let teshi_dir = tmp.join("teshi");
        fs::create_dir_all(&teshi_dir).unwrap();
        let path = teshi_dir.join("auth.json");
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        let mgr = CredentialManager { path };
        let mut map: HashMap<String, CredentialEntry> = HashMap::new();
        map.insert(
            "deepseek".into(),
            CredentialEntry {
                credential_type: "api_key".into(),
                key: "ds-key-abcdef".to_string(),
            },
        );
        mgr.save(&map).unwrap();
        let loaded = mgr.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["deepseek"].key, "ds-key-abcdef");
        let _ = fs::remove_file(mgr.path());
    }
}
