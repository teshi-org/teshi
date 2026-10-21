//! Requirements controls and modal ownership through a real terminal pointer.

#[path = "support/requirement_project.rs"]
mod requirement_project;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use teshi_core::authoring::{
    DocumentRevision, HierarchyPath, RequirementDocumentIndex, RequirementDocumentMeta,
    ReviewState, TestPoint, TestPointsFile, TextRange, create_requirement_link,
};
use teshi_tui_runner::{Launch, Terminal};

const ALPHA: &str = "# Alpha requirement\nalpha first\nalpha second\n";
const BETA: &str = "# Beta requirement\nbeta body\n";

struct Fixture {
    _root: tempfile::TempDir,
    config: Launch,
    store: PathBuf,
    points: PathBuf,
    source: PathBuf,
    artifact: PathBuf,
}

impl Fixture {
    fn new(case: &str, narrow: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let store = root.path().join("requirements");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&store).unwrap();
        let source = project.join("sample.feature");
        fs::write(&source, "Feature: Pointer fixture\n").unwrap();
        fs::write(store.join("alpha.md"), ALPHA).unwrap();
        fs::write(store.join("beta.md"), BETA).unwrap();
        let mut index = RequirementDocumentIndex::initialize_empty_store();
        for (id, title, iteration) in [
            ("alpha", "Alpha requirement", "Sprint A"),
            ("beta", "Beta requirement", "Sprint B"),
        ] {
            let mut meta = RequirementDocumentMeta::new(
                id,
                format!("{id}.md"),
                title,
                DocumentRevision::new(format!("fixture-{id}")),
            );
            meta.iteration = Some(iteration.into());
            index.documents.push(meta);
        }
        requirement_project::associate(&project, &mut index);
        fs::write(
            store.join("_teshi.json"),
            serde_json::to_vec_pretty(&index).unwrap(),
        )
        .unwrap();
        let mut test_points = TestPointsFile::default();
        for (number, quote) in [(1, "alpha first"), (2, "alpha second")] {
            let start = ALPHA.find(quote).unwrap();
            test_points.test_points.push(TestPoint {
                id: format!("tp-{number}"),
                title: format!("Linked point {number}"),
                objective: format!("Verify {quote}"),
                preconditions: None,
                expected_outcomes: None,
                hierarchy_path: HierarchyPath::new(vec!["Pointer".into()]),
                review_state: ReviewState::Proposed,
                requirement_links: vec![
                    create_requirement_link(
                        index.store_id.clone().unwrap(),
                        "alpha",
                        "fixture-alpha",
                        ALPHA,
                        TextRange::new(
                            start.try_into().unwrap(),
                            (start + quote.len()).try_into().unwrap(),
                        ),
                    )
                    .unwrap(),
                ],
                scenario_refs: Vec::new(),
            });
        }
        let points = project.join("testpoints/testpoints.json");
        fs::create_dir(points.parent().unwrap()).unwrap();
        fs::write(&points, serde_json::to_vec_pretty(&test_points).unwrap()).unwrap();
        let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
        let config = Launch {
            program: binary.to_string_lossy().into_owned(),
            args: vec![
                project.to_string_lossy().into_owned(),
                "--requirements-root".into(),
                store.to_string_lossy().into_owned(),
            ],
            cwd: Some(root.path().to_string_lossy().into_owned()),
            env: std::iter::once(("TERM".into(), "xterm-256color".into()))
                .chain(
                    [
                        "APPDATA",
                        "LOCALAPPDATA",
                        "XDG_CONFIG_HOME",
                        "XDG_DATA_HOME",
                        "TESHI_APP_DATA_DIR",
                        "TESHI_SESSION_DIR",
                    ]
                    .into_iter()
                    .map(|key| {
                        (
                            key.into(),
                            root.path().join(key).to_string_lossy().into_owned(),
                        )
                    }),
                )
                .collect(),
            cols: if narrow { 55 } else { 150 },
            rows: if narrow { 16 } else { 30 },
        };
        let artifact =
            PathBuf::from(std::env::var_os("TESHI_REQUIREMENTS_POINTER_ARTIFACTS").unwrap()).join(
                format!("{case}-{}", if narrow { "narrow" } else { "normal" }),
            );
        fs::create_dir_all(&artifact).unwrap();
        Self {
            _root: root,
            config,
            store,
            points,
            source,
            artifact,
        }
    }

    fn launch(&self) -> Terminal {
        let terminal = Terminal::launch(self.config.clone()).unwrap();
        terminal.expect_text("sample", true, 10000).unwrap();
        terminal.key("4").unwrap();
        terminal
            .expect_text("# Alpha requirement", true, 10000)
            .unwrap();
        self.capture(&terminal, "initial");
        terminal
    }

    fn capture(&self, terminal: &Terminal, name: &str) -> PathBuf {
        std::thread::sleep(Duration::from_millis(100));
        fs::write(
            self.artifact.join(format!("{name}.txt")),
            terminal.screen().unwrap(),
        )
        .unwrap();
        let svg = self.artifact.join(format!("{name}.svg"));
        terminal.screenshot(&svg).unwrap();
        fs::copy(
            self.store.join("_teshi.json"),
            self.artifact.join(format!("{name}-index.json")),
        )
        .unwrap();
        fs::copy(
            &self.points,
            self.artifact.join(format!("{name}-testpoints.json")),
        )
        .unwrap();
        for name in ["alpha", "beta", "doc-1", "doc-2"] {
            let path = self.store.join(format!("{name}.md"));
            if path.is_file() {
                fs::copy(path, self.artifact.join(format!("{name}-latest.md"))).unwrap();
            }
        }
        svg
    }

    fn click(&self, terminal: &Terminal, label: &str, stage: &str) {
        self.capture(terminal, &format!("{stage}-before"));
        terminal.click_text(label, 1500).unwrap();
        self.capture(terminal, &format!("{stage}-after"));
    }

    fn click_new(&self, terminal: &Terminal) {
        let label = if terminal.screen().unwrap().contains("New doc") {
            "New doc"
        } else {
            "New"
        };
        self.click(terminal, label, "new");
    }

    fn index(&self) -> RequirementDocumentIndex {
        serde_json::from_slice(&fs::read(self.store.join("_teshi.json")).unwrap()).unwrap()
    }

    fn points(&self) -> TestPointsFile {
        serde_json::from_slice(&fs::read(&self.points).unwrap()).unwrap()
    }

    fn finish(&self, terminal: Terminal) {
        assert_eq!(
            fs::read_to_string(&self.source).unwrap(),
            "Feature: Pointer fixture\n"
        );
        terminal.key("q").unwrap();
        terminal.expect_exit(0, 10000).unwrap();
    }
}

fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        predicate(),
        "persisted state did not match within 5 seconds"
    );
}

fn new_document(narrow: bool) {
    let fixture = Fixture::new("new-save", narrow);
    let terminal = fixture.launch();
    fixture.click_new(&terminal);
    terminal.expect_text("# Document 1", true, 1500).unwrap();
    fixture.click(&terminal, "Save", "save-new");
    wait_until(|| fixture.store.join("doc-1.md").is_file());
    assert_eq!(
        fs::read_to_string(fixture.store.join("doc-1.md")).unwrap(),
        "# Document 1\n\n"
    );
    assert_eq!(fixture.index().documents.len(), 3);
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        ALPHA
    );
    fixture.capture(&terminal, "new-persisted");
    fixture.finish(terminal);
}

fn filter_and_group(narrow: bool) {
    let fixture = Fixture::new("filter-group", narrow);
    let terminal = fixture.launch();
    fixture.click(&terminal, "Filter", "open-filter");
    terminal
        .expect_text("Filter iteration", true, 1500)
        .unwrap();
    terminal.write("\x1b[<0;5;1M\x1b[<0;5;1m").unwrap();
    fixture.capture(&terminal, "filter-background-owned");
    terminal
        .expect_text("Filter iteration", true, 1500)
        .unwrap();
    fixture.click(&terminal, "Sprint B", "choose-filter");
    terminal
        .expect_text("# Beta requirement", true, 1500)
        .unwrap();
    terminal
        .expect_text("Filter iteration", false, 1500)
        .unwrap();
    terminal
        .expect_text("# Alpha requirement", false, 1500)
        .unwrap();
    fixture.click(&terminal, "Filter", "reset-filter");
    fixture.click(&terminal, "All", "choose-all");
    terminal
        .expect_text("# Beta requirement", true, 1500)
        .unwrap();
    fixture.click(&terminal, "Group", "group-iterations");
    terminal.expect_text("Sprint A", true, 1500).unwrap();
    terminal.expect_text("Sprint B", true, 1500).unwrap();
    fixture.click(&terminal, "Group", "group-paths");
    terminal.expect_text("Sprint B", false, 1500).unwrap();
    fixture.click(&terminal, "Filter", "filter-unassigned");
    fixture.click(&terminal, "Unassigned", "choose-unassigned");
    terminal
        .expect_text("# Alpha requirement", false, 1500)
        .unwrap();
    terminal
        .expect_text("# Beta requirement", false, 1500)
        .unwrap();
    fixture.capture(&terminal, "unassigned-empty-view");
    fixture.click(&terminal, "Filter", "filter-from-empty");
    fixture.click(&terminal, "All", "restore-all-from-empty");
    terminal
        .expect_text("# Alpha requirement", true, 1500)
        .unwrap();
    assert_eq!(fixture.index().documents.len(), 2);
    fixture.finish(terminal);
}

fn iteration(narrow: bool) {
    let fixture = Fixture::new("iteration-modal", narrow);
    let terminal = fixture.launch();
    fixture.click(&terminal, "Iteration", "open-iteration");
    terminal.expect_text("Set iteration", true, 1500).unwrap();
    // An underlying tab and row remain visible while the modal owns input.
    terminal.write("\x1b[<0;5;1M\x1b[<0;5;1m").unwrap();
    for _ in "Sprint A".chars() {
        terminal.key("Backspace").unwrap();
    }
    terminal.write("迭代甲jkl").unwrap();
    fixture.capture(&terminal, "iteration-input-and-background-click");
    terminal.expect_text("Set iteration", true, 1500).unwrap();
    fixture.click(&terminal, "Confirm", "confirm-iteration");
    wait_until(|| fixture.index().documents[0].iteration.as_deref() == Some("迭代甲jkl"));
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        ALPHA
    );
    fixture.click(&terminal, "Iteration", "reopen-iteration");
    terminal.write("discarded").unwrap();
    fixture.click(&terminal, "Cancel", "cancel-iteration");
    terminal.expect_text("Set iteration", false, 1500).unwrap();
    assert_eq!(
        fixture.index().documents[0].iteration.as_deref(),
        Some("迭代甲jkl")
    );
    fixture.capture(&terminal, "iteration-persisted");
    fixture.finish(terminal);
}

fn deletion(narrow: bool) {
    let fixture = Fixture::new("delete-modal", narrow);
    let terminal = fixture.launch();
    fixture.click(&terminal, "Delete", "delete-open");
    terminal
        .expect_text("Delete requirement", true, 1500)
        .unwrap();
    terminal.write("\x1b[<0;5;1M\x1b[<0;5;1m").unwrap();
    terminal
        .expect_text("Delete requirement", true, 1500)
        .unwrap();
    fixture.click(&terminal, "Cancel", "delete-cancel");
    terminal
        .expect_text("Delete requirement", false, 1500)
        .unwrap();
    assert_eq!(fixture.index().documents.len(), 2);
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        ALPHA
    );
    fixture.click(&terminal, "Delete", "delete-open-again");
    fixture.click(&terminal, "Confirm", "delete-confirm");
    wait_until(|| {
        !fixture
            .index()
            .documents
            .iter()
            .any(|document| document.id == "alpha")
    });
    assert!(!fixture.store.join("alpha.md").exists());
    terminal
        .expect_text("# Beta requirement", true, 1500)
        .unwrap();
    let points = fixture.points();
    assert_eq!(points.test_points.len(), 2);
    assert!(
        points
            .test_points
            .iter()
            .flat_map(|point| &point.requirement_links)
            .all(|link| link.resolution == teshi_core::authoring::ResolutionState::Stale)
    );
    fixture.capture(&terminal, "deletion-persisted");
    fixture.finish(terminal);
}

fn create_point(narrow: bool) {
    let fixture = Fixture::new("selection-point", narrow);
    let terminal = fixture.launch();
    terminal.drag_text("alpha first", 1500).unwrap();
    fixture.capture(&terminal, "source-selection");
    // The compact Points tab shares the button's prefix. Resolve the actual
    // footer row rather than allowing a text match to click the tab instead.
    let screen = terminal.screen().unwrap();
    let (row, footer) = screen.lines().enumerate().last().unwrap();
    let col = footer.find("Point").unwrap();
    fixture.capture(&terminal, "create-point-before");
    terminal
        .write(&format!("\x1b[<0;{};{}M", col + 1, row + 1))
        .unwrap();
    terminal
        .write(&format!("\x1b[<0;{};{}m", col + 1, row + 1))
        .unwrap();
    fixture.capture(&terminal, "create-point-after");
    wait_until(|| fixture.points().test_points.len() == 3);
    let points = fixture.points();
    let created = points.test_points.last().unwrap();
    assert_eq!(created.review_state, ReviewState::Proposed);
    assert_eq!(created.requirement_links[0].document_id, "alpha");
    assert_eq!(created.requirement_links[0].quote.quote, "alpha first");
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        ALPHA
    );
    fixture.capture(&terminal, "created-point-persisted");
    fixture.finish(terminal);
}

fn dirty_modal(narrow: bool, save: bool) {
    let fixture = Fixture::new(if save { "dirty-save" } else { "dirty-discard" }, narrow);
    let terminal = fixture.launch();
    terminal.click_text("alpha first", 1500).unwrap();
    terminal.key("i").unwrap();
    terminal.write("draft ").unwrap();
    terminal.key("Escape").unwrap();
    fixture.click_new(&terminal);
    terminal
        .expect_text("Unsaved requirement", true, 1500)
        .unwrap();
    terminal.write("\x1b[<0;5;1M\x1b[<0;5;1m").unwrap();
    fixture.click(&terminal, "Cancel", "dirty-cancel");
    terminal
        .expect_text("Unsaved requirement", false, 1500)
        .unwrap();
    terminal
        .expect_text("draft alpha first", true, 1500)
        .unwrap();
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        ALPHA
    );
    fixture.click_new(&terminal);
    fixture.click(
        &terminal,
        if save { "Save" } else { "Discard" },
        "dirty-resolve",
    );
    terminal.expect_text("# Document 1", true, 1500).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        if save {
            "# Alpha requirement\ndraft alpha first\nalpha second\n"
        } else {
            ALPHA
        }
    );
    fixture.click(&terminal, "Save", "save-new-after-dirty");
    wait_until(|| fixture.store.join("doc-1.md").is_file());
    fixture.capture(&terminal, "dirty-resolution-persisted");
    fixture.finish(terminal);
}

fn selected_linked(screen: &str, title: &str) -> bool {
    screen.lines().any(|line| {
        line.rsplit('│')
            .nth(1)
            .is_some_and(|pane| pane.contains(title) && pane.contains('›'))
    })
}

fn priority_roundtrip(narrow: bool) {
    use teshi_core::authoring::RequirementPriority;
    let fixture = Fixture::new("priority-roundtrip", narrow);
    let terminal = fixture.launch();
    fixture.click(&terminal, "Priority: Unset", "priority-open");
    terminal
        .expect_text("Requirement priority", true, 1500)
        .unwrap();
    terminal.key("Down").unwrap();
    fixture.click(&terminal, "Cancel", "priority-cancel");
    assert_eq!(fixture.index().documents[0].priority, None);
    // Metadata edits must neither save nor discard the dirty Markdown buffer.
    terminal.click_text("alpha first", 1500).unwrap();
    terminal.key("i").unwrap();
    terminal.write("draft ").unwrap();
    terminal.key("Escape").unwrap();
    fixture.click(&terminal, "Priority: Unset", "priority-dirty-open");
    terminal.write("\x1b[<0;5;1M\x1b[<0;5;1m").unwrap();
    terminal
        .expect_text("Requirement priority", true, 1500)
        .unwrap();
    fixture.click(&terminal, "P1", "priority-p1");
    wait_until(|| fixture.index().documents[0].priority == Some(RequirementPriority::P1));
    terminal.expect_text("Priority: P1", true, 1500).unwrap();
    terminal
        .expect_text("draft alpha first", true, 1500)
        .unwrap();
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        ALPHA
    );
    fixture.click(&terminal, "Save", "priority-save-body");
    wait_until(|| {
        fs::read_to_string(fixture.store.join("alpha.md"))
            .unwrap()
            .contains("draft ")
    });
    assert_eq!(
        fixture.index().documents[0].priority,
        Some(RequirementPriority::P1)
    );
    let before = fixture.index();
    fixture.finish(terminal);
    let terminal = fixture.launch();
    terminal.expect_text("Priority: P1", true, 1500).unwrap();
    assert_eq!(fixture.index(), before);
    // The keyboard entry remains on the tree, independent of the editor's Vim keys.
    terminal.key("p").unwrap();
    terminal
        .expect_text("Requirement priority", true, 1500)
        .unwrap();
    terminal.key("Up").unwrap();
    terminal.key("Up").unwrap();
    terminal.key("Enter").unwrap();
    wait_until(|| fixture.index().documents[0].priority.is_none());
    terminal.expect_text("Priority: Unset", true, 1500).unwrap();
    fixture.capture(&terminal, "priority-cleared");
    assert_eq!(fixture.index().documents[0].id, "alpha");
    assert_eq!(
        fixture.index().documents[0].iteration.as_deref(),
        Some("Sprint A")
    );
    assert_eq!(fixture.index().documents[1], before.documents[1]);
    fixture.finish(terminal);
}

fn priority_write_failure(narrow: bool) {
    use teshi_core::authoring::RequirementPriority;
    let fixture = Fixture::new("priority-write-failure", narrow);
    let terminal = fixture.launch();
    let original = fs::read(fixture.store.join("_teshi.json")).unwrap();
    fs::create_dir(fixture.store.join("_teshi.tmp")).unwrap();
    fixture.click(&terminal, "Priority: Unset", "priority-failure-open");
    fixture.click(&terminal, "P0", "priority-failed-save");
    terminal
        .expect_text("Requirement priority", true, 1500)
        .unwrap();
    assert_eq!(
        fs::read(fixture.store.join("_teshi.json")).unwrap(),
        original
    );
    fs::remove_dir(fixture.store.join("_teshi.tmp")).unwrap();
    terminal.key("Enter").unwrap();
    wait_until(|| fixture.index().documents[0].priority == Some(RequirementPriority::P0));
    terminal.expect_text("Priority: P0", true, 1500).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.store.join("alpha.md")).unwrap(),
        ALPHA
    );
    fixture.capture(&terminal, "priority-retry-persisted");
    fixture.finish(terminal);
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn linked_point_pointer_selection_matches_keyboard_after_resize() {
    let fixture = Fixture::new("linked-selection", false);
    let terminal = fixture.launch();
    for _ in 0..2 {
        terminal.key("Ctrl+w").unwrap();
        terminal.key("l").unwrap();
    }
    fixture.capture(&terminal, "linked-keyboard-baseline");
    assert!(
        selected_linked(&terminal.screen().unwrap(), "Linked point 1"),
        "keyboard-selected linked row must be visibly highlighted before testing pointer parity"
    );
    fixture.click(&terminal, "Linked point 2", "linked-second");
    fixture.capture(&terminal, "linked-second-selected");
    assert!(selected_linked(
        &terminal.screen().unwrap(),
        "Linked point 2"
    ));
    assert!(!selected_linked(
        &terminal.screen().unwrap(),
        "Linked point 1"
    ));
    terminal.key("k").unwrap();
    fixture.capture(&terminal, "linked-keyboard-first");
    assert!(selected_linked(
        &terminal.screen().unwrap(),
        "Linked point 1"
    ));
    assert!(!selected_linked(
        &terminal.screen().unwrap(),
        "Linked point 2"
    ));
    terminal.resize(55, 16).unwrap();
    terminal.expect_text("Linked point 2", false, 1500).unwrap();
    fixture.capture(&terminal, "linked-hidden-narrow");
    terminal.resize(150, 30).unwrap();
    terminal.expect_text("Linked point 2", true, 1500).unwrap();
    fixture.click(&terminal, "Linked point 2", "linked-after-resize");
    fixture.capture(&terminal, "linked-restored-selection");
    assert!(selected_linked(
        &terminal.screen().unwrap(),
        "Linked point 2"
    ));
    assert_eq!(fixture.points().test_points.len(), 2);
    fixture.finish(terminal);
}

macro_rules! acceptance {
    ($name:ident, $function:ident, $narrow:expr $(, $argument:expr)*) => {
        #[test]
        #[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
        fn $name() { $function($narrow $(, $argument)*); }
    };
}

acceptance!(new_and_save_normal, new_document, false);
acceptance!(priority_roundtrip_normal, priority_roundtrip, false);
acceptance!(priority_roundtrip_narrow, priority_roundtrip, true);
acceptance!(priority_write_failure_normal, priority_write_failure, false);
acceptance!(priority_write_failure_narrow, priority_write_failure, true);
acceptance!(new_and_save_narrow, new_document, true);
acceptance!(filter_group_normal, filter_and_group, false);
acceptance!(filter_group_narrow, filter_and_group, true);
acceptance!(iteration_input_normal, iteration, false);
acceptance!(iteration_input_narrow, iteration, true);
acceptance!(delete_confirm_cancel_normal, deletion, false);
acceptance!(delete_confirm_cancel_narrow, deletion, true);
acceptance!(selection_creates_point_normal, create_point, false);
acceptance!(selection_creates_point_narrow, create_point, true);
acceptance!(dirty_save_cancel_normal, dirty_modal, false, true);
acceptance!(dirty_save_cancel_narrow, dirty_modal, true, true);
acceptance!(dirty_discard_cancel_normal, dirty_modal, false, false);
acceptance!(dirty_discard_cancel_narrow, dirty_modal, true, false);
