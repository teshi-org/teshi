//! `teshi requirements` commands for the user-level requirement store.

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use teshi_engine::{
    ImportProjectOptions, ImportProjectPlan, import_project_requirements, requirements_data_dir,
};

use super::RequirementsCommand;

/// Handles `teshi requirements path` and `teshi requirements import-project`.
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
