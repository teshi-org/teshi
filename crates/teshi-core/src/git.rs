//! Optional Git status and diff projection for Gherkin Feature files.
//!
//! The public model in this module deliberately does not expose a Git command
//! or patch parser to callers. [`GitRepository`] is the replaceable boundary;
//! the current [`GitCliRepository`] adapter is kept here so TUI code only
//! consumes Teshi-owned status and line-diff types.

use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::gherkin::{BddFeature, BddProject, BddScenario, parse_feature};

/// Working-tree status of a Feature file relative to `HEAD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileGitStatus {
    /// The file has no staged or unstaged change relative to `HEAD`.
    #[default]
    Unmodified,
    /// The file exists in the index or working tree but not in `HEAD`.
    Added,
    /// The file exists in both `HEAD` and the working tree with content changes.
    Modified,
    /// The file exists in `HEAD` but not in the working tree.
    Deleted,
    /// The file is not tracked by Git.
    Untracked,
}

impl FileGitStatus {
    /// Returns the compact marker used by the Feature list.
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Unmodified => " ",
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Untracked => "??",
        }
    }
}

/// Change state of a line or parsed Gherkin block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffStatus {
    /// The item is present on both sides without a relevant change.
    #[default]
    Unchanged,
    /// The item only exists in the working tree.
    Added,
    /// The item only exists in `HEAD`.
    Deleted,
    /// The item exists on both sides but its source changed.
    Modified,
}

impl DiffStatus {
    /// Returns the compact marker used by the Scenario and Step views.
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Unchanged => " ",
            Self::Added => "+",
            Self::Deleted => "-",
            Self::Modified => "~",
        }
    }
}

/// One line from a Git hunk, including its old and new source locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDiffLine {
    /// Whether this line was added, deleted, modified by pairing, or context.
    pub status: DiffStatus,
    /// 1-based line number in `HEAD`, or `None` for an added line.
    pub old_line_number: Option<usize>,
    /// 1-based line number in the working tree, or `None` for a deleted line.
    pub new_line_number: Option<usize>,
    /// Source text without the Git prefix (`+`, `-`, or space).
    pub text: String,
}

/// A unified-diff hunk with source-positioned lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDiffHunk {
    /// First old-file line represented by the hunk.
    pub old_start: usize,
    /// Number of old-file lines represented by the hunk.
    pub old_count: usize,
    /// First new-file line represented by the hunk.
    pub new_start: usize,
    /// Number of new-file lines represented by the hunk.
    pub new_count: usize,
    /// Context and changed lines in source order.
    pub lines: Vec<GitDiffLine>,
}

/// A complete file diff obtained from a repository adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileDiff {
    /// Content from `HEAD`, or `None` when the path did not exist there.
    pub old_content: Option<String>,
    /// Current working-tree content, or `None` when the file was deleted.
    pub new_content: Option<String>,
    /// Parsed unified diff hunks.
    pub hunks: Vec<GitDiffHunk>,
    /// Flattened hunk lines in source order.
    pub lines: Vec<GitDiffLine>,
    /// True when the adapter had to derive a line diff from file contents.
    pub used_content_fallback: bool,
}

impl GitFileDiff {
    /// Builds a deterministic content diff for adapter fallbacks and tests.
    ///
    /// This is also the safe fallback when Git cannot provide a textual hunk,
    /// such as a binary or malformed patch. It retains both deleted and added
    /// lines instead of hiding an unmappable change.
    pub fn from_contents(old_content: Option<&str>, new_content: Option<&str>) -> Self {
        let lines = content_diff_lines(old_content.unwrap_or(""), new_content.unwrap_or(""));
        let hunk = GitDiffHunk {
            old_start: if old_content.is_some() && !old_content.unwrap_or("").is_empty() {
                1
            } else {
                0
            },
            old_count: old_content.map_or(0, |content| content.lines().count()),
            new_start: if new_content.is_some() && !new_content.unwrap_or("").is_empty() {
                1
            } else {
                0
            },
            new_count: new_content.map_or(0, |content| content.lines().count()),
            lines: lines.clone(),
        };
        Self {
            old_content: old_content.map(str::to_string),
            new_content: new_content.map(str::to_string),
            hunks: if lines.is_empty() {
                Vec::new()
            } else {
                vec![hunk]
            },
            lines,
            used_content_fallback: true,
        }
    }
}

/// A status entry returned by a repository adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusEntry {
    /// Path relative to the repository root, using the repository's path form.
    pub path: PathBuf,
    /// Status of this path relative to `HEAD`.
    pub status: FileGitStatus,
}

/// A repository access failure. Git errors are intentionally local to the
/// optional enhancement and are not propagated into the Feature browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    /// Sanitized diagnostic suitable for a status log or test assertion.
    pub message: String,
}

impl GitError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for GitError {}

/// Replaceable repository/status/diff boundary used by the Feature model.
pub trait GitRepository {
    /// Returns the discovered repository root.
    fn repository_root(&self) -> &Path;

    /// Lists changed paths relative to `HEAD`.
    fn status_entries(&self) -> Result<Vec<GitStatusEntry>, GitError>;

    /// Loads one file's old/new content and its unified diff.
    fn file_diff(&self, path: &Path) -> Result<GitFileDiff, GitError>;

    /// Loads multiple files' old/new content and unified diffs as one batch.
    ///
    /// Adapters that can batch their repository access should override this
    /// method. The default keeps the boundary compatible with simple test or
    /// future adapters that only support one file at a time.
    fn file_diffs(&self, paths: &[PathBuf]) -> Vec<(PathBuf, Result<GitFileDiff, GitError>)> {
        paths
            .iter()
            .cloned()
            .map(|path| {
                let result = self.file_diff(&path);
                (path, result)
            })
            .collect()
    }
}

/// Current Git adapter. Its process invocation is intentionally confined to
/// this module so a future `gix` implementation can replace this type without
/// touching TUI or Feature mapping code.
#[derive(Debug, Clone)]
pub struct GitCliRepository {
    root: PathBuf,
    command: OsString,
}

impl GitCliRepository {
    /// Discovers a repository containing `path` using the default `git` command.
    ///
    /// Returns `Ok(None)` when `path` is not inside a repository. A missing Git
    /// executable or another invocation failure is returned as [`GitError`].
    pub fn discover(path: &Path) -> Result<Option<Self>, GitError> {
        Self::discover_with_command(path, OsStr::new("git"))
    }

    /// Discovers a repository using an explicit command, primarily for adapter tests.
    pub fn discover_with_command(
        path: &Path,
        command: impl AsRef<OsStr>,
    ) -> Result<Option<Self>, GitError> {
        let working_dir = if path.is_file() {
            path.parent().unwrap_or(Path::new("."))
        } else {
            path
        };
        let output = Command::new(command.as_ref())
            .current_dir(working_dir)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|error| GitError::new(format!("could not run Git: {error}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if is_not_repository_message(&stderr) {
                return Ok(None);
            }
            return Err(GitError::new(format!(
                "Git repository discovery failed: {}",
                compact_process_message(&stderr)
            )));
        }

        let raw_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw_root.is_empty() {
            return Err(GitError::new("Git returned an empty repository root"));
        }
        let root = PathBuf::from(raw_root)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
        Ok(Some(Self {
            root,
            command: command.as_ref().to_os_string(),
        }))
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.command);
        command.current_dir(&self.root);
        command
    }

    fn relative_path(&self, path: &Path) -> Result<PathBuf, GitError> {
        if path.is_absolute() {
            return path
                .strip_prefix(&self.root)
                .map(Path::to_path_buf)
                .map_err(|_| {
                    GitError::new(format!("path is outside repository: {}", path.display()))
                });
        }
        Ok(path.to_path_buf())
    }

    fn working_tree_content(&self, path: &Path) -> Result<Option<String>, GitError> {
        let full_path = self.root.join(path);
        match std::fs::read_to_string(full_path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(GitError::new(format!(
                "could not read working tree file: {error}"
            ))),
        }
    }

    fn head_contents(
        &self,
        paths: &[PathBuf],
    ) -> Result<HashMap<String, Option<String>>, GitError> {
        if paths.is_empty() {
            return Ok(HashMap::new());
        }

        let mut child = self
            .command()
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| GitError::new(format!("could not read Git HEAD: {error}")))?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| GitError::new("could not write Git HEAD requests"))?;
            for path in paths {
                writeln!(stdin, "HEAD:{}", git_path(path)).map_err(|error| {
                    GitError::new(format!("could not request Git HEAD content: {error}"))
                })?;
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|error| GitError::new(format!("could not read Git HEAD: {error}")))?;
        if !output.status.success() {
            // An empty repository has no HEAD yet. Preserve the old adapter
            // behaviour and let the content side be treated as added.
            return Ok(paths.iter().map(|path| (path_key(path), None)).collect());
        }
        parse_cat_file_batch(&output.stdout, paths)
    }

    fn diff_patch(&self, paths: &[PathBuf]) -> Result<String, GitError> {
        if paths.is_empty() {
            return Ok(String::new());
        }
        let output = self
            .command()
            .args([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--no-renames",
                "--unified=3",
                "HEAD",
                "--",
            ])
            .args(paths)
            .output()
            .map_err(|error| GitError::new(format!("could not read Git diff: {error}")))?;
        if !output.status.success() {
            return Ok(String::new());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn batch_file_diffs(
        &self,
        paths: &[PathBuf],
    ) -> Result<HashMap<String, GitFileDiff>, GitError> {
        if paths.is_empty() {
            return Ok(HashMap::new());
        }
        let old_contents = self.head_contents(paths)?;
        let mut new_contents = HashMap::with_capacity(paths.len());
        for path in paths {
            new_contents.insert(path_key(path), self.working_tree_content(path)?);
        }
        let patch = self.diff_patch(paths)?;
        let mut diffs = HashMap::with_capacity(paths.len());
        for path in paths {
            let key = path_key(path);
            let old_content = old_contents.get(&key).cloned().unwrap_or(None);
            let new_content = new_contents.get(&key).cloned().unwrap_or(None);
            let diff = if old_content.is_none() || new_content.is_none() {
                GitFileDiff::from_contents(old_content.as_deref(), new_content.as_deref())
            } else {
                let parsed = patch_section(&patch, path)
                    .map(|section| {
                        parse_unified_diff(section, old_content.clone(), new_content.clone())
                    })
                    .unwrap_or_else(|| {
                        GitFileDiff::from_contents(old_content.as_deref(), new_content.as_deref())
                    });
                if parsed.lines.is_empty() && old_content != new_content {
                    GitFileDiff::from_contents(old_content.as_deref(), new_content.as_deref())
                } else {
                    parsed
                }
            };
            diffs.insert(key, diff);
        }
        Ok(diffs)
    }
}

impl GitRepository for GitCliRepository {
    fn repository_root(&self) -> &Path {
        &self.root
    }

    fn status_entries(&self) -> Result<Vec<GitStatusEntry>, GitError> {
        let output = self
            .command()
            .args([
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--no-renames",
            ])
            .output()
            .map_err(|error| GitError::new(format!("could not read Git status: {error}")))?;
        if !output.status.success() {
            return Err(GitError::new(format!(
                "Git status failed: {}",
                compact_process_message(&String::from_utf8_lossy(&output.stderr))
            )));
        }
        Ok(parse_porcelain_status(&output.stdout))
    }

    fn file_diff(&self, path: &Path) -> Result<GitFileDiff, GitError> {
        let path = self.relative_path(path)?;
        let mut diffs = self.batch_file_diffs(std::slice::from_ref(&path))?;
        diffs
            .remove(&path_key(&path))
            .ok_or_else(|| GitError::new("Git diff batch omitted the requested path"))
    }

    fn file_diffs(&self, paths: &[PathBuf]) -> Vec<(PathBuf, Result<GitFileDiff, GitError>)> {
        let relative_paths: Result<Vec<_>, _> =
            paths.iter().map(|path| self.relative_path(path)).collect();
        let relative_paths = match relative_paths {
            Ok(paths) => paths,
            Err(error) => {
                return paths
                    .iter()
                    .cloned()
                    .map(|path| (path, Err(error.clone())))
                    .collect();
            }
        };
        match self.batch_file_diffs(&relative_paths) {
            Ok(mut diffs) => paths
                .iter()
                .zip(relative_paths)
                .map(|(original, relative)| {
                    let result = diffs
                        .remove(&path_key(&relative))
                        .ok_or_else(|| GitError::new("Git diff batch omitted the requested path"));
                    (original.clone(), result)
                })
                .collect(),
            Err(error) => paths
                .iter()
                .cloned()
                .map(|path| (path, Err(error.clone())))
                .collect(),
        }
    }
}

/// Per-Scenario status and the hunk lines mapped to its source range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioGitStatus {
    /// Scenario name from the corresponding source version.
    pub name: String,
    /// Old source line for this Scenario, when it existed in `HEAD`.
    pub old_line_number: Option<usize>,
    /// New source line for this Scenario, when it exists in the working tree.
    pub new_line_number: Option<usize>,
    /// Scenario-level status.
    pub status: DiffStatus,
    /// Git hunk lines belonging to this Scenario range.
    pub diff_lines: Vec<GitDiffLine>,
}

/// Status and hunk projection for a Background block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionGitStatus {
    /// Status of the Background block.
    pub status: DiffStatus,
    /// Git hunk lines belonging to the Background range.
    pub diff_lines: Vec<GitDiffLine>,
}

/// Git-aware view for one current or HEAD-only Feature file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureGitView {
    /// Path supplied by the project or repository status listing.
    pub path: PathBuf,
    /// File-level status relative to `HEAD`.
    pub file_status: FileGitStatus,
    /// File diff, when Git supplied or derived one.
    pub diff: Option<GitFileDiff>,
    /// Current Scenarios followed by HEAD-only deleted Scenarios.
    pub scenarios: Vec<ScenarioGitStatus>,
    /// Background status, if either source version contains a Background.
    pub background: Option<SectionGitStatus>,
    /// Changed lines outside mapped Background and Scenario ranges.
    pub feature_diff_lines: Vec<GitDiffLine>,
}

impl FeatureGitView {
    fn plain(path: PathBuf, file_status: FileGitStatus) -> Self {
        Self {
            path,
            file_status,
            diff: None,
            scenarios: Vec::new(),
            background: None,
            feature_diff_lines: Vec::new(),
        }
    }

    /// Finds the mapped status for a current Scenario by source line and name.
    pub fn scenario_status(&self, scenario: &BddScenario) -> DiffStatus {
        self.scenarios
            .iter()
            .find(|item| {
                item.new_line_number == Some(scenario.line_number) && item.name == scenario.name
            })
            .map(|item| item.status)
            .unwrap_or(DiffStatus::Unchanged)
    }

    /// Finds the mapped hunk lines for a current Scenario.
    pub fn scenario_diff_lines(&self, scenario: &BddScenario) -> Option<&[GitDiffLine]> {
        self.scenarios
            .iter()
            .find(|item| {
                item.new_line_number == Some(scenario.line_number) && item.name == scenario.name
            })
            .map(|item| item.diff_lines.as_slice())
    }

    /// Returns HEAD-only Scenarios that no longer exist in the working tree.
    pub fn deleted_scenarios(&self) -> impl Iterator<Item = &ScenarioGitStatus> {
        self.scenarios
            .iter()
            .filter(|item| item.new_line_number.is_none())
    }
}

/// Project path scope used when including HEAD-only deleted Feature files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitProjectScope {
    /// Include Feature files at any depth below the project root.
    #[default]
    Recursive,
    /// Include only direct child Feature files of the project root.
    Shallow,
    /// Include only paths already present in the parsed project.
    CurrentFiles,
}

/// Git status and diff data aligned with the current project Feature order.
#[derive(Debug, Clone, Default)]
pub struct FeatureGitStatusModel {
    /// Project path scope used to build this model.
    pub scope: GitProjectScope,
    /// Discovered repository root, if Git discovery succeeded.
    pub repository_root: Option<PathBuf>,
    /// Whether a repository adapter was available for this project.
    pub available: bool,
    /// Non-fatal repository or per-file diagnostic.
    pub error: Option<String>,
    /// Views aligned with `BddProject::features`.
    pub current: Vec<FeatureGitView>,
    /// HEAD-only deleted Feature files.
    pub deleted: Vec<FeatureGitView>,
}

impl FeatureGitStatusModel {
    /// Creates an unavailable projection aligned with the current project
    /// order. It is used while an asynchronous repository refresh is pending.
    pub fn empty_for_project(project: &BddProject, scope: GitProjectScope) -> Self {
        plain_model(project, scope)
    }

    /// Returns the current-file view at the same index as the project Feature.
    pub fn current_at(&self, index: usize) -> Option<&FeatureGitView> {
        self.current.get(index)
    }
}

/// Loads optional Git status for a parsed project using the current adapter.
///
/// All Git failures are converted into an unavailable model so callers can
/// continue displaying and editing Features normally.
pub fn load_feature_git_status(project: &BddProject) -> FeatureGitStatusModel {
    load_feature_git_status_with_scope(project, GitProjectScope::Recursive)
}

/// Loads optional Git status while honoring the project's Feature scan scope.
pub fn load_feature_git_status_with_scope(
    project: &BddProject,
    scope: GitProjectScope,
) -> FeatureGitStatusModel {
    load_feature_git_status_with_scope_and_content_overrides(project, scope, &HashMap::new())
}

/// Loads optional Git status while using in-memory content for selected current
/// Feature files.
///
/// Git status is still discovered from the repository, but a supplied content
/// override becomes the new side of the mapped diff. This keeps the projection
/// aligned with unsaved editor buffers without pretending that those buffers
/// have already been written to disk.
pub fn load_feature_git_status_with_scope_and_content_overrides(
    project: &BddProject,
    scope: GitProjectScope,
    content_overrides: &HashMap<PathBuf, String>,
) -> FeatureGitStatusModel {
    let mut model = plain_model(project, scope);
    let repository = match GitCliRepository::discover(&project.root_dir) {
        Ok(Some(repository)) => repository,
        Ok(None) => return model,
        Err(error) => {
            model.error = Some(error.to_string());
            return model;
        }
    };
    load_feature_git_status_with_repository_and_scope_and_content_overrides(
        project,
        &repository,
        scope,
        content_overrides,
    )
}

/// Loads Git status through an injected repository implementation.
pub fn load_feature_git_status_with_repository<R: GitRepository>(
    project: &BddProject,
    repository: &R,
) -> FeatureGitStatusModel {
    load_feature_git_status_with_repository_and_scope(
        project,
        repository,
        GitProjectScope::Recursive,
    )
}

/// Loads Git status through an injected repository within a project scan scope.
pub fn load_feature_git_status_with_repository_and_scope<R: GitRepository>(
    project: &BddProject,
    repository: &R,
    scope: GitProjectScope,
) -> FeatureGitStatusModel {
    load_feature_git_status_with_repository_and_scope_and_content_overrides(
        project,
        repository,
        scope,
        &HashMap::new(),
    )
}

/// Loads Git status through an injected repository while overlaying selected
/// current-file contents from memory.
pub fn load_feature_git_status_with_repository_and_scope_and_content_overrides<R: GitRepository>(
    project: &BddProject,
    repository: &R,
    scope: GitProjectScope,
    content_overrides: &HashMap<PathBuf, String>,
) -> FeatureGitStatusModel {
    let mut model = plain_model(project, scope);
    model.available = true;
    model.repository_root = Some(repository.repository_root().to_path_buf());

    let entries = match repository.status_entries() {
        Ok(entries) => entries,
        Err(error) => {
            model.error = Some(error.to_string());
            return model;
        }
    };
    let by_path: HashMap<String, FileGitStatus> = entries
        .iter()
        .map(|entry| (path_key(&entry.path), entry.status))
        .collect();
    let current_keys: HashSet<String> = project
        .features
        .iter()
        .filter_map(|feature| repo_relative_path(repository.repository_root(), &feature.file_path))
        .map(|path| path_key(&path))
        .collect();
    let project_prefix = repo_relative_path(repository.repository_root(), &project.root_dir);

    let mut diff_paths = Vec::new();
    let mut requested_paths = HashSet::new();
    for feature in &project.features {
        let relative_path = repo_relative_path(repository.repository_root(), &feature.file_path);
        let status = relative_path
            .as_ref()
            .and_then(|path| by_path.get(&path_key(path)).copied())
            .unwrap_or(FileGitStatus::Unmodified);
        let override_content = relative_path.as_ref().and_then(|relative_path| {
            content_overrides.get(&feature.file_path).or_else(|| {
                content_overrides.get(&repository.repository_root().join(relative_path))
            })
        });
        if status == FileGitStatus::Unmodified && override_content.is_none() {
            continue;
        }
        let Some(relative_path) = relative_path else {
            continue;
        };
        if requested_paths.insert(path_key(&relative_path)) {
            diff_paths.push(relative_path);
        }
    }

    let deleted_entries: Vec<GitStatusEntry> = entries
        .iter()
        .filter(|entry| {
            entry.status == FileGitStatus::Deleted
                && is_feature_path(&entry.path)
                && !current_keys.contains(&path_key(&entry.path))
                && path_is_in_project_scope(&entry.path, project_prefix.as_deref(), scope)
        })
        .cloned()
        .collect();
    for entry in &deleted_entries {
        if requested_paths.insert(path_key(&entry.path)) {
            diff_paths.push(entry.path.clone());
        }
    }

    let mut diffs: HashMap<String, Result<GitFileDiff, GitError>> = repository
        .file_diffs(&diff_paths)
        .into_iter()
        .map(|(path, result)| (path_key(&path), result))
        .collect();

    for (index, feature) in project.features.iter().enumerate() {
        let relative_path = repo_relative_path(repository.repository_root(), &feature.file_path);
        let status = relative_path
            .as_ref()
            .and_then(|path| by_path.get(&path_key(path)).copied())
            .unwrap_or(FileGitStatus::Unmodified);
        let override_content = relative_path.as_ref().and_then(|relative_path| {
            content_overrides.get(&feature.file_path).or_else(|| {
                content_overrides.get(&repository.repository_root().join(relative_path))
            })
        });
        if status == FileGitStatus::Unmodified && override_content.is_none() {
            continue;
        }
        let Some(relative_path) = relative_path else {
            continue;
        };
        let Some(diff_result) = diffs.remove(&path_key(&relative_path)) else {
            model.error = Some(format!(
                "Git diff batch omitted {}",
                relative_path.display()
            ));
            model.current[index].file_status = status;
            continue;
        };
        match diff_result {
            Ok(diff) => {
                let (status, diff) = if let Some(content) = override_content {
                    let status = status_from_contents(diff.old_content.as_deref(), Some(content));
                    let diff = GitFileDiff::from_contents(
                        diff.old_content.as_deref(),
                        Some(content.as_str()),
                    );
                    (status, diff)
                } else {
                    (status, diff)
                };
                if status == FileGitStatus::Unmodified {
                    continue;
                }
                model.current[index] =
                    map_feature_git_diff(feature.file_path.clone(), status, diff);
            }
            Err(error) => {
                model.error = Some(error.to_string());
                model.current[index].file_status = status;
            }
        }
    }

    for entry in deleted_entries {
        let path_key_value = path_key(&entry.path);
        let Some(diff_result) = diffs.remove(&path_key_value) else {
            model.error = Some(format!("Git diff batch omitted {}", entry.path.display()));
            model
                .deleted
                .push(FeatureGitView::plain(entry.path, entry.status));
            continue;
        };
        match diff_result {
            Ok(diff) => model
                .deleted
                .push(map_feature_git_diff(entry.path, entry.status, diff)),
            Err(error) => {
                model.error = Some(error.to_string());
                model
                    .deleted
                    .push(FeatureGitView::plain(entry.path, entry.status));
            }
        }
    }
    model
}

fn status_from_contents(old_content: Option<&str>, new_content: Option<&str>) -> FileGitStatus {
    match (old_content, new_content) {
        (None, None) => FileGitStatus::Unmodified,
        (None, Some(_)) => FileGitStatus::Added,
        (Some(_), None) => FileGitStatus::Deleted,
        (Some(old), Some(new)) if old == new => FileGitStatus::Unmodified,
        (Some(_), Some(_)) => FileGitStatus::Modified,
    }
}

/// Maps a repository diff to Scenario and Background ranges using Gherkin AST positions.
pub fn map_feature_git_diff(
    path: PathBuf,
    file_status: FileGitStatus,
    diff: GitFileDiff,
) -> FeatureGitView {
    let old_feature = diff
        .old_content
        .as_deref()
        .map(|content| parse_feature(content, path.clone()));
    let new_feature = diff
        .new_content
        .as_deref()
        .map(|content| parse_feature(content, path.clone()));
    let old_scenarios = old_feature
        .as_ref()
        .map(BddFeature::all_scenarios)
        .unwrap_or_default();
    let new_scenarios = new_feature
        .as_ref()
        .map(BddFeature::all_scenarios)
        .unwrap_or_default();

    let mut used_old = vec![false; old_scenarios.len()];
    let mut scenarios = Vec::with_capacity(old_scenarios.len().max(new_scenarios.len()));
    for (new_index, new_scenario) in new_scenarios.iter().enumerate() {
        let old_index = old_scenarios.iter().enumerate().find_map(|(index, old)| {
            (!used_old[index] && old.name == new_scenario.name && old.kind == new_scenario.kind)
                .then_some(index)
        });
        if let Some(index) = old_index {
            used_old[index] = true;
        }
        let old_range = old_index.and_then(|index| scenario_range(old_feature.as_ref(), index));
        let new_range = scenario_range(new_feature.as_ref(), new_index);
        let diff_lines = collect_section_lines(&diff, old_range, new_range);
        let status = if old_index.is_none() {
            DiffStatus::Added
        } else if diff_lines.iter().any(is_changed_line) {
            DiffStatus::Modified
        } else {
            DiffStatus::Unchanged
        };
        scenarios.push(ScenarioGitStatus {
            name: new_scenario.name.clone(),
            old_line_number: old_index.map(|index| old_scenarios[index].line_number),
            new_line_number: Some(new_scenario.line_number),
            status,
            diff_lines,
        });
    }

    for (old_index, old_scenario) in old_scenarios.iter().enumerate() {
        if used_old[old_index] {
            continue;
        }
        let old_range = scenario_range(old_feature.as_ref(), old_index);
        let diff_lines = collect_section_lines(&diff, old_range, None);
        scenarios.push(ScenarioGitStatus {
            name: old_scenario.name.clone(),
            old_line_number: Some(old_scenario.line_number),
            new_line_number: None,
            status: DiffStatus::Deleted,
            diff_lines,
        });
    }

    let background = map_background_status(old_feature.as_ref(), new_feature.as_ref(), &diff);
    let feature_diff_lines = diff
        .lines
        .iter()
        .filter(|line| {
            is_changed_line(line)
                && !scenarios
                    .iter()
                    .flat_map(|scenario| &scenario.diff_lines)
                    .chain(
                        background
                            .iter()
                            .flat_map(|section| section.diff_lines.iter()),
                    )
                    .any(|mapped| mapped == *line)
        })
        .cloned()
        .collect();
    FeatureGitView {
        path,
        file_status,
        diff: Some(diff),
        scenarios,
        background,
        feature_diff_lines,
    }
}

fn plain_model(project: &BddProject, scope: GitProjectScope) -> FeatureGitStatusModel {
    FeatureGitStatusModel {
        scope,
        current: project
            .features
            .iter()
            .map(|feature| {
                FeatureGitView::plain(feature.file_path.clone(), FileGitStatus::Unmodified)
            })
            .collect(),
        ..FeatureGitStatusModel::default()
    }
}

fn path_is_in_project_scope(
    path: &Path,
    project_prefix: Option<&Path>,
    scope: GitProjectScope,
) -> bool {
    if scope == GitProjectScope::CurrentFiles {
        return false;
    }
    let Some(project_prefix) = project_prefix else {
        return false;
    };
    let Ok(project_path) = path.strip_prefix(project_prefix) else {
        return false;
    };
    !project_path.as_os_str().is_empty()
        && (scope == GitProjectScope::Recursive || project_path.parent() == Some(Path::new("")))
}

fn map_background_status(
    old_feature: Option<&BddFeature>,
    new_feature: Option<&BddFeature>,
    diff: &GitFileDiff,
) -> Option<SectionGitStatus> {
    let old_background = old_feature.and_then(|feature| feature.background.as_ref());
    let new_background = new_feature.and_then(|feature| feature.background.as_ref());
    if old_background.is_none() && new_background.is_none() {
        return None;
    }
    let old_range = old_background.map(|background| {
        line_range(
            background.line_number,
            old_feature
                .and_then(|feature| {
                    feature
                        .all_scenarios()
                        .first()
                        .map(|scenario| scenario.line_number)
                })
                .unwrap_or_else(|| {
                    old_feature.map_or(background.line_number, |feature| feature.line_count + 1)
                }),
        )
    });
    let new_range = new_background.map(|background| {
        line_range(
            background.line_number,
            new_feature
                .and_then(|feature| {
                    feature
                        .all_scenarios()
                        .first()
                        .map(|scenario| scenario.line_number)
                })
                .unwrap_or_else(|| {
                    new_feature.map_or(background.line_number, |feature| feature.line_count + 1)
                }),
        )
    });
    let diff_lines = collect_section_lines(diff, old_range, new_range);
    let status = match (old_background, new_background) {
        (None, Some(_)) => DiffStatus::Added,
        (Some(_), None) => DiffStatus::Deleted,
        (Some(_), Some(_)) if diff_lines.iter().any(is_changed_line) => DiffStatus::Modified,
        (Some(_), Some(_)) => DiffStatus::Unchanged,
        (None, None) => return None,
    };
    Some(SectionGitStatus { status, diff_lines })
}

#[derive(Debug, Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

fn line_range(start: usize, end_exclusive: usize) -> LineRange {
    LineRange {
        start,
        end: end_exclusive.saturating_sub(1).max(start),
    }
}

fn scenario_range(feature: Option<&BddFeature>, index: usize) -> Option<LineRange> {
    let feature = feature?;
    let scenarios = feature.all_scenarios();
    let scenario = scenarios.get(index)?;
    let next_scenario_line = scenarios
        .get(index + 1)
        .map(|next| next.line_number)
        .unwrap_or_else(|| feature.line_count + 1);
    let next_rule_line = feature
        .rules
        .iter()
        .map(|rule| rule.line_number)
        .filter(|line| *line > scenario.line_number)
        .min()
        .unwrap_or(feature.line_count + 1);
    Some(line_range(
        scenario.line_number,
        next_scenario_line.min(next_rule_line),
    ))
}

fn collect_section_lines(
    diff: &GitFileDiff,
    old_range: Option<LineRange>,
    new_range: Option<LineRange>,
) -> Vec<GitDiffLine> {
    if old_range.is_none() && new_range.is_none() {
        return Vec::new();
    }
    let intersects = |line: &GitDiffLine, changed_only: bool| {
        if changed_only && !is_changed_line(line) {
            return false;
        }
        old_range.is_some_and(|range| {
            line.old_line_number
                .is_some_and(|number| number >= range.start && number <= range.end)
        }) || new_range.is_some_and(|range| {
            line.new_line_number
                .is_some_and(|number| number >= range.start && number <= range.end)
        })
    };
    if !diff.lines.iter().any(|line| intersects(line, true)) {
        return Vec::new();
    }
    diff.lines
        .iter()
        .filter(|line| intersects(line, false))
        .cloned()
        .collect()
}

fn is_changed_line(line: &GitDiffLine) -> bool {
    line.status != DiffStatus::Unchanged
}

fn is_feature_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("feature"))
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn repo_relative_path(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = absolute_path(root)
        .canonicalize()
        .unwrap_or_else(|_| absolute_path(root));
    let path = absolute_path(path)
        .canonicalize()
        .unwrap_or_else(|_| absolute_path(path));
    path.strip_prefix(&root).ok().map(Path::to_path_buf)
}

fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn path_key(path: &Path) -> String {
    let value = git_path(path);
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn parse_cat_file_batch(
    output: &[u8],
    paths: &[PathBuf],
) -> Result<HashMap<String, Option<String>>, GitError> {
    let mut contents = HashMap::with_capacity(paths.len());
    let mut offset = 0usize;
    for path in paths {
        let header_end = output
            .get(offset..)
            .and_then(|remaining| remaining.iter().position(|byte| *byte == b'\n'))
            .map(|relative| offset + relative)
            .ok_or_else(|| GitError::new("Git HEAD batch response had no header"))?;
        let header = String::from_utf8_lossy(&output[offset..header_end]);
        offset = header_end + 1;
        let mut fields = header.split_whitespace();
        let _object_id = fields.next();
        let object_type = fields
            .next()
            .ok_or_else(|| GitError::new("Git HEAD batch response had an invalid header"))?;
        if object_type == "missing" {
            contents.insert(path_key(path), None);
            continue;
        }
        let size = fields
            .next()
            .ok_or_else(|| GitError::new("Git HEAD batch response omitted content size"))?
            .parse::<usize>()
            .map_err(|error| GitError::new(format!("invalid Git HEAD content size: {error}")))?;
        let content_end = offset
            .checked_add(size)
            .filter(|end| *end <= output.len())
            .ok_or_else(|| GitError::new("Git HEAD batch response was truncated"))?;
        let content = String::from_utf8(output[offset..content_end].to_vec())
            .map_err(|error| GitError::new(format!("Git HEAD content is not UTF-8: {error}")))?;
        offset = content_end;
        if output.get(offset) == Some(&b'\n') {
            offset += 1;
        }
        contents.insert(path_key(path), Some(content));
    }
    Ok(contents)
}

fn patch_section<'a>(patch: &'a str, path: &Path) -> Option<&'a str> {
    let expected_header = format!("diff --git a/{0} b/{0}", git_path(path));
    let mut starts = patch
        .match_indices("diff --git ")
        .map(|(index, _)| index)
        .peekable();
    while let Some(start) = starts.next() {
        let end = starts.peek().copied().unwrap_or(patch.len());
        let section = &patch[start..end];
        let header_end = section.find('\n').unwrap_or(section.len());
        if section[..header_end] == expected_header {
            return Some(section);
        }
    }
    None
}

fn is_not_repository_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("not a git repository") || lower.contains("不是 git 仓库")
}

fn compact_process_message(message: &str) -> String {
    let compact = message.trim().replace(['\r', '\n'], " ");
    if compact.is_empty() {
        "unknown error".to_string()
    } else {
        compact
    }
}

fn parse_porcelain_status(output: &[u8]) -> Vec<GitStatusEntry> {
    let records: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.len() < 4 {
            continue;
        }
        let x = record[0] as char;
        let y = record[1] as char;
        let path = PathBuf::from(String::from_utf8_lossy(&record[3..]).into_owned());
        entries.push(GitStatusEntry {
            path,
            status: status_from_porcelain(x, y),
        });
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            index += 1;
        }
    }
    entries
}

fn status_from_porcelain(x: char, y: char) -> FileGitStatus {
    if x == '?' && y == '?' {
        FileGitStatus::Untracked
    } else if x == 'A' {
        // An index-added path did not exist in HEAD. If the working tree
        // deleted that staged path again (`AD`), HEAD and the working tree
        // agree that the path is absent, so it is not a deleted HEAD file.
        if y == 'D' {
            FileGitStatus::Unmodified
        } else {
            FileGitStatus::Added
        }
    } else if x == 'D' {
        // An index-deleted path did exist in HEAD. Re-adding it in the
        // working tree (`DA`) means the HEAD-to-working-tree view is a
        // modification, not a deletion.
        if matches!(y, 'A' | 'M' | 'U') {
            FileGitStatus::Modified
        } else {
            FileGitStatus::Deleted
        }
    } else if y == 'D' {
        FileGitStatus::Deleted
    } else {
        FileGitStatus::Modified
    }
}

#[derive(Debug)]
struct RawHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    old_cursor: usize,
    new_cursor: usize,
    lines: Vec<GitDiffLine>,
}

fn parse_unified_diff(
    patch: &str,
    old_content: Option<String>,
    new_content: Option<String>,
) -> GitFileDiff {
    let mut hunks: Vec<GitDiffHunk> = Vec::new();
    let mut current: Option<RawHunk> = None;
    for raw_line in patch.lines() {
        if raw_line.starts_with("@@ ") {
            if let Some(hunk) = current.take() {
                hunks.push(finish_hunk(hunk));
            }
            let Some((old_start, old_count, new_start, new_count)) = parse_hunk_header(raw_line)
            else {
                continue;
            };
            current = Some(RawHunk {
                old_start,
                old_count,
                new_start,
                new_count,
                old_cursor: old_start,
                new_cursor: new_start,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = current.as_mut() else {
            continue;
        };
        if raw_line.starts_with('\\') {
            continue;
        }
        let Some(prefix) = raw_line.chars().next() else {
            continue;
        };
        let text = raw_line.get(1..).unwrap_or_default().to_string();
        match prefix {
            ' ' => {
                hunk.lines.push(GitDiffLine {
                    status: DiffStatus::Unchanged,
                    old_line_number: nonzero_line(hunk.old_cursor),
                    new_line_number: nonzero_line(hunk.new_cursor),
                    text,
                });
                hunk.old_cursor = hunk.old_cursor.saturating_add(1);
                hunk.new_cursor = hunk.new_cursor.saturating_add(1);
            }
            '+' => {
                hunk.lines.push(GitDiffLine {
                    status: DiffStatus::Added,
                    old_line_number: None,
                    new_line_number: nonzero_line(hunk.new_cursor),
                    text,
                });
                hunk.new_cursor = hunk.new_cursor.saturating_add(1);
            }
            '-' => {
                hunk.lines.push(GitDiffLine {
                    status: DiffStatus::Deleted,
                    old_line_number: nonzero_line(hunk.old_cursor),
                    new_line_number: None,
                    text,
                });
                hunk.old_cursor = hunk.old_cursor.saturating_add(1);
            }
            _ => {}
        }
    }
    if let Some(hunk) = current.take() {
        hunks.push(finish_hunk(hunk));
    }
    let lines = hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter().cloned())
        .collect();
    GitFileDiff {
        old_content,
        new_content,
        hunks,
        lines,
        used_content_fallback: false,
    }
}

fn finish_hunk(hunk: RawHunk) -> GitDiffHunk {
    GitDiffHunk {
        old_start: hunk.old_start,
        old_count: hunk.old_count,
        new_start: hunk.new_start,
        new_count: hunk.new_count,
        lines: hunk.lines,
    }
}

fn nonzero_line(line: usize) -> Option<usize> {
    (line > 0).then_some(line)
}

fn parse_hunk_header(header: &str) -> Option<(usize, usize, usize, usize)> {
    let mut parts = header.split_whitespace();
    if parts.next()? != "@@" {
        return None;
    }
    let (old_start, old_count) = parse_hunk_range(parts.next()?)?;
    let (new_start, new_count) = parse_hunk_range(parts.next()?)?;
    Some((old_start, old_count, new_start, new_count))
}

fn parse_hunk_range(value: &str) -> Option<(usize, usize)> {
    let value = value.trim_start_matches(['-', '+']);
    let mut parts = value.split(',');
    let start = parts.next()?.parse().ok()?;
    let count = parts.next().map_or(Some(1), |count| count.parse().ok())?;
    Some((start, count))
}

fn content_diff_lines(old_content: &str, new_content: &str) -> Vec<GitDiffLine> {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();
    let matches = patience_matches(&old_lines, &new_lines);
    let mut lines = Vec::with_capacity(old_lines.len() + new_lines.len());
    let (mut old_cursor, mut new_cursor) = (0usize, 0usize);

    for (old_index, new_index) in matches {
        append_content_diff_gap(
            &old_lines, &new_lines, old_cursor, old_index, new_cursor, new_index, &mut lines,
        );
        lines.push(GitDiffLine {
            status: DiffStatus::Unchanged,
            old_line_number: Some(old_index + 1),
            new_line_number: Some(new_index + 1),
            text: new_lines[new_index].to_string(),
        });
        old_cursor = old_index + 1;
        new_cursor = new_index + 1;
    }

    append_content_diff_gap(
        &old_lines,
        &new_lines,
        old_cursor,
        old_lines.len(),
        new_cursor,
        new_lines.len(),
        &mut lines,
    );
    lines
}

/// Finds ordered equal-line anchors without allocating an old-by-new matrix.
///
/// Lines that occur exactly once in both inputs are selected with a longest
/// increasing subsequence. Equal prefixes and suffixes in each anchor gap are
/// retained as additional matches; ambiguous interior blocks are represented
/// as deletions followed by additions. This keeps memory linear in the input
/// size and makes the fallback deterministic for very large files.
fn patience_matches(old_lines: &[&str], new_lines: &[&str]) -> Vec<(usize, usize)> {
    let anchors = patience_anchors(old_lines, new_lines);
    let mut matches = Vec::new();
    let (mut old_cursor, mut new_cursor) = (0usize, 0usize);

    for &(old_index, new_index) in &anchors {
        append_boundary_matches(
            old_lines,
            new_lines,
            &mut old_cursor,
            old_index,
            &mut new_cursor,
            new_index,
            &mut matches,
        );
        matches.push((old_index, new_index));
        old_cursor = old_index + 1;
        new_cursor = new_index + 1;
    }

    append_boundary_matches(
        old_lines,
        new_lines,
        &mut old_cursor,
        old_lines.len(),
        &mut new_cursor,
        new_lines.len(),
        &mut matches,
    );
    matches
}

fn patience_anchors(old_lines: &[&str], new_lines: &[&str]) -> Vec<(usize, usize)> {
    let old_positions = unique_line_positions(old_lines);
    let new_positions = unique_line_positions(new_lines);
    let candidates: Vec<_> = old_lines
        .iter()
        .enumerate()
        .filter_map(|(old_index, line)| {
            let Some(Some(unique_old_index)) = old_positions.get(line) else {
                return None;
            };
            let Some(Some(new_index)) = new_positions.get(line) else {
                return None;
            };
            (*unique_old_index == old_index).then_some((old_index, *new_index))
        })
        .collect();
    longest_increasing_subsequence(&candidates)
}

fn unique_line_positions<'a>(lines: &[&'a str]) -> HashMap<&'a str, Option<usize>> {
    let mut positions = HashMap::with_capacity(lines.len());
    for (index, line) in lines.iter().copied().enumerate() {
        match positions.entry(line) {
            Entry::Vacant(entry) => {
                entry.insert(Some(index));
            }
            Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    positions
}

fn longest_increasing_subsequence(candidates: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![None; candidates.len()];
    for (index, &(_, new_index)) in candidates.iter().enumerate() {
        let position = tails.partition_point(|&tail| candidates[tail].1 < new_index);
        if position > 0 {
            previous[index] = Some(tails[position - 1]);
        }
        if position == tails.len() {
            tails.push(index);
        } else {
            tails[position] = index;
        }
    }

    let Some(&last) = tails.last() else {
        return Vec::new();
    };
    let mut selected = Vec::with_capacity(tails.len());
    let mut current = Some(last);
    while let Some(index) = current {
        selected.push(candidates[index]);
        current = previous[index];
    }
    selected.reverse();
    selected
}

fn append_boundary_matches(
    old_lines: &[&str],
    new_lines: &[&str],
    old_start: &mut usize,
    old_end: usize,
    new_start: &mut usize,
    new_end: usize,
    matches: &mut Vec<(usize, usize)>,
) {
    while *old_start < old_end
        && *new_start < new_end
        && old_lines[*old_start] == new_lines[*new_start]
    {
        matches.push((*old_start, *new_start));
        *old_start += 1;
        *new_start += 1;
    }

    let mut old_suffix_end = old_end;
    let mut new_suffix_end = new_end;
    let mut suffix = Vec::new();
    while *old_start < old_suffix_end
        && *new_start < new_suffix_end
        && old_lines[old_suffix_end - 1] == new_lines[new_suffix_end - 1]
    {
        old_suffix_end -= 1;
        new_suffix_end -= 1;
        suffix.push((old_suffix_end, new_suffix_end));
    }
    suffix.reverse();
    matches.extend(suffix);
}

fn append_content_diff_gap(
    old_lines: &[&str],
    new_lines: &[&str],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
    lines: &mut Vec<GitDiffLine>,
) {
    lines.extend((old_start..old_end).map(|old_index| GitDiffLine {
        status: DiffStatus::Deleted,
        old_line_number: Some(old_index + 1),
        new_line_number: None,
        text: old_lines[old_index].to_string(),
    }));
    lines.extend((new_start..new_end).map(|new_index| GitDiffLine {
        status: DiffStatus::Added,
        old_line_number: None,
        new_line_number: Some(new_index + 1),
        text: new_lines[new_index].to_string(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct FakeRepository {
        root: PathBuf,
        entries: Vec<GitStatusEntry>,
        diffs: HashMap<String, GitFileDiff>,
        fail_status: Cell<bool>,
    }

    impl GitRepository for FakeRepository {
        fn repository_root(&self) -> &Path {
            &self.root
        }

        fn status_entries(&self) -> Result<Vec<GitStatusEntry>, GitError> {
            if self.fail_status.get() {
                Err(GitError::new("fixture status failure"))
            } else {
                Ok(self.entries.clone())
            }
        }

        fn file_diff(&self, path: &Path) -> Result<GitFileDiff, GitError> {
            self.diffs
                .get(&path_key(path))
                .cloned()
                .ok_or_else(|| GitError::new("fixture diff failure"))
        }
    }

    fn project(root: &Path, content: &str, name: &str) -> BddProject {
        BddProject {
            root_dir: root.to_path_buf(),
            features: vec![parse_feature(content, root.join(name))],
        }
    }

    fn fake_with_diff(
        root: &Path,
        path: &str,
        status: FileGitStatus,
        old_content: Option<&str>,
        new_content: Option<&str>,
    ) -> FakeRepository {
        FakeRepository {
            root: root.to_path_buf(),
            entries: vec![GitStatusEntry {
                path: PathBuf::from(path),
                status,
            }],
            diffs: HashMap::from([(
                path.to_string(),
                GitFileDiff::from_contents(old_content, new_content),
            )]),
            fail_status: Cell::new(false),
        }
    }

    #[test]
    fn modified_feature_status_is_project_aligned() {
        let root = PathBuf::from("/fixture");
        let old = "Feature: Backup\n  Scenario: Restore\n    Given a backup exists\n";
        let new = "Feature: Backup\n  Scenario: Restore\n    Given a backup exists\n    Then files are restored\n";
        let project = project(&root, new, "backup.feature");
        let repository = fake_with_diff(
            &root,
            "backup.feature",
            FileGitStatus::Modified,
            Some(old),
            Some(new),
        );
        let model = load_feature_git_status_with_repository(&project, &repository);
        assert!(model.available);
        assert_eq!(model.current[0].file_status, FileGitStatus::Modified);
        assert_eq!(model.current[0].scenarios[0].status, DiffStatus::Modified);
    }

    #[test]
    fn content_fallback_preserves_repeated_unchanged_lines() {
        let content = "Feature: A\n  Scenario: S\n    Given same\n    And same\n";
        let diff = GitFileDiff::from_contents(Some(content), Some(content));

        assert_eq!(diff.lines.len(), 4);
        assert!(
            diff.lines
                .iter()
                .all(|line| line.status == DiffStatus::Unchanged)
        );
    }

    #[test]
    fn content_fallback_handles_large_disjoint_files_without_a_matrix() {
        let old = (0..4096)
            .map(|index| format!("old-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..4096)
            .map(|index| format!("new-{index}"))
            .collect::<Vec<_>>()
            .join("\n");

        let diff = GitFileDiff::from_contents(Some(&old), Some(&new));

        assert_eq!(diff.lines.len(), 8192);
        assert!(
            diff.lines[..4096]
                .iter()
                .all(|line| line.status == DiffStatus::Deleted)
        );
        assert!(
            diff.lines[4096..]
                .iter()
                .all(|line| line.status == DiffStatus::Added)
        );
    }

    #[test]
    fn untracked_feature_and_scenario_are_added() {
        let root = PathBuf::from("/fixture");
        let new = "Feature: Login\n  Scenario: New\n    Given the login page is open\n";
        let project = project(&root, new, "login.feature");
        let repository = fake_with_diff(
            &root,
            "login.feature",
            FileGitStatus::Untracked,
            None,
            Some(new),
        );
        let model = load_feature_git_status_with_repository(&project, &repository);
        assert_eq!(model.current[0].file_status, FileGitStatus::Untracked);
        assert_eq!(model.current[0].scenarios[0].status, DiffStatus::Added);
        assert!(
            model.current[0]
                .diff
                .as_ref()
                .unwrap()
                .lines
                .iter()
                .any(|line| { line.status == DiffStatus::Added && line.text.contains("Given") })
        );
    }

    #[test]
    fn staged_added_feature_has_added_file_status() {
        let root = PathBuf::from("/fixture");
        let new = "Feature: Added\n  Scenario: New\n    Given a new file\n";
        let project = project(&root, new, "added.feature");
        let repository = fake_with_diff(
            &root,
            "added.feature",
            FileGitStatus::Added,
            None,
            Some(new),
        );
        let model = load_feature_git_status_with_repository(&project, &repository);
        assert_eq!(model.current[0].file_status, FileGitStatus::Added);
        assert_eq!(model.current[0].scenarios[0].status, DiffStatus::Added);
    }

    #[test]
    fn deleted_step_retains_old_text_and_added_step_has_new_text() {
        let root = PathBuf::from("/fixture");
        let old = "Feature: Backup\n  Scenario: Restore\n    Given a backup exists\n    And the application is closed\n";
        let new = "Feature: Backup\n  Scenario: Restore\n    Given a backup exists\n    And the application is not running\n    When I restore the backup\n";
        let path = root.join("backup.feature");
        let diff = GitFileDiff::from_contents(Some(old), Some(new));
        let view = map_feature_git_diff(path, FileGitStatus::Modified, diff);
        let scenario = &view.scenarios[0];
        assert_eq!(scenario.status, DiffStatus::Modified);
        assert!(scenario.diff_lines.iter().any(|line| {
            line.status == DiffStatus::Deleted && line.text.contains("application is closed")
        }));
        assert!(
            scenario.diff_lines.iter().any(|line| {
                line.status == DiffStatus::Added && line.text.contains("not running")
            })
        );
        assert!(scenario.diff_lines.iter().any(|line| {
            line.status == DiffStatus::Added && line.text.contains("restore the backup")
        }));
    }

    #[test]
    fn feature_level_changes_are_kept_outside_scenario_mapping() {
        let root = PathBuf::from("/fixture");
        let old = "Feature: Old title\n  Old description\n  Scenario: Restore\n    Given a backup exists\n";
        let new = "Feature: New title\n  New description\n  Scenario: Restore\n    Given a snapshot exists\n";
        let view = map_feature_git_diff(
            root.join("backup.feature"),
            FileGitStatus::Modified,
            GitFileDiff::from_contents(Some(old), Some(new)),
        );

        assert!(
            view.feature_diff_lines
                .iter()
                .any(|line| line.text.contains("Feature: Old title"))
        );
        assert!(
            view.feature_diff_lines
                .iter()
                .any(|line| line.text.contains("New description"))
        );
        assert!(
            view.scenarios[0]
                .diff_lines
                .iter()
                .any(|line| line.text.contains("snapshot exists"))
        );
    }

    #[test]
    fn rule_title_and_description_between_scenarios_are_feature_level_changes() {
        let root = PathBuf::from("/fixture");
        let old = "Feature: A\n  Scenario: First\n    Given first\n\n  Rule: Login\n    Old rule description\n    Scenario: Second\n      Given second\n";
        let new = "Feature: A\n  Scenario: First\n    Given first\n\n  Rule: Renamed login\n    New rule description\n    Scenario: Second\n      Given second\n";
        let view = map_feature_git_diff(
            root.join("rules.feature"),
            FileGitStatus::Modified,
            GitFileDiff::from_contents(Some(old), Some(new)),
        );

        assert_eq!(view.scenarios[0].status, DiffStatus::Unchanged);
        assert!(view.scenarios[0].diff_lines.is_empty());
        assert!(
            view.feature_diff_lines
                .iter()
                .any(|line| line.text.contains("Renamed login"))
        );
        assert!(
            view.feature_diff_lines
                .iter()
                .any(|line| line.text.contains("New rule description"))
        );
    }

    #[test]
    fn content_overrides_map_unsaved_feature_content() {
        let root = PathBuf::from("/fixture");
        let disk = "Feature: A\n  Scenario: S\n    Given disk\n";
        let buffer = "Feature: A\n  Scenario: S\n    Given unsaved\n";
        let project = project(&root, disk, "a.feature");
        let repository = fake_with_diff(
            &root,
            "a.feature",
            FileGitStatus::Unmodified,
            Some(disk),
            Some(disk),
        );
        let overrides = HashMap::from([(root.join("a.feature"), buffer.to_string())]);

        let model = load_feature_git_status_with_repository_and_scope_and_content_overrides(
            &project,
            &repository,
            GitProjectScope::CurrentFiles,
            &overrides,
        );

        assert_eq!(model.current[0].file_status, FileGitStatus::Modified);
        assert!(
            model.current[0].scenarios[0]
                .diff_lines
                .iter()
                .any(|line| { line.status == DiffStatus::Added && line.text.contains("unsaved") })
        );
    }

    #[test]
    fn deleted_scenario_is_kept_as_head_only_model_data() {
        let root = PathBuf::from("/fixture");
        let old = "Feature: A\n  Scenario: Removed\n    Given old\n";
        let new = "Feature: A\n";
        let view = map_feature_git_diff(
            root.join("a.feature"),
            FileGitStatus::Modified,
            GitFileDiff::from_contents(Some(old), Some(new)),
        );
        let deleted: Vec<_> = view.deleted_scenarios().collect();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].status, DiffStatus::Deleted);
        assert!(
            deleted[0]
                .diff_lines
                .iter()
                .any(|line| line.text.contains("Given old"))
        );
    }

    #[test]
    fn non_git_project_keeps_unmodified_feature_view() {
        let root = tempfile::tempdir().expect("temp root");
        let feature_path = root.path().join("plain.feature");
        let content = "Feature: Plain\n  Scenario: S\n    Given x\n";
        std::fs::write(&feature_path, content).expect("feature");
        let project = project(root.path(), content, "plain.feature");
        let model = load_feature_git_status(&project);
        assert!(!model.available);
        assert_eq!(model.current[0].file_status, FileGitStatus::Unmodified);
    }

    #[test]
    fn status_failure_does_not_remove_project_views() {
        let root = PathBuf::from("/fixture");
        let content = "Feature: A\n  Scenario: S\n    Given x\n";
        let project = project(&root, content, "a.feature");
        let repository = FakeRepository {
            root,
            entries: Vec::new(),
            diffs: HashMap::new(),
            fail_status: Cell::new(true),
        };
        let model = load_feature_git_status_with_repository(&project, &repository);
        assert!(model.available);
        assert!(model.error.is_some());
        assert_eq!(model.current.len(), 1);
        assert_eq!(model.current[0].file_status, FileGitStatus::Unmodified);
    }

    #[test]
    fn diff_failure_keeps_feature_browsable() {
        let root = PathBuf::from("/fixture");
        let content = "Feature: A\n  Scenario: S\n    Given x\n";
        let project = project(&root, content, "a.feature");
        let repository = FakeRepository {
            root,
            entries: vec![GitStatusEntry {
                path: PathBuf::from("a.feature"),
                status: FileGitStatus::Modified,
            }],
            diffs: HashMap::new(),
            fail_status: Cell::new(false),
        };
        let model = load_feature_git_status_with_repository(&project, &repository);
        assert!(model.available);
        assert!(model.error.is_some());
        assert_eq!(model.current.len(), 1);
        assert_eq!(model.current[0].file_status, FileGitStatus::Modified);
        assert!(model.current[0].diff.is_none());
    }

    #[test]
    fn repository_root_can_differ_from_project_root() {
        let repository_root = PathBuf::from("/repo");
        let project_root = repository_root.join("features");
        let content = "Feature: A\n  Scenario: S\n    Given x\n";
        let project = project(&project_root, content, "a.feature");
        let repository = fake_with_diff(
            &repository_root,
            "features/a.feature",
            FileGitStatus::Modified,
            Some(content),
            Some("Feature: A\n  Scenario: S\n    Given y\n"),
        );
        let model = load_feature_git_status_with_repository(&project, &repository);
        assert_eq!(model.current[0].file_status, FileGitStatus::Modified);
        assert_eq!(model.current[0].scenarios[0].status, DiffStatus::Modified);
    }

    #[test]
    fn deleted_features_respect_project_root_and_scan_depth() {
        let repository_root = PathBuf::from("/repo");
        let project_root = repository_root.join("project");
        let content = "Feature: A\n  Scenario: S\n    Given x\n";
        let project = project(&project_root, content, "a.feature");
        let deleted = "Feature: Deleted\n  Scenario: S\n    Given old\n";
        let paths = [
            "project/direct.feature",
            "project/nested/deep.feature",
            "sibling/outside.feature",
        ];
        let repository = FakeRepository {
            root: repository_root,
            entries: paths
                .iter()
                .map(|path| GitStatusEntry {
                    path: PathBuf::from(path),
                    status: FileGitStatus::Deleted,
                })
                .collect(),
            diffs: paths
                .iter()
                .map(|path| {
                    (
                        (*path).to_string(),
                        GitFileDiff::from_contents(Some(deleted), None),
                    )
                })
                .collect(),
            fail_status: Cell::new(false),
        };

        let recursive = load_feature_git_status_with_repository_and_scope(
            &project,
            &repository,
            GitProjectScope::Recursive,
        );
        assert_eq!(recursive.deleted.len(), 2);
        assert!(
            recursive
                .deleted
                .iter()
                .all(|view| view.path.starts_with("project"))
        );

        let shallow = load_feature_git_status_with_repository_and_scope(
            &project,
            &repository,
            GitProjectScope::Shallow,
        );
        assert_eq!(shallow.deleted.len(), 1);
        assert_eq!(shallow.deleted[0].path, PathBuf::from(paths[0]));

        let current_files = load_feature_git_status_with_repository_and_scope(
            &project,
            &repository,
            GitProjectScope::CurrentFiles,
        );
        assert!(current_files.deleted.is_empty());
    }

    #[test]
    fn porcelain_parser_handles_file_statuses_and_untracked_paths() {
        let output = b" M backup.feature\0A  added.feature\0D  old.feature\0?? new.feature\0AD added-then-removed.feature\0DA deleted-then-restored.feature\0MD modified-then-removed.feature\0";
        let entries = parse_porcelain_status(output);
        assert_eq!(entries.len(), 7);
        assert_eq!(entries[0].status, FileGitStatus::Modified);
        assert_eq!(entries[1].status, FileGitStatus::Added);
        assert_eq!(entries[2].status, FileGitStatus::Deleted);
        assert_eq!(entries[3].status, FileGitStatus::Untracked);
        assert_eq!(entries[4].status, FileGitStatus::Unmodified);
        assert_eq!(entries[5].status, FileGitStatus::Modified);
        assert_eq!(entries[6].status, FileGitStatus::Deleted);
    }

    #[test]
    fn unified_diff_parser_keeps_deleted_and_added_lines_separate() {
        let patch = "@@ -1,3 +1,3 @@\n Feature: A\n-    When I save\n+    When I create a snapshot\n   Then done\n";
        let diff = parse_unified_diff(
            patch,
            Some("Feature: A\n    When I save\n  Then done\n".into()),
            Some("Feature: A\n    When I create a snapshot\n  Then done\n".into()),
        );
        assert_eq!(diff.lines[1].status, DiffStatus::Deleted);
        assert_eq!(diff.lines[2].status, DiffStatus::Added);
        assert_eq!(diff.lines[1].old_line_number, Some(2));
        assert_eq!(diff.lines[2].new_line_number, Some(2));
    }

    #[test]
    fn cli_adapter_reads_modified_untracked_and_deleted_features() {
        let probe = Command::new("git").arg("--version").output();
        if probe.as_ref().is_err() || !probe.as_ref().is_ok_and(|output| output.status.success()) {
            return;
        }
        let root = tempfile::tempdir().expect("temp root");
        let backup = "Feature: Backup\n  Scenario: Restore\n    Given a backup exists\n";
        let old = "Feature: Old\n  Scenario: Removed\n    Given old\n";
        let renamed = "Feature: Renamed\n  Scenario: Kept\n    Given stable\n";
        std::fs::write(root.path().join("backup.feature"), backup).expect("backup");
        std::fs::write(root.path().join("old.feature"), old).expect("old");
        std::fs::write(root.path().join("rename-source.feature"), renamed).expect("rename source");
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(root.path())
                .args(args)
                .output()
                .expect("git command");
            assert!(
                output.status.success(),
                "git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "Teshi Test"]);
        run(&["config", "user.email", "teshi@example.test"]);
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "initial"]);

        std::fs::write(
            root.path().join("backup.feature"),
            "Feature: Backup\n  Scenario: Restore\n    Given a snapshot exists\n",
        )
        .expect("modify backup");
        std::fs::write(
            root.path().join("new.feature"),
            "Feature: New\n  Scenario: Added\n    Given new\n",
        )
        .expect("new");
        std::fs::remove_file(root.path().join("old.feature")).expect("delete old");
        std::fs::rename(
            root.path().join("rename-source.feature"),
            root.path().join("rename-target.feature"),
        )
        .expect("rename feature");

        let repository = GitCliRepository::discover(root.path())
            .expect("discover Git")
            .expect("repository");
        let entries = repository.status_entries().expect("status");
        let statuses: HashMap<_, _> = entries
            .iter()
            .map(|entry| (entry.path.to_string_lossy().to_string(), entry.status))
            .collect();
        assert_eq!(
            statuses.get("backup.feature"),
            Some(&FileGitStatus::Modified)
        );
        assert_eq!(statuses.get("new.feature"), Some(&FileGitStatus::Untracked));
        assert_eq!(statuses.get("old.feature"), Some(&FileGitStatus::Deleted));
        assert_eq!(
            statuses.get("rename-source.feature"),
            Some(&FileGitStatus::Deleted)
        );
        assert_eq!(
            statuses.get("rename-target.feature"),
            Some(&FileGitStatus::Untracked)
        );

        let diff = repository
            .file_diff(Path::new("backup.feature"))
            .expect("backup diff");
        assert!(diff.lines.iter().any(|line| {
            line.status == DiffStatus::Deleted && line.text.contains("a backup exists")
        }));
        assert!(diff.lines.iter().any(|line| {
            line.status == DiffStatus::Added && line.text.contains("a snapshot exists")
        }));
        let deleted = repository
            .file_diff(Path::new("old.feature"))
            .expect("deleted diff");
        assert!(
            deleted.lines.iter().any(|line| {
                line.status == DiffStatus::Deleted && line.text.contains("Given old")
            })
        );
        let renamed_target = repository
            .file_diff(Path::new("rename-target.feature"))
            .expect("renamed target diff");
        assert!(renamed_target.old_content.is_none());
        assert_eq!(renamed_target.new_content.as_deref(), Some(renamed));
    }
}
