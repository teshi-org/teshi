//! `teshi install-skill` — copy bundled Agent Skills into user discovery paths.
//!
//! Canonical copies live under `~/.agents/skills/<name>`. Supported Agent
//! products get a symlink only when their skills parent directory already
//! exists. Skills are resolved from the local share tree or a source checkout;
//! they are never downloaded.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

/// Relative discovery directories under the user home, excluding `~/.agents`.
const DISCOVERY_PARENTS: &[&str] = &[
    ".cursor/skills",
    ".claude/skills",
    ".codex/skills",
    ".config/agents/skills",
    ".gemini/skills",
];

const REQUIRED_SKILL: &str = "playwright-locator";

const MISSING_SOURCE_HINT: &str = "\
could not find bundled skills at share/teshi-browser-testing/skills next to the teshi CLI.
Install teshi with WinGet (`winget install teshi-org.teshi`) or a GitHub Release archive \
so the share tree is installed beside the CLI, then retry `teshi install-skill`.
Do not download SKILL.md from GitHub.";

/// Locations used to resolve Skill sources and install destinations.
#[derive(Debug, Clone)]
struct InstallPaths {
    home: PathBuf,
    exe: PathBuf,
}

/// Planned action for one discovery-path symlink.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LinkAction {
    Create { link: PathBuf },
    ReplaceSymlink { link: PathBuf },
    SkipMissingParent { parent: PathBuf },
    SkipRealDirectory { path: PathBuf },
}

/// One Skill in the install plan.
#[derive(Debug, Clone)]
struct PlannedSkill {
    name: String,
    source: PathBuf,
    entity: PathBuf,
    links: Vec<LinkAction>,
}

/// Resolved copy/link plan. Dry-run prints this without writing.
#[derive(Debug, Clone)]
struct InstallPlan {
    packaged_root: PathBuf,
    skills: Vec<PlannedSkill>,
}

/// Copies bundled Agent Skills into `~/.agents` and links Agent discovery paths.
///
/// # Errors
///
/// Returns an error when Skill sources cannot be resolved, stdin is not a TTY
/// and `--yes` is missing, or a write/link operation fails (including Windows
/// symlink privilege failures).
pub fn handle_install_skill(dry_run: bool, yes: bool) -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve the user home directory"))?;
    let exe = std::env::current_exe().context("resolve current executable")?;
    run_install_skill(
        &InstallPaths { home, exe },
        dry_run,
        yes,
        io::stdin().is_terminal(),
        &mut io::stdout(),
    )
}

fn run_install_skill(
    paths: &InstallPaths,
    dry_run: bool,
    yes: bool,
    tty: bool,
    out: &mut impl Write,
) -> Result<()> {
    let plan = build_plan(paths)?;
    writeln!(out, "{}", format_plan(&plan))?;

    if dry_run {
        writeln!(out, "Dry-run: no files were written.")?;
        return Ok(());
    }

    if !yes {
        if !tty {
            bail!(
                "stdin is not a TTY; re-run with --yes after reviewing `teshi install-skill --dry-run`"
            );
        }
        let proceed = inquire::Confirm::new("Proceed with skill install?")
            .with_default(false)
            .prompt()
            .context("read confirmation")?;
        if !proceed {
            writeln!(out, "Aborted.")?;
            return Ok(());
        }
    }

    apply_plan(&plan, out)
}

fn build_plan(paths: &InstallPaths) -> Result<InstallPlan> {
    let (packaged_root, extras_root) = resolve_skill_roots(&paths.exe)?;
    let mut sources = skill_dirs(&packaged_root)?;
    if !sources.contains_key(REQUIRED_SKILL) {
        bail!(
            "bundled skills at {} do not include `{REQUIRED_SKILL}`.\n{MISSING_SOURCE_HINT}",
            packaged_root.display()
        );
    }
    if let Some(extra) = extras_root {
        for (name, path) in skill_dirs(&extra)? {
            // Packaged copies win so repo `skills/playwright-locator` cannot replace
            // the external-agent Skill of the same folder name.
            sources.entry(name).or_insert(path);
        }
    }

    let agents_root = paths.home.join(".agents").join("skills");
    let mut skills = Vec::new();
    for (name, source) in sources {
        let entity = agents_root.join(&name);
        let links = DISCOVERY_PARENTS
            .iter()
            .map(|rel| {
                let parent = join_home(&paths.home, rel);
                plan_link(parent.join(&name), &parent)
            })
            .collect();
        skills.push(PlannedSkill {
            name,
            source,
            entity,
            links,
        });
    }
    Ok(InstallPlan {
        packaged_root,
        skills,
    })
}

/// Resolves the packaged skills directory, then an optional repo `skills/` extras tree.
fn resolve_skill_roots(exe: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
    let Some(exe_dir) = exe.parent() else {
        bail!("{MISSING_SOURCE_HINT}");
    };

    let share_candidates = [
        exe_dir
            .join("share")
            .join("teshi-browser-testing")
            .join("skills"),
        exe_dir
            .join("..")
            .join("share")
            .join("teshi-browser-testing")
            .join("skills"),
    ];
    for candidate in &share_candidates {
        if candidate.is_dir() {
            return Ok((candidate.clone(), extras_skills_dir(candidate)));
        }
    }

    let mut dir = exe_dir.to_path_buf();
    loop {
        let checkout = dir
            .join("agent-packages")
            .join("teshi-browser-testing")
            .join("skills");
        if checkout.is_dir() {
            return Ok((checkout.clone(), extras_skills_dir(&checkout)));
        }
        if !dir.pop() {
            break;
        }
    }

    bail!("{MISSING_SOURCE_HINT}");
}

/// Repo `skills/` next to `agent-packages/`; `None` for an installed `share/` tree.
fn extras_skills_dir(packaged_skills: &Path) -> Option<PathBuf> {
    let package = packaged_skills.parent()?;
    if package.file_name()?.to_str()? != "teshi-browser-testing" {
        return None;
    }
    let agent_packages = package.parent()?;
    if agent_packages.file_name()?.to_str()? != "agent-packages" {
        return None;
    }
    let extras = agent_packages.parent()?.join("skills");
    extras.is_dir().then_some(extras)
}

fn skill_dirs(skills_root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut map = BTreeMap::new();
    if !skills_root.is_dir() {
        return Ok(map);
    }
    for entry in fs::read_dir(skills_root)
        .with_context(|| format!("read skill directory {}", skills_root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() || !path.join("SKILL.md").is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        map.insert(name.to_string(), path);
    }
    Ok(map)
}

fn join_home(home: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(home.to_path_buf(), |path, part| path.join(part))
}

fn plan_link(link: PathBuf, parent: &Path) -> LinkAction {
    if !parent.is_dir() {
        return LinkAction::SkipMissingParent {
            parent: parent.to_path_buf(),
        };
    }
    match fs::symlink_metadata(&link) {
        Err(_) => LinkAction::Create { link },
        Ok(meta) if meta.file_type().is_symlink() => LinkAction::ReplaceSymlink { link },
        Ok(_) => LinkAction::SkipRealDirectory { path: link },
    }
}

fn format_plan(plan: &InstallPlan) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Skill source: {}", plan.packaged_root.display()));
    lines.push(String::new());
    for skill in &plan.skills {
        lines.push(format!("{}:", skill.name));
        lines.push(format!(
            "  copy  {} -> {}",
            skill.source.display(),
            skill.entity.display()
        ));
        for action in &skill.links {
            match action {
                LinkAction::Create { link } | LinkAction::ReplaceSymlink { link } => {
                    lines.push(format!(
                        "  link  {} -> {}",
                        link.display(),
                        skill.entity.display()
                    ));
                }
                LinkAction::SkipMissingParent { parent } => {
                    lines.push(format!(
                        "  skip  {} (parent directory does not exist)",
                        parent.display()
                    ));
                }
                LinkAction::SkipRealDirectory { path } => {
                    lines.push(format!(
                        "  skip  {} (existing directory, not a symlink; not overwritten)",
                        path.display()
                    ));
                }
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

fn apply_plan(plan: &InstallPlan, out: &mut impl Write) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    for skill in &plan.skills {
        copy_dir_all(&skill.source, &skill.entity)?;
        writeln!(
            out,
            "copied {} -> {}",
            skill.source.display(),
            skill.entity.display()
        )?;
        for action in &skill.links {
            match action {
                LinkAction::SkipMissingParent { parent } => {
                    writeln!(out, "skipped link (missing parent): {}", parent.display())?;
                }
                LinkAction::SkipRealDirectory { path } => {
                    writeln!(
                        out,
                        "skipped existing directory (not overwritten): {}",
                        path.display()
                    )?;
                }
                LinkAction::Create { link } | LinkAction::ReplaceSymlink { link } => {
                    match create_dir_symlink(link, &skill.entity) {
                        Ok(()) => writeln!(
                            out,
                            "linked {} -> {}",
                            link.display(),
                            skill.entity.display()
                        )?,
                        Err(error) => errors.push(error.to_string()),
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    writeln!(out, "Skill install complete.")?;
    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!("copy {} -> {}", src_path.display(), dst_path.display())
            })?;
        }
    }
    Ok(())
}

fn create_dir_symlink(link: &Path, target: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(link) {
        if meta.file_type().is_symlink() {
            fs::remove_file(link)
                .or_else(|_| fs::remove_dir(link))
                .with_context(|| format!("replace symlink {}", link.display()))?;
        } else {
            bail!("refusing to overwrite {} (not a symlink)", link.display());
        }
    }
    symlink_dir(target, link)
}

fn symlink_dir(target: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .with_context(|| format!("symlink {} -> {}", link.display(), target.display()))
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link).map_err(|error| {
            anyhow!(
                "Failed to create directory symlink {} -> {}: {error}. \
                 Enable Windows Developer Mode or run from an elevated prompt, then retry `teshi install-skill`.",
                link.display(),
                target.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, name: &str, body: &str) {
        let skill = dir.join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), body).unwrap();
    }

    fn share_layout(root: &Path) -> PathBuf {
        let skills = root
            .join("share")
            .join("teshi-browser-testing")
            .join("skills");
        write_skill(&skills, REQUIRED_SKILL, "packaged playwright-locator\n");
        root.join("teshi")
    }

    fn msi_layout(root: &Path) -> PathBuf {
        let skills = root
            .join("share")
            .join("teshi-browser-testing")
            .join("skills");
        write_skill(&skills, REQUIRED_SKILL, "packaged playwright-locator\n");
        let exe_dir = root.join("bin");
        fs::create_dir_all(&exe_dir).unwrap();
        exe_dir.join("teshi")
    }

    fn checkout_layout(root: &Path) -> PathBuf {
        let packaged = root
            .join("agent-packages")
            .join("teshi-browser-testing")
            .join("skills");
        write_skill(&packaged, REQUIRED_SKILL, "packaged playwright-locator\n");
        write_skill(
            &root.join("skills"),
            REQUIRED_SKILL,
            "repo playwright-locator must lose\n",
        );
        write_skill(&root.join("skills"), "bdd-feature", "repo bdd-feature\n");
        write_skill(
            &root.join("skills"),
            "winapp-regression",
            "repo winapp-regression\n",
        );
        let exe_dir = root.join("target").join("debug");
        fs::create_dir_all(&exe_dir).unwrap();
        exe_dir.join("teshi")
    }

    fn paths(home: &Path, exe: PathBuf) -> InstallPaths {
        InstallPaths {
            home: home.to_path_buf(),
            exe,
        }
    }

    #[test]
    fn test_dry_run_does_not_create_files() {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let exe = share_layout(&root.path().join("install"));
        let mut out = Vec::new();
        run_install_skill(&paths(&home, exe), true, false, false, &mut out).unwrap();
        assert!(!home.join(".agents").exists());
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Dry-run: no files were written."));
        assert!(text.contains(REQUIRED_SKILL));
    }

    fn assert_share_skills(path: &Path) {
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("skills")
        );
        let package = path.parent().unwrap();
        assert_eq!(
            package.file_name().and_then(|name| name.to_str()),
            Some("teshi-browser-testing")
        );
        assert_eq!(
            package
                .parent()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("share")
        );
        assert!(path.join(REQUIRED_SKILL).join("SKILL.md").is_file());
    }

    #[test]
    fn test_resolves_share_next_to_exe() {
        let root = TempDir::new().unwrap();
        let exe = share_layout(root.path());
        let (packaged, extras) = resolve_skill_roots(&exe).unwrap();
        assert_share_skills(&packaged);
        assert!(extras.is_none());
    }

    #[test]
    fn test_resolves_share_beside_msi_bin() {
        let root = TempDir::new().unwrap();
        let exe = msi_layout(root.path());
        let (packaged, extras) = resolve_skill_roots(&exe).unwrap();
        assert_share_skills(&packaged);
        assert!(extras.is_none());
    }

    #[test]
    fn test_resolves_checkout_and_prefers_packaged_name() {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let exe = checkout_layout(&root.path().join("repo"));
        let plan = build_plan(&paths(&home, exe)).unwrap();
        let locator = plan
            .skills
            .iter()
            .find(|skill| skill.name == REQUIRED_SKILL)
            .unwrap();
        let body = fs::read_to_string(locator.source.join("SKILL.md")).unwrap();
        assert!(body.contains("packaged playwright-locator"));
        assert!(!body.contains("must lose"));
        let names: Vec<_> = plan
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect();
        assert!(names.contains(&"bdd-feature"));
        assert!(names.contains(&"winapp-regression"));
    }

    #[test]
    fn test_missing_source_points_at_winget_not_github_skill_download() {
        let root = TempDir::new().unwrap();
        let exe = root.path().join("empty").join("teshi");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let error = resolve_skill_roots(&exe).unwrap_err().to_string();
        assert!(error.contains("winget install teshi-org.teshi"), "{error}");
        assert!(
            error.contains("Do not download SKILL.md from GitHub"),
            "{error}"
        );
        assert!(!error.contains("raw.githubusercontent.com"), "{error}");
    }

    #[test]
    fn test_non_tty_without_yes_refuses_to_write() {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let exe = share_layout(&root.path().join("install"));
        let mut out = Vec::new();
        let error = run_install_skill(&paths(&home, exe), false, false, false, &mut out)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--yes"), "{error}");
        assert!(!home.join(".agents").exists());
    }

    #[test]
    fn test_apply_copies_skill_and_skips_missing_parent() {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let exe = share_layout(&root.path().join("install"));
        let mut out = Vec::new();
        run_install_skill(&paths(&home, exe), false, true, false, &mut out).unwrap();
        let entity = home.join(".agents").join("skills").join(REQUIRED_SKILL);
        assert!(entity.join("SKILL.md").is_file());
        assert!(!home.join(".cursor").exists());
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("skipped link (missing parent)"));
    }

    #[test]
    fn test_apply_creates_symlink_when_parent_exists() {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        let cursor_skills = home.join(".cursor").join("skills");
        fs::create_dir_all(&cursor_skills).unwrap();
        let exe = share_layout(&root.path().join("install"));
        let mut out = Vec::new();
        let result = run_install_skill(&paths(&home, exe), false, true, false, &mut out);
        let entity = home.join(".agents").join("skills").join(REQUIRED_SKILL);
        assert!(entity.join("SKILL.md").is_file());
        let link = cursor_skills.join(REQUIRED_SKILL);
        match result {
            Ok(()) => {
                let meta = fs::symlink_metadata(&link).unwrap();
                assert!(
                    meta.file_type().is_symlink(),
                    "expected symlink at {}",
                    link.display()
                );
                assert_eq!(
                    fs::read_to_string(link.join("SKILL.md")).unwrap().trim(),
                    "packaged playwright-locator"
                );
            }
            Err(error) => {
                let message = error.to_string();
                assert!(
                    message.contains("Developer Mode") || message.contains("elevated prompt"),
                    "unexpected symlink failure: {message}"
                );
            }
        }
    }

    #[test]
    fn test_does_not_overwrite_existing_real_directory() {
        let root = TempDir::new().unwrap();
        let home = root.path().join("home");
        let cursor_skill = home.join(".cursor").join("skills").join(REQUIRED_SKILL);
        fs::create_dir_all(&cursor_skill).unwrap();
        fs::write(cursor_skill.join("SKILL.md"), "user owned\n").unwrap();
        let exe = share_layout(&root.path().join("install"));
        let mut out = Vec::new();
        run_install_skill(&paths(&home, exe), false, true, false, &mut out).unwrap();
        let body = fs::read_to_string(cursor_skill.join("SKILL.md")).unwrap();
        assert_eq!(body, "user owned\n");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("not overwritten"));
        assert!(
            home.join(".agents")
                .join("skills")
                .join(REQUIRED_SKILL)
                .join("SKILL.md")
                .is_file()
        );
    }
}
