//! Integration tests for project venv resolution and Connect Chrome import preflight.
//!
//! Set `TESHI_TEST_PROJECT` to a directory with `.venv` (uv or stdlib) to run locally.

use std::path::PathBuf;

use teshi_runtime::python_env::{
    check_failure_detail, is_uv_trampoline_failure, is_uv_trampoline_shim, resolve_project_venv,
    run_import_preflight,
};

fn feedback_project() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("TESHI_TEST_PROJECT") {
        let path = PathBuf::from(path);
        if path.is_dir() {
            return Some(path);
        }
    }
    let fixed = PathBuf::from(r"D:\Dev\CloudFlareWorker\feedback");
    fixed.is_dir().then_some(fixed)
}

#[test]
#[cfg(windows)]
fn chrome_import_preflight_never_uses_uv_trampoline_shim() {
    let Some(root) = feedback_project() else {
        eprintln!("skip: set TESHI_TEST_PROJECT or create D:\\Dev\\CloudFlareWorker\\feedback");
        return;
    };

    let project_root = root.canonicalize().expect("canonicalize project");
    let venv = resolve_project_venv(&project_root).expect("project venv");
    assert!(
        !is_uv_trampoline_shim(&venv.python_exe, &venv.root),
        "python_exe must not be .venv/Scripts/python.exe (uv trampoline), got {}",
        venv.python_exe.display()
    );

    let output = run_import_preflight(&venv, "import websockets").expect("spawn python");
    let detail = check_failure_detail(&output);
    assert!(
        output.status.success(),
        "import websockets failed: {detail}"
    );
    assert!(
        !is_uv_trampoline_failure(&detail),
        "unexpected trampoline error: {detail}"
    );
}

#[test]
#[cfg(windows)]
fn chrome_import_preflight_non_canonical_project_root() {
    let Some(root) = feedback_project() else {
        return;
    };
    let venv = resolve_project_venv(&root).expect("venv");
    let output = run_import_preflight(&venv, "import websockets").expect("spawn");
    assert!(output.status.success(), "{}", check_failure_detail(&output));
}

#[test]
fn resolve_returns_none_when_uv_home_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let scripts = dir.path().join("Scripts");
    std::fs::create_dir_all(&scripts).expect("scripts");
    std::fs::write(scripts.join("python.exe"), "").expect("shim");
    std::fs::write(
        dir.path().join("pyvenv.cfg"),
        "home = C:\\no-such-uv-python-root\nuv = 0.5.0\n",
    )
    .expect("pyvenv.cfg");

    assert!(resolve_project_venv(dir.path()).is_none());
}
