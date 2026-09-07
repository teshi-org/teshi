//! Product version shown by `teshi -V`, the TUI header, logs, and diagnostics.
//!
//! Cargo `[workspace.package] version` is the SemVer. CI may inject build
//! identity through `TESHI_BUILD_CHANNEL`, `TESHI_BUILD_DATE`, and `TESHI_GIT_SHA`.

use std::sync::LazyLock;

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
    /// `{semver} (nightly 2026-09-07, cb4361a)`. Incomplete identity is ignored
    /// so a half-set CI environment cannot look like a shipped nightly.
    pub fn display_string(&self) -> String {
        match (self.channel, self.build_date, self.git_sha) {
            (Some(channel), Some(date), Some(sha)) => match hyphenate_yyyymmdd(date) {
                Some(pretty) => format!("{} ({channel} {pretty}, {sha})", self.semver),
                None => self.semver.to_string(),
            },
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
    fn nightly_display_includes_channel_date_and_sha() {
        let info = VersionInfo {
            semver: "0.7.10",
            channel: Some("nightly"),
            build_date: Some("20260907"),
            git_sha: Some("cb4361a"),
        };
        assert_eq!(
            info.display_string(),
            "0.7.10 (nightly 2026-09-07, cb4361a)"
        );
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
