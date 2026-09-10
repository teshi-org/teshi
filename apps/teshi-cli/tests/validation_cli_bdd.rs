//! Black-box validation acceptance scenarios via `teshi run` and an independent runner.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn teshi_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_teshi")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let fallback =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/teshi.exe");
            if fallback.exists() {
                fallback
            } else {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/teshi")
            }
        })
}

fn runner_binary(teshi: &Path) -> PathBuf {
    teshi.with_file_name(if cfg!(windows) {
        "teshi-validation-cli-runner.exe"
    } else {
        "teshi-validation-cli-runner"
    })
}

fn ensure_runner(teshi: &Path) -> PathBuf {
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "-p",
            "teshi-validation-cli-runner",
            "--locked",
            "--quiet",
        ])
        .status()
        .expect("spawn cargo to build teshi-validation-cli-runner");
    assert!(
        status.success(),
        "failed to build teshi-validation-cli-runner"
    );
    let runner = runner_binary(teshi);
    assert!(
        runner.exists(),
        "validation runner binary missing at {}",
        runner.display()
    );
    runner
}

fn features_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features")
}

fn is_validation_feature(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|content| {
        content.lines().any(|line| {
            line.split_whitespace()
                .any(|token| token == "@validation-e2e")
        })
    })
}

fn copy_validation_features(locale: &str, destination: &Path) {
    let source = features_dir().join(locale);
    let target = destination.join(locale);
    fs::create_dir_all(&target).expect("create locale directory");
    for entry in fs::read_dir(source).expect("read feature locale") {
        let entry = entry.expect("read feature entry");
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".feature") && is_validation_feature(&entry.path()) {
            fs::copy(entry.path(), target.join(name)).expect("copy validation Feature");
        }
    }
}

fn run_locale(locale: &str) {
    let teshi = teshi_binary();
    assert!(
        teshi.exists(),
        "teshi binary missing at {}",
        teshi.display()
    );
    let runner = ensure_runner(&teshi);
    let project = tempfile::tempdir().expect("create validation E2E project");
    let app_data = tempfile::tempdir().expect("create validation E2E app-data");
    copy_validation_features(locale, project.path());

    let output = Command::new(&teshi)
        .current_dir(project.path())
        .env("TESHI_BIN", &teshi)
        .env("TESHI_APP_DATA_DIR", app_data.path())
        .env_remove("TESHI_ENGINE_CMD")
        .env_remove("TESHI_REQUIREMENTS_DIR")
        .args([
            "run",
            "--runner-cmd",
            runner.to_str().expect("runner path utf-8"),
            locale,
        ])
        .output()
        .expect("spawn teshi validation E2E");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        output.status.success(),
        "{locale}: teshi run exited unsuccessfully.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("end_run passed=17 failed=0 skipped=0"),
        "{locale}: validation E2E failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !combined.contains("case_failed"),
        "{locale}: a validation scenario failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn validation_en_us_features_pass_against_built_teshi() {
    run_locale("en-US");
}

#[test]
fn validation_zh_cn_features_pass_against_built_teshi() {
    run_locale("zh-CN");
}
