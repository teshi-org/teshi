//! Python venv parsing and error classification (pure logic).
//! Filesystem probing and resolution live in `teshi-engine`.

use std::collections::HashMap;

/// Parse `key = value` lines from a `pyvenv.cfg` file's content string.
pub fn parse_pyvenv_cfg_content(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

/// True when the stderr detail indicates a uv trampoline failure.
pub fn is_uv_trampoline_failure(detail: &str) -> bool {
    detail.contains("uv trampoline") || detail.contains("uv internal error")
}

/// True when the stderr detail indicates a missing Python module.
pub fn is_missing_module_failure(detail: &str) -> bool {
    detail.contains("ModuleNotFoundError") || detail.contains("No module named")
}

/// True when the stderr detail indicates an untrusted mount failure.
pub fn is_untrusted_mount_failure(detail: &str) -> bool {
    detail.contains("os error 448") || detail.contains("untrusted") || detail.contains("装载点")
}

/// Format a check failure detail from process output.
pub fn check_failure_detail(stderr: &str, stdout: &str, exit_code: Option<i32>) -> String {
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    format!("exit code {}", exit_code.unwrap_or(-1))
}

/// Build a user-facing message for an import check failure.
pub fn import_check_failed_message(detail: &str, packages: &str) -> String {
    if is_uv_trampoline_failure(detail) {
        format!("Could not run the project Python interpreter ({detail}).")
    } else {
        format!("{packages} are not installed in the venv ({detail}).")
    }
}

/// Build a human-readable hint for a venv python failure.
pub fn venv_python_failure_hint(
    detail: &str,
    pip_hint: &str,
    _venv_root: &str,
    uv_managed: bool,
) -> String {
    if is_uv_trampoline_failure(detail) || is_untrusted_mount_failure(detail) {
        let uv_note = if uv_managed {
            "uv managed venv: run `uv python install` and `uv pip install websockets`, \
             or recreate with `python -m venv .venv`."
        } else {
            "Recreate the venv: `python -m venv .venv`."
        };
        format!("{pip_hint}\n{uv_note}")
    } else if is_missing_module_failure(detail) {
        pip_hint.to_string()
    } else if detail.contains("entity not found") || detail.contains("os error 2") {
        format!(
            "{pip_hint}\n\
             The venv Python launcher could not be started. Run `uv python install` or recreate `.venv`."
        )
    } else {
        pip_hint.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pyvenv_cfg_reads_key_value_pairs() {
        let content = "home = C:\\python\nuv = 0.5.0\nexecutable = C:\\python\\python.exe\n";
        let cfg = parse_pyvenv_cfg_content(content);
        assert_eq!(cfg.get("uv").map(String::as_str), Some("0.5.0"));
    }
}
