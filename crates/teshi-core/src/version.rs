//! Product version shown by `teshi -V`, the TUI header, logs, and diagnostics.
//!
//! Cargo `[workspace.package] version` is the SemVer. CI may inject build
//! identity through `TESHI_BUILD_CHANNEL`, `TESHI_BUILD_DATE`, and `TESHI_GIT_SHA`.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// Release stream. Missing build metadata is deliberately not stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    /// Published stable releases.
    Stable,
    /// Published development snapshots.
    Nightly,
    /// Local or unidentified builds.
    Dev,
}

/// Machine-readable product identity, independent of shell/library versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIdentity {
    /// Product SemVer.
    pub semver: String,
    /// Explicit release stream.
    pub channel: ReleaseChannel,
    /// Full source commit, empty for local builds.
    pub git_sha: String,
    /// UTC RFC3339 build timestamp, empty for local builds.
    pub build_timestamp: String,
    /// Monotonic CI run identity (not a lexically ordered SHA).
    pub build_sequence: u64,
}

/// Returns the compiled product identity; incomplete release identity is dev.
pub fn build_identity() -> BuildIdentity {
    let channel = match option_env!("TESHI_BUILD_CHANNEL") {
        Some("stable") => ReleaseChannel::Stable,
        Some("nightly") => ReleaseChannel::Nightly,
        _ => ReleaseChannel::Dev,
    };
    let mut identity = BuildIdentity {
        semver: env!("CARGO_PKG_VERSION").into(),
        channel,
        git_sha: option_env!("TESHI_GIT_SHA").unwrap_or_default().into(),
        build_timestamp: option_env!("TESHI_BUILD_TIMESTAMP")
            .unwrap_or_default()
            .into(),
        build_sequence: option_env!("TESHI_BUILD_SEQUENCE")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    };
    if identity.git_sha.len() != 40
        || !identity.git_sha.bytes().all(|b| b.is_ascii_hexdigit())
        || identity.build_sequence == 0
        || chrono::DateTime::parse_from_rfc3339(&identity.build_timestamp).is_err()
    {
        identity.channel = ReleaseChannel::Dev;
    }
    identity
}

/// Rust target triple used to compile this product.
pub const BUILD_TARGET: &str = env!("TESHI_TARGET");

/// Teshi product version: workspace SemVer plus optional build identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionInfo {
    /// Workspace SemVer, for example `0.7.10`.
    pub semver: &'static str,
    /// Build channel when CI injected one, typically `nightly`.
    pub channel: Option<&'static str>,
    /// UTC calendar day `YYYYMMDD` when CI injected one.
    pub build_date: Option<&'static str>,
    /// Short git object name when CI injected one.
    pub git_sha: Option<&'static str>,
}

impl VersionInfo {
    /// Formats the string clap, the TUI, and logs should show.
    ///
    /// Stable builds are SemVer only. A complete nightly identity becomes
    /// `{semver}-nightly (cb4361a)`. Incomplete identity is ignored so a
    /// half-set CI environment cannot look like a shipped nightly.
    pub fn display_string(&self) -> String {
        match (self.channel, self.build_date, self.git_sha) {
            (Some(channel), Some(date), Some(sha)) if channel != "stable" => {
                if hyphenate_yyyymmdd(date).is_some() {
                    format!(
                        "{}-{channel} ({})",
                        self.semver,
                        sha.chars().take(7).collect::<String>()
                    )
                } else {
                    self.semver.to_string()
                }
            }
            _ => self.semver.to_string(),
        }
    }
}

/// Product version for this compiled binary.
pub fn version_info() -> VersionInfo {
    VersionInfo {
        semver: env!("CARGO_PKG_VERSION"),
        channel: nonempty_env(option_env!("TESHI_BUILD_CHANNEL")),
        build_date: nonempty_env(option_env!("TESHI_BUILD_DATE")),
        git_sha: nonempty_env(option_env!("TESHI_GIT_SHA")),
    }
}

/// Cached display string for clap `-V` and other process-wide surfaces.
pub fn version_display() -> &'static str {
    static DISPLAY: LazyLock<String> = LazyLock::new(|| version_info().display_string());
    DISPLAY.as_str()
}

fn nonempty_env(value: Option<&'static str>) -> Option<&'static str> {
    value.filter(|item| !item.is_empty())
}

fn hyphenate_yyyymmdd(date: &str) -> Option<String> {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..8]))
}

#[cfg(test)]
mod tests {
    use super::{VersionInfo, hyphenate_yyyymmdd};

    #[test]
    fn stable_display_is_semver_only() {
        let info = VersionInfo {
            semver: "0.7.10",
            channel: None,
            build_date: None,
            git_sha: None,
        };
        assert_eq!(info.display_string(), "0.7.10");
    }

    #[test]
    fn nightly_display_includes_channel_and_sha() {
        let info = VersionInfo {
            semver: "0.7.10",
            channel: Some("nightly"),
            build_date: Some("20260907"),
            git_sha: Some("cb4361a"),
        };
        assert_eq!(info.display_string(), "0.7.10-nightly (cb4361a)");
    }

    #[test]
    fn incomplete_nightly_identity_does_not_look_released() {
        let info = VersionInfo {
            semver: "0.7.10",
            channel: Some("nightly"),
            build_date: Some("20260907"),
            git_sha: None,
        };
        assert_eq!(info.display_string(), "0.7.10");
    }

    #[test]
    fn invalid_build_date_falls_back_to_semver() {
        let info = VersionInfo {
            semver: "0.7.10",
            channel: Some("nightly"),
            build_date: Some("2026-09-07"),
            git_sha: Some("cb4361a"),
        };
        assert_eq!(info.display_string(), "0.7.10");
        assert_eq!(
            hyphenate_yyyymmdd("20260907").as_deref(),
            Some("2026-09-07")
        );
    }

    #[test]
    fn compiled_version_info_uses_workspace_semver() {
        let info = super::version_info();
        assert_eq!(info.semver, env!("CARGO_PKG_VERSION"));
        assert!(
            super::version_display().starts_with(info.semver),
            "display {}",
            super::version_display()
        );
    }
}
