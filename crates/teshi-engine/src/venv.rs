//! Project virtualenv resolution and subprocess environment for browser sidecar and terminal.
//! Pure parsing and error classification live in `teshi-core::venv`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use teshi_core::venv::{
    is_missing_module_failure, is_untrusted_mount_failure, is_uv_trampoline_failure,
    parse_pyvenv_cfg_content,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Resolved project `.venv` / `venv` for Python subprocesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVenv {
    pub root: PathBuf,
    pub python_exe: PathBuf,
    /// When set, passed as `PYTHONPATH` (uv base interpreter + project site-packages).
    pub site_packages: Option<PathBuf>,
    pub uv_managed: bool,
}

/// Finds `.venv` or `venv` under `project_root` and resolves a runnable interpreter.
pub fn resolve_project_venv(project_root: &Path) -> Option<ResolvedVenv> {
    for name in [".venv", "venv"] {
        let root = project_root.join(name);
        if let Some(resolved) = resolve_venv_at(&root) {
            return Some(resolved);
        }
    }
    None
}

fn resolve_venv_at(venv_root: &Path) -> Option<ResolvedVenv> {
    let shim = venv_scripts_python(venv_root)?;
    let uv_managed = is_uv_managed_venv(venv_root);

    let (python_exe, site_packages) = if uv_managed {
        let (python_exe, site_packages) = resolve_uv_python(venv_root)?;
        (python_exe, site_packages)
    } else {
        (dunce::simplified(&shim).to_path_buf(), None)
    };

    let root = dunce::simplified(venv_root).to_path_buf();

    tracing::debug!(
        venv = %root.display(),
        python = %python_exe.display(),
        uv_managed,
        site_packages = ?site_packages.as_ref().map(|p| p.display().to_string()),
        "resolved project venv interpreter"
    );

    Some(ResolvedVenv {
        root,
        python_exe,
        site_packages,
        uv_managed,
    })
}

fn venv_scripts_python(venv_root: &Path) -> Option<PathBuf> {
    let shim = venv_root.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    shim.is_file().then_some(shim)
}

/// For uv venvs, use base CPython from `pyvenv.cfg` — never the trampoline shim.
fn resolve_uv_python(venv_root: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let site_packages = venv_site_packages(venv_root);
    let cfg = parse_pyvenv_cfg(venv_root)?;

    if let Some(exe) = cfg.get("executable") {
        let path = PathBuf::from(exe.trim());
        if let Some(resolved) = existing_python_executable(&path) {
            return Some((resolved, site_packages));
        }
    }

    if let Some(home) = cfg.get("home") {
        let home = PathBuf::from(home.trim());
        for candidate in home_python_candidates(&home) {
            if let Some(resolved) = existing_python_executable(&candidate) {
                return Some((resolved, site_packages));
            }
        }
        tracing::warn!(
            home = %home.display(),
            "uv venv base interpreter from pyvenv.cfg `home` is not runnable"
        );
    }

    None
}

/// Returns a normalized path when `path` is an existing Python executable (follows junctions).
fn existing_python_executable(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(dunce::simplified(path).to_path_buf());
    }
    path.canonicalize()
        .ok()
        .filter(|p| p.is_file())
        .map(|p| dunce::simplified(&p).to_path_buf())
}

fn venv_site_packages(venv_root: &Path) -> Option<PathBuf> {
    let site = venv_root.join("Lib").join("site-packages");
    site.is_dir()
        .then(|| dunce::simplified(&site).to_path_buf())
}

fn home_python_candidates(home: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![home.join("python.exe")]
    } else {
        vec![
            home.join("bin").join("python3"),
            home.join("bin").join("python"),
            home.join("python"),
        ]
    }
}

/// True when `pyvenv.cfg` contains a `uv =` version line.
pub fn is_uv_managed_venv(venv_root: &Path) -> bool {
    parse_pyvenv_cfg(venv_root).is_some_and(|cfg| cfg.contains_key("uv"))
}

/// Parse `key = value` lines from `.venv/pyvenv.cfg`.
pub fn parse_pyvenv_cfg(venv_root: &Path) -> Option<HashMap<String, String>> {
    let content = std::fs::read_to_string(venv_root.join("pyvenv.cfg")).ok()?;
    Some(parse_pyvenv_cfg_content(&content))
}

/// True when this interpreter is the uv trampoline under `Scripts/python.exe` / `bin/python`.
pub fn is_uv_trampoline_shim(python_exe: &Path, venv_root: &Path) -> bool {
    let shim = venv_scripts_python(venv_root);
    shim.is_some_and(|shim| {
        dunce::simplified(python_exe) == dunce::simplified(&shim)
            || python_exe
                .canonicalize()
                .ok()
                .zip(shim.canonicalize().ok())
                .map(|(a, b)| a == b)
                .unwrap_or(false)
    })
}

/// Build the same Python command used for Connect Chrome / Embedded preflight checks.
pub fn build_import_check_command(venv: &ResolvedVenv) -> Command {
    let mut cmd = Command::new(&venv.python_exe);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    apply_venv_to_command(&mut cmd, venv);
    cmd
}

/// Run `python -c <snippet>` using the sidecar preflight command configuration.
pub fn run_import_preflight(
    venv: &ResolvedVenv,
    import_snippet: &str,
) -> std::io::Result<std::process::Output> {
    build_import_check_command(venv)
        .args(["-c", import_snippet])
        .output()
}

/// Configure the child process environment for Python.
pub fn apply_venv_to_command(cmd: &mut Command, venv: &ResolvedVenv) {
    if let Some(site) = &venv.site_packages {
        cmd.env("PYTHONPATH", site);
        cmd.env("VIRTUAL_ENV", &venv.root);
    }
}

pub fn check_failure_detail(check: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&check.stderr);
    let stdout = String::from_utf8_lossy(&check.stdout);
    teshi_core::venv::check_failure_detail(&stderr, &stdout, check.status.code())
}

pub fn import_check_failed_message(check: &std::process::Output, packages: &str) -> String {
    let detail = check_failure_detail(check);
    teshi_core::venv::import_check_failed_message(&detail, packages)
}

pub fn venv_python_failure_hint(detail: &str, pip_hint: &str, venv_root: &Path) -> String {
    teshi_core::venv::venv_python_failure_hint(
        detail,
        pip_hint,
        &venv_root.display().to_string(),
        is_uv_managed_venv(venv_root),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parse_pyvenv_cfg_reads_key_value_pairs() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyvenv.cfg"),
            "home = C:\\python\nuv = 0.5.0\nexecutable = C:\\python\\python.exe\n",
        )
        .unwrap();
        let cfg = parse_pyvenv_cfg(dir.path()).unwrap();
        assert_eq!(cfg.get("uv").map(String::as_str), Some("0.5.0"));
    }

    fn home_python_name() -> &'static str {
        if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        }
    }

    #[test]
    fn resolve_uv_venv_uses_home_python_and_site_packages() {
        let dir = tempdir().unwrap();
        let scripts = dir
            .path()
            .join(if cfg!(windows) { "Scripts" } else { "bin" });
        fs::create_dir_all(&scripts).unwrap();
        let shim = scripts.join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        fs::write(&shim, "").unwrap();

        let site = dir.path().join("Lib").join("site-packages");
        fs::create_dir_all(&site).unwrap();

        let base = tempdir().unwrap();
        let real_python = base.path().join(home_python_name());
        fs::write(&real_python, "").unwrap();

        fs::write(
            dir.path().join("pyvenv.cfg"),
            format!("home = {}\nuv = 0.5.0\n", base.path().display()),
        )
        .unwrap();

        let resolved = resolve_venv_at(dir.path()).unwrap();
        assert!(resolved.uv_managed);
        assert_eq!(resolved.python_exe, real_python);
        assert!(!is_uv_trampoline_shim(&resolved.python_exe, dir.path()));
        assert_eq!(resolved.site_packages.as_deref(), Some(site.as_path()));
    }

    #[test]
    fn uv_managed_venv_without_runnable_home_returns_none() {
        let dir = tempdir().unwrap();
        let scripts = dir.path().join("Scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(scripts.join("python.exe"), "").unwrap();
        fs::write(
            dir.path().join("pyvenv.cfg"),
            "home = C:\\missing\\python-root\nuv = 0.5.0\n",
        )
        .unwrap();
        assert!(resolve_venv_at(dir.path()).is_none());
    }

    #[test]
    fn standard_venv_uses_shim_without_site_packages() {
        let dir = tempdir().unwrap();
        let scripts = dir
            .path()
            .join(if cfg!(windows) { "Scripts" } else { "bin" });
        fs::create_dir_all(&scripts).unwrap();
        let shim = scripts.join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        fs::write(&shim, "").unwrap();
        fs::write(
            dir.path().join("pyvenv.cfg"),
            "home = C:\\python\ninclude-system-site-packages = false\n",
        )
        .unwrap();

        let resolved = resolve_venv_at(dir.path()).unwrap();
        assert!(!resolved.uv_managed);
        assert_eq!(resolved.python_exe, shim);
        assert!(resolved.site_packages.is_none());
    }

    #[test]
    fn is_uv_trampoline_failure_detects_uv_errors() {
        assert!(is_uv_trampoline_failure(
            "error: uv trampoline failed to spawn Python child process"
        ));
    }

    fn output_with_stderr(stderr: &str) -> std::process::Output {
        #[cfg(windows)]
        let status = Command::new("cmd")
            .args(["/C", "exit", "1"])
            .status()
            .expect("cmd");
        #[cfg(not(windows))]
        let status = Command::new("sh")
            .args(["-c", "exit 1"])
            .status()
            .expect("sh");
        std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn import_check_message_distinguishes_trampoline_from_missing_package() {
        let trampoline = output_with_stderr("uv trampoline failed");
        assert!(import_check_failed_message(&trampoline, "websockets").contains("Could not run"));

        let missing = output_with_stderr("ModuleNotFoundError: No module named 'websockets'");
        assert!(import_check_failed_message(&missing, "websockets").contains("not installed"));
    }

    /// Regression: `open_project` stores a canonicalized root; preflight must still resolve uv base Python.
    #[test]
    #[cfg(windows)]
    fn feedback_uv_venv_preflight_matches_sidecar_spawn() {
        const FEEDBACK: &str = r"D:\Dev\CloudFlareWorker\feedback";
        let root = Path::new(FEEDBACK);
        if !root.is_dir() {
            eprintln!("skip feedback_uv_venv_preflight_matches_sidecar_spawn: {FEEDBACK} missing");
            return;
        }

        for project_root in [
            root.to_path_buf(),
            root.canonicalize().expect("canonicalize"),
        ] {
            let venv = resolve_project_venv(&project_root).expect("resolve venv");
            assert!(venv.uv_managed, "project_root={}", project_root.display());
            assert!(
                !is_uv_trampoline_shim(&venv.python_exe, &venv.root),
                "must not use uv trampoline shim, got {}",
                venv.python_exe.display()
            );
            assert!(
                venv.site_packages.is_some(),
                "uv venv should set PYTHONPATH to site-packages"
            );

            let check =
                run_import_preflight(&venv, "import websockets").expect("spawn preflight python");
            let detail = check_failure_detail(&check);
            assert!(
                check.status.success(),
                "preflight failed for {}: {detail}",
                project_root.display()
            );
            assert!(
                !is_uv_trampoline_failure(&detail),
                "stderr must not be uv trampoline: {detail}"
            );
        }
    }
}
