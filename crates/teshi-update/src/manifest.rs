//! Versioned release and bundle contracts. All paths are portable relative paths.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use teshi_core::version::{BuildIdentity, ReleaseChannel};

use crate::{Result, invalid};

/// Current updater protocol and manifest version.
pub const PROTOCOL: u32 = 1;
/// Installed inventory file, excluded from its own inventory to avoid self-hashing.
pub const BUNDLE_MANIFEST: &str = "teshi-bundle.json";
/// Supported release triples; never substitute a host architecture.
pub const TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
];

/// Authority responsible for updating the installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// Dedicated extracted archive. Check-only; never self-updates.
    Portable,
    /// Registered Windows Installer / WinGet package. Check-only; never self-updates.
    Msi,
    /// Per-user Windows setup.exe install. The only in-app update backend.
    Exe,
    /// Explicitly managed outside Teshi.
    External,
    /// No verified installation contract.
    Unknown,
}

/// Relative directory prefix for shipped executables.
pub fn executable_prefix(kind: InstallKind) -> &'static str {
    match kind {
        InstallKind::Msi | InstallKind::Exe => "bin/",
        _ => "",
    }
}

/// Absolute path to a shipped executable inside an installation root.
pub fn shipped_executable(root: &Path, kind: InstallKind, file_name: &str) -> PathBuf {
    match kind {
        InstallKind::Msi | InstallKind::Exe => root.join("bin").join(file_name),
        _ => root.join(file_name),
    }
}

/// A file owned by the shipped bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFile {
    /// Forward-slash relative path.
    pub path: String,
    /// Exact uncompressed size.
    pub size: u64,
    /// Lowercase SHA256.
    pub sha256: String,
    /// Whether to restore executable permissions on Unix.
    pub executable: bool,
}

/// Embedded installation identity and managed inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    /// Schema version.
    pub schema: u32,
    /// Build shared by every executable in the bundle.
    pub identity: BuildIdentity,
    /// Exact compilation target.
    pub target: String,
    /// Installation authority.
    pub kind: InstallKind,
    /// Layout version, currently 1.
    pub layout: u32,
    /// Optional external-manager guidance, never executed.
    pub update_explanation: Option<String>,
    /// Complete payload inventory, excluding this manifest.
    pub files: Vec<ManagedFile>,
}

/// One payload in a published release.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAsset {
    /// Exact GitHub asset name.
    pub name: String,
    /// Target triple.
    pub target: String,
    /// Portable archive, MSI package, or Windows setup.exe.
    pub kind: InstallKind,
    /// Exact compressed/download size.
    pub size: u64,
    /// Lowercase SHA256 of the download.
    pub sha256: String,
}

/// External release asset describing every installable bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    /// Schema version.
    pub schema: u32,
    /// Minimum supported updater protocol.
    pub minimum_updater: u32,
    /// Build identity shared across targets.
    pub identity: BuildIdentity,
    /// Exact release tag.
    pub tag: String,
    /// Payloads with integrity metadata.
    pub assets: Vec<ReleaseAsset>,
}

/// Validates a complete published identity.
///
/// # Errors
/// Rejects development identities, malformed SemVer/SHA/time and missing sequence.
pub fn validate_identity(identity: &BuildIdentity) -> Result<()> {
    let version = semver::Version::parse(&identity.semver).map_err(|e| invalid(e.to_string()))?;
    if identity.channel == ReleaseChannel::Dev
        || !version.pre.is_empty()
        || !version.build.is_empty()
        || identity.git_sha.len() != 40
        || !identity
            .git_sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || identity.build_sequence == 0
        || chrono::DateTime::parse_from_rfc3339(&identity.build_timestamp).is_err()
    {
        return Err(invalid("Incomplete or invalid release identity"));
    }
    Ok(())
}

/// Validates a normalized cross-platform file path, including Windows aliases.
///
/// # Errors
/// Rejects traversal, reserved names, alternate streams and ambiguous paths.
pub fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 240
        || path.contains('\\')
        || path.contains(':')
        || path
            .chars()
            .any(|c| c.is_control() || "<>\"|?*".contains(c))
    {
        return Err(invalid(format!("Unsafe bundle path: {path}")));
    }
    for part in path.split('/') {
        let base = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || base == "CON"
            || base == "PRN"
            || base == "AUX"
            || base == "NUL"
            || (base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.as_bytes()[3].is_ascii_digit())
            || part.to_ascii_lowercase().starts_with(".teshi-update")
        {
            return Err(invalid(format!("Unsafe bundle path: {path}")));
        }
    }
    Ok(())
}

pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl BundleManifest {
    /// Checks protocol, identity, inventory uniqueness and mandatory executables.
    ///
    /// # Errors
    /// Returns an invalid-manifest error for malformed or unsupported metadata.
    pub fn validate(&self) -> Result<()> {
        validate_identity(&self.identity)?;
        if self.schema != PROTOCOL
            || self.layout != 1
            || !TARGETS.contains(&self.target.as_str())
            || self.files.is_empty()
            || self.files.len() > 50_000
            || self.kind == InstallKind::Unknown
            || (self.kind == InstallKind::Exe && !self.target.contains("windows"))
        {
            return Err(invalid("Unsupported bundle schema, layout or target"));
        }
        let mut names = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.files {
            validate_path(&file.path)?;
            total = total
                .checked_add(file.size)
                .ok_or_else(|| invalid("Bundle size overflow"))?;
            if !valid_hash(&file.sha256)
                || !names.insert(file.path.to_ascii_lowercase())
                || file.path.eq_ignore_ascii_case(BUNDLE_MANIFEST)
                || total > 8 * 1024 * 1024 * 1024
            {
                return Err(invalid(
                    "Invalid bundle hash, duplicate path or expansion size",
                ));
            }
        }
        for name in &names {
            let mut parent = name.as_str();
            while let Some((head, _)) = parent.rsplit_once('/') {
                if names.contains(head) {
                    return Err(invalid("File/directory inventory conflict"));
                }
                parent = head;
            }
        }
        let prefix = executable_prefix(self.kind);
        let suffix = if self.target.contains("windows") {
            ".exe"
        } else {
            ""
        };
        for binary in ["teshi", "teshi-update-helper"] {
            let path = format!("{prefix}{binary}{suffix}");
            if !self.files.iter().any(|f| f.path == path && f.executable) {
                return Err(invalid(format!("Bundle missing executable: {path}")));
            }
        }
        Ok(())
    }
}

impl ReleaseManifest {
    /// Checks identity and exact target/asset names; refuses future protocols.
    ///
    /// # Errors
    /// Rejects inconsistent or unsupported release metadata.
    pub fn validate(&self) -> Result<()> {
        validate_identity(&self.identity)?;
        if self.schema != PROTOCOL || self.minimum_updater > PROTOCOL || self.assets.is_empty() {
            return Err(invalid("Release requires a newer updater or has no assets"));
        }
        let expected = match self.identity.channel {
            ReleaseChannel::Stable => format!("v{}", self.identity.semver),
            ReleaseChannel::Nightly => {
                let date = self
                    .tag
                    .split("-nightly.")
                    .nth(1)
                    .and_then(|s| s.split('.').next())
                    .ok_or_else(|| invalid("Invalid nightly tag"))?;
                chrono::NaiveDate::parse_from_str(date, "%Y%m%d")
                    .map_err(|e| invalid(e.to_string()))?;
                format!(
                    "v{}-nightly.{date}.{}",
                    self.identity.semver,
                    &self.identity.git_sha[..7]
                )
            }
            ReleaseChannel::Dev => return Err(invalid("Development release")),
        };
        if self.tag != expected {
            return Err(invalid("Release tag and identity disagree"));
        }
        let mut names = BTreeSet::new();
        for asset in &self.assets {
            let expected_name = asset_name(&self.tag, &asset.target, asset.kind)?;
            if asset.name != expected_name
                || !valid_hash(&asset.sha256)
                || asset.size == 0
                || asset.size > 4 * 1024 * 1024 * 1024
                || !names.insert(&asset.name)
            {
                return Err(invalid("Invalid release asset identity, size or checksum"));
            }
        }
        Ok(())
    }
}

/// Exact archive/MSI name for an existing Teshi release platform.
///
/// # Errors
/// Returns an error for unsupported targets or installation kinds.
pub fn asset_name(tag: &str, target: &str, kind: InstallKind) -> Result<String> {
    if !TARGETS.contains(&target) {
        return Err(invalid("Unsupported target"));
    }
    match kind {
        InstallKind::Msi if target == "x86_64-pc-windows-msvc" => {
            Ok(format!("teshi-{tag}-x64.msi"))
        }
        InstallKind::Exe if target == "x86_64-pc-windows-msvc" => {
            Ok(format!("teshi-{tag}-x64-setup.exe"))
        }
        InstallKind::Portable => {
            let extension = if target.contains("windows") {
                "zip"
            } else {
                "tar.gz"
            };
            Ok(format!("teshi-{tag}-{target}.{extension}"))
        }
        _ => Err(invalid("Unsupported asset installation kind")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_cross_platform_path_aliases() {
        for path in [
            "../a",
            "/root",
            "C:/a",
            "a\\b",
            "a/../b",
            "AUX.txt",
            "a:stream",
            "a.",
            "a//b",
            ".teshi-update-lock",
            "a\n",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
        }
        assert!(validate_path("share/.codex-plugin/plugin.json").is_ok());
    }
}
