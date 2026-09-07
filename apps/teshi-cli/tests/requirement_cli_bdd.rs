//! End-to-end requirement CLI scenarios via `teshi run` and the NDJSON runner.

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
        "teshi-requirement-cli-runner.exe"
    } else {
        "teshi-requirement-cli-runner"
    })
}

fn ensure_runner(teshi: &Path) -> PathBuf {
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "teshi-requirement-cli-runner", "--quiet"])
        .status()
        .expect("spawn cargo to build teshi-requirement-cli-runner");
    assert!(
        status.success(),
        "failed to build teshi-requirement-cli-runner"
    );
    let runner = runner_binary(teshi);
    assert!(
        runner.exists(),
        "runner binary missing at {}",
        runner.display()
    );
    runner
}

fn features_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features")
}

/// True when the feature is tagged `@cli`, independent of English or Chinese filenames.
fn is_cli_feature(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|content| {
        content
            .lines()
            .any(|line| line.split_whitespace().any(|token| token == "@cli"))
    })
}

fn copy_cli_features(locale: &str, dst: &Path) {
    let src = features_dir().join(locale);
    let dest = dst.join(locale);
    fs::create_dir_all(&dest).unwrap();
    for entry in fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".feature") && is_cli_feature(&entry.path()) {
            fs::copy(entry.path(), dest.join(name)).unwrap();
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

    let tmp = tempfile::tempdir().expect("temp project");
    copy_cli_features(locale, tmp.path());

    let output = Command::new(&teshi)
        .current_dir(tmp.path())
        .env("TESHI_BIN", &teshi)
        .env("TESHI_APP_DATA_DIR", tmp.path().join("app-data"))
        .env_remove("TESHI_ENGINE_CMD")
        .args([
            "run",
            "--runner-cmd",
            runner.to_str().expect("runner path utf-8"),
            locale,
        ])
        .output()
        .expect("spawn teshi run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("end_run"),
        "{locale}: teshi run did not emit end_run.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("end_run passed=16 failed=0 skipped=0"),
        "{locale}: requirement CLI E2E failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !combined.contains("case_failed"),
        "{locale}: a scenario failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn requirement_cli_en_us_features_pass_through_teshi_run() {
    run_locale("en-US");
}

#[test]
fn requirement_cli_zh_cn_features_pass_through_teshi_run() {
    run_locale("zh-CN");
}
