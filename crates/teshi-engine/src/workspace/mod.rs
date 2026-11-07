//! User-level workspace registry.
//!
//! A workspace groups one or more member directories that share requirement
//! links, test points, and generated state. Membership is keyed by the
//! normalized directory path, so a workspace spans repositories and plain
//! directories alike; nothing here depends on git metadata or on
//! `.teshi/project.json` being present in a member.

pub mod config;
pub mod worktree;

pub use config::{InitReport, WorkspaceConfig};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::app_data::app_data_dir;
use crate::fs_util::{with_exclusive_lock, write_json_unlocked};

/// Environment override for the user-level workspaces directory.
pub const WORKSPACES_DIR_ENV: &str = "TESHI_WORKSPACES_DIR";

/// Port base of the first workspace; member ports are `base + slot`.
///
/// It sits above `teshi web`'s default port (20253) so no member claims it.
pub const DEFAULT_PORT_BASE: u16 = 20_300;

/// Ports reserved per workspace; also the maximum number of member slots.
///
/// Each workspace owns `[port_base, port_base + PORT_BLOCK_SIZE)`, so members of
/// different workspaces never share a port.
pub const PORT_BLOCK_SIZE: u16 = 100;

/// Registry schema version written to [`WorkspaceRegistry::version`].
pub const REGISTRY_VERSION: u32 = 1;

const REGISTRY_FILE: &str = "index.json";

/// All workspaces known to this user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRegistry {
    /// Schema version of the on-disk file.
    pub version: u32,
    /// Workspaces in creation order.
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
}

impl Default for WorkspaceRegistry {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            workspaces: Vec::new(),
        }
    }
}

/// A named group of member directories sharing one project identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// Stable workspace identity, independent of name and paths.
    pub id: String,
    /// Display name, unique within the registry.
    pub name: String,
    /// Requirement-library project identity shared by all members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Base port; a member listens on `port_base + slot`.
    pub port_base: u16,
    /// Build directory shared by every member, injected as `CARGO_TARGET_DIR`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dir: Option<PathBuf>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// Member directories in registration order.
    pub members: Vec<WorkspaceMember>,
}

impl Workspace {
    /// Finds a member whose registered path is `path`.
    #[must_use]
    pub fn member_for_path(&self, path: &Path) -> Option<&WorkspaceMember> {
        let normalized = normalize_existing_dir(path).ok()?;
        self.members.iter().find(|member| member.path == normalized)
    }

    /// Lowest free slot, so archived members' ports are reused before the
    /// block runs out.
    fn next_slot(&self) -> Result<u16> {
        (0..PORT_BLOCK_SIZE)
            .find(|slot| self.members.iter().all(|member| member.slot != *slot))
            .with_context(|| {
                format!(
                    "workspace '{}' already has {PORT_BLOCK_SIZE} members",
                    self.name
                )
            })
    }
}

/// Lowest port block not used by any workspace.
fn next_port_base(registry: &WorkspaceRegistry) -> Result<u16> {
    (0u32..)
        .map(|block| u32::from(DEFAULT_PORT_BASE) + block * u32::from(PORT_BLOCK_SIZE))
        .take_while(|base| base + u32::from(PORT_BLOCK_SIZE) <= u32::from(u16::MAX) + 1)
        .filter_map(|base| u16::try_from(base).ok())
        .find(|base| {
            registry
                .workspaces
                .iter()
                .all(|workspace| workspace.port_base != *base)
        })
        .context("no free port block left for a new workspace")
}

/// One directory belonging to a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMember {
    /// Display name, unique within the workspace.
    pub name: String,
    /// Absolute normalized directory path.
    pub path: PathBuf,
    /// Checked-out branch, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Port slot inside the workspace.
    pub slot: u16,
}

/// Resolves the user-level directory holding the registry.
///
/// Priority: explicit override, then [`WORKSPACES_DIR_ENV`], then
/// `<app data>/workspaces`. The directory is not created here.
///
/// # Errors
///
/// Returns an error when the app data directory cannot be resolved.
pub fn workspaces_dir(override_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = override_dir.filter(|dir| !dir.as_os_str().is_empty()) {
        return absolute(dir);
    }
    if let Some(raw) = std::env::var_os(WORKSPACES_DIR_ENV) {
        let trimmed = raw.to_string_lossy().trim().to_owned();
        if !trimmed.is_empty() {
            return absolute(Path::new(&trimmed));
        }
    }
    Ok(app_data_dir()?.join("workspaces"))
}

/// Reads the registry, returning an empty one when the file is absent.
///
/// # Errors
///
/// Returns an error when the file exists but cannot be read or parsed.
pub fn load_registry(workspaces_root: &Path) -> Result<WorkspaceRegistry> {
    read_registry_unlocked(&registry_path(workspaces_root))
}

/// Replaces the registry file under an exclusive lock.
///
/// # Errors
///
/// Returns an error when the lock cannot be taken or the file cannot be written.
pub fn save_registry(workspaces_root: &Path, registry: &WorkspaceRegistry) -> Result<()> {
    let path = registry_path(workspaces_root);
    with_exclusive_lock(&path, || write_json_unlocked(&path, registry))
}

/// Creates a workspace owning `dirs`.
///
/// Member names are derived from directory names and de-duplicated with a
/// numeric suffix; slots are assigned from zero. Every directory must exist and
/// must not already belong to another workspace.
///
/// # Errors
///
/// Returns an error for an empty or duplicate name, an empty directory list, a
/// missing or non-directory path, a directory owned by another workspace, an
/// exhausted port range, or a failed registry write.
pub fn create_workspace(workspaces_root: &Path, name: &str, dirs: &[PathBuf]) -> Result<Workspace> {
    create_workspace_with_target(workspaces_root, name, dirs, None)
}

/// Creates a workspace with an explicit shared build directory.
///
/// When the first member is a Cargo project, `target_dir` defaults to
/// `<first member>/target` so parallel members reuse one dependency build
/// instead of paying a cold compile each. Other workspaces get no default.
///
/// # Errors
///
/// Same as [`create_workspace`].
pub fn create_workspace_with_target(
    workspaces_root: &Path,
    name: &str,
    dirs: &[PathBuf],
    target_dir: Option<PathBuf>,
) -> Result<Workspace> {
    let name = name.trim();
    if name.is_empty() {
        bail!("workspace name must not be empty");
    }
    if dirs.is_empty() {
        bail!("workspace needs at least one --dir");
    }
    let normalized: Vec<PathBuf> = dirs
        .iter()
        .map(|dir| normalize_existing_dir(dir))
        .collect::<Result<_>>()?;
    // Agents run from other directories, so a relative build path would point
    // somewhere different for each of them.
    let target_dir = target_dir.as_deref().map(absolute).transpose()?;

    let project_id = normalized
        .iter()
        .find_map(|dir| project_id_from_member(dir))
        .or_else(|| Some(uuid::Uuid::new_v4().to_string()));

    let workspace = update_registry(workspaces_root, |registry| {
        if registry
            .workspaces
            .iter()
            .any(|workspace| workspace.name == name)
        {
            bail!("workspace '{name}' already exists");
        }
        for dir in &normalized {
            ensure_unclaimed(registry, dir)?;
        }
        let mut workspace = Workspace {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_owned(),
            project_id: project_id.clone(),
            port_base: next_port_base(registry)?,
            target_dir: target_dir.or_else(|| default_target_dir(&normalized)),
            created_at: chrono::Utc::now().to_rfc3339(),
            members: Vec::new(),
        };
        for dir in &normalized {
            workspace.push_member(dir.clone())?;
        }
        registry.workspaces.push(workspace.clone());
        Ok(workspace)
    })?;
    config::ensure_config(workspaces_root, &workspace.id)?;
    Ok(workspace)
}

/// Adds `dir` to an existing workspace.
///
/// `reference` matches a workspace id or name; when omitted and the registry
/// holds exactly one workspace, that workspace is used.
///
/// # Errors
///
/// Returns an error when the directory is missing or already registered, when
/// the reference is unknown or ambiguous, or when the registry write fails.
pub fn attach_member(
    workspaces_root: &Path,
    reference: Option<&str>,
    dir: &Path,
) -> Result<Workspace> {
    let normalized = normalize_existing_dir(dir)?;

    update_registry(workspaces_root, |registry| {
        ensure_unclaimed(registry, &normalized)?;
        let index = match reference.map(str::trim).filter(|value| !value.is_empty()) {
            Some(reference) => registry
                .workspaces
                .iter()
                .position(|workspace| workspace.id == reference || workspace.name == reference)
                .with_context(|| format!("unknown workspace '{reference}'"))?,
            None if registry.workspaces.len() == 1 => 0,
            None if registry.workspaces.is_empty() => bail!(
                "no workspace yet; create one with `teshi workspace new --name <name> --dir <path>`"
            ),
            None => {
                let names: Vec<&str> = registry
                    .workspaces
                    .iter()
                    .map(|workspace| workspace.name.as_str())
                    .collect();
                bail!(
                    "several workspaces exist ({}); pass --workspace",
                    names.join(", ")
                )
            }
        };
        let workspace = &mut registry.workspaces[index];
        workspace.push_member(normalized.clone())?;
        Ok(workspace.clone())
    })
}

/// Finds a workspace containing `path`.
#[must_use]
pub fn workspace_for_path<'a>(
    registry: &'a WorkspaceRegistry,
    path: &Path,
) -> Option<&'a Workspace> {
    let normalized = normalize_existing_dir(path).ok()?;
    registry
        .workspaces
        .iter()
        .find(|workspace| workspace.member_for_path(&normalized).is_some())
}

/// Outcome of [`archive_member`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveReport {
    /// Workspace the member belonged to, after removal.
    pub workspace: Workspace,
    /// Removed member record.
    pub member: WorkspaceMember,
    /// Whether `git worktree remove` deleted the directory.
    pub worktree_removed: bool,
    /// Whether the shared build directory lived inside the removed worktree
    /// and was therefore unset.
    pub target_dir_cleared: bool,
}

/// Removes a member from a workspace.
///
/// Linked git worktrees are deleted with `git worktree remove`, which keeps the
/// branch; plain directories are only unregistered. A dirty worktree, or one
/// whose state cannot be determined, is refused unless `force` is set. So is a
/// worktree containing the shared build directory; with `force` it is removed
/// and the workspace's `target_dir` is cleared. The last member cannot be
/// archived.
///
/// # Errors
///
/// Returns an error for an unknown workspace or member, an attempt to archive the
/// last member, a dirty worktree or one holding the shared build directory
/// without `force`, a failed `git worktree remove`, or a failed registry write.
pub fn archive_member(
    workspaces_root: &Path,
    workspace_ref: Option<&str>,
    member_ref: &str,
    force: bool,
) -> Result<ArchiveReport> {
    // The whole check-remove-unregister sequence holds the registry lock so a
    // concurrent attach or archive cannot observe a half-archived member.
    update_registry(workspaces_root, |registry| {
        let index = match workspace_ref
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(reference) => registry
                .workspaces
                .iter()
                .position(|workspace| workspace.id == reference || workspace.name == reference)
                .with_context(|| format!("unknown workspace '{reference}'"))?,
            None if registry.workspaces.len() == 1 => 0,
            None => bail!("several workspaces exist; pass a workspace reference"),
        };
        let workspace = &mut registry.workspaces[index];
        let member = find_member(workspace, member_ref)
            .with_context(|| format!("unknown member '{member_ref}'"))?
            .clone();
        if workspace.members.len() <= 1 {
            bail!(
                "workspace '{}' has only one member; archiving it would empty the workspace",
                workspace.name
            );
        }

        let linked = worktree::linked_worktree_common_dir(&member.path).is_some();
        // `git status` ignores ignored files, but `git worktree remove` deletes
        // them, so a shared build directory inside the worktree would vanish
        // for every member without the dirty check noticing.
        let holds_target_dir = linked
            && workspace
                .target_dir
                .as_deref()
                .is_some_and(|target| is_within(target, &member.path));
        if linked {
            // An undeterminable state counts as dirty: deleting the directory
            // is irreversible, so only an explicit --force may skip the check.
            if !force && worktree::is_dirty(&member.path) != Some(false) {
                bail!(
                    "{} has uncommitted changes or its state is unknown; commit them or pass --force",
                    member.path.display()
                );
            }
            if !force && holds_target_dir {
                bail!(
                    "{} holds the workspace's shared build directory, which removing the worktree \
                     would delete; pass --force to remove it and clear the shared build directory",
                    member.path.display()
                );
            }
            worktree::remove_worktree(&member.path, force)?;
        }
        if holds_target_dir {
            workspace.target_dir = None;
        }

        workspace.members.retain(|other| other.path != member.path);
        Ok(ArchiveReport {
            workspace: workspace.clone(),
            member,
            worktree_removed: linked,
            target_dir_cleared: holds_target_dir,
        })
    })
}

/// Finds a member by display name or by path.
#[must_use]
pub fn find_member<'a>(workspace: &'a Workspace, reference: &str) -> Option<&'a WorkspaceMember> {
    let path = Path::new(reference);
    let normalized = normalize_existing_dir(path).ok();
    workspace.members.iter().find(|member| {
        member.name == reference
            || member.path.as_path() == path
            || normalized.as_deref() == Some(member.path.as_path())
    })
}

/// Finds a workspace by id or display name.
#[must_use]
pub fn find_workspace<'a>(
    registry: &'a WorkspaceRegistry,
    reference: &str,
) -> Option<&'a Workspace> {
    registry
        .workspaces
        .iter()
        .find(|workspace| workspace.id == reference || workspace.name == reference)
}

/// Builds the environment that tells a spawned agent where it is.
///
/// Returns `None` when `member` belongs to no workspace. Errors only when the
/// registry itself cannot be read.
///
/// # Errors
///
/// Returns an error when the registry file cannot be read or parsed.
pub fn agent_env_at(
    workspaces_root: &Path,
    member: &Path,
) -> Result<Option<HashMap<String, String>>> {
    let registry = load_registry(workspaces_root)?;
    let Some(workspace) = workspace_for_path(&registry, member) else {
        return Ok(None);
    };
    let Some(found) = workspace.member_for_path(member) else {
        return Ok(None);
    };
    let mut env = HashMap::new();
    env.insert("TESHI_WORKSPACE_ID".to_owned(), workspace.id.clone());
    env.insert("TESHI_WORKSPACE_NAME".to_owned(), workspace.name.clone());
    env.insert(
        "TESHI_WORKSPACE_STATE_DIR".to_owned(),
        workspaces_root.join(&workspace.id).display().to_string(),
    );
    env.insert(
        "TESHI_WORKSPACE_ROOT".to_owned(),
        found.path.display().to_string(),
    );
    env.insert(
        "TESHI_WORKSPACE_PORT".to_owned(),
        (u32::from(workspace.port_base) + u32::from(found.slot)).to_string(),
    );
    env.insert("TESHI_WORKTREE_NAME".to_owned(), found.name.clone());
    if let Some(branch) = &found.branch {
        env.insert("TESHI_WORKTREE_BRANCH".to_owned(), branch.clone());
    }
    if let Some(target_dir) = &workspace.target_dir {
        let value = target_dir.display().to_string();
        env.insert("TESHI_WORKSPACE_TARGET_DIR".to_owned(), value.clone());
        // Agents building inside a member reuse one dependency build.
        env.insert("CARGO_TARGET_DIR".to_owned(), value);
    }
    let members: Vec<serde_json::Value> = workspace
        .members
        .iter()
        .map(|member| {
            serde_json::json!({
                "name": member.name,
                "path": member.path,
                "branch": member.branch,
                "slot": member.slot,
            })
        })
        .collect();
    env.insert(
        "TESHI_WORKSPACE_MEMBERS".to_owned(),
        serde_json::Value::Array(members).to_string(),
    );
    Ok(Some(env))
}

/// Best-effort [`agent_env_at`] for the user-level registry.
///
/// Agent startup must not fail because the registry is missing or unreadable, so
/// failures degrade to "no workspace context". A `CARGO_TARGET_DIR` already set
/// for this process wins over the workspace's shared build directory.
#[must_use]
pub fn agent_env(member: &Path) -> HashMap<String, String> {
    let mut env = workspaces_dir(None)
        .ok()
        .and_then(|root| agent_env_at(&root, member).ok().flatten())
        .unwrap_or_default();
    if std::env::var_os("CARGO_TARGET_DIR").is_some() {
        env.remove("CARGO_TARGET_DIR");
    }
    env
}

/// Last modification time of the registry file, if it exists.
///
/// Lets long-lived readers such as the TUI notice CLI edits cheaply without
/// re-parsing the registry on every frame.
#[must_use]
pub fn registry_modified(workspaces_root: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(registry_path(workspaces_root))
        .and_then(|metadata| metadata.modified())
        .ok()
}

/// Membership of one directory, for display in the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceContext {
    /// Workspace display name.
    pub workspace_name: String,
    /// Workspace identity.
    pub workspace_id: String,
    /// Member name inside the workspace.
    pub member_name: String,
    /// Checked-out branch, when known.
    pub branch: Option<String>,
    /// Port slot inside the workspace.
    pub slot: u16,
    /// Number of members sharing the workspace.
    pub member_count: usize,
}

impl WorkspaceContext {
    /// Compact label such as `main @ dev/x`, or just the member name.
    #[must_use]
    pub fn badge(&self) -> String {
        match &self.branch {
            Some(branch) => format!("{} @ {branch}", self.member_name),
            None => self.member_name.clone(),
        }
    }
}

/// Resolves display context for `member`.
///
/// # Errors
///
/// Returns an error when the registry file cannot be read or parsed.
pub fn context_at(workspaces_root: &Path, member: &Path) -> Result<Option<WorkspaceContext>> {
    let registry = load_registry(workspaces_root)?;
    let Some(workspace) = workspace_for_path(&registry, member) else {
        return Ok(None);
    };
    let Some(found) = workspace.member_for_path(member) else {
        return Ok(None);
    };
    Ok(Some(WorkspaceContext {
        workspace_name: workspace.name.clone(),
        workspace_id: workspace.id.clone(),
        member_name: found.name.clone(),
        branch: found.branch.clone(),
        slot: found.slot,
        member_count: workspace.members.len(),
    }))
}

/// Best-effort [`context_at`] for the user-level registry.
#[must_use]
pub fn context(member: &Path) -> Option<WorkspaceContext> {
    workspaces_dir(None)
        .ok()
        .and_then(|root| context_at(&root, member).ok().flatten())
}

/// Requirement identity owned by the workspace containing `dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProjectIdentity {
    /// Workspace identity, used to persist a newly assigned project id.
    pub workspace_id: String,
    /// Shared requirement-library project id, absent before first use.
    pub project_id: Option<String>,
}

/// Resolves the workspace-level requirement identity for `dir`.
///
/// The registry is the source of truth: every member of a workspace reports the
/// same project id, which is why members no longer need their own
/// `.teshi/project.json`.
///
/// # Errors
///
/// Returns an error when the registry file cannot be read or parsed.
pub fn project_identity(dir: &Path) -> Result<Option<WorkspaceProjectIdentity>> {
    let root = workspaces_dir(None)?;
    let registry = load_registry(&root)?;
    let Some(workspace) = workspace_for_path(&registry, dir) else {
        return Ok(None);
    };
    Ok(Some(WorkspaceProjectIdentity {
        workspace_id: workspace.id.clone(),
        project_id: workspace.project_id.clone(),
    }))
}

/// Stores the requirement project id assigned to a workspace, unless one exists.
///
/// The check runs under the registry lock, so concurrent first uses from two
/// members agree on a single id. Returns the id now stored, which is the
/// existing one when another caller won the race.
///
/// # Errors
///
/// Returns an error when the workspace is unknown or the registry cannot be
/// written.
pub fn set_project_id(workspace_id: &str, project_id: &str) -> Result<String> {
    let root = workspaces_dir(None)?;
    update_registry(&root, |registry| {
        let workspace = registry
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
            .with_context(|| format!("unknown workspace '{workspace_id}'"))?;
        Ok(workspace
            .project_id
            .get_or_insert_with(|| project_id.to_owned())
            .clone())
    })
}

/// Canonical test-points file for `project_root`.
///
/// Members of one workspace share a single file, so parallel checkouts never
/// fight over `<project>/testpoints/testpoints.json`. Unregistered directories
/// keep the legacy per-project path.
///
/// # Errors
///
/// Returns an error when the registry cannot be resolved, read, or parsed.
/// Falling back to the legacy path in that case would silently split one
/// workspace's test points across files.
pub fn testpoints_path(project_root: &Path) -> Result<PathBuf> {
    let root = workspaces_dir(None)?;
    let registry = load_registry(&root)?;
    Ok(match workspace_for_path(&registry, project_root) {
        Some(workspace) => root
            .join(&workspace.id)
            .join("testpoints")
            .join("testpoints.json"),
        None => teshi_core::authoring::testpoints_file(project_root),
    })
}

/// Shared build directory used when none is given: `<first member>/target`,
/// but only for a Cargo project, where `CARGO_TARGET_DIR` means something.
fn default_target_dir(members: &[PathBuf]) -> Option<PathBuf> {
    members
        .first()
        .filter(|dir| dir.join("Cargo.toml").is_file())
        .map(|dir| dir.join("target"))
}

impl Workspace {
    /// Registers `dir` with a de-duplicated name and the lowest free slot.
    fn push_member(&mut self, dir: PathBuf) -> Result<()> {
        let slot = self.next_slot()?;
        let base = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "member".to_owned());
        let mut name = base.clone();
        let mut suffix = 2;
        while self.members.iter().any(|member| member.name == name) {
            name = format!("{base}-{suffix}");
            suffix += 1;
        }
        self.members.push(WorkspaceMember {
            name,
            branch: worktree::current_branch(&dir),
            path: dir,
            slot,
        });
        Ok(())
    }
}

/// Reads a legacy per-directory project id, if the member has one.
fn project_id_from_member(dir: &Path) -> Option<String> {
    let bytes = std::fs::read(dir.join(".teshi").join("project.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("id")?.as_str().map(str::to_owned)
}

fn registry_path(workspaces_root: &Path) -> PathBuf {
    workspaces_root.join(REGISTRY_FILE)
}

fn read_registry_unlocked(path: &Path) -> Result<WorkspaceRegistry> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let registry: WorkspaceRegistry = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", path.display()))?;
            Ok(registry)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(WorkspaceRegistry::default())
        }
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn update_registry<T>(
    workspaces_root: &Path,
    update: impl FnOnce(&mut WorkspaceRegistry) -> Result<T>,
) -> Result<T> {
    let path = registry_path(workspaces_root);
    with_exclusive_lock(&path, || {
        let mut registry = read_registry_unlocked(&path)?;
        if registry.version > REGISTRY_VERSION {
            bail!(
                "workspace registry version {} is newer than the supported version {REGISTRY_VERSION}",
                registry.version
            );
        }
        registry.version = REGISTRY_VERSION;
        let value = update(&mut registry)?;
        write_json_unlocked(&path, &registry)?;
        Ok(value)
    })
}

/// Rejects a directory that already belongs to any workspace.
fn ensure_unclaimed(registry: &WorkspaceRegistry, dir: &Path) -> Result<()> {
    if let Some(workspace) = workspace_for_path(registry, dir) {
        bail!(
            "{} already belongs to workspace '{}'",
            dir.display(),
            workspace.name
        );
    }
    Ok(())
}

/// Canonicalizes an existing directory so member paths are comparable.
///
/// Windows paths are case-insensitive; canonicalization resolves the on-disk
/// spelling, which keeps equality checks stable regardless of how the user
/// typed the path.
fn normalize_existing_dir(path: &Path) -> Result<PathBuf> {
    let canonical = dunce::canonicalize(path)
        .with_context(|| format!("resolve directory {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("not a directory: {}", path.display());
    }
    Ok(canonical)
}

/// Whether `path` is `dir` or lies inside it.
///
/// `path` may not exist yet (a build directory before the first build), so it is
/// canonicalized only when possible; `dir` is an already normalized member path.
fn is_within(path: &Path, dir: &Path) -> bool {
    dunce::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .starts_with(dir)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().context("resolve current directory")?;
    Ok(cwd.join(path))
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
    fn missing_registry_loads_empty() {
        let root = tempfile::tempdir().unwrap();
        let registry = load_registry(root.path()).unwrap();
        assert!(registry.workspaces.is_empty());
        assert_eq!(registry.version, REGISTRY_VERSION);
    }

    #[test]
    fn creates_workspace_with_slots_and_names() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");

        let workspace =
            create_workspace(root.path(), "demo", &[first.clone(), second.clone()]).unwrap();

        assert_eq!(workspace.members.len(), 2);
        assert_eq!(workspace.members[0].name, "alpha");
        assert_eq!(workspace.members[0].slot, 0);
        assert_eq!(workspace.members[1].name, "beta");
        assert_eq!(workspace.members[1].slot, 1);
        assert_eq!(
            workspace.members[0].path,
            dunce::canonicalize(&first).unwrap()
        );
        assert_eq!(workspace.port_base, DEFAULT_PORT_BASE);

        let reloaded = load_registry(root.path()).unwrap();
        assert_eq!(reloaded.workspaces.len(), 1);
        assert_eq!(reloaded.workspaces[0].id, workspace.id);
    }

    #[test]
    fn rejects_duplicate_workspace_name() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        create_workspace(root.path(), "demo", &[first]).unwrap();
        let second = dir(root.path(), "beta");

        let error = create_workspace(root.path(), "demo", &[second]).unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
    }

    #[test]
    fn rejects_directory_owned_by_another_workspace() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        create_workspace(root.path(), "demo", std::slice::from_ref(&first)).unwrap();

        let error = attach_member(root.path(), None, &first).unwrap_err();
        assert!(error.to_string().contains("already belongs"), "{error}");
    }

    #[test]
    fn rejects_missing_directory() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");

        let error = create_workspace(root.path(), "demo", &[missing]).unwrap_err();
        assert!(error.to_string().contains("resolve directory"), "{error}");
    }

    #[test]
    fn attach_uses_the_only_workspace() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        create_workspace(root.path(), "demo", &[first]).unwrap();
        let second = dir(root.path(), "beta");

        let workspace = attach_member(root.path(), None, &second).unwrap();
        assert_eq!(workspace.name, "demo");
        assert_eq!(workspace.members.len(), 2);
        assert_eq!(workspace.members[1].slot, 1);
    }

    #[test]
    fn attach_requires_reference_when_several_workspaces_exist() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        create_workspace(root.path(), "one", &[first]).unwrap();
        let second = dir(root.path(), "beta");
        create_workspace(root.path(), "two", &[second]).unwrap();
        let third = dir(root.path(), "gamma");

        let error = attach_member(root.path(), None, &third).unwrap_err();
        assert!(error.to_string().contains("--workspace"), "{error}");

        let workspace = attach_member(root.path(), Some("two"), &third).unwrap();
        assert_eq!(workspace.name, "two");
    }

    #[test]
    fn resolves_workspace_from_member_path_with_redundant_segments() {
        let root = tempfile::tempdir().unwrap();
        let member = dir(root.path(), "alpha");
        let workspace =
            create_workspace(root.path(), "demo", std::slice::from_ref(&member)).unwrap();

        let registry = load_registry(root.path()).unwrap();
        let indirect = member.join("..").join("alpha");
        let found = workspace_for_path(&registry, &indirect).expect("workspace for member");
        assert_eq!(found.id, workspace.id);
    }

    #[test]
    fn member_names_are_deduplicated() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = {
            let nested = root.path().join("nested");
            std::fs::create_dir_all(&nested).unwrap();
            dir(&nested, "alpha")
        };

        let workspace = create_workspace(root.path(), "demo", &[first, second]).unwrap();
        assert_eq!(workspace.members[0].name, "alpha");
        assert_eq!(workspace.members[1].name, "alpha-2");
    }

    /// Restores `TESHI_WORKSPACES_DIR` on drop, including when a test panics.
    struct EnvRestore(Option<std::ffi::OsString>);

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            // SAFETY: dropped while the shared test env lock is still held.
            match self.0.take() {
                Some(previous) => unsafe { std::env::set_var(WORKSPACES_DIR_ENV, previous) },
                None => unsafe { std::env::remove_var(WORKSPACES_DIR_ENV) },
            }
        }
    }

    /// Runs `f` with `TESHI_WORKSPACES_DIR` pointing at a temporary registry.
    fn with_workspaces_env<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = crate::app_data::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Declared after the lock guard so it drops first, while still locked.
        let _restore = EnvRestore(std::env::var_os(WORKSPACES_DIR_ENV));
        // SAFETY: the shared test env lock serializes every override.
        unsafe { std::env::set_var(WORKSPACES_DIR_ENV, dir) };
        f()
    }

    #[test]
    fn workspace_supplies_one_requirement_identity_for_all_members() {
        let root = tempfile::tempdir().unwrap();
        let registry_root = root.path().join("registry");
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");

        with_workspaces_env(&registry_root, || {
            let workspace =
                create_workspace(&registry_root, "demo", &[first.clone(), second.clone()]).unwrap();

            let resolved = crate::authoring::read_requirement_project(&second)
                .unwrap()
                .expect("workspace project id");
            assert_eq!(Some(resolved.id.clone()), workspace.project_id);
            assert_eq!(
                crate::authoring::read_requirement_project(&first)
                    .unwrap()
                    .map(|project| project.id),
                workspace.project_id
            );
            assert!(!second.join(".teshi").join("project.json").exists());
        });
    }

    #[test]
    fn workspace_members_share_one_testpoints_file() {
        let root = tempfile::tempdir().unwrap();
        let registry_root = root.path().join("registry");
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");

        with_workspaces_env(&registry_root, || {
            let workspace =
                create_workspace(&registry_root, "demo", &[first.clone(), second.clone()]).unwrap();
            let expected = registry_root
                .join(&workspace.id)
                .join("testpoints")
                .join("testpoints.json");

            assert_eq!(testpoints_path(&first).unwrap(), expected);
            assert_eq!(testpoints_path(&second).unwrap(), expected);
        });
    }

    #[test]
    fn unregistered_directory_keeps_the_legacy_testpoints_path() {
        let root = tempfile::tempdir().unwrap();
        let registry_root = root.path().join("registry");
        let outside = dir(root.path(), "outside");

        with_workspaces_env(&registry_root, || {
            assert_eq!(
                testpoints_path(&outside).unwrap(),
                outside.join("testpoints").join("testpoints.json")
            );
        });
    }

    #[test]
    fn corrupt_registry_fails_testpoints_resolution_instead_of_falling_back() {
        let root = tempfile::tempdir().unwrap();
        let registry_root = root.path().join("registry");
        std::fs::create_dir_all(&registry_root).unwrap();
        std::fs::write(registry_root.join(REGISTRY_FILE), "{not json").unwrap();
        let member = dir(root.path(), "alpha");

        with_workspaces_env(&registry_root, || {
            let error = testpoints_path(&member).unwrap_err();
            assert!(error.to_string().contains("parse"), "{error}");
        });
    }

    #[test]
    fn existing_project_id_wins_over_a_concurrent_assignment() {
        let root = tempfile::tempdir().unwrap();
        let registry_root = root.path().join("registry");
        let member = dir(root.path(), "alpha");

        with_workspaces_env(&registry_root, || {
            let workspace =
                create_workspace(&registry_root, "demo", std::slice::from_ref(&member)).unwrap();
            let existing = workspace.project_id.clone().expect("assigned at creation");

            let stored = set_project_id(&workspace.id, "late-candidate").unwrap();

            assert_eq!(stored, existing);
            let registry = load_registry(&registry_root).unwrap();
            assert_eq!(registry.workspaces[0].project_id, Some(existing));
        });
    }

    #[test]
    fn each_workspace_gets_its_own_port_block() {
        let root = tempfile::tempdir().unwrap();
        let first = create_workspace(root.path(), "one", &[dir(root.path(), "alpha")]).unwrap();
        let second = create_workspace(root.path(), "two", &[dir(root.path(), "beta")]).unwrap();

        assert_eq!(first.port_base, DEFAULT_PORT_BASE);
        assert_eq!(second.port_base, DEFAULT_PORT_BASE + PORT_BLOCK_SIZE);
        assert!(
            first.port_base > 20_253,
            "teshi web's default port stays free"
        );
    }

    #[test]
    fn archived_slots_are_reused() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");
        let third = dir(root.path(), "gamma");
        create_workspace(root.path(), "demo", &[first, second, third]).unwrap();
        archive_member(root.path(), Some("demo"), "beta", false).unwrap();

        let workspace =
            attach_member(root.path(), Some("demo"), &dir(root.path(), "delta")).unwrap();

        let delta = workspace
            .members
            .iter()
            .find(|member| member.name == "delta")
            .unwrap();
        assert_eq!(delta.slot, 1);
    }

    #[test]
    fn empty_name_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let member = dir(root.path(), "alpha");

        let error = create_workspace(root.path(), "   ", &[member]).unwrap_err();
        assert!(error.to_string().contains("must not be empty"), "{error}");
    }

    #[test]
    fn agent_env_describes_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");
        let workspace = create_workspace(root.path(), "demo", &[first, second.clone()]).unwrap();

        let env = agent_env_at(root.path(), &second)
            .unwrap()
            .expect("member env");

        assert_eq!(env["TESHI_WORKSPACE_ID"], workspace.id);
        assert_eq!(env["TESHI_WORKSPACE_NAME"], "demo");
        assert_eq!(env["TESHI_WORKTREE_NAME"], "beta");
        assert_eq!(
            env["TESHI_WORKSPACE_PORT"],
            (u32::from(DEFAULT_PORT_BASE) + 1).to_string()
        );
        assert_eq!(
            env["TESHI_WORKSPACE_STATE_DIR"],
            root.path().join(&workspace.id).display().to_string()
        );
        let members: serde_json::Value =
            serde_json::from_str(&env["TESHI_WORKSPACE_MEMBERS"]).unwrap();
        assert_eq!(members.as_array().unwrap().len(), 2);
        assert_eq!(members[1]["name"], "beta");
    }

    #[test]
    fn context_reports_member_and_siblings() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");
        create_workspace(root.path(), "demo", &[first, second.clone()]).unwrap();

        let context = context_at(root.path(), &second)
            .unwrap()
            .expect("member context");

        assert_eq!(context.workspace_name, "demo");
        assert_eq!(context.member_name, "beta");
        assert_eq!(context.slot, 1);
        assert_eq!(context.member_count, 2);
        assert_eq!(context.badge(), "beta");
    }

    #[test]
    fn badge_includes_the_branch_when_known() {
        let context = WorkspaceContext {
            workspace_name: "demo".to_owned(),
            workspace_id: "id".to_owned(),
            member_name: "beta".to_owned(),
            branch: Some("dev/x".to_owned()),
            slot: 0,
            member_count: 2,
        };

        assert_eq!(context.badge(), "beta @ dev/x");
    }

    #[test]
    fn non_cargo_workspace_has_no_default_target_dir() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");

        let workspace =
            create_workspace(root.path(), "demo", std::slice::from_ref(&first)).unwrap();

        assert_eq!(workspace.target_dir, None);
        let env = agent_env_at(root.path(), &first)
            .unwrap()
            .expect("member env");
        assert!(!env.contains_key("CARGO_TARGET_DIR"));
    }

    #[test]
    fn shared_target_dir_defaults_to_first_member() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        std::fs::write(first.join("Cargo.toml"), "[workspace]\n").unwrap();
        let second = dir(root.path(), "beta");
        let workspace =
            create_workspace(root.path(), "demo", &[first.clone(), second.clone()]).unwrap();

        let expected = dunce::canonicalize(&first).unwrap().join("target");
        assert_eq!(workspace.target_dir, Some(expected.clone()));

        let env = agent_env_at(root.path(), &second)
            .unwrap()
            .expect("member env");
        assert_eq!(env["CARGO_TARGET_DIR"], expected.display().to_string());
        assert_eq!(
            env["TESHI_WORKSPACE_TARGET_DIR"],
            expected.display().to_string()
        );
    }

    #[test]
    fn explicit_target_dir_overrides_the_default() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let shared = root.path().join("shared-target");

        let workspace = create_workspace_with_target(
            root.path(),
            "demo",
            std::slice::from_ref(&first),
            Some(shared.clone()),
        )
        .unwrap();

        assert_eq!(workspace.target_dir, Some(shared));
    }

    #[test]
    fn agent_env_is_absent_for_unregistered_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = dir(root.path(), "outside");

        assert!(agent_env_at(root.path(), &outside).unwrap().is_none());
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_repo(root: &Path) -> PathBuf {
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.email", "teshi@example.com"]);
        git(&repo, &["config", "user.name", "Teshi Test"]);
        std::fs::write(repo.join("README.md"), "fixture\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "init"]);
        repo
    }

    #[test]
    fn archive_unregisters_plain_directories_without_deleting_them() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");
        create_workspace(root.path(), "demo", &[first, second.clone()]).unwrap();

        let report = archive_member(root.path(), Some("demo"), "beta", false).unwrap();

        assert_eq!(report.member.name, "beta");
        assert!(!report.worktree_removed);
        assert!(second.is_dir(), "plain directories are never deleted");
        let registry = load_registry(root.path()).unwrap();
        assert_eq!(registry.workspaces[0].members.len(), 1);
    }

    #[test]
    fn archive_rejects_the_last_member() {
        let root = tempfile::tempdir().unwrap();
        let only = dir(root.path(), "alpha");
        create_workspace(root.path(), "demo", std::slice::from_ref(&only)).unwrap();

        let error = archive_member(root.path(), Some("demo"), "alpha", false).unwrap_err();
        assert!(error.to_string().contains("only one member"), "{error}");
    }

    #[test]
    fn archive_removes_linked_worktrees_and_keeps_the_branch() {
        let root = tempfile::tempdir().unwrap();
        let repo = git_repo(root.path());
        let worktree = root.path().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/x",
                worktree.to_str().unwrap(),
                "main",
            ],
        );
        create_workspace(root.path(), "demo", &[repo.clone(), worktree.clone()]).unwrap();

        let report = archive_member(root.path(), Some("demo"), "wt", false).unwrap();

        assert!(report.worktree_removed);
        assert!(!worktree.exists());
        let branches = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["branch", "--list", "feat/x"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&branches.stdout).contains("feat/x"));
        let registry = load_registry(root.path()).unwrap();
        assert_eq!(registry.workspaces[0].members.len(), 1);
    }

    #[test]
    fn archive_refuses_dirty_worktrees_without_force() {
        let root = tempfile::tempdir().unwrap();
        let repo = git_repo(root.path());
        let worktree = root.path().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/x",
                worktree.to_str().unwrap(),
                "main",
            ],
        );
        std::fs::write(worktree.join("scratch.txt"), "uncommitted").unwrap();
        create_workspace(root.path(), "demo", &[repo, worktree.clone()]).unwrap();

        let error = archive_member(root.path(), Some("demo"), "wt", false).unwrap_err();
        assert!(error.to_string().contains("uncommitted changes"), "{error}");

        let report = archive_member(root.path(), Some("demo"), "wt", true).unwrap();
        assert!(report.worktree_removed);
        assert!(!worktree.exists());
    }

    #[test]
    fn archive_protects_a_shared_build_directory_inside_the_worktree() {
        let root = tempfile::tempdir().unwrap();
        let repo = git_repo(root.path());
        let worktree = root.path().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/x",
                worktree.to_str().unwrap(),
                "main",
            ],
        );
        std::fs::write(worktree.join("Cargo.toml"), "[workspace]\n").unwrap();
        git(&worktree, &["add", "Cargo.toml"]);
        git(&worktree, &["commit", "-m", "cargo"]);
        // The linked worktree leads, so the default shared build dir lives in it.
        let workspace = create_workspace(root.path(), "demo", &[worktree.clone(), repo]).unwrap();
        assert!(workspace.target_dir.is_some());

        let error = archive_member(root.path(), Some("demo"), "wt", false).unwrap_err();
        assert!(
            error.to_string().contains("shared build directory"),
            "{error}"
        );
        assert!(worktree.exists(), "a refused archive deletes nothing");

        let report = archive_member(root.path(), Some("demo"), "wt", true).unwrap();
        assert!(report.worktree_removed);
        assert!(report.target_dir_cleared);
        assert_eq!(report.workspace.target_dir, None);
        let registry = load_registry(root.path()).unwrap();
        assert_eq!(registry.workspaces[0].target_dir, None);
    }

    #[test]
    fn relative_target_dir_is_stored_as_absolute() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");

        let workspace = create_workspace_with_target(
            root.path(),
            "demo",
            std::slice::from_ref(&first),
            Some(PathBuf::from("shared-target")),
        )
        .unwrap();

        let stored = workspace.target_dir.expect("explicit target dir");
        assert!(stored.is_absolute(), "{}", stored.display());
    }
}
