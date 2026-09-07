//! `teshi requirements` commands for the user-level requirement store.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use teshi_core::authoring::RequirementIterationFilter;
use teshi_engine::{
    ImportProjectOptions, ImportProjectPlan, RequirementStoreError, import_project_requirements,
    list_requirement_documents, read_requirement_document, requirements_data_dir,
    resolve_requirement_ref_in_store, set_requirement_documents_iteration,
    update_requirement_document_body,
};

use super::RequirementsCommand;

/// Captured stdout/stderr and process status for a store command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequirementsCommandOutput {
    /// Bytes written to stdout, including a trailing newline when non-empty.
    pub stdout: String,
    /// Bytes written to stderr, including a trailing newline when non-empty.
    pub stderr: String,
    /// Process exit code: `0` success, `2` ambiguous ref, `1` other errors.
    pub exit_code: i32,
}

type EditorLauncher = Box<dyn Fn(&Path) -> Result<(), String>>;

/// I/O hooks so tests can drive `edit` without a real terminal or GUI editor.
pub(crate) struct RequirementsRuntime {
    /// Whether stdin is an interactive TTY.
    pub stdin_is_terminal: bool,
    /// Optional preloaded stdin body used by tests instead of reading the process stdin.
    pub stdin_body: Option<String>,
    /// Launches an editor against a temporary Markdown file.
    pub launch_editor: EditorLauncher,
}

impl RequirementsRuntime {
    /// Production hooks: real stdin TTY detection and `$VISUAL`/`$EDITOR`.
    pub fn production() -> Self {
        Self {
            stdin_is_terminal: io::stdin().is_terminal(),
            stdin_body: None,
            launch_editor: Box::new(spawn_system_editor),
        }
    }
}

#[derive(Debug)]
enum CliError {
    Store {
        err: RequirementStoreError,
        temp_file: Option<PathBuf>,
    },
    EditorUnavailable,
    EditorFailed(String),
    MissingEditInput,
    FileAndStdin,
    Io(anyhow::Error),
}

impl CliError {
    fn store(err: RequirementStoreError) -> Self {
        Self::Store {
            err,
            temp_file: None,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Store { err, .. } => err.code(),
            Self::EditorUnavailable => "editor_unavailable",
            Self::EditorFailed(_) => "editor_failed",
            Self::MissingEditInput => "missing_edit_input",
            Self::FileAndStdin => "conflicting_edit_input",
            Self::Io(_) => "requirement_store_io",
        }
    }

    fn exit_code(&self) -> i32 {
        match self {
            Self::Store { err, .. } => err.exit_code(),
            _ => 1,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store { err, temp_file } => {
                write!(f, "{err}")?;
                if let Some(path) = temp_file {
                    write!(f, "; unsaved editor buffer kept at {}", path.display())?;
                }
                Ok(())
            }
            Self::EditorUnavailable => f.write_str(
                "no editor configured; set $VISUAL or $EDITOR (Windows falls back to notepad)",
            ),
            Self::EditorFailed(message) => write!(f, "editor failed: {message}"),
            Self::MissingEditInput => {
                f.write_str("non-interactive edit requires --file or Markdown on stdin")
            }
            Self::FileAndStdin => f.write_str("use either --file or stdin, not both"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

#[derive(Serialize)]
struct DocumentMetaJson {
    id: String,
    title: String,
    path: String,
    iteration: Option<String>,
    revision: String,
}

#[derive(Serialize)]
struct ListEnvelopeJson {
    store_id: String,
    store_path: String,
    documents: Vec<DocumentMetaJson>,
}

#[derive(Serialize)]
struct ShowEnvelopeJson {
    store_id: String,
    store_path: String,
    id: String,
    title: String,
    path: String,
    iteration: Option<String>,
    revision: String,
    body: String,
}

#[derive(Serialize)]
struct MatchJson {
    id: String,
    path: String,
    title: String,
}

#[derive(Serialize)]
struct ErrorEnvelopeJson {
    code: String,
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<MatchJson>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temp_file: Option<String>,
}

#[derive(Serialize)]
struct MutationOkJson {
    ok: bool,
    ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iteration: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

/// Handles `teshi requirements` including path, import, and the store control plane.
///
/// Non-zero store-command failures call [`std::process::exit`] after writing output
/// so JSON errors are not duplicated by anyhow.
///
/// # Errors
///
/// Returns an error when path resolution or import fails.
pub fn handle_requirements_command(
    action: &RequirementsCommand,
    cli_override: Option<&Path>,
) -> Result<()> {
    match action {
        RequirementsCommand::Path => print_requirements_path(cli_override),
        RequirementsCommand::ImportProject {
            project,
            dry_run,
            yes,
        } => import_project(cli_override, project.as_deref(), *dry_run, *yes),
        _ => {
            let output =
                execute_store_command(action, cli_override, &RequirementsRuntime::production());
            if !output.stdout.is_empty() {
                print!("{}", output.stdout);
            }
            if !output.stderr.is_empty() {
                eprint!("{}", output.stderr);
            }
            if output.exit_code != 0 {
                std::process::exit(output.exit_code);
            }
            Ok(())
        }
    }
}

/// Runs list/show/edit/iteration commands without exiting the process.
pub(crate) fn execute_store_command(
    action: &RequirementsCommand,
    cli_override: Option<&Path>,
    runtime: &RequirementsRuntime,
) -> RequirementsCommandOutput {
    match run_store_command(action, cli_override, runtime) {
        Ok(stdout) => RequirementsCommandOutput {
            stdout: ensure_trailing_newline(stdout),
            stderr: String::new(),
            exit_code: 0,
        },
        Err((json, err)) => emit_error(json, err),
    }
}

fn ensure_trailing_newline(mut text: String) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text
    } else {
        text.push('\n');
        text
    }
}

fn emit_error(json: bool, err: CliError) -> RequirementsCommandOutput {
    let exit_code = err.exit_code();
    if json {
        let matches = match &err {
            CliError::Store {
                err: RequirementStoreError::Ambiguous { matches, .. },
                ..
            } => Some(
                matches
                    .iter()
                    .map(|item| MatchJson {
                        id: item.id.clone(),
                        path: item.path.clone(),
                        title: item.title.clone(),
                    })
                    .collect(),
            ),
            _ => None,
        };
        let temp_file = match &err {
            CliError::Store {
                temp_file: Some(path),
                ..
            } => Some(path.display().to_string()),
            _ => None,
        };
        let payload = ErrorEnvelopeJson {
            code: err.code().to_string(),
            error: err.to_string(),
            matches,
            temp_file,
        };
        let stdout = serde_json::to_string(&payload).unwrap_or_else(|_| {
            r#"{"code":"requirement_store_io","error":"failed to serialize error"}"#.to_string()
        });
        RequirementsCommandOutput {
            stdout: ensure_trailing_newline(stdout),
            stderr: String::new(),
            exit_code,
        }
    } else {
        RequirementsCommandOutput {
            stdout: String::new(),
            stderr: ensure_trailing_newline(err.to_string()),
            exit_code,
        }
    }
}

fn run_store_command(
    action: &RequirementsCommand,
    cli_override: Option<&Path>,
    runtime: &RequirementsRuntime,
) -> Result<String, (bool, CliError)> {
    let requirements_root =
        requirements_data_dir(cli_override).map_err(|err| (false, CliError::Io(err)))?;
    match action {
        RequirementsCommand::List {
            iteration,
            unassigned,
            json,
        } => list_documents(&requirements_root, iteration.as_deref(), *unassigned, *json)
            .map_err(|err| (*json, err)),
        RequirementsCommand::Show { reference, json } => {
            show_document(&requirements_root, reference, *json).map_err(|err| (*json, err))
        }
        RequirementsCommand::Edit {
            reference,
            file,
            force,
            json,
        } => edit_document(
            &requirements_root,
            reference,
            file.as_deref(),
            *force,
            *json,
            runtime,
        )
        .map_err(|err| (*json, err)),
        RequirementsCommand::SetIteration {
            refs,
            iteration,
            json,
        } => mutate_iteration(&requirements_root, refs, Some(iteration), *json)
            .map_err(|err| (*json, err)),
        RequirementsCommand::ClearIteration { refs, json } => {
            mutate_iteration(&requirements_root, refs, None, *json).map_err(|err| (*json, err))
        }
        RequirementsCommand::Path | RequirementsCommand::ImportProject { .. } => {
            unreachable!("path/import-project are handled before store commands")
        }
    }
}

fn list_documents(
    requirements_root: &Path,
    iteration: Option<&str>,
    unassigned: bool,
    json: bool,
) -> Result<String, CliError> {
    let filter = if unassigned {
        RequirementIterationFilter::Unassigned
    } else if let Some(name) = iteration {
        RequirementIterationFilter::Named(name.to_string())
    } else {
        RequirementIterationFilter::All
    };
    let listed = list_requirement_documents(requirements_root, &filter).map_err(CliError::store)?;
    if json {
        let envelope = ListEnvelopeJson {
            store_id: listed.store_id.to_string(),
            store_path: listed.store_path.display().to_string(),
            documents: listed
                .documents
                .iter()
                .map(|doc| DocumentMetaJson {
                    id: doc.id.clone(),
                    title: doc.title.clone(),
                    path: doc.path.clone(),
                    iteration: doc.iteration.clone(),
                    revision: doc.revision.as_str().to_string(),
                })
                .collect(),
        };
        serde_json::to_string(&envelope).map_err(|err| CliError::Io(err.into()))
    } else {
        let mut lines = String::new();
        for doc in &listed.documents {
            let iteration = doc.iteration.as_deref().unwrap_or("-");
            lines.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                doc.id, iteration, doc.path, doc.title
            ));
        }
        if lines.ends_with('\n') {
            lines.pop();
        }
        Ok(lines)
    }
}

fn show_document(
    requirements_root: &Path,
    reference: &str,
    json: bool,
) -> Result<String, CliError> {
    let document_id =
        resolve_requirement_ref_in_store(requirements_root, reference).map_err(CliError::store)?;
    let document =
        read_requirement_document(requirements_root, &document_id).map_err(CliError::store)?;
    if json {
        let listed =
            list_requirement_documents(requirements_root, &RequirementIterationFilter::All)
                .map_err(CliError::store)?;
        let envelope = ShowEnvelopeJson {
            store_id: listed.store_id.to_string(),
            store_path: listed.store_path.display().to_string(),
            id: document.meta.id,
            title: document.meta.title,
            path: document.meta.path,
            iteration: document.meta.iteration,
            revision: document.meta.revision.as_str().to_string(),
            body: document.body,
        };
        serde_json::to_string(&envelope).map_err(|err| CliError::Io(err.into()))
    } else {
        Ok(document.body)
    }
}

fn mutate_iteration(
    requirements_root: &Path,
    refs: &[String],
    iteration: Option<&str>,
    json: bool,
) -> Result<String, CliError> {
    let mut ids = Vec::with_capacity(refs.len());
    for reference in refs {
        ids.push(
            resolve_requirement_ref_in_store(requirements_root, reference)
                .map_err(CliError::store)?,
        );
    }
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    set_requirement_documents_iteration(requirements_root, &id_refs, iteration)
        .map_err(CliError::store)?;
    if json {
        serde_json::to_string(&MutationOkJson {
            ok: true,
            ids,
            iteration: Some(iteration.map(str::to_string)),
            revision: None,
        })
        .map_err(|err| CliError::Io(err.into()))
    } else {
        Ok(String::new())
    }
}

fn edit_document(
    requirements_root: &Path,
    reference: &str,
    file: Option<&Path>,
    force: bool,
    json: bool,
    runtime: &RequirementsRuntime,
) -> Result<String, CliError> {
    let document_id =
        resolve_requirement_ref_in_store(requirements_root, reference).map_err(CliError::store)?;
    let current =
        read_requirement_document(requirements_root, &document_id).map_err(CliError::store)?;
    let expected_revision = current.meta.revision.as_str().to_string();

    let (body, temp_file) = load_edit_body(&document_id, &current.body, file, runtime)?;
    if body == current.body {
        if let Some(path) = &temp_file {
            let _ = fs::remove_file(path);
        }
        return if json {
            serde_json::to_string(&MutationOkJson {
                ok: true,
                ids: vec![document_id],
                iteration: None,
                revision: Some(expected_revision),
            })
            .map_err(|err| CliError::Io(err.into()))
        } else {
            Ok(String::new())
        };
    }

    match update_requirement_document_body(
        requirements_root,
        &document_id,
        &body,
        Some(&expected_revision),
        force,
    ) {
        Ok(meta) => {
            if let Some(path) = &temp_file {
                let _ = fs::remove_file(path);
            }
            if json {
                serde_json::to_string(&MutationOkJson {
                    ok: true,
                    ids: vec![document_id],
                    iteration: None,
                    revision: Some(meta.revision.as_str().to_string()),
                })
                .map_err(|err| CliError::Io(err.into()))
            } else {
                Ok(String::new())
            }
        }
        Err(err) => Err(CliError::Store { err, temp_file }),
    }
}

fn load_edit_body(
    document_id: &str,
    current_body: &str,
    file: Option<&Path>,
    runtime: &RequirementsRuntime,
) -> Result<(String, Option<PathBuf>), CliError> {
    let stdin_body = read_runtime_stdin(runtime)?;
    if file.is_some() && !stdin_body.is_empty() {
        return Err(CliError::FileAndStdin);
    }
    if let Some(path) = file {
        let body = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))
            .map_err(CliError::Io)?;
        return Ok((body, None));
    }
    if !runtime.stdin_is_terminal {
        if stdin_body.is_empty() {
            return Err(CliError::MissingEditInput);
        }
        return Ok((stdin_body, None));
    }
    let temp_path = write_edit_temp(document_id, current_body)?;
    match (runtime.launch_editor)(&temp_path) {
        Ok(()) => {}
        Err(err) if err == "editor_unavailable" => return Err(CliError::EditorUnavailable),
        Err(err) => return Err(CliError::EditorFailed(err)),
    }
    let body = fs::read_to_string(&temp_path)
        .with_context(|| format!("read {}", temp_path.display()))
        .map_err(CliError::Io)?;
    Ok((body, Some(temp_path)))
}

fn read_runtime_stdin(runtime: &RequirementsRuntime) -> Result<String, CliError> {
    if let Some(body) = &runtime.stdin_body {
        return Ok(body.clone());
    }
    if runtime.stdin_is_terminal {
        return Ok(String::new());
    }
    let mut body = String::new();
    io::stdin()
        .read_to_string(&mut body)
        .context("read stdin")
        .map_err(CliError::Io)?;
    Ok(body)
}

fn write_edit_temp(document_id: &str, body: &str) -> Result<PathBuf, CliError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let safe_id: String = document_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let path = std::env::temp_dir().join(format!("teshi-edit-{safe_id}-{nanos}.md"));
    fs::write(&path, body)
        .with_context(|| format!("write {}", path.display()))
        .map_err(CliError::Io)?;
    Ok(path)
}

fn resolve_editor_program() -> Option<String> {
    for key in ["VISUAL", "EDITOR"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    #[cfg(windows)]
    {
        Some("notepad".to_string())
    }
    #[cfg(not(windows))]
    None
}

fn command_from_editor_spec(spec: &str) -> Command {
    let mut parts = spec.split_whitespace();
    let program = parts.next().unwrap_or("true");
    let mut command = Command::new(program);
    command.args(parts);
    command
}

fn spawn_system_editor(path: &Path) -> Result<(), String> {
    let spec = resolve_editor_program().ok_or_else(|| "editor_unavailable".to_string())?;
    let mut command = command_from_editor_spec(&spec);
    command.arg(path);
    let status = command
        .status()
        .map_err(|err| format!("launch editor '{spec}': {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("editor '{spec}' exited with {status}"))
    }
}

fn print_requirements_path(cli_override: Option<&Path>) -> Result<()> {
    let path = requirements_data_dir(cli_override)?;
    println!("{}", path.display());
    Ok(())
}

fn import_project(
    cli_override: Option<&Path>,
    project: Option<&Path>,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let requirements_root = requirements_data_dir(cli_override)?;
    let project_root = match project {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().context("resolve current directory")?,
    };
    let plan = import_project_requirements(
        &project_root,
        &requirements_root,
        ImportProjectOptions {
            dry_run,
            apply: false,
        },
    )?;
    print_import_plan(&plan)?;
    if dry_run {
        return Ok(());
    }
    if plan.has_conflicts && !yes && !confirm_import()? {
        println!("Import cancelled; requirement store and project test points were not modified.");
        return Ok(());
    }
    let applied = import_project_requirements(
        &project_root,
        &requirements_root,
        ImportProjectOptions {
            dry_run: false,
            apply: true,
        },
    )?;
    println!(
        "Imported {} document(s) into {} (store_id {}).",
        applied.copied_documents,
        applied.target_store_path.display(),
        applied.target_store_id
    );
    Ok(())
}

fn print_import_plan(plan: &ImportProjectPlan) -> Result<()> {
    println!("Target store: {}", plan.target_store_path.display());
    println!("Target store_id: {}", plan.target_store_id);
    println!("Source project: {}", plan.source_project.display());
    if plan.mappings.is_empty() {
        println!("No requirement documents found to import.");
        return Ok(());
    }
    println!("Planned mappings:");
    for mapping in &plan.mappings {
        println!(
            "  {} -> {}  (path {} -> {}) [{}]",
            mapping.source_id,
            mapping.target_id,
            mapping.source_path,
            mapping.target_path,
            mapping.action
        );
    }
    if plan.has_conflicts {
        println!("Conflicts require confirmation before writing.");
    }
    Ok(())
}

fn confirm_import() -> Result<bool> {
    if !io::stdin().is_terminal() {
        bail!("import has conflicts; pass --yes to apply the remapping plan without a prompt");
    }
    print!("Apply this import plan? [y/N] ");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("read confirmation")?;
    let trimmed = line.trim().to_ascii_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::authoring::RequirementDocumentMeta;
    use teshi_engine::{
        compute_document_revision, initialize_requirement_store, save_requirement_document_index,
        save_requirement_markdown,
    };

    fn seed_store(root: &Path) {
        let mut index = initialize_requirement_store(root).unwrap();
        let login = "# Login\n";
        let mobile = "# Login\n\nmobile\n";
        let checkout = "# Checkout\n";
        index.documents = vec![
            RequirementDocumentMeta::new(
                "doc-12",
                "auth/login.md",
                "登录需求",
                compute_document_revision(login),
            ),
            RequirementDocumentMeta::new(
                "doc-37",
                "mobile/login.md",
                "登录需求",
                compute_document_revision(mobile),
            ),
            RequirementDocumentMeta::new(
                "doc-9",
                "shop/checkout.md",
                "Checkout",
                compute_document_revision(checkout),
            ),
        ];
        fs::create_dir_all(root.join("auth")).unwrap();
        fs::create_dir_all(root.join("mobile")).unwrap();
        fs::create_dir_all(root.join("shop")).unwrap();
        save_requirement_markdown(root, &mut index, "auth/login.md", login).unwrap();
        save_requirement_markdown(root, &mut index, "mobile/login.md", mobile).unwrap();
        save_requirement_markdown(root, &mut index, "shop/checkout.md", checkout).unwrap();
        index.documents[0].iteration = Some("Sprint 12".into());
        save_requirement_document_index(root, &index).unwrap();
    }

    fn runtime_non_tty(stdin: &str) -> RequirementsRuntime {
        RequirementsRuntime {
            stdin_is_terminal: false,
            stdin_body: Some(stdin.to_string()),
            launch_editor: Box::new(|_| Err("editor should not launch".into())),
        }
    }

    fn run(store: &Path, action: RequirementsCommand) -> RequirementsCommandOutput {
        execute_store_command(&action, Some(store), &runtime_non_tty(""))
    }

    #[test]
    fn list_json_filters_iteration_and_empty_named_iteration_exits_zero() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let output = run(
            store.path(),
            RequirementsCommand::List {
                iteration: Some("Sprint 12".into()),
                unassigned: false,
                json: true,
            },
        );
        assert_eq!(output.exit_code, 0);
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(value["documents"].as_array().unwrap().len(), 1);
        assert_eq!(value["documents"][0]["id"], "doc-12");

        let empty = run(
            store.path(),
            RequirementsCommand::List {
                iteration: Some("Missing".into()),
                unassigned: false,
                json: true,
            },
        );
        assert_eq!(empty.exit_code, 0);
        let empty_value: serde_json::Value = serde_json::from_str(&empty.stdout).unwrap();
        assert!(empty_value["documents"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_unassigned_excludes_named_iteration() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let output = run(
            store.path(),
            RequirementsCommand::List {
                iteration: None,
                unassigned: true,
                json: false,
            },
        );
        assert_eq!(output.exit_code, 0);
        assert!(!output.stdout.contains("doc-12"));
        assert!(output.stdout.contains("doc-37"));
        assert!(output.stdout.contains("doc-9"));
    }

    #[test]
    fn show_id_prints_body_and_json_includes_metadata() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let text = run(
            store.path(),
            RequirementsCommand::Show {
                reference: "doc-9".into(),
                json: false,
            },
        );
        assert_eq!(text.exit_code, 0);
        assert_eq!(text.stdout, "# Checkout\n");

        let json = run(
            store.path(),
            RequirementsCommand::Show {
                reference: "shop/checkout.md".into(),
                json: true,
            },
        );
        let value: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
        assert_eq!(value["id"], "doc-9");
        assert_eq!(value["body"], "# Checkout\n");
        assert!(value.get("store_id").is_some());
    }

    #[test]
    fn show_ambiguous_title_exits_two_with_json_code() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let output = run(
            store.path(),
            RequirementsCommand::Show {
                reference: "登录需求".into(),
                json: true,
            },
        );
        assert_eq!(output.exit_code, 2);
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(value["code"], "ambiguous_requirement_ref");
        assert_eq!(value["matches"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn show_missing_json_uses_not_found_code() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let output = run(
            store.path(),
            RequirementsCommand::Show {
                reference: "missing-id".into(),
                json: true,
            },
        );
        assert_eq!(output.exit_code, 1);
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(value["code"], "requirement_not_found");
    }

    #[test]
    fn set_and_clear_iteration_are_atomic_for_unknown_ids() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let failed = run(
            store.path(),
            RequirementsCommand::SetIteration {
                refs: vec!["doc-12".into(), "missing".into()],
                iteration: "Sprint 13".into(),
                json: true,
            },
        );
        assert_eq!(failed.exit_code, 1);
        let listed = list_requirement_documents(
            store.path(),
            &RequirementIterationFilter::Named("Sprint 12".into()),
        )
        .unwrap();
        assert_eq!(listed.documents.len(), 1);

        let ok = run(
            store.path(),
            RequirementsCommand::SetIteration {
                refs: vec!["doc-12".into(), "doc-9".into()],
                iteration: "Sprint 13".into(),
                json: true,
            },
        );
        assert_eq!(ok.exit_code, 0);
        let cleared = run(
            store.path(),
            RequirementsCommand::ClearIteration {
                refs: vec!["doc-12".into(), "doc-9".into()],
                json: false,
            },
        );
        assert_eq!(cleared.exit_code, 0);
        let unassigned =
            list_requirement_documents(store.path(), &RequirementIterationFilter::Unassigned)
                .unwrap();
        assert!(unassigned.documents.iter().any(|doc| doc.id == "doc-12"));
        assert!(unassigned.documents.iter().any(|doc| doc.id == "doc-9"));
    }

    #[test]
    fn set_iteration_rejects_invalid_name() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let output = run(
            store.path(),
            RequirementsCommand::SetIteration {
                refs: vec!["doc-12".into()],
                iteration: "   ".into(),
                json: true,
            },
        );
        assert_eq!(output.exit_code, 1);
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(value["code"], "invalid_iteration_name");
    }

    #[test]
    fn edit_file_and_stdin_and_missing_input() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let replacement = store.path().join("new.md");
        fs::write(&replacement, "# Login\n\nfrom file\n").unwrap();
        let via_file = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-12".into(),
                file: Some(replacement.clone()),
                force: false,
                json: true,
            },
            Some(store.path()),
            &runtime_non_tty(""),
        );
        assert_eq!(via_file.exit_code, 0);
        assert_eq!(
            read_requirement_document(store.path(), "doc-12")
                .unwrap()
                .body,
            "# Login\n\nfrom file\n"
        );

        let via_stdin = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-12".into(),
                file: None,
                force: false,
                json: false,
            },
            Some(store.path()),
            &runtime_non_tty("# Login\n\nfrom stdin\n"),
        );
        assert_eq!(via_stdin.exit_code, 0);
        assert_eq!(
            read_requirement_document(store.path(), "doc-12")
                .unwrap()
                .body,
            "# Login\n\nfrom stdin\n"
        );

        let missing = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-12".into(),
                file: None,
                force: false,
                json: true,
            },
            Some(store.path()),
            &runtime_non_tty(""),
        );
        assert_eq!(missing.exit_code, 1);
        let value: serde_json::Value = serde_json::from_str(&missing.stdout).unwrap();
        assert_eq!(value["code"], "missing_edit_input");

        let conflict = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-12".into(),
                file: Some(replacement),
                force: false,
                json: true,
            },
            Some(store.path()),
            &runtime_non_tty("# also stdin\n"),
        );
        assert_eq!(conflict.exit_code, 1);
        let conflict_value: serde_json::Value = serde_json::from_str(&conflict.stdout).unwrap();
        assert_eq!(conflict_value["code"], "conflicting_edit_input");
    }

    #[test]
    fn edit_revision_conflict_force_and_noop() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let store_path = store.path().to_path_buf();
        let raced_path = store_path.clone();
        let tty = RequirementsRuntime {
            stdin_is_terminal: true,
            stdin_body: None,
            launch_editor: Box::new(move |path| {
                update_requirement_document_body(
                    &raced_path,
                    "doc-9",
                    "# Checkout\n\nraced\n",
                    None,
                    true,
                )
                .unwrap();
                fs::write(path, "# Checkout\n\nfrom editor\n").unwrap();
                Ok(())
            }),
        };
        let conflict = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-9".into(),
                file: None,
                force: false,
                json: true,
            },
            Some(store.path()),
            &tty,
        );
        assert_eq!(conflict.exit_code, 1);
        let value: serde_json::Value = serde_json::from_str(&conflict.stdout).unwrap();
        assert_eq!(value["code"], "revision_conflict");
        assert!(value["temp_file"].as_str().is_some());
        assert_eq!(
            read_requirement_document(store.path(), "doc-9")
                .unwrap()
                .body,
            "# Checkout\n\nraced\n"
        );

        let replacement = store.path().join("forced.md");
        fs::write(&replacement, "# Checkout\n\nforced\n").unwrap();
        let forced = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-9".into(),
                file: Some(replacement),
                force: true,
                json: true,
            },
            Some(store.path()),
            &runtime_non_tty(""),
        );
        assert_eq!(forced.exit_code, 0);
        assert_eq!(
            read_requirement_document(store.path(), "doc-9")
                .unwrap()
                .body,
            "# Checkout\n\nforced\n"
        );

        let noop_file = store.path().join("same.md");
        let current = read_requirement_document(store.path(), "doc-9").unwrap();
        fs::write(&noop_file, &current.body).unwrap();
        let revision = current.meta.revision.clone();
        let noop = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-9".into(),
                file: Some(noop_file),
                force: false,
                json: true,
            },
            Some(store.path()),
            &runtime_non_tty(""),
        );
        assert_eq!(noop.exit_code, 0);
        assert_eq!(
            read_requirement_document(store.path(), "doc-9")
                .unwrap()
                .meta
                .revision,
            revision
        );
    }

    #[test]
    fn tty_editor_saves_and_injected_launcher_avoids_gui() {
        let store = tempfile::tempdir().unwrap();
        seed_store(store.path());
        let tty = RequirementsRuntime {
            stdin_is_terminal: true,
            stdin_body: None,
            launch_editor: Box::new(|path| {
                fs::write(path, "# Login\n\nedited in tty\n").unwrap();
                Ok(())
            }),
        };
        let output = execute_store_command(
            &RequirementsCommand::Edit {
                reference: "doc-12".into(),
                file: None,
                force: false,
                json: false,
            },
            Some(store.path()),
            &tty,
        );
        assert_eq!(output.exit_code, 0);
        assert_eq!(
            read_requirement_document(store.path(), "doc-12")
                .unwrap()
                .body,
            "# Login\n\nedited in tty\n"
        );
    }
}
