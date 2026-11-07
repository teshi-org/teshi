//! Declarative per-workspace initialization.
//!
//! `workspace.toml` lives in the workspace state directory (not in any member)
//! and declares what a new member needs before it can build or run: files copied
//! from the first member, setup commands, and extra environment for those
//! commands.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = "workspace.toml";
/// How many stderr lines a failed setup command reports.
const SETUP_STDERR_LINES: usize = 20;

/// `workspace.toml` contents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// Paths relative to the first member, copied into every other member.
    pub include: Vec<String>,
    /// Shell commands run inside a member after copying.
    pub setup: Vec<String>,
    /// Extra environment variables for setup commands.
    pub env: BTreeMap<String, String>,
}

/// What [`initialize_member`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitReport {
    /// Relative paths copied into the member.
    pub copied: Vec<String>,
    /// Relative paths skipped because the destination already exists.
    pub skipped: Vec<String>,
    /// Declared paths missing from the source member.
    pub missing: Vec<String>,
    /// Number of setup commands that ran successfully.
    pub setup_commands: usize,
}

impl InitReport {
    /// Whether the member needed no work at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.copied.is_empty() && self.skipped.is_empty() && self.setup_commands == 0
    }
}

/// Path of the config file for one workspace.
#[must_use]
pub fn config_path(workspaces_root: &Path, workspace_id: &str) -> PathBuf {
    workspaces_root.join(workspace_id).join(CONFIG_FILE)
}

/// Reads the config, returning defaults when the file is absent.
///
/// # Errors
///
/// Returns an error when the file exists but cannot be read or parsed.
pub fn load_config(workspaces_root: &Path, workspace_id: &str) -> Result<WorkspaceConfig> {
    let path = config_path(workspaces_root, workspace_id);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkspaceConfig::default());
        }
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let text = String::from_utf8(bytes).with_context(|| format!("decode {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

/// Writes the config, creating the workspace state directory.
///
/// # Errors
///
/// Returns an error when the file cannot be serialized or written.
pub fn save_config(
    workspaces_root: &Path,
    workspace_id: &str,
    config: &WorkspaceConfig,
) -> Result<()> {
    let path = config_path(workspaces_root, workspace_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(config).context("serialize workspace config")?;
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))
}

/// Writes an empty config when the workspace has none yet.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn ensure_config(workspaces_root: &Path, workspace_id: &str) -> Result<PathBuf> {
    let path = config_path(workspaces_root, workspace_id);
    if !path.is_file() {
        save_config(workspaces_root, workspace_id, &WorkspaceConfig::default())?;
    }
    Ok(path)
}

/// Applies `workspace.toml` to one member directory.
///
/// `include` entries are copied from `source` (the first member) and existing
/// destinations are never overwritten; `setup` commands then run in `member`.
///
/// # Errors
///
/// Returns an error for an unsafe `include` path, an I/O failure, or a failing
/// setup command.
pub fn initialize_member(
    source: &Path,
    member: &Path,
    config: &WorkspaceConfig,
) -> Result<InitReport> {
    let mut report = InitReport::default();
    if source != member {
        for entry in &config.include {
            match copy_include(source, member, entry)? {
                IncludeOutcome::Copied => report.copied.push(entry.clone()),
                IncludeOutcome::Skipped => report.skipped.push(entry.clone()),
                IncludeOutcome::Missing => report.missing.push(entry.clone()),
            }
        }
    }
    for command in &config.setup {
        run_setup(member, command, &config.env)?;
        report.setup_commands += 1;
    }
    Ok(report)
}

enum IncludeOutcome {
    Copied,
    Skipped,
    Missing,
}

fn copy_include(source: &Path, member: &Path, relative: &str) -> Result<IncludeOutcome> {
    let relative_path = validate_relative(relative)?;
    let from = source.join(&relative_path);
    if !from.exists() {
        return Ok(IncludeOutcome::Missing);
    }
    let to = member.join(&relative_path);
    if to.exists() {
        return Ok(IncludeOutcome::Skipped);
    }
    copy_entry(&from, &to)
        .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    Ok(IncludeOutcome::Copied)
}

fn copy_entry(from: &Path, to: &Path) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(from).with_context(|| format!("inspect {}", from.display()))?;
    // Following a directory symlink can recurse forever or copy a tree from
    // outside the member, so only regular directories are descended into.
    if metadata.file_type().is_symlink() && from.is_dir() {
        bail!(
            "include contains a directory symlink, which is not copied: {}",
            from.display()
        );
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(to).with_context(|| format!("create {}", to.display()))?;
        for entry in std::fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
            let entry = entry?;
            let child = entry.path();
            let name = entry.file_name();
            copy_entry(&child, &to.join(name))?;
        }
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::copy(from, to)
        .with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    Ok(())
}

/// Rejects absolute paths and parent traversal in declared includes.
fn validate_relative(relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative.trim());
    if relative.trim().is_empty() {
        bail!("include entry must not be empty");
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("include entry must stay inside the member: '{relative}'");
    }
    Ok(path.to_path_buf())
}

fn run_setup(member: &Path, command: &str, env: &BTreeMap<String, String>) -> Result<()> {
    let mut process = shell(command);
    process.current_dir(member).envs(env);
    let output = process
        .output()
        .with_context(|| format!("run setup command '{command}'"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().rev().take(SETUP_STDERR_LINES).collect();
    let tail = lines.into_iter().rev().collect::<Vec<_>>().join("\n");
    bail!(
        "setup command '{command}' failed in {} (status {})\n{tail}",
        member.display(),
        output.status
    )
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;

    // `arg` would escape embedded quotes as `\"`, which cmd does not understand,
    // so the command line is passed verbatim exactly as the user wrote it.
    let mut process = Command::new("cmd");
    process.arg("/C").raw_arg(command);
    process
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.arg("-c").arg(command);
    process
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn missing_config_loads_default() {
        let root = tempfile::tempdir().unwrap();
        let config = load_config(root.path(), "missing").unwrap();
        assert_eq!(config, WorkspaceConfig::default());
    }

    #[test]
    fn config_round_trips_through_toml() {
        let root = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig {
            include: vec!["teshi.toml".to_owned()],
            setup: vec!["git rev-parse HEAD".to_owned()],
            env: BTreeMap::from([("KEY".to_owned(), "value".to_owned())]),
        };
        save_config(root.path(), "ws-1", &config).unwrap();
        assert_eq!(load_config(root.path(), "ws-1").unwrap(), config);
    }

    #[test]
    fn include_copies_files_and_directories() {
        let root = tempfile::tempdir().unwrap();
        let source = dir(root.path(), "source");
        let member = dir(root.path(), "member");
        std::fs::write(source.join("teshi.toml"), "port = 1").unwrap();
        let nested = source.join("config");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("local.toml"), "x").unwrap();

        let config = WorkspaceConfig {
            include: vec!["teshi.toml".to_owned(), "config".to_owned()],
            ..WorkspaceConfig::default()
        };
        let report = initialize_member(&source, &member, &config).unwrap();

        assert_eq!(report.copied.len(), 2);
        assert_eq!(
            std::fs::read_to_string(member.join("teshi.toml")).unwrap(),
            "port = 1"
        );
        assert_eq!(
            std::fs::read_to_string(member.join("config/local.toml")).unwrap(),
            "x"
        );
    }

    #[test]
    fn include_never_overwrites_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let source = dir(root.path(), "source");
        let member = dir(root.path(), "member");
        std::fs::write(source.join("teshi.toml"), "new").unwrap();
        std::fs::write(member.join("teshi.toml"), "keep").unwrap();

        let config = WorkspaceConfig {
            include: vec!["teshi.toml".to_owned()],
            ..WorkspaceConfig::default()
        };
        let report = initialize_member(&source, &member, &config).unwrap();

        assert_eq!(report.skipped, vec!["teshi.toml".to_owned()]);
        assert_eq!(
            std::fs::read_to_string(member.join("teshi.toml")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn missing_include_is_reported_not_fatal() {
        let root = tempfile::tempdir().unwrap();
        let source = dir(root.path(), "source");
        let member = dir(root.path(), "member");

        let config = WorkspaceConfig {
            include: vec!["absent.toml".to_owned()],
            ..WorkspaceConfig::default()
        };
        let report = initialize_member(&source, &member, &config).unwrap();
        assert_eq!(report.missing, vec!["absent.toml".to_owned()]);
    }

    #[test]
    fn parent_traversal_in_include_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let source = dir(root.path(), "source");
        let member = dir(root.path(), "member");

        let config = WorkspaceConfig {
            include: vec!["../secret".to_owned()],
            ..WorkspaceConfig::default()
        };
        let error = initialize_member(&source, &member, &config).unwrap_err();
        assert!(error.to_string().contains("stay inside"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlinks_in_includes_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let source = dir(root.path(), "source");
        let member = dir(root.path(), "member");
        let nested = dir(&source, "config");
        std::os::unix::fs::symlink(&source, nested.join("loop")).unwrap();

        let config = WorkspaceConfig {
            include: vec!["config".to_owned()],
            ..WorkspaceConfig::default()
        };
        let error = initialize_member(&source, &member, &config).unwrap_err();
        assert!(
            format!("{error:#}").contains("directory symlink"),
            "{error:#}"
        );
    }

    #[test]
    fn setup_commands_run_with_declared_env() {
        let root = tempfile::tempdir().unwrap();
        let member = dir(root.path(), "member");
        let config = WorkspaceConfig {
            setup: vec![write_marker_command()],
            env: BTreeMap::from([("TESHI_INIT_MARKER".to_owned(), "ok".to_owned())]),
            ..WorkspaceConfig::default()
        };

        let report = initialize_member(&member, &member, &config).unwrap();

        assert_eq!(report.setup_commands, 1);
        let marker = std::fs::read_to_string(member.join("marker.txt")).unwrap();
        assert_eq!(marker.trim(), "ok");
    }

    #[test]
    fn setup_commands_keep_quoted_arguments_intact() {
        let root = tempfile::tempdir().unwrap();
        let member = dir(root.path(), "member");
        let config = WorkspaceConfig {
            setup: vec![quoted_path_command()],
            ..WorkspaceConfig::default()
        };

        initialize_member(&member, &member, &config).unwrap();

        assert!(
            member.join("marker file.txt").is_file(),
            "a quoted path with a space must reach the shell unchanged"
        );
    }

    #[test]
    fn failing_setup_command_reports_stderr() {
        let root = tempfile::tempdir().unwrap();
        let member = dir(root.path(), "member");
        let config = WorkspaceConfig {
            setup: vec![failing_command()],
            ..WorkspaceConfig::default()
        };

        let error = initialize_member(&member, &member, &config).unwrap_err();
        assert!(error.to_string().contains("setup command"), "{error}");
    }

    #[cfg(windows)]
    fn write_marker_command() -> String {
        "echo %TESHI_INIT_MARKER%> marker.txt".to_owned()
    }

    #[cfg(not(windows))]
    fn write_marker_command() -> String {
        "printf '%s' \"$TESHI_INIT_MARKER\" > marker.txt".to_owned()
    }

    #[cfg(windows)]
    fn quoted_path_command() -> String {
        "echo ok> \"marker file.txt\"".to_owned()
    }

    #[cfg(not(windows))]
    fn quoted_path_command() -> String {
        "printf ok > \"marker file.txt\"".to_owned()
    }

    #[cfg(windows)]
    fn failing_command() -> String {
        "exit /b 3".to_owned()
    }

    #[cfg(not(windows))]
    fn failing_command() -> String {
        "exit 3".to_owned()
    }
}
