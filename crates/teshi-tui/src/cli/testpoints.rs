//! `teshi testpoints` commands for project-local test points.
//!
//! Test points persist at `<project>/testpoints/testpoints.json` and link to
//! requirement documents in the user-level store. Every mutation rewrites that
//! file whole, so it is atomic but not lock-protected against a concurrent TUI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use teshi_core::authoring::{
    RequirementLink, RequirementStoreId, ResolutionState, ReviewState, TestPoint, TestPointsFile,
    TextRange, create_requirement_link,
};
use teshi_engine::{load_authoring_artifacts, requirements_data_dir, save_test_points};

use super::{TestpointArgs, TestpointLinkCommand, TestpointsCommand};

/// Failure from the test-point control plane.
#[derive(Debug)]
pub(crate) enum TestpointError {
    /// No test point carries the requested id.
    UnknownTestPoint(String),
    /// The current requirement store has no document with this id.
    UnknownDocument(String),
    /// The range is malformed, empty, or outside the target document.
    InvalidRange(String),
    /// No link matched the supplied selector.
    LinkNotFound(String),
    /// More than one link matched the supplied selector.
    AmbiguousLink(String),
    /// The requirement store has no identity, so no anchor can be created.
    StoreUninitialized,
    /// I/O or parse failure.
    Io(String),
}

impl TestpointError {
    fn io(err: impl std::fmt::Display) -> Self {
        Self::Io(err.to_string())
    }

    /// Stable machine-readable failure code for CLI/JSON adapters.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::UnknownTestPoint(_) => "unknown_test_point",
            Self::UnknownDocument(_) => "unknown_document",
            Self::InvalidRange(_) => "invalid_range",
            Self::LinkNotFound(_) => "link_not_found",
            Self::AmbiguousLink(_) => "ambiguous_link",
            Self::StoreUninitialized => "requirement_store_uninitialized",
            Self::Io(_) => "testpoints_io",
        }
    }
}

impl std::fmt::Display for TestpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTestPoint(id) => write!(f, "test point '{id}' not found"),
            Self::UnknownDocument(id) => write!(f, "requirement document '{id}' not found"),
            Self::InvalidRange(message) => f.write_str(message),
            Self::LinkNotFound(selector) => {
                write!(f, "no requirement link matched {selector}")
            }
            Self::AmbiguousLink(selector) => {
                write!(f, "more than one requirement link matched {selector}")
            }
            Self::StoreUninitialized => f.write_str(
                "requirement store has no store_id; initialize it before linking test points",
            ),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for TestpointError {}

/// A parsed `--document`/`--range` pair used to select one existing link.
///
/// An omitted field matches anything, so an empty selector means "the only link
/// on this test point".
struct LinkSelector {
    document: Option<String>,
    position: Option<TextRange>,
}

impl LinkSelector {
    fn parse(document: Option<&str>, range: Option<&str>) -> Result<Self, TestpointError> {
        Ok(Self {
            document: document.map(str::to_string),
            position: range.map(parse_range).transpose()?,
        })
    }

    fn matches(&self, link: &RequirementLink) -> bool {
        self.document
            .as_deref()
            .is_none_or(|document| link.document_id == document)
            && self
                .position
                .is_none_or(|position| link.position == position)
    }

    fn describe(&self) -> String {
        match (&self.document, self.position) {
            (Some(document), Some(position)) => format!(
                "document '{document}' at [{}:{}]",
                position.start.offset(),
                position.end.offset()
            ),
            (Some(document), None) => format!("document '{document}'"),
            (None, Some(position)) => {
                format!("[{}:{}]", position.start.offset(), position.end.offset())
            }
            (None, None) => "the supplied selector".to_string(),
        }
    }
}

/// Requirement documents and test points read from one project + store pair.
struct TestpointContext {
    store_id: Option<RequirementStoreId>,
    /// Indexed document id to its current revision.
    revisions: BTreeMap<String, String>,
    /// Readable document id to its Markdown body.
    bodies: BTreeMap<String, String>,
    file: TestPointsFile,
}

impl TestpointContext {
    fn load(project_root: &Path, cli_override: Option<&Path>) -> Result<Self, TestpointError> {
        let requirements_root = requirements_data_dir(cli_override).map_err(TestpointError::io)?;
        let loaded = load_authoring_artifacts(project_root, &requirements_root)
            .map_err(TestpointError::io)?;
        let Some(artifacts) = loaded.artifacts else {
            return Ok(Self {
                store_id: None,
                revisions: BTreeMap::new(),
                bodies: BTreeMap::new(),
                file: TestPointsFile::default(),
            });
        };
        Ok(Self {
            store_id: artifacts.index.store_id.clone(),
            revisions: artifacts
                .index
                .documents
                .iter()
                .map(|doc| (doc.id.clone(), doc.revision.as_str().to_string()))
                .collect(),
            bodies: artifacts
                .documents
                .iter()
                .map(|doc| (doc.meta.id.clone(), doc.body.clone()))
                .collect(),
            file: artifacts.test_points,
        })
    }

    fn index_of(&self, test_point_id: &str) -> Result<usize, TestpointError> {
        self.file
            .test_points
            .iter()
            .position(|test_point| test_point.id == test_point_id)
            .ok_or_else(|| TestpointError::UnknownTestPoint(test_point_id.to_string()))
    }

    /// Builds a fresh, resolved anchor against the current document revision.
    fn build_link(
        &self,
        document: &str,
        range_text: &str,
    ) -> Result<RequirementLink, TestpointError> {
        let store_id = self
            .store_id
            .clone()
            .ok_or(TestpointError::StoreUninitialized)?;
        let revision = self
            .revisions
            .get(document)
            .ok_or_else(|| TestpointError::UnknownDocument(document.to_string()))?;
        let body = self
            .bodies
            .get(document)
            .ok_or_else(|| TestpointError::UnknownDocument(document.to_string()))?;
        let range = parse_range(range_text)?;
        create_requirement_link(store_id, document, revision.clone(), body, range).ok_or_else(
            || {
                TestpointError::InvalidRange(format!(
                    "range '{range_text}' is empty or outside document '{document}'"
                ))
            },
        )
    }
}

/// Answers how many and which links a selector matches.
fn matching_links(links: &[RequirementLink], selector: &LinkSelector) -> Vec<usize> {
    links
        .iter()
        .enumerate()
        .filter(|(_, link)| selector.matches(link))
        .map(|(index, _)| index)
        .collect()
}

/// Any link change invalidates a prior approval.
///
/// There is deliberately no operation that moves a test point to `Approved`:
/// human review in the Test Points tab is the only path to that state.
fn reset_approval(test_point: &mut TestPoint) {
    if test_point.review_state == ReviewState::Approved {
        test_point.review_state = ReviewState::Proposed;
    }
}

fn parse_range(text: &str) -> Result<TextRange, TestpointError> {
    let invalid = || TestpointError::InvalidRange(format!("range '{text}' must be START:END"));
    let (start, end) = text.split_once(':').ok_or_else(invalid)?;
    let start: u32 = start.trim().parse().map_err(|_| invalid())?;
    let end: u32 = end.trim().parse().map_err(|_| invalid())?;
    Ok(TextRange::new(start, end))
}

fn review_state_str(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Proposed => "proposed",
        ReviewState::Approved => "approved",
        ReviewState::Rejected => "rejected",
        ReviewState::NeedsReview => "needs_review",
    }
}

fn resolution_str(resolution: ResolutionState) -> &'static str {
    match resolution {
        ResolutionState::Resolved => "resolved",
        ResolutionState::Stale => "stale",
    }
}

#[derive(Serialize)]
struct RangeJson {
    start: u32,
    end: u32,
}

#[derive(Serialize)]
struct LinkJson {
    document_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    store_id: Option<String>,
    revision: String,
    position: RangeJson,
    resolution: &'static str,
    quote: String,
}

#[derive(Serialize)]
struct TestPointJson {
    id: String,
    title: String,
    review_state: &'static str,
    links: Vec<LinkJson>,
}

#[derive(Serialize)]
struct TestpointsEnvelopeJson {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    store_id: Option<String>,
    test_points: Vec<TestPointJson>,
}

#[derive(Serialize)]
struct TestpointsErrorJson {
    code: &'static str,
    error: String,
}

fn link_json(link: &RequirementLink) -> LinkJson {
    LinkJson {
        document_id: link.document_id.clone(),
        store_id: link.store_id.as_ref().map(ToString::to_string),
        revision: link.document_revision.clone(),
        position: RangeJson {
            start: link.position.start.offset(),
            end: link.position.end.offset(),
        },
        resolution: resolution_str(link.resolution),
        quote: link.quote.quote.clone(),
    }
}

fn test_point_json(test_point: &TestPoint) -> TestPointJson {
    TestPointJson {
        id: test_point.id.clone(),
        title: test_point.title.clone(),
        review_state: review_state_str(test_point.review_state),
        links: test_point.requirement_links.iter().map(link_json).collect(),
    }
}

fn envelope(context: &TestpointContext, test_points: Vec<TestPointJson>) -> String {
    let payload = TestpointsEnvelopeJson {
        ok: true,
        store_id: context.store_id.as_ref().map(ToString::to_string),
        test_points,
    };
    serde_json::to_string(&payload).unwrap_or_else(|err| {
        format!(
            r#"{{"ok":false,"code":"testpoints_io","error":"{}"}}"#,
            err.to_string().replace('"', "'")
        )
    })
}

fn resolve_project_root(project: Option<&Path>) -> PathBuf {
    project
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn list_command(
    project: Option<&Path>,
    cli_override: Option<&Path>,
    json: bool,
) -> Result<String, TestpointError> {
    let context = TestpointContext::load(&resolve_project_root(project), cli_override)?;
    if json {
        return Ok(envelope(
            &context,
            context
                .file
                .test_points
                .iter()
                .map(test_point_json)
                .collect(),
        ));
    }
    let mut out = String::new();
    for test_point in &context.file.test_points {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            test_point.id,
            review_state_str(test_point.review_state),
            test_point.title
        ));
        for link in &test_point.requirement_links {
            out.push_str(&format!(
                "  -> {} [{}:{}] {}\n",
                link.document_id,
                link.position.start.offset(),
                link.position.end.offset(),
                resolution_str(link.resolution)
            ));
        }
    }
    Ok(out.trim_end_matches('\n').to_string())
}

/// Location, identity, and output mode shared by every `link` mutation.
struct LinkRequest<'a> {
    project: Option<&'a Path>,
    cli_override: Option<&'a Path>,
    test_point_id: &'a str,
    json: bool,
}

impl LinkRequest<'_> {
    /// Loads the project root and the test points this request mutates.
    fn load(&self) -> Result<(PathBuf, TestpointContext), TestpointError> {
        let project_root = resolve_project_root(self.project);
        let context = TestpointContext::load(&project_root, self.cli_override)?;
        Ok((project_root, context))
    }
}

fn link_add(
    request: &LinkRequest<'_>,
    document: &str,
    range_text: &str,
) -> Result<String, TestpointError> {
    let (project_root, mut context) = request.load()?;
    let index = context.index_of(request.test_point_id)?;
    let link = context.build_link(document, range_text)?;

    let test_point = &mut context.file.test_points[index];
    test_point.requirement_links.push(link);
    reset_approval(test_point);
    save_test_points(&project_root, &context.file).map_err(TestpointError::io)?;

    Ok(render_mutation(&context, index, request.json))
}

fn link_remove(
    request: &LinkRequest<'_>,
    document: Option<&str>,
    range_text: Option<&str>,
) -> Result<String, TestpointError> {
    let (project_root, mut context) = request.load()?;
    let index = context.index_of(request.test_point_id)?;
    let selector = LinkSelector::parse(document, range_text)?;
    let matches = matching_links(
        &context.file.test_points[index].requirement_links,
        &selector,
    );
    let link_index = single_match(&matches, &selector)?;

    let test_point = &mut context.file.test_points[index];
    test_point.requirement_links.remove(link_index);
    reset_approval(test_point);
    save_test_points(&project_root, &context.file).map_err(TestpointError::io)?;

    Ok(render_mutation(&context, index, request.json))
}

fn link_update(
    request: &LinkRequest<'_>,
    document: Option<&str>,
    range_text: Option<&str>,
    to_document: &str,
    to_range: &str,
) -> Result<String, TestpointError> {
    let (project_root, mut context) = request.load()?;
    let index = context.index_of(request.test_point_id)?;
    let selector = LinkSelector::parse(document, range_text)?;
    let matches = matching_links(
        &context.file.test_points[index].requirement_links,
        &selector,
    );
    let link_index = single_match(&matches, &selector)?;
    let replacement = context.build_link(to_document, to_range)?;

    let test_point = &mut context.file.test_points[index];
    test_point.requirement_links[link_index] = replacement;
    reset_approval(test_point);
    save_test_points(&project_root, &context.file).map_err(TestpointError::io)?;

    Ok(render_mutation(&context, index, request.json))
}

fn single_match(matches: &[usize], selector: &LinkSelector) -> Result<usize, TestpointError> {
    match matches {
        [] => Err(TestpointError::LinkNotFound(selector.describe())),
        [index] => Ok(*index),
        _ => Err(TestpointError::AmbiguousLink(selector.describe())),
    }
}

fn render_mutation(context: &TestpointContext, index: usize, json: bool) -> String {
    if !json {
        return String::new();
    }
    envelope(
        context,
        vec![test_point_json(&context.file.test_points[index])],
    )
}

fn link_request<'a>(base: &'a TestpointArgs, cli_override: Option<&'a Path>) -> LinkRequest<'a> {
    LinkRequest {
        project: base.project.as_deref(),
        cli_override,
        test_point_id: &base.test_point_id,
        json: base.json,
    }
}

/// Handles `teshi testpoints`, exiting non-zero after writing the failure.
///
/// # Errors
///
/// Returns an error only when the JSON failure envelope cannot be constructed.
pub fn handle_testpoints_command(
    action: &TestpointsCommand,
    cli_override: Option<&Path>,
) -> Result<()> {
    let outcome = match action {
        TestpointsCommand::List { project, json } => {
            list_command(project.as_deref(), cli_override, *json)
        }
        TestpointsCommand::Link { action } => match action {
            TestpointLinkCommand::Add {
                base,
                document,
                range,
            } => link_add(&link_request(base, cli_override), document, range),
            TestpointLinkCommand::Remove {
                base,
                document,
                range,
            } => link_remove(
                &link_request(base, cli_override),
                document.as_deref(),
                range.as_deref(),
            ),
            TestpointLinkCommand::Update {
                base,
                document,
                range,
                to_document,
                to_range,
            } => link_update(
                &link_request(base, cli_override),
                document.as_deref(),
                range.as_deref(),
                to_document,
                to_range,
            ),
        },
    };

    match outcome {
        Ok(stdout) => {
            if !stdout.is_empty() {
                println!("{stdout}");
            }
            Ok(())
        }
        Err(err) => {
            if action_wants_json(action) {
                let payload = TestpointsErrorJson {
                    code: err.code(),
                    error: err.to_string(),
                };
                let stdout = serde_json::to_string(&payload)
                    .unwrap_or_else(|_| format!(r#"{{"code":"{}"}}"#, err.code()));
                println!("{stdout}");
            } else {
                eprintln!("{err}");
            }
            std::process::exit(1);
        }
    }
}

fn action_wants_json(action: &TestpointsCommand) -> bool {
    match action {
        TestpointsCommand::List { json, .. } => *json,
        TestpointsCommand::Link { action } => match action {
            TestpointLinkCommand::Add { base, .. }
            | TestpointLinkCommand::Remove { base, .. }
            | TestpointLinkCommand::Update { base, .. } => base.json,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::authoring::{
        HierarchyPath, RequirementDocumentMeta, RequirementIterationFilter,
    };
    use teshi_engine::{
        compute_document_revision, initialize_requirement_store, list_requirement_documents,
        save_requirement_document_index, save_requirement_markdown,
        update_requirement_document_body,
    };

    const LOGIN_BODY: &str = "# Login\n\nUsers can sign in.\n";
    const CHECKOUT_BODY: &str = "# Checkout\n\nShoppers can pay.\n";
    /// Sentence `tp-1` anchors in `auth/login.md`; the rewrite below drops it.
    const LOGIN_NEEDLE: &str = "Users can sign in.";
    const CHECKOUT_NEEDLE: &str = "Shoppers can pay.";

    struct Fixture {
        store: tempfile::TempDir,
        project: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let store = tempfile::tempdir().unwrap();
            let project = tempfile::tempdir().unwrap();
            let mut index = initialize_requirement_store(store.path()).unwrap();
            index.documents = vec![
                RequirementDocumentMeta::new(
                    "doc-12",
                    "auth/login.md",
                    "Login",
                    compute_document_revision(LOGIN_BODY),
                ),
                RequirementDocumentMeta::new(
                    "doc-9",
                    "shop/checkout.md",
                    "Checkout",
                    compute_document_revision(CHECKOUT_BODY),
                ),
            ];
            save_requirement_markdown(store.path(), &mut index, "auth/login.md", LOGIN_BODY)
                .unwrap();
            save_requirement_markdown(store.path(), &mut index, "shop/checkout.md", CHECKOUT_BODY)
                .unwrap();
            save_requirement_document_index(store.path(), &index).unwrap();
            Self { store, project }
        }

        fn project(&self) -> &Path {
            self.project.path()
        }

        fn store(&self) -> &Path {
            self.store.path()
        }

        fn store_id(&self) -> RequirementStoreId {
            list_requirement_documents(self.store(), &RequirementIterationFilter::All)
                .unwrap()
                .store_id
        }

        fn anchor(&self, document_id: &str, body: &str, needle: &str) -> RequirementLink {
            let byte = body
                .find(needle)
                .expect("needle appears in the fixture body");
            let start = body[..byte].chars().count() as u32;
            let end = start + needle.chars().count() as u32;
            create_requirement_link(
                self.store_id(),
                document_id,
                compute_document_revision(body).as_str(),
                body,
                TextRange::new(start, end),
            )
            .expect("needle range is a valid anchor")
        }

        fn seed(&self, test_points: Vec<TestPoint>) {
            save_test_points(
                self.project(),
                &TestPointsFile {
                    test_points,
                    ..Default::default()
                },
            )
            .unwrap();
        }

        fn seed_tp1(&self, documents: &[&str]) {
            let links = documents
                .iter()
                .map(|document| match *document {
                    "doc-12" => self.anchor("doc-12", LOGIN_BODY, LOGIN_NEEDLE),
                    other => self.anchor(other, CHECKOUT_BODY, CHECKOUT_NEEDLE),
                })
                .collect();
            self.seed(vec![test_point("tp-1", links)]);
        }

        fn load(&self) -> TestPointsFile {
            let path = teshi_core::authoring::testpoints_file(self.project());
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        }

        fn set_state(&self, id: &str, state: ReviewState) {
            let mut file = self.load();
            let slot = file
                .test_points
                .iter_mut()
                .find(|test_point| test_point.id == id)
                .expect("test point present");
            slot.review_state = state;
            save_test_points(self.project(), &file).unwrap();
        }

        fn list_json(&self) -> serde_json::Value {
            let output = list_command(Some(self.project()), Some(self.store()), true)
                .expect("list succeeds");
            serde_json::from_str(&output).unwrap()
        }

        fn add(
            &self,
            test_point: &str,
            document: &str,
            range: &str,
        ) -> Result<String, TestpointError> {
            link_add(&self.request(test_point), document, range)
        }

        fn remove(
            &self,
            test_point: &str,
            document: Option<&str>,
        ) -> Result<String, TestpointError> {
            link_remove(&self.request(test_point), document, None)
        }

        fn update(
            &self,
            test_point: &str,
            selector: Option<&str>,
            selector_range: Option<&str>,
            to_document: &str,
            to_range: &str,
        ) -> Result<String, TestpointError> {
            link_update(
                &self.request(test_point),
                selector,
                selector_range,
                to_document,
                to_range,
            )
        }

        fn request<'a>(&'a self, test_point_id: &'a str) -> LinkRequest<'a> {
            LinkRequest {
                project: Some(self.project()),
                cli_override: Some(self.store()),
                test_point_id,
                json: true,
            }
        }
    }

    fn test_point(id: &str, requirement_links: Vec<RequirementLink>) -> TestPoint {
        TestPoint {
            id: id.into(),
            title: id.to_uppercase(),
            objective: format!("Verify {id}"),
            preconditions: None,
            expected_outcomes: None,
            hierarchy_path: HierarchyPath::new(vec!["Auth".into()]),
            review_state: ReviewState::Proposed,
            requirement_links,
            scenario_refs: Vec::new(),
        }
    }

    fn point<'a>(value: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
        value["test_points"]
            .as_array()
            .unwrap()
            .iter()
            .find(|test_point| test_point["id"] == id)
            .expect("test point present in the envelope")
    }

    fn links(value: &serde_json::Value, id: &str) -> Vec<serde_json::Value> {
        point(value, id)["links"].as_array().unwrap().clone()
    }

    #[test]
    fn list_reports_state_and_links_per_test_point() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        let mut file = fixture.load();
        file.test_points.push(test_point("tp-2", Vec::new()));
        save_test_points(fixture.project(), &file).unwrap();

        let value = fixture.list_json();
        assert!(value["store_id"].is_string());
        assert_eq!(links(&value, "tp-1").len(), 1);
        assert_eq!(links(&value, "tp-1")[0]["document_id"], "doc-12");
        assert_eq!(links(&value, "tp-1")[0]["resolution"], "resolved");
        assert_eq!(point(&value, "tp-1")["review_state"], "proposed");
        assert!(links(&value, "tp-2").is_empty());
    }

    #[test]
    fn list_without_a_test_point_file_succeeds_and_is_empty() {
        let fixture = Fixture::new();
        assert!(
            fixture.list_json()["test_points"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let text = list_command(Some(fixture.project()), Some(fixture.store()), false).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn add_appends_a_resolved_link() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);

        let value: serde_json::Value =
            serde_json::from_str(&fixture.add("tp-1", "doc-9", "0:14").unwrap()).unwrap();
        assert_eq!(links(&value, "tp-1").len(), 2);

        let reloaded = fixture.list_json();
        assert_eq!(links(&reloaded, "tp-1").len(), 2);
        assert_eq!(links(&reloaded, "tp-1")[1]["document_id"], "doc-9");
        assert_eq!(links(&reloaded, "tp-1")[1]["resolution"], "resolved");
    }

    #[test]
    fn mutating_links_returns_an_approved_test_point_to_proposed() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        fixture.set_state("tp-1", ReviewState::Approved);

        let value: serde_json::Value =
            serde_json::from_str(&fixture.add("tp-1", "doc-9", "0:14").unwrap()).unwrap();
        assert_eq!(point(&value, "tp-1")["review_state"], "proposed");
        assert_eq!(
            fixture.load().test_points[0].review_state,
            ReviewState::Proposed
        );
    }

    #[test]
    fn rejected_targets_fail_with_a_code_and_leave_the_file_untouched() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        let before = fixture.load();

        let cases = [
            ("missing", "doc-9", "0:14", "unknown_test_point"),
            ("tp-1", "missing", "0:14", "unknown_document"),
            ("tp-1", "doc-9", "4:4", "invalid_range"),
            ("tp-1", "doc-9", "0:999", "invalid_range"),
            ("tp-1", "doc-9", "nonsense", "invalid_range"),
        ];
        for (id, document, range, code) in cases {
            let error = fixture.add(id, document, range).unwrap_err();
            assert_eq!(error.code(), code, "for {id}/{document}/{range}");
            assert_eq!(
                fixture.load(),
                before,
                "wrote after {id}/{document}/{range}"
            );
        }
    }

    #[test]
    fn remove_deletes_one_link_and_keeps_the_rest() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        fixture.add("tp-1", "doc-9", "0:14").unwrap();

        fixture.remove("tp-1", Some("doc-12")).unwrap();

        let value = fixture.list_json();
        assert_eq!(links(&value, "tp-1").len(), 1);
        assert_eq!(links(&value, "tp-1")[0]["document_id"], "doc-9");
    }

    #[test]
    fn remove_without_a_match_fails_without_writing() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        let before = fixture.load();

        let error = fixture.remove("tp-1", Some("doc-9")).unwrap_err();
        assert_eq!(error.code(), "link_not_found");
        assert_eq!(fixture.load(), before);
    }

    #[test]
    fn remove_without_a_selector_rejects_a_multi_link_test_point() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        fixture.add("tp-1", "doc-9", "0:14").unwrap();

        let error = fixture.remove("tp-1", None).unwrap_err();
        assert_eq!(error.code(), "ambiguous_link");
    }

    #[test]
    fn update_replaces_the_only_link_instead_of_appending() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);

        fixture.update("tp-1", None, None, "doc-9", "0:14").unwrap();

        let value = fixture.list_json();
        assert_eq!(links(&value, "tp-1").len(), 1);
        assert_eq!(links(&value, "tp-1")[0]["document_id"], "doc-9");
        assert_eq!(links(&value, "tp-1")[0]["resolution"], "resolved");
    }

    #[test]
    fn update_can_select_by_document_and_range() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        fixture.add("tp-1", "doc-9", "0:14").unwrap();

        fixture
            .update("tp-1", Some("doc-9"), Some("0:14"), "doc-12", "0:10")
            .unwrap();

        let value = fixture.list_json();
        assert_eq!(links(&value, "tp-1").len(), 2);
        assert_eq!(links(&value, "tp-1")[0]["position"]["start"], 9);
        assert_eq!(links(&value, "tp-1")[1]["document_id"], "doc-12");
        assert_eq!(links(&value, "tp-1")[1]["position"]["start"], 0);
        assert_eq!(links(&value, "tp-1")[1]["position"]["end"], 10);
    }

    #[test]
    fn rewriting_the_linked_body_stales_the_anchor_and_needs_review() {
        let fixture = Fixture::new();
        fixture.seed_tp1(&["doc-12"]);
        fixture.set_state("tp-1", ReviewState::Approved);
        update_requirement_document_body(
            fixture.store(),
            "doc-12",
            "# Login\n\nBody replaced.\n",
            None,
            true,
        )
        .unwrap();

        let value = fixture.list_json();
        assert_eq!(links(&value, "tp-1").len(), 1);
        assert_eq!(links(&value, "tp-1")[0]["resolution"], "stale");
        assert_eq!(point(&value, "tp-1")["review_state"], "needs_review");
    }

    #[test]
    fn parse_range_requires_two_numbers() {
        assert_eq!(parse_range("0:14").unwrap(), TextRange::new(0, 14));
        for text in ["", "0", "0:", ":4", "a:b", "0:14:20"] {
            assert_eq!(
                parse_range(text).unwrap_err().code(),
                "invalid_range",
                "accepted {text:?}"
            );
        }
    }
}
