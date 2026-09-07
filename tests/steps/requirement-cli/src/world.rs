//! Isolated requirement-store world for CLI E2E steps.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use teshi_core::authoring::RequirementDocumentMeta;
use teshi_engine::{
    compute_document_revision, initialize_requirement_store, save_requirement_document_index,
    save_requirement_markdown,
};

/// Canonical login document body used by the sample fixture.
const LOGIN_BODY: &str = "# Login\n\nUsers can sign in.\n";
/// Canonical mobile-login document body used by the sample fixture.
const MOBILE_BODY: &str = "# Login\n\nMobile users can sign in.\n";
/// Canonical checkout document body used by the sample fixture.
pub const CHECKOUT_BODY: &str = "# Checkout\n\nShoppers can pay.\n";

/// Captured result of one `teshi requirements` invocation.
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// Process exit status code, or `-1` when the OS status has no code.
    pub status: i32,
    /// UTF-8 stdout.
    pub stdout: String,
    /// UTF-8 stderr.
    pub stderr: String,
}

/// Per-scenario store and last command result.
pub struct World {
    teshi: PathBuf,
    store: tempfile::TempDir,
    app_data: tempfile::TempDir,
    last: Option<CommandResult>,
    revisions: HashMap<String, String>,
}

impl World {
    /// Creates an empty isolated store. Call [`Self::seed_sample`] from Background.
    pub fn new(teshi: PathBuf) -> Result<Self> {
        Ok(Self {
            teshi,
            store: tempfile::tempdir().context("create requirement store temp dir")?,
            app_data: tempfile::tempdir().context("create TESHI_APP_DATA_DIR temp dir")?,
            last: None,
            revisions: HashMap::new(),
        })
    }

    /// Seeds login, mobile-login, and checkout documents matching the feature description.
    pub fn seed_sample(&mut self) -> Result<()> {
        let root = self.store.path();
        let mut index = initialize_requirement_store(root).context("initialize store")?;
        index.documents = vec![
            RequirementDocumentMeta::new(
                "doc-12",
                "auth/login.md",
                "Login",
                compute_document_revision(LOGIN_BODY),
            ),
            RequirementDocumentMeta::new(
                "doc-37",
                "mobile/login.md",
                "Login",
                compute_document_revision(MOBILE_BODY),
            ),
            RequirementDocumentMeta::new(
                "doc-9",
                "shop/checkout.md",
                "Checkout",
                compute_document_revision(CHECKOUT_BODY),
            ),
        ];
        save_requirement_markdown(root, &mut index, "auth/login.md", LOGIN_BODY)?;
        save_requirement_markdown(root, &mut index, "mobile/login.md", MOBILE_BODY)?;
        save_requirement_markdown(root, &mut index, "shop/checkout.md", CHECKOUT_BODY)?;
        index.documents[0].iteration = Some("Sprint 12".into());
        save_requirement_document_index(root, &index)?;
        self.revisions = index
            .documents
            .iter()
            .map(|doc| (doc.id.clone(), doc.revision.as_str().to_string()))
            .collect();
        Ok(())
    }

    /// Last `teshi requirements` invocation, if any.
    pub fn last(&self) -> Result<&CommandResult> {
        self.last.as_ref().context("no teshi command has run yet")
    }

    /// Revision captured when the sample store was seeded.
    pub fn seeded_revision(&self, document_id: &str) -> Result<&str> {
        self.revisions
            .get(document_id)
            .map(String::as_str)
            .with_context(|| format!("no seeded revision for {document_id}"))
    }

    /// Isolated store root passed as `--requirements-root`.
    pub fn store_path(&self) -> &Path {
        self.store.path()
    }

    /// Runs `teshi --requirements-root <store> requirements ...` with stdin closed.
    pub fn run(&mut self, args: &[&str]) -> Result<&CommandResult> {
        self.run_with_stdin(args, None)
    }

    /// Runs a requirements command, optionally piping `stdin_body`.
    pub fn run_with_stdin(
        &mut self,
        args: &[&str],
        stdin_body: Option<&str>,
    ) -> Result<&CommandResult> {
        let store = self.store.path();
        let mut cmd = Command::new(&self.teshi);
        cmd.arg("--requirements-root")
            .arg(store)
            .arg("requirements")
            .args(args)
            .env("TESHI_APP_DATA_DIR", self.app_data.path())
            .env_remove("TESHI_REQUIREMENTS_DIR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if stdin_body.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", self.teshi.display()))?;
        if let Some(body) = stdin_body
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin.write_all(body.as_bytes()).context("write stdin")?;
        }
        let output = child
            .wait_with_output()
            .context("wait for teshi requirements")?;
        self.last = Some(CommandResult {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
        self.last()
    }

    /// Runs a requirements command without recording it as the scenario When result.
    pub fn inspect(&self, args: &[&str]) -> Result<CommandResult> {
        let output = Command::new(&self.teshi)
            .arg("--requirements-root")
            .arg(self.store.path())
            .arg("requirements")
            .args(args)
            .env("TESHI_APP_DATA_DIR", self.app_data.path())
            .env_remove("TESHI_REQUIREMENTS_DIR")
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("inspect via {}", self.teshi.display()))?;
        if !output.status.success() {
            bail!(
                "inspect command failed: {}{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
        }
        Ok(CommandResult {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Resolves the Teshi binary under test.
pub fn locate_teshi_bin() -> Result<PathBuf> {
    if let Ok(bin) = std::env::var("TESHI_BIN") {
        let path = PathBuf::from(bin);
        if path.exists() {
            return Ok(path);
        }
        bail!("TESHI_BIN does not exist: {}", path.display());
    }
    let exe = std::env::current_exe().context("current_exe")?;
    let dir = exe.parent().context("runner exe has no parent")?;
    for name in ["teshi.exe", "teshi"] {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("teshi binary not found; set TESHI_BIN");
}
