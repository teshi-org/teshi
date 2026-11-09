//! `teshi workspace` subcommands.
//!
//! The registry lives in the user-level app data directory; these commands
//! register directories, optionally create a linked git worktree, and apply the
//! workspace's `workspace.toml` to new members.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use teshi_engine::workspace::{self, InitReport, Workspace, WorkspaceRegistry, config, worktree};

use crate::cli::WorkspaceCommand;

/// Runs one `teshi workspace` subcommand.
///
/// # Errors
///
/// Returns an error when the registry cannot be resolved, the request is
/// invalid, or the registry cannot be written.
pub fn handle_workspace_command(action: &WorkspaceCommand) -> Result<()> {
    let root = workspace::workspaces_dir(None)?;
    println!("{}", execute_workspace_command(action, &root)?);
    Ok(())
}

/// Executes a subcommand against an explicit registry root.
///
/// Keeping the root explicit makes the command testable without touching the
/// user-level registry or process environment.
fn execute_workspace_command(action: &WorkspaceCommand, root: &Path) -> Result<String> {
    match action {
        WorkspaceCommand::New {
            name,
            dirs,
            from,
            branch,
            worktree: worktree_dir,
            base,
            target_dir,
            json,
        } => {
            let request = new_worktree_request(
                from.as_deref(),
                branch.as_deref(),
                worktree_dir.as_deref(),
                base.as_deref(),
            )?;
            let mut members = dirs.clone();
            if let Some(request) = &request {
                // The repository leads so later workspace.toml includes copy from it.
                members.insert(0, request.repo.clone());
            }
            if members.is_empty() {
                bail!("workspace needs at least one --dir or --from");
            }
            // Validate everything the registry will check before creating the
            // worktree, so a rejected request leaves no branch or directory.
            let registry = workspace::load_registry(root)?;
            ensure_name_available(&registry, name)?;
            for dir in &members {
                ensure_unclaimed(&registry, dir)?;
            }
            let created = request.as_ref().map(WorktreeRequest::create).transpose()?;
            members.extend(created.iter().cloned());
            let workspace = with_rollback(request.as_ref(), created.as_deref(), || {
                workspace::create_workspace_with_target(root, name, &members, target_dir.clone())
            })?;
            // A fresh workspace has an empty workspace.toml, so there is nothing
            // to initialize yet; `attach` and `init` apply it later.
            if *json {
                return Ok(serde_json::json!({"ok": true, "workspace": workspace}).to_string());
            }
            Ok(format!(
                "Created workspace {} ({}) with {} member(s)",
                workspace.name,
                workspace.id,
                workspace.members.len()
            ))
        }
        WorkspaceCommand::Attach {
            path,
            workspace: reference,
            from,
            branch,
            base,
            no_init,
            json,
        } => {
            let request =
                attach_worktree_request(from.as_deref(), branch.as_deref(), path, base.as_deref())?;
            let created = match &request {
                Some(request) => {
                    let registry = workspace::load_registry(root)?;
                    ensure_attach_target(&registry, reference.as_deref())?;
                    Some(request.create()?)
                }
                None => None,
            };
            let member_path = created.clone().unwrap_or_else(|| path.clone());
            let workspace = with_rollback(request.as_ref(), created.as_deref(), || {
                workspace::attach_member(root, reference.as_deref(), &member_path)
            })?;
            let path = &member_path;
            let reports = if *no_init {
                Vec::new()
            } else {
                let config = config::load_config(root, &workspace.id)?;
                init_member(&workspace, path, &config)?
                    .into_iter()
                    .collect()
            };
            if *json {
                return Ok(serde_json::json!({
                    "ok": true,
                    "workspace": workspace,
                    "init": reports_json(&reports),
                })
                .to_string());
            }
            let member = workspace
                .member_for_path(path)
                .with_context(|| format!("{} is missing from the registry", path.display()))?;
            let mut output = format!(
                "Attached {} as '{}' to workspace {} ({})",
                member.path.display(),
                member.name,
                workspace.name,
                workspace.id
            );
            output.push('\n');
            output.push_str(&render_reports(&reports));
            Ok(output)
        }
        WorkspaceCommand::Init {
            workspace: reference,
            json,
        } => {
            let registry = workspace::load_registry(root)?;
            let workspace = select_workspace(&registry, reference.as_deref())?.clone();
            let reports = init_extra_members(root, &workspace)?;
            if *json {
                return Ok(serde_json::json!({
                    "ok": true,
                    "workspace": workspace,
                    "init": reports_json(&reports),
                })
                .to_string());
            }
            Ok(render_reports(&reports))
        }
        WorkspaceCommand::Archive {
            member,
            workspace: reference,
            force,
            json,
        } => {
            let report = workspace::archive_member(root, reference.as_deref(), member, *force)?;
            if *json {
                return Ok(serde_json::json!({
                    "ok": true,
                    "workspace": report.workspace,
                    "member": report.member,
                    "worktree_removed": report.worktree_removed,
                    "target_dir_cleared": report.target_dir_cleared,
                })
                .to_string());
            }
            let mut output = format!(
                "Archived '{}' from workspace {} ({})",
                report.member.name,
                report.workspace.name,
                if report.worktree_removed {
                    "worktree removed, branch kept"
                } else {
                    "directory kept, unregistered only"
                }
            );
            if report.target_dir_cleared {
                output.push_str(
                    "\nThe shared build directory was inside the removed worktree; \
                     members no longer share one",
                );
            }
            Ok(output)
        }
        WorkspaceCommand::List { json } => {
            let registry = workspace::load_registry(root)?;
            if *json {
                return Ok(serde_json::json!({
                    "ok": true,
                    "registry": root,
                    "workspaces": registry.workspaces,
                })
                .to_string());
            }
            Ok(render_list(&registry, root))
        }
        WorkspaceCommand::Show {
            workspace: reference,
            json,
        } => {
            let registry = workspace::load_registry(root)?;
            let workspace = select_workspace(&registry, reference.as_deref())?;
            if *json {
                return Ok(workspace_envelope(workspace));
            }
            Ok(render_show(workspace))
        }
    }
}

/// A linked worktree the command creates before registering it.
struct WorktreeRequest {
    repo: PathBuf,
    branch: String,
    path: PathBuf,
    base: Option<String>,
}

impl WorktreeRequest {
    fn create(&self) -> Result<PathBuf> {
        worktree::add_worktree(&self.repo, &self.branch, &self.path, self.base.as_deref())
    }
}

/// Parses the `--from/--branch/--worktree/--base` group of `workspace new`.
fn new_worktree_request(
    from: Option<&Path>,
    branch: Option<&str>,
    worktree_dir: Option<&Path>,
    base: Option<&str>,
) -> Result<Option<WorktreeRequest>> {
    match (from, branch, worktree_dir) {
        (Some(repo), Some(branch), Some(path)) => Ok(Some(WorktreeRequest {
            repo: repo.to_path_buf(),
            branch: branch.to_owned(),
            path: path.to_path_buf(),
            base: base.map(str::to_owned),
        })),
        (None, None, None) if base.is_none() => Ok(None),
        (None, None, None) => bail!("--base requires --from"),
        (Some(_), _, _) => bail!("--from requires both --branch and --worktree"),
        (None, _, _) => bail!("--branch, --worktree, and --base require --from"),
    }
}

/// Parses the `--from/--branch/--base` group of `workspace attach`; the
/// positional path is where the worktree goes.
fn attach_worktree_request(
    from: Option<&Path>,
    branch: Option<&str>,
    path: &Path,
    base: Option<&str>,
) -> Result<Option<WorktreeRequest>> {
    match (from, branch) {
        (Some(repo), Some(branch)) => Ok(Some(WorktreeRequest {
            repo: repo.to_path_buf(),
            branch: branch.to_owned(),
            path: path.to_path_buf(),
            base: base.map(str::to_owned),
        })),
        (None, None) if base.is_none() => Ok(None),
        (Some(_), None) => bail!("--from requires --branch"),
        (None, _) => bail!("--branch and --base require --from"),
    }
}

/// Runs `register`, discarding a just-created worktree if registration fails.
fn with_rollback<T>(
    request: Option<&WorktreeRequest>,
    created: Option<&Path>,
    register: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let error = match register() {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    if let (Some(request), Some(path)) = (request, created)
        && let Err(cleanup) = worktree::discard_worktree(&request.repo, path, &request.branch)
    {
        bail!(
            "{error:#}; removing the new worktree {} also failed: {cleanup:#}",
            path.display()
        );
    }
    Err(error)
}

fn ensure_name_available(registry: &WorkspaceRegistry, name: &str) -> Result<()> {
    let name = name.trim();
    if registry
        .workspaces
        .iter()
        .any(|workspace| workspace.name == name)
    {
        bail!("workspace '{name}' already exists");
    }
    Ok(())
}

fn ensure_unclaimed(registry: &WorkspaceRegistry, dir: &Path) -> Result<()> {
    if let Some(owner) = workspace::workspace_for_path(registry, dir) {
        bail!(
            "{} already belongs to workspace '{}'",
            dir.display(),
            owner.name
        );
    }
    Ok(())
}

/// Checks that `attach` will find exactly one target workspace.
fn ensure_attach_target(registry: &WorkspaceRegistry, reference: Option<&str>) -> Result<()> {
    match reference.map(str::trim).filter(|value| !value.is_empty()) {
        Some(reference) => {
            workspace::find_workspace(registry, reference)
                .with_context(|| format!("unknown workspace '{reference}'"))?;
        }
        None if registry.workspaces.len() == 1 => {}
        None if registry.workspaces.is_empty() => bail!(
            "no workspace yet; create one with `teshi workspace new --name <name> --dir <path>`"
        ),
        None => bail!("several workspaces exist; pass --workspace"),
    }
    Ok(())
}

fn init_extra_members(root: &Path, workspace: &Workspace) -> Result<Vec<(String, InitReport)>> {
    let Some(source) = workspace.members.first() else {
        return Ok(Vec::new());
    };
    let config = config::load_config(root, &workspace.id)?;
    let mut reports = Vec::new();
    for member in workspace.members.iter().skip(1) {
        let report = config::initialize_member(&source.path, &member.path, &config)?;
        reports.push((member.name.clone(), report));
    }
    Ok(reports)
}

fn init_member(
    workspace: &Workspace,
    path: &Path,
    config: &config::WorkspaceConfig,
) -> Result<Option<(String, InitReport)>> {
    let Some(member) = workspace.member_for_path(path) else {
        return Ok(None);
    };
    let Some(source) = workspace.members.first() else {
        return Ok(None);
    };
    let report = config::initialize_member(&source.path, &member.path, config)?;
    Ok(Some((member.name.clone(), report)))
}

fn select_workspace<'a>(
    registry: &'a WorkspaceRegistry,
    reference: Option<&str>,
) -> Result<&'a Workspace> {
    match reference.map(str::trim).filter(|value| !value.is_empty()) {
        Some(reference) => workspace::find_workspace(registry, reference)
            .with_context(|| format!("unknown workspace '{reference}'")),
        None => {
            let cwd = std::env::current_dir().context("resolve current directory")?;
            if let Some(workspace) = workspace::workspace_for_path(registry, &cwd) {
                return Ok(workspace);
            }
            if registry.workspaces.len() == 1 {
                return Ok(&registry.workspaces[0]);
            }
            bail!(
                "{} belongs to no workspace; pass a workspace id or name",
                cwd.display()
            )
        }
    }
}

fn workspace_envelope(workspace: &Workspace) -> String {
    serde_json::json!({"ok": true, "workspace": workspace}).to_string()
}

fn reports_json(reports: &[(String, InitReport)]) -> serde_json::Value {
    serde_json::Value::Array(
        reports
            .iter()
            .map(|(member, report)| {
                serde_json::json!({
                    "member": member,
                    "copied": report.copied,
                    "skipped": report.skipped,
                    "missing": report.missing,
                    "setup_commands": report.setup_commands,
                })
            })
            .collect(),
    )
}

fn render_reports(reports: &[(String, InitReport)]) -> String {
    if reports.is_empty() {
        return "No additional members to initialize".to_owned();
    }
    let mut lines = vec![format!("Initialized {} member(s):", reports.len())];
    for (member, report) in reports {
        lines.push(format!(
            "  {member}: copied {}, skipped {}, missing {}, setup {}",
            report.copied.len(),
            report.skipped.len(),
            report.missing.len(),
            report.setup_commands
        ));
    }
    lines.join("\n")
}

fn render_list(registry: &WorkspaceRegistry, root: &Path) -> String {
    if registry.workspaces.is_empty() {
        return format!("No workspaces registered in {}", root.display());
    }
    registry
        .workspaces
        .iter()
        .map(|workspace| {
            let members: Vec<String> = workspace
                .members
                .iter()
                .map(|member| format!("{}={}", member.name, member.path.display()))
                .collect();
            format!(
                "{}\t{}\t{} member(s): {}",
                workspace.id,
                workspace.name,
                workspace.members.len(),
                members.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_show(workspace: &Workspace) -> String {
    let mut lines = vec![
        format!("{} ({})", workspace.name, workspace.id),
        format!("  port base {}", workspace.port_base),
    ];
    for member in &workspace.members {
        lines.push(format!(
            "  slot {}\t{}\t{}\t{}",
            member.slot,
            member.name,
            member.branch.as_deref().unwrap_or("-"),
            member.path.display()
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    fn dir(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn run(root: &Path, action: WorkspaceCommand) -> Result<String> {
        execute_workspace_command(&action, root)
    }

    fn json(value: &str) -> Value {
        serde_json::from_str(value).expect("json output")
    }

    fn new_command(name: &str, dirs: Vec<PathBuf>) -> WorkspaceCommand {
        WorkspaceCommand::New {
            name: name.to_owned(),
            dirs,
            from: None,
            branch: None,
            worktree: None,
            base: None,
            target_dir: None,
            json: true,
        }
    }

    fn attach_command(path: PathBuf, workspace: Option<&str>, no_init: bool) -> WorkspaceCommand {
        WorkspaceCommand::Attach {
            path,
            workspace: workspace.map(str::to_owned),
            from: None,
            branch: None,
            base: None,
            no_init,
            json: true,
        }
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

    fn branch_exists(repo: &Path, branch: &str) -> bool {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["branch", "--list", branch])
            .output()
            .expect("run git");
        !output.stdout.is_empty()
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

    #[test]
    fn lists_no_workspaces_initially() {
        let root = tempfile::tempdir().unwrap();
        let output = run(root.path(), WorkspaceCommand::List { json: false }).unwrap();
        assert!(output.contains("No workspaces registered"), "{output}");
    }

    #[test]
    fn creates_lists_and_shows_a_workspace() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");

        let created = run(root.path(), new_command("demo", vec![first, second])).unwrap();
        let created = json(&created);
        assert_eq!(created["workspace"]["name"], "demo");
        assert_eq!(created["workspace"]["members"].as_array().unwrap().len(), 2);

        let listed = run(root.path(), WorkspaceCommand::List { json: true }).unwrap();
        let listed = json(&listed);
        assert_eq!(listed["workspaces"].as_array().unwrap().len(), 1);

        let shown = run(
            root.path(),
            WorkspaceCommand::Show {
                workspace: Some("demo".to_owned()),
                json: false,
            },
        )
        .unwrap();
        assert!(shown.contains("demo ("), "{shown}");
        assert!(shown.contains("slot 1"), "{shown}");
    }

    #[test]
    fn workspace_state_directory_holds_the_config() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let created = run(root.path(), new_command("demo", vec![first])).unwrap();
        let created = json(&created);
        let id = created["workspace"]["id"].as_str().unwrap();

        assert!(config::config_path(root.path(), id).is_file());
    }

    #[test]
    fn init_copies_declared_includes_into_new_members() {
        let root = tempfile::tempdir().unwrap();
        let source = dir(root.path(), "alpha");
        std::fs::write(source.join("teshi.toml"), "port = 1").unwrap();
        let created = run(root.path(), new_command("demo", vec![source])).unwrap();
        let workspace: Workspace =
            serde_json::from_value(json(&created)["workspace"].clone()).unwrap();
        config::save_config(
            root.path(),
            &workspace.id,
            &config::WorkspaceConfig {
                include: vec!["teshi.toml".to_owned()],
                ..config::WorkspaceConfig::default()
            },
        )
        .unwrap();

        let member = dir(root.path(), "beta");
        let output = run(
            root.path(),
            attach_command(member.clone(), Some("demo"), false),
        )
        .unwrap();

        let output = json(&output);
        assert_eq!(output["init"][0]["copied"].as_array().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(member.join("teshi.toml")).unwrap(),
            "port = 1"
        );
    }

    #[test]
    fn no_init_skips_copying() {
        let root = tempfile::tempdir().unwrap();
        let source = dir(root.path(), "alpha");
        std::fs::write(source.join("teshi.toml"), "port = 1").unwrap();
        let created = run(root.path(), new_command("demo", vec![source])).unwrap();
        let workspace: Workspace =
            serde_json::from_value(json(&created)["workspace"].clone()).unwrap();
        config::save_config(
            root.path(),
            &workspace.id,
            &config::WorkspaceConfig {
                include: vec!["teshi.toml".to_owned()],
                ..config::WorkspaceConfig::default()
            },
        )
        .unwrap();

        let member = dir(root.path(), "beta");
        run(
            root.path(),
            attach_command(member.clone(), Some("demo"), true),
        )
        .unwrap();

        assert!(!member.join("teshi.toml").exists());
    }

    #[test]
    fn attaches_a_directory_to_the_only_workspace() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        run(root.path(), new_command("demo", vec![first])).unwrap();
        let second = dir(root.path(), "beta");

        let output = run(
            root.path(),
            WorkspaceCommand::Attach {
                path: second,
                workspace: None,
                from: None,
                branch: None,
                base: None,
                no_init: true,
                json: false,
            },
        )
        .unwrap();
        assert!(output.contains("Attached"), "{output}");

        let registry = workspace::load_registry(root.path()).unwrap();
        assert_eq!(registry.workspaces[0].members.len(), 2);
    }

    #[test]
    fn new_requires_from_for_worktree_flags() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let error = run(
            root.path(),
            WorkspaceCommand::New {
                name: "demo".to_owned(),
                dirs: vec![first],
                from: None,
                branch: Some("feature/x".to_owned()),
                worktree: None,
                base: None,
                target_dir: None,
                json: false,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("require --from"), "{error}");
    }

    #[test]
    fn new_from_a_claimed_repository_creates_no_worktree() {
        let root = tempfile::tempdir().unwrap();
        let repo = git_repo(root.path());
        run(root.path(), new_command("first", vec![repo.clone()])).unwrap();
        let target = root.path().join("worktree");

        let error = run(
            root.path(),
            WorkspaceCommand::New {
                name: "second".to_owned(),
                dirs: Vec::new(),
                from: Some(repo.clone()),
                branch: Some("feature/x".to_owned()),
                worktree: Some(target.clone()),
                base: None,
                target_dir: None,
                json: true,
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("already belongs"), "{error}");
        assert!(!target.exists(), "no worktree directory is left behind");
        assert!(
            !branch_exists(&repo, "feature/x"),
            "no branch is left behind"
        );
    }

    #[test]
    fn attach_from_repository_adds_a_new_worktree_member() {
        let root = tempfile::tempdir().unwrap();
        let repo = git_repo(root.path());
        run(root.path(), new_command("demo", vec![repo.clone()])).unwrap();
        let target = root.path().join("worktree");

        let output = run(
            root.path(),
            WorkspaceCommand::Attach {
                path: target.clone(),
                workspace: None,
                from: Some(repo.clone()),
                branch: Some("feature/x".to_owned()),
                base: Some("main".to_owned()),
                no_init: true,
                json: true,
            },
        )
        .unwrap();

        let output = json(&output);
        let members = output["workspace"]["members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[1]["branch"], "feature/x");
        assert!(target.join("README.md").is_file());
    }

    #[test]
    fn attach_from_with_unknown_workspace_creates_no_worktree() {
        let root = tempfile::tempdir().unwrap();
        let repo = git_repo(root.path());
        run(root.path(), new_command("demo", vec![repo.clone()])).unwrap();
        let target = root.path().join("worktree");

        let error = run(
            root.path(),
            WorkspaceCommand::Attach {
                path: target.clone(),
                workspace: Some("missing".to_owned()),
                from: Some(repo.clone()),
                branch: Some("feature/x".to_owned()),
                base: None,
                no_init: true,
                json: true,
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown workspace"), "{error}");
        assert!(!target.exists());
        assert!(!branch_exists(&repo, "feature/x"));
    }

    #[test]
    fn new_from_repository_creates_a_worktree_member() {
        let root = tempfile::tempdir().unwrap();
        let repo = git_repo(root.path());
        let target = root.path().join("worktree");

        let output = run(
            root.path(),
            WorkspaceCommand::New {
                name: "demo".to_owned(),
                dirs: Vec::new(),
                from: Some(repo),
                branch: Some("feature/x".to_owned()),
                worktree: Some(target.clone()),
                base: Some("main".to_owned()),
                target_dir: None,
                json: true,
            },
        )
        .unwrap();

        let output = json(&output);
        let members = output["workspace"]["members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[1]["branch"], "feature/x");
        assert!(target.join("README.md").is_file());
    }

    #[test]
    fn archives_a_member_without_deleting_plain_directories() {
        let root = tempfile::tempdir().unwrap();
        let first = dir(root.path(), "alpha");
        let second = dir(root.path(), "beta");
        run(
            root.path(),
            new_command("demo", vec![first, second.clone()]),
        )
        .unwrap();

        let output = run(
            root.path(),
            WorkspaceCommand::Archive {
                member: "beta".to_owned(),
                workspace: None,
                force: false,
                json: false,
            },
        )
        .unwrap();

        assert!(output.contains("Archived"), "{output}");
        assert!(output.contains("directory kept"), "{output}");
        assert!(second.is_dir());
        let registry = workspace::load_registry(root.path()).unwrap();
        assert_eq!(registry.workspaces[0].members.len(), 1);
    }

    #[test]
    fn show_without_reference_and_outside_any_workspace_fails() {
        let root = tempfile::tempdir().unwrap();
        let error = run(
            root.path(),
            WorkspaceCommand::Show {
                workspace: None,
                json: false,
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("belongs to no workspace"),
            "{error}"
        );
    }

    #[test]
    fn unknown_workspace_reference_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let error = run(
            root.path(),
            WorkspaceCommand::Show {
                workspace: Some("missing".to_owned()),
                json: false,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown workspace"), "{error}");
    }
}
