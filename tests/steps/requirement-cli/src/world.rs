//! Isolated requirement-store world for CLI E2E steps.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use teshi_core::authoring::{
    RequirementDocumentMeta, RequirementLink, RequirementStoreId, ReviewState, TestPoint,
    TestPointsFile, TextRange, create_requirement_link, testpoints_file,
};
use teshi_engine::{
    compute_document_revision, initialize_requirement_store, save_requirement_document_index,
    save_requirement_markdown, save_test_points,
};

/// Canonical login document body used by the sample fixture.
const LOGIN_BODY: &str = "# Login\n\nUsers can sign in.\n";
/// Canonical mobile-login document body used by the sample fixture.
const MOBILE_BODY: &str = "# Login\n\nMobile users can sign in.\n";
/// Canonical checkout document body used by the sample fixture.
pub const CHECKOUT_BODY: &str = "# Checkout\n\nShoppers can pay.\n";
/// Sentence the sample test point anchors inside `auth/login.md`.
pub const LOGIN_LINK_NEEDLE: &str = "Users can sign in.";
/// Sentence the sample test point anchors inside `shop/checkout.md`.
pub const CHECKOUT_LINK_NEEDLE: &str = "Shoppers can pay.";
/// Sentence the sample test point anchors inside `mobile/login.md`.
pub const MOBILE_LINK_NEEDLE: &str = "Mobile users can sign in.";

/// Anchor sentence the sample fixture uses inside the seeded `document_id`.
pub fn link_needle(document_id: &str) -> Result<&'static str> {
    match document_id {
        "doc-12" => Ok(LOGIN_LINK_NEEDLE),
        "doc-37" => Ok(MOBILE_LINK_NEEDLE),
        "doc-9" => Ok(CHECKOUT_LINK_NEEDLE),
        other => bail!("no sample anchor sentence for {other}"),
    }
}

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

/// Per-scenario store, project, and last command result.
pub struct World {
    teshi: PathBuf,
    store: tempfile::TempDir,
    project: tempfile::TempDir,
    app_data: tempfile::TempDir,
    last: Option<CommandResult>,
    tab: Option<teshi_tui::RequirementsTabSnapshot>,
    revisions: HashMap<String, String>,
    store_id: Option<RequirementStoreId>,
}

impl World {
    /// Creates an empty isolated store and project. Call [`Self::seed_sample`]
    /// and [`Self::seed_test_point`] from Background.
    pub fn new(teshi: PathBuf) -> Result<Self> {
        Ok(Self {
            teshi,
            store: tempfile::tempdir().context("create requirement store temp dir")?,
            project: tempfile::tempdir().context("create project temp dir")?,
            app_data: tempfile::tempdir().context("create TESHI_APP_DATA_DIR temp dir")?,
            last: None,
            tab: None,
            revisions: HashMap::new(),
            store_id: None,
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
        self.store_id = index.store_id.clone();
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

    /// Canonical body captured when the sample store was seeded.
    pub fn seeded_body(&self, document_id: &str) -> Result<&'static str> {
        match document_id {
            "doc-12" => Ok(LOGIN_BODY),
            "doc-37" => Ok(MOBILE_BODY),
            "doc-9" => Ok(CHECKOUT_BODY),
            other => bail!("no seeded body for {other}"),
        }
    }

    /// Ensures the isolated project has `test_point_id`, linked to each document.
    ///
    /// Re-seeding the same id replaces its links, so a Background step and a
    /// scenario-local step can both describe the same test point.
    pub fn seed_test_point(&mut self, test_point_id: &str, documents: &[&str]) -> Result<()> {
        let links = documents
            .iter()
            .map(|document| self.build_link(document))
            .collect::<Result<Vec<RequirementLink>>>()?;
        let mut file = self.load_project_test_points()?;
        file.test_points
            .retain(|test_point| test_point.id != test_point_id);
        file.test_points.push(TestPoint {
            id: test_point_id.to_string(),
            title: test_point_id.to_uppercase(),
            objective: format!("Verify {test_point_id}"),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: teshi_core::authoring::HierarchyPath::new(vec!["Auth".into()]),
            review_state: ReviewState::Proposed,
            requirement_links: links,
            scenario_refs: Vec::new(),
        });
        self.save_project_test_points(&file)
    }

    /// Appends one requirement link to an already seeded test point.
    pub fn add_test_point_link(&mut self, test_point_id: &str, document: &str) -> Result<()> {
        let link = self.build_link(document)?;
        let mut file = self.load_project_test_points()?;
        let test_point = file
            .test_points
            .iter_mut()
            .find(|test_point| test_point.id == test_point_id)
            .with_context(|| format!("test point {test_point_id} is not seeded"))?;
        test_point.requirement_links.push(link);
        self.save_project_test_points(&file)
    }

    /// Overwrites a seeded test point's review state, standing in for human review.
    pub fn set_test_point_review_state(
        &mut self,
        test_point_id: &str,
        state: ReviewState,
    ) -> Result<()> {
        let mut file = self.load_project_test_points()?;
        let test_point = file
            .test_points
            .iter_mut()
            .find(|test_point| test_point.id == test_point_id)
            .with_context(|| format!("test point {test_point_id} is not seeded"))?;
        test_point.review_state = state;
        self.save_project_test_points(&file)
    }

    fn build_link(&self, document_id: &str) -> Result<RequirementLink> {
        let store_id = self
            .store_id
            .clone()
            .context("seed the sample requirement store before linking test points")?;
        let revision = self.seeded_revision(document_id)?.to_string();
        let body = self.seeded_body(document_id)?;
        let needle = link_needle(document_id)?;
        let byte = body
            .find(needle)
            .with_context(|| format!("anchor sentence {needle:?} is absent from {document_id}"))?;
        let start = body[..byte].chars().count() as u32;
        let end = start + needle.chars().count() as u32;
        create_requirement_link(
            store_id,
            document_id,
            revision,
            body,
            TextRange::new(start, end),
        )
        .with_context(|| format!("cannot anchor {needle:?} in {document_id}"))
    }

    fn load_project_test_points(&self) -> Result<TestPointsFile> {
        let path = testpoints_file(self.project.path());
        if !path.is_file() {
            return Ok(TestPointsFile::default());
        }
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    fn save_project_test_points(&self, file: &TestPointsFile) -> Result<()> {
        save_test_points(self.project.path(), file)
    }

    /// Runs `teshi --requirements-root <store> testpoints ... --project <project>`.
    pub fn run_testpoints(&mut self, args: &[&str]) -> Result<&CommandResult> {
        let result = self.testpoints_command(args)?;
        self.last = Some(result);
        self.last()
    }

    /// Runs a testpoints command without recording it as the scenario When result.
    pub fn inspect_testpoints(&self, args: &[&str]) -> Result<CommandResult> {
        let result = self.testpoints_command(args)?;
        if result.status != 0 {
            bail!(
                "testpoints inspect failed (exit {}): {}{}",
                result.status,
                result.stderr,
                result.stdout
            );
        }
        Ok(result)
    }

    fn testpoints_command(&self, args: &[&str]) -> Result<CommandResult> {
        let output = Command::new(&self.teshi)
            .arg("--requirements-root")
            .arg(self.store.path())
            .arg("testpoints")
            .args(args)
            .arg("--project")
            .arg(self.project.path())
            .env("TESHI_APP_DATA_DIR", self.app_data.path())
            .env_remove("TESHI_REQUIREMENTS_DIR")
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("spawn {}", self.teshi.display()))?;
        Ok(CommandResult {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Isolated store root passed as `--requirements-root`.
    pub fn store_path(&self) -> &Path {
        self.store.path()
    }

    /// Requirements-tab snapshot from opening teshi with no PATH.
    pub fn tab(&self) -> Result<&teshi_tui::RequirementsTabSnapshot> {
        self.tab
            .as_ref()
            .context("teshi has not been opened without a project path yet")
    }

    /// Constructs TUI state as `teshi` with no PATH against this store.
    pub fn open_tui_without_project(&mut self) -> Result<&teshi_tui::RequirementsTabSnapshot> {
        let _guard = AppDataEnvGuard::apply(self.app_data.path());
        let snapshot = teshi_tui::requirements_tab_without_project(self.store.path())?;
        self.tab = Some(snapshot);
        self.tab()
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

/// Restores `TESHI_APP_DATA_DIR` when dropped so TUI construction stays isolated.
struct AppDataEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl AppDataEnvGuard {
    fn apply(path: &Path) -> Self {
        let previous = std::env::var_os("TESHI_APP_DATA_DIR");
        unsafe {
            std::env::set_var("TESHI_APP_DATA_DIR", path);
            std::env::remove_var("TESHI_REQUIREMENTS_DIR");
        }
        Self { previous }
    }
}

impl Drop for AppDataEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("TESHI_APP_DATA_DIR", value),
                None => std::env::remove_var("TESHI_APP_DATA_DIR"),
            }
        }
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
