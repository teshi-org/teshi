//! `git worktree` helpers used when a workspace creates a new member.
//!
//! Members are not required to be git checkouts; these helpers only serve the
//! optional `--from` path and best-effort branch reporting.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Branch checked out in `dir`, or `None` for a detached HEAD or non-repo.
#[must_use]
pub fn current_branch(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if branch.is_empty() || branch == "HEAD" {
        return None;
    }
    Some(branch)
}

/// Top-level directory of the repository containing `dir`.
#[must_use]
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if root.is_empty() {
        return None;
    }
    Some(PathBuf::from(root))
}

/// Creates a worktree at `path` on a new `branch`, based on `base`.
///
/// `path` must not exist yet; `base` defaults to whatever `git` picks, which is
/// `HEAD` of `repo`.
///
/// # Errors
///
/// Returns an error when `repo` is not a repository, `path` already exists, `git`
/// rejects the request (for example a duplicate branch), or the created
/// directory cannot be resolved.
pub fn add_worktree(repo: &Path, branch: &str, path: &Path, base: Option<&str>) -> Result<PathBuf> {
    if branch.trim().is_empty() {
        bail!("branch name must not be empty");
    }
    if path.exists() {
        bail!("worktree path already exists: {}", path.display());
    }
    let repo = dunce::canonicalize(repo)
        .with_context(|| format!("resolve repository {}", repo.display()))?;

    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "-b", branch])
        .arg(path);
    if let Some(base) = base {
        command.arg(base);
    }
    let output = command.output().context("run git worktree add")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git worktree add failed for branch '{branch}' (status {}): {}",
            output.status,
            stderr.trim()
        );
    }
    dunce::canonicalize(path)
        .with_context(|| format!("resolve created worktree {}", path.display()))
}

/// Common git directory when `dir` is a linked worktree.
///
/// A linked worktree has a `.git` *file* pointing at the main repository; the
/// main working tree has a `.git` directory. Only the former is removable with
/// `git worktree remove`.
#[must_use]
pub fn linked_worktree_common_dir(dir: &Path) -> Option<PathBuf> {
    if !dir.join(".git").is_file() {
        return None;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let common = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if common.is_empty() {
        return None;
    }
    let path = PathBuf::from(common);
    Some(if path.is_absolute() {
        path
    } else {
        dir.join(path)
    })
}

/// Whether the working tree has uncommitted changes.
///
/// `None` means the state could not be determined (not a repository or `git`
/// failed), which callers must treat as "unknown", not "clean".
#[must_use]
pub fn is_dirty(dir: &Path) -> Option<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    output.status.success().then_some(!output.stdout.is_empty())
}

/// Removes a linked worktree from its repository.
///
/// The branch is kept. `force` discards uncommitted changes, which `git` refuses
/// to do otherwise.
///
/// # Errors
///
/// Returns an error when `dir` is not a linked worktree or `git` refuses the
/// removal (for example because the tree is dirty and `force` is false).
pub fn remove_worktree(dir: &Path, force: bool) -> Result<()> {
    let common = linked_worktree_common_dir(dir).with_context(|| {
        format!(
            "{} is not a linked worktree; nothing to remove",
            dir.display()
        )
    })?;
    let dir =
        dunce::canonicalize(dir).with_context(|| format!("resolve worktree {}", dir.display()))?;
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(common)
        .args(["worktree", "remove"]);
    if force {
        command.arg("--force");
    }
    command.arg(&dir);
    let output = command.output().context("run git worktree remove")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git worktree remove failed for {} (status {}): {}",
            dir.display(),
            output.status,
            stderr.trim()
        );
    }
    Ok(())
}

/// Undoes [`add_worktree`]: force-removes the worktree and deletes its branch.
///
/// Only for rolling back a worktree created moments ago by the same command,
/// where the branch holds no work yet. Every step runs even if an earlier one
/// fails, so as much as possible is cleaned up.
///
/// # Errors
///
/// Returns the first failure, after attempting all cleanup steps.
pub fn discard_worktree(repo: &Path, path: &Path, branch: &str) -> Result<()> {
    let removed = remove_worktree(path, true);
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["branch", "-D", branch])
        .output()
        .context("run git branch -D");
    removed?;
    let output = output?;
    if !output.status.success() {
        bail!(
            "git branch -D {branch} failed (status {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> std::process::Output {
        let output = Command::new("git")
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
        output
    }

    fn init_repo(root: &Path) -> PathBuf {
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
    fn reports_branch_and_repo_root() {
        let root = tempfile::tempdir().unwrap();
        let repo = init_repo(root.path());

        assert_eq!(current_branch(&repo).as_deref(), Some("main"));
        let expected = dunce::canonicalize(&repo).unwrap();
        assert_eq!(
            repo_root(&repo).map(|path| dunce::canonicalize(path).unwrap()),
            Some(expected)
        );
    }

    #[test]
    fn non_repository_reports_no_branch() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(current_branch(root.path()), None);
        assert_eq!(repo_root(root.path()), None);
    }

    #[test]
    fn adds_a_worktree_on_a_new_branch() {
        let root = tempfile::tempdir().unwrap();
        let repo = init_repo(root.path());
        let target = root.path().join("worktree");

        let created = add_worktree(&repo, "feature/x", &target, Some("main")).unwrap();

        assert_eq!(created, dunce::canonicalize(&target).unwrap());
        assert_eq!(current_branch(&target).as_deref(), Some("feature/x"));
    }

    #[test]
    fn discard_removes_the_worktree_and_its_branch() {
        let root = tempfile::tempdir().unwrap();
        let repo = init_repo(root.path());
        let target = root.path().join("worktree");
        add_worktree(&repo, "feature/x", &target, None).unwrap();

        discard_worktree(&repo, &target, "feature/x").unwrap();

        assert!(!target.exists());
        let branches = git(&repo, &["branch", "--list", "feature/x"]);
        assert!(branches.stdout.is_empty(), "branch should be deleted");
    }

    #[test]
    fn rejects_existing_target_path() {
        let root = tempfile::tempdir().unwrap();
        let repo = init_repo(root.path());
        let target = root.path().join("worktree");
        std::fs::create_dir(&target).unwrap();

        let error = add_worktree(&repo, "feature/x", &target, None).unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
    }

    #[test]
    fn duplicate_branch_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let repo = init_repo(root.path());

        let error = add_worktree(&repo, "main", &root.path().join("second"), None).unwrap_err();
        assert!(
            error.to_string().contains("git worktree add failed"),
            "{error}"
        );
    }
}
