//! Project-scoped execution targets referenced by feature-level bindings.
//!
//! A `[targets.<name>]` table describes *how* to start or find the target; the
//! binding records *which* profile a Feature uses. Launch paths are resolved
//! against the project root so committed bindings never carry machine paths.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use teshi_core::locator::StepBindingAttach;

use crate::run_routing::{RouteConfigError, RUNNER_CONFIG_FILE};

/// How to reach one Feature's target.
#[derive(Debug, Clone, Default)]
pub struct TargetProfile {
    /// Profile name from `[targets.<name>]`.
    pub name: String,
    /// `winapp` or `browser`.
    pub kind: String,
    /// Executable to launch, relative to the project root.
    pub launch: Option<PathBuf>,
    /// Arguments passed to `launch`.
    pub args: Vec<String>,
    /// Working directory for `launch`, relative to the project root.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables for the launched target.
    pub env: HashMap<String, String>,
    /// Window identity used when attaching without launching.
    pub attach: Option<StepBindingAttach>,
    /// Start an embedded browser sidecar before replaying.
    pub embedded: bool,
    /// Optional URL the embedded browser should open first.
    pub navigate: Option<String>,
}

impl TargetProfile {
    /// Whether this profile targets a native Windows window.
    #[must_use]
    pub fn is_winapp(&self) -> bool {
        self.kind.eq_ignore_ascii_case("winapp")
    }

    /// Whether this profile targets a browser session.
    #[must_use]
    pub fn is_browser(&self) -> bool {
        self.kind.eq_ignore_ascii_case("browser")
    }
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    targets: HashMap<String, TargetToml>,
}

#[derive(Debug, Default, Deserialize)]
struct TargetToml {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    launch: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    attach: Option<StepBindingAttach>,
    #[serde(default)]
    embedded: bool,
    #[serde(default)]
    navigate: Option<String>,
}

/// Loads one `[targets.<name>]` profile from `<project_root>/teshi.toml`.
///
/// # Errors
/// Returns an error when the configuration file cannot be read or parsed.
pub fn load_target_profile(
    project_root: &Path,
    name: &str,
) -> Result<Option<TargetProfile>, RouteConfigError> {
    let path = project_root.join(RUNNER_CONFIG_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|source| RouteConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let file: ConfigFile = toml::from_str(&raw).map_err(|error| RouteConfigError::Parse {
        path: path.clone(),
        message: error.to_string(),
    })?;
    let Some(profile) = file.targets.get(name) else {
        return Ok(None);
    };
    let launch = std::env::var(launch_env(name))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| profile.launch.clone())
        .map(|value| resolve(project_root, &value));
    let cwd = profile
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| resolve(project_root, value));
    Ok(Some(TargetProfile {
        name: name.to_string(),
        kind: profile.kind.clone(),
        launch,
        args: profile.args.clone(),
        cwd,
        env: profile.env.clone(),
        attach: profile.attach.clone(),
        embedded: profile.embedded,
        navigate: profile.navigate.clone(),
    }))
}

fn launch_env(name: &str) -> String {
    let normalized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("TESHI_TARGET_{normalized}_LAUNCH")
}

fn resolve(project_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_winapp_profile_with_a_project_relative_launch() {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("teshi.toml"),
            r#"
[targets.teshi-desktop]
kind = "winapp"
launch = "target/debug/teshi-desktop.exe"
args = ["--fixture"]
[targets.teshi-desktop.attach]
title = "teshi"
"#,
        )
        .unwrap();

        let profile = load_target_profile(project.path(), "teshi-desktop")
            .unwrap()
            .expect("profile");
        assert!(profile.is_winapp());
        assert_eq!(
            profile.launch.as_deref(),
            Some(
                project
                    .path()
                    .join("target/debug/teshi-desktop.exe")
                    .as_path()
            )
        );
        assert_eq!(profile.args, vec!["--fixture"]);
        assert_eq!(
            profile.attach.and_then(|attach| attach.title).as_deref(),
            Some("teshi")
        );
    }

    #[test]
    fn missing_profile_is_none_and_anonymous_tables_are_ignored() {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("teshi.toml"),
            "[[runners]]\nname = \"x\"\ncmd = \"x\"\n",
        )
        .unwrap();
        assert!(load_target_profile(project.path(), "missing")
            .unwrap()
            .is_none());
    }
}
