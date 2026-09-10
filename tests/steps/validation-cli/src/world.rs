//! Isolated black-box world for validation CLI scenarios.

use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Result captured from one target Teshi child process.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandResult {
    pub fn combined(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_str(self.stdout.trim()).context("parse target JSON output")
    }
}

/// A fresh project, app-data directory, and command history for one scenario.
pub struct World {
    teshi: PathBuf,
    project: tempfile::TempDir,
    app_data: tempfile::TempDir,
    last: Option<CommandResult>,
    previous: Option<CommandResult>,
    active_before: Option<String>,
    runner_marker: PathBuf,
    daemon_started: bool,
}

impl World {
    pub fn new(teshi: PathBuf) -> Result<Self> {
        Ok(Self {
            teshi,
            project: tempfile::tempdir().context("create validation E2E project")?,
            app_data: tempfile::tempdir().context("create validation E2E app-data")?,
            last: None,
            previous: None,
            active_before: None,
            runner_marker: PathBuf::new(),
            daemon_started: false,
        })
    }

    pub fn malformed_path(&self) -> &'static str {
        "features/malformed.feature"
    }

    pub fn valid_path(&self) -> &'static str {
        "features/valid.feature"
    }

    pub fn valid_zh_path(&self) -> &'static str {
        "features/valid-zh.feature"
    }

    pub fn invalid_path(&self) -> &'static str {
        "features/invalid.feature"
    }

    pub fn unrecognized_path(&self) -> &'static str {
        "features/unrecognized.feature"
    }

    pub fn warning_path(&self) -> &'static str {
        "features/warning.feature"
    }

    pub fn repeated_given_path(&self) -> &'static str {
        "features/repeated-given.feature"
    }

    pub fn write_feature(&self, relative: &str, source: &str) -> Result<()> {
        let path = self.project.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create fixture directory")?;
        }
        fs::write(path, source).context("write Feature fixture")?;
        Ok(())
    }

    pub fn seed_malformed_zh_feature(&self) -> Result<()> {
        self.write_feature(
            self.malformed_path(),
            "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n",
        )
    }

    pub fn seed_valid_feature(&self) -> Result<()> {
        self.write_feature(
            self.valid_path(),
            "Feature: Validation fixtures\n  Scenario: Prose and attachments\n    This description is legal before the first step.\n    Given the validation data exists\n      | name | value |\n      | user | teshi |\n    When the validation action runs\n      \"\"\"\n      payload\n      \"\"\"\n    Then the validation result is visible\n",
        )
    }

    pub fn seed_valid_zh_feature(&self) -> Result<()> {
        self.write_feature(
            self.valid_zh_path(),
            "# language: zh-CN\n功能: 登录\n  场景: 成功\n    假如 用户已经登录\n    当 用户查看首页\n    那么 首页可见\n",
        )
    }

    pub fn seed_invalid_feature(&self) -> Result<()> {
        self.write_feature(
            self.invalid_path(),
            "Feature: Invalid\n  Scenario: Missing separator\n    Givenready\n",
        )
    }

    pub fn seed_unrecognized_feature(&self) -> Result<()> {
        self.write_feature(
            self.unrecognized_path(),
            "Feature: Unrecognized\n  Scenario: Broken\n    Given the precondition is ready\n    not a step\n",
        )
    }

    pub fn seed_warning_feature(&self) -> Result<()> {
        self.write_feature(
            self.warning_path(),
            "Feature: Warning\n  Scenario: Starts with When\n    When the operation runs\n    Then the operation succeeds\n",
        )
    }

    pub fn seed_repeated_given_feature(&self) -> Result<()> {
        self.write_feature(
            self.repeated_given_path(),
            "Feature: Repeated Given\n  Scenario: Preconditions only\n    Given the account exists\n    And the account is active\n",
        )
    }

    pub fn seed_selected_directory_features(&self) -> Result<()> {
        self.write_feature(
            "selected/ok.feature",
            "Feature: Selected\n  Scenario: Works\n    Given the page is open\n    When the action runs\n    Then the result is visible\n",
        )?;
        self.write_feature(
            "sibling/broken.feature",
            "Feature: Sibling\n  Scenario: Broken\n    Givenready\n",
        )
    }

    pub fn run(&mut self, args: &[String]) -> Result<&CommandResult> {
        self.run_with_marker(args, false)
    }

    pub fn run_str(&mut self, args: &[&str]) -> Result<&CommandResult> {
        let args = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        self.run(&args)
    }

    pub fn run_with_marker(&mut self, args: &[String], marker: bool) -> Result<&CommandResult> {
        let mut command = Command::new(&self.teshi);
        command
            .current_dir(self.project.path())
            .args(args)
            .env("TESHI_APP_DATA_DIR", self.app_data.path())
            .env_remove("TESHI_REQUIREMENTS_DIR")
            .env_remove("TESHI_ENGINE_CMD");

        if marker {
            let marker_path = self.project.path().join("nested-runner-started");
            let _ = fs::remove_file(&marker_path);
            command.env("TESHI_VALIDATION_E2E_MARKER", &marker_path);
            self.runner_marker = marker_path;
        }

        let output = command
            .output()
            .with_context(|| format!("spawn target Teshi at {}", self.teshi.display()))?;
        self.previous = self.last.take();
        self.last = Some(CommandResult {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
        self.last()
    }

    pub fn last(&self) -> Result<&CommandResult> {
        self.last.as_ref().context("no target command has run")
    }

    pub fn previous(&self) -> Result<&CommandResult> {
        self.previous
            .as_ref()
            .context("no previous target command has run")
    }

    pub fn select_valid_step_and_snapshot(&mut self) -> Result<()> {
        self.seed_valid_feature()?;
        let result = self.run_str(&[
            "steps",
            "select",
            "--feature",
            self.valid_path(),
            "--line",
            "4",
        ])?;
        if result.status != 0 {
            bail!(
                "selecting valid active step failed ({}): {}{}",
                result.status,
                result.stdout,
                result.stderr
            );
        }
        self.active_before = Some(
            fs::read_to_string(self.project.path().join(".teshi/active-step.json"))
                .context("read active-step snapshot")?,
        );
        Ok(())
    }

    pub fn start_daemon(&mut self) -> Result<()> {
        fs::create_dir_all(self.project.path().join(".teshi"))
            .context("create daemon project directory")?;
        let result = self.run_str(&["daemon", "start"])?;
        if result.status != 0 {
            bail!(
                "starting daemon failed ({}): {}{}",
                result.status,
                result.stdout,
                result.stderr
            );
        }
        self.daemon_started = true;

        let manifest_path = self.project.path().join(".teshi/daemon.json");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(manifest) = fs::read_to_string(&manifest_path)
                && let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&manifest)
                && let Some(port) = manifest["port"].as_u64()
                && let Ok(address) = format!("127.0.0.1:{port}").parse()
                && TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_ok()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "daemon did not become ready; manifest: {}",
                    fs::read_to_string(&manifest_path).unwrap_or_default()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn run_selected_directory(&mut self) -> Result<()> {
        let mut args = vec!["run".to_string(), "selected".to_string()];
        if cfg!(windows) {
            args.extend([
                "--runner-cmd".to_string(),
                "cmd.exe".to_string(),
                "--runner-arg".to_string(),
                "/C".to_string(),
                "--runner-arg".to_string(),
                "exit".to_string(),
                "--runner-arg".to_string(),
                "0".to_string(),
            ]);
        } else {
            args.extend([
                "--runner-cmd".to_string(),
                "sh".to_string(),
                "--runner-arg".to_string(),
                "-c".to_string(),
                "--runner-arg".to_string(),
                "exit 0".to_string(),
            ]);
        }
        self.run(&args).map(|_| ())
    }

    pub fn active_step_is_unchanged(&self) -> Result<()> {
        let expected = self
            .active_before
            .as_deref()
            .context("active-step snapshot was not captured")?;
        let current = fs::read_to_string(self.project.path().join(".teshi/active-step.json"))
            .context("read current active-step state")?;
        if current != expected {
            bail!(
                "active-step state changed unexpectedly\nexpected:\n{expected}\nactual:\n{current}"
            );
        }
        Ok(())
    }

    pub fn runner_marker_is_absent(&self) -> bool {
        !self.runner_marker.as_os_str().is_empty() && !self.runner_marker.exists()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if !self.daemon_started {
            return;
        }
        let _ = Command::new(&self.teshi)
            .current_dir(self.project.path())
            .args(["daemon", "stop"])
            .env("TESHI_APP_DATA_DIR", self.app_data.path())
            .env_remove("TESHI_REQUIREMENTS_DIR")
            .env_remove("TESHI_ENGINE_CMD")
            .output();
    }
}

/// Resolves the target binary and never falls back to PATH discovery.
pub fn locate_teshi_bin() -> Result<PathBuf> {
    if let Ok(bin) = std::env::var("TESHI_BIN") {
        let path = PathBuf::from(bin);
        if path.exists() {
            return Ok(path);
        }
        bail!("TESHI_BIN does not exist: {}", path.display());
    }
    bail!("TESHI_BIN is required for validation E2E; refusing PATH discovery")
}
