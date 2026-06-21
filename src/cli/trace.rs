//! `teshi trace` CLI commands: list and inspect exploration traces.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::TraceCommand;

/// Project-relative path to the traces directory.
fn traces_dir(project_root: &Path) -> PathBuf {
    project_root.join(".teshi").join("traces")
}

fn resolve_project_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    // Walk up to find .teshi/traces
    let mut dir = cwd.clone();
    loop {
        if dir.join(".teshi").join("traces").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    Ok(cwd)
}

use std::path::PathBuf;

pub fn handle_trace_command(action: &TraceCommand) -> Result<()> {
    let project_root = resolve_project_root()?;
    match action {
        TraceCommand::List => list_traces(&project_root),
        TraceCommand::Show { id } => show_trace(&project_root, id),
    }
}

fn list_traces(project_root: &Path) -> Result<()> {
    let dir = traces_dir(project_root);
    if !dir.is_dir() {
        println!(
            "No exploration traces found ({} does not exist)",
            dir.display()
        );
        return Ok(());
    }

    let mut entries: Vec<_> = fs::read_dir(&dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "jsonl"))
        .collect();
    entries.sort_by_key(|e| e.path().metadata().and_then(|m| m.created()).ok());

    if entries.is_empty() {
        println!("No exploration traces found in {}", dir.display());
        return Ok(());
    }

    println!("Exploration traces:");
    println!("{:<30} {:<10} {}", "Session ID", "Actions", "Path");
    println!("{}", "-".repeat(80));
    for entry in &entries {
        let path = entry.path();
        let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        let line_count = fs::read_to_string(&path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        println!("{:<30} {:<10} {}", filename, line_count, path.display());
    }
    Ok(())
}

fn show_trace(project_root: &Path, id: &str) -> Result<()> {
    let trace_path = traces_dir(project_root).join(format!("{}.jsonl", id));
    let content = fs::read_to_string(&trace_path)
        .with_context(|| format!("trace not found: {}", trace_path.display()))?;

    println!("Trace: {}", id);
    println!("{}", "=".repeat(60));
    for (i, line) in content.lines().enumerate() {
        if i >= 50 {
            println!("... ({} more lines)", content.lines().count() - 50);
            break;
        }
        if let Ok(action) = serde_json::from_str::<serde_json::Value>(line) {
            let step = action
                .get("step_index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let act = action.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let ref_id = action.get("ref_id").and_then(|v| v.as_str()).unwrap_or("-");
            let text = action.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let ok = action.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let status = if ok { "✓" } else { "✗" };
            println!(
                "  [{:>2}] {} {} ref={} text={}",
                step, status, act, ref_id, text
            );
        }
    }
    Ok(())
}
