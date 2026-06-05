//! Append-only locator verification log consumed by strict `steps propose`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// One successful highlight+execute verification before proposing a binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocatorVerifyRecord {
    pub ts_ms: u128,
    pub step_line: u32,
    pub selector: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_arg: Option<String>,
    pub ok: bool,
}

fn verify_log_path(project_root: &Path) -> std::path::PathBuf {
    project_root
        .join(".teshi")
        .join("logs")
        .join("locator-verify.jsonl")
}

/// Appends a verification record for strict propose gating.
pub fn append_locator_verify(project_root: &Path, record: &LocatorVerifyRecord) -> Result<()> {
    let path = verify_log_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

/// Returns true when strict mode is off or a matching verified record exists.
pub fn locator_verify_satisfied(
    project_root: &Path,
    step_line: u32,
    selector: &str,
    action: &str,
) -> Result<()> {
    if !locator_strict_enabled() {
        return Ok(());
    }
    let path = verify_log_path(project_root);
    if !path.is_file() {
        return Err(anyhow!(
            "TESHI_LOCATOR_STRICT=1: no verification log at {}; run `teshi browser verify` first",
            path.display()
        ));
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    for line in text.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let record: LocatorVerifyRecord =
            serde_json::from_str(line).context("parse locator-verify.jsonl line")?;
        if record.ok
            && record.step_line == step_line
            && record.selector == selector
            && record.action == action
        {
            return Ok(());
        }
    }
    Err(anyhow!(
        "TESHI_LOCATOR_STRICT=1: no verified record for step_line={step_line} selector={selector} action={action}; run `teshi browser verify`"
    ))
}

/// Whether `steps propose` requires a prior `browser verify` record.
pub fn locator_strict_enabled() -> bool {
    matches!(
        std::env::var_os("TESHI_LOCATOR_STRICT").as_deref(),
        Some(v) if v == "1" || v == "true"
    )
}

/// Builds a JSON summary for CLI output after verify.
pub fn verify_record_json(project_root: &Path, record: &LocatorVerifyRecord) -> serde_json::Value {
    json!({
        "ok": record.ok,
        "step_line": record.step_line,
        "selector": record.selector,
        "action": record.action,
        "value_arg": record.value_arg,
        "log": verify_log_path(project_root).display().to_string()
    })
}
