//! Test Points pointer controls, independent scrolling, and modal ownership in a real PTY.

#[path = "support/requirement_project.rs"]
mod requirement_project;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use teshi_core::authoring::{DocumentRevision, RequirementDocumentIndex, RequirementDocumentMeta};
use teshi_tui_runner::{Launch, Terminal};

struct Fixture {
    _root: tempfile::TempDir,
    config: Launch,
    points: PathBuf,
    output: PathBuf,
    unchanged: Vec<(PathBuf, Vec<u8>)>,
}

impl Fixture {
    fn new(name: &str, cols: u16, rows: u16) -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let store = root.path().join("requirements");
        fs::create_dir_all(project.join("testpoints")).unwrap();
        fs::create_dir(&store).unwrap();
        let mut feature = String::from("Feature: Test point pointer fixture\n");
        for number in 0..24 {
            feature.push_str(&format!(
                "  @teshi-tp:point-00\n  Scenario: REFERENCE_{number:02}\n    Given SCENARIO_SENTINEL_{number:02}\n"
            ));
        }
        let source = project.join("sample.feature");
        fs::write(&source, &feature).unwrap();
        let mut unchanged = vec![(source, feature.into_bytes())];
        let mut index = RequirementDocumentIndex::initialize_empty_store();
        let store_id = serde_json::to_value(&index).unwrap()["store_id"].clone();
        let mut links = Vec::new();
        for number in 0..24 {
            let title = format!("SOURCE_{number:02}");
            let quote = format!("QUOTE_{number:02}");
            let body = format!("# {title}\n\n{quote} verification intent.\n");
            let filename = format!("source-{number:02}.md");
            let path = store.join(&filename);
            fs::write(&path, &body).unwrap();
            unchanged.push((path, body.clone().into_bytes()));
            index.documents.push(RequirementDocumentMeta::new(
                format!("source-{number:02}"),
                filename,
                title,
                DocumentRevision::new(format!("fixture-{number:02}")),
            ));
            let start = body.find(&quote).unwrap();
            links.push(json!({
                "store_id": store_id, "document_id": format!("source-{number:02}"),
                "document_revision": format!("fixture-{number:02}"),
                "position": {"start": start, "end": start + quote.len()},
                "quote": {"quote": quote}, "resolution": "resolved"
            }));
        }
        requirement_project::associate(&project, &mut index);
        let index_path = store.join("_teshi.json");
        let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
        fs::write(&index_path, &index_bytes).unwrap();
        unchanged.push((index_path, index_bytes));
        let scenarios: Vec<_> = (0..24).map(|number| json!({
            "feature_path": "sample.feature", "scenario_name": format!("REFERENCE_{number:02}"),
            "scenario_line": 3 + number * 3
        })).collect();
        let points = project.join("testpoints/testpoints.json");
        let values: Vec<_> = (0..41)
            .map(|number| {
                json!({
                    "id": format!("point-{number:02}"), "title": format!("POINT_{number:02}"),
                    "objective": format!("OBJECTIVE_{number:02}"),
                    "preconditions": format!("PRE_{number:02}"),
                    "expected_outcomes": format!("EXPECTED_{number:02}"),
                    "hierarchy_path": ["Control"], "review_state": "proposed",
                    "requirement_links": if number == 0 { links.clone() } else if number == 40 {
                        vec![json!({"store_id": store_id, "document_id": "missing-source",
                            "document_revision": "missing", "position": {"start": 0,"end": 1},
                            "quote": {"quote": "missing"}, "resolution": "stale"})]
                    } else { Vec::new() },
                    "scenario_refs": if number == 0 { scenarios.clone() } else { Vec::new() }
                })
            })
            .collect();
        fs::write(
            &points,
            serde_json::to_vec_pretty(&json!({"version": 1, "test_points": values})).unwrap(),
        )
        .unwrap();
        let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
        let mut env: Vec<_> = [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "TESHI_APP_DATA_DIR",
            "TESHI_SESSION_DIR",
            "TESHI_ACP_STORE_DIR",
        ]
        .into_iter()
        .map(|key| {
            (
                key.into(),
                root.path().join(key).to_string_lossy().into_owned(),
            )
        })
        .collect();
        env.extend([
            ("TESHI_AGENT_BACKEND".into(), "native".into()),
            ("TESHI_LLM_API_KEY".into(), String::new()),
            ("NO_COLOR".into(), String::new()),
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
        ]);
        let config = Launch {
            program: binary.to_string_lossy().into_owned(),
            args: vec![
                project.to_string_lossy().into_owned(),
                "--requirements-root".into(),
                store.to_string_lossy().into_owned(),
            ],
            cwd: Some(root.path().to_string_lossy().into_owned()),
            env,
            cols,
            rows,
        };
        let output = std::env::var_os("TESHI_TESTPOINTS_ARTIFACTS")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.teshi/artifacts/p4-controls-20261017/testpoints")
            })
            .join(name);
        fs::create_dir_all(&output).unwrap();
        fs::write(
            output.join("launch.json"),
            serde_json::to_vec_pretty(&config).unwrap(),
        )
        .unwrap();
        Self {
            _root: root,
            config,
            points,
            output,
            unchanged,
        }
    }

    fn launch(&self) -> Terminal {
        let terminal = Terminal::launch(self.config.clone()).unwrap();
        terminal.expect_text("sample", true, 10000).unwrap();
        if self.config.cols < 60 {
            terminal.click_text("Points [5]", 10000).unwrap();
        } else {
            terminal.click_text("Test Points [5]", 10000).unwrap();
        }
        terminal
            .expect_text("Intent & review", true, 10000)
            .unwrap();
        if !terminal.screen().unwrap().contains("POINT_00") {
            terminal.click_text("Control", 10000).unwrap();
            terminal.click_text("Control", 10000).unwrap();
        }
        self.capture(&terminal, "opened");
        terminal
    }

    fn records(&self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Ok(bytes) = fs::read(&self.points)
                && let Ok(records) = serde_json::from_slice(&bytes)
            {
                return records;
            }
            assert!(
                Instant::now() < deadline,
                "unreadable test-point checkpoint"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn point(&self, number: usize) -> Value {
        self.records()["test_points"][number].clone()
    }

    fn capture(&self, terminal: &Terminal, label: &str) {
        std::thread::sleep(Duration::from_millis(100));
        fs::write(
            self.output.join(format!("{label}.txt")),
            terminal.screen().unwrap(),
        )
        .unwrap();
        terminal
            .screenshot(&self.output.join(format!("{label}.svg")))
            .unwrap();
        fs::write(
            self.output.join(format!("{label}-testpoints.json")),
            fs::read(&self.points).unwrap(),
        )
        .unwrap();
    }

    fn wait(&self, terminal: &Terminal, label: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.capture(terminal, label);
        assert!(condition(), "{label}; evidence: {}", self.output.display());
    }

    fn finish(&self, terminal: &Terminal) {
        for (path, before) in &self.unchanged {
            assert_eq!(
                &fs::read(path).unwrap(),
                before,
                "unrelated source changed: {}",
                path.display()
            );
        }
        terminal.key("q").unwrap();
        terminal.expect_exit(0, 10000).unwrap();
    }
}

fn pane(screen: &str, start: usize, end: usize) -> String {
    screen
        .lines()
        .skip(3)
        .take(screen.lines().count().saturating_sub(5))
        .map(|line| {
            line.chars()
                .skip(start)
                .take(end - start)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click_footer(terminal: &Terminal, text: &str) {
    let screen = terminal.screen().unwrap();
    let (row, line) = screen.lines().enumerate().last().unwrap();
    let index = line
        .find(text)
        .unwrap_or_else(|| panic!("footer control {text:?} is absent:\n{screen}"));
    let col = line[..index].chars().count();
    terminal
        .write(&format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            col + 1,
            row + 1,
            col + 1,
            row + 1
        ))
        .unwrap();
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_tree_pointer_and_three_pane_wheels_keep_independent_context() {
    let fixture = Fixture::new("tree-wheel", 150, 30);
    let terminal = fixture.launch();
    let before = terminal.screen().unwrap();
    terminal.scroll_at(50, 8, false, false).unwrap();
    fixture.capture(&terminal, "details-wheel");
    let details = terminal.screen().unwrap();
    assert_eq!(pane(&before, 0, 30), pane(&details, 0, 30));
    assert_eq!(pane(&before, 105, 150), pane(&details, 105, 150));
    assert_ne!(pane(&before, 30, 105), pane(&details, 30, 105));
    terminal.scroll_at(120, 8, false, false).unwrap();
    fixture.capture(&terminal, "excerpts-wheel");
    let excerpts = terminal.screen().unwrap();
    assert_eq!(pane(&details, 0, 30), pane(&excerpts, 0, 30));
    assert_eq!(pane(&details, 30, 105), pane(&excerpts, 30, 105));
    assert_ne!(pane(&details, 105, 150), pane(&excerpts, 105, 150));
    for _ in 0..2 {
        terminal.scroll_at(5, 8, false, false).unwrap();
    }
    fixture.capture(&terminal, "tree-wheel");
    let tree = terminal.screen().unwrap();
    assert_eq!(pane(&excerpts, 30, 105), pane(&tree, 30, 105));
    assert_eq!(pane(&excerpts, 105, 150), pane(&tree, 105, 150));
    assert_ne!(pane(&excerpts, 0, 30), pane(&tree, 0, 30));
    terminal.click_text("POINT_08", 1000).unwrap();
    fixture.wait(&terminal, "tree-row-selected", || {
        terminal.screen().unwrap().contains("Title: POINT_08")
    });
    let svg = fs::read_to_string(fixture.output.join("tree-row-selected.svg")).unwrap();
    let selected = svg
        .split("<text ")
        .find_map(|fragment| {
            let (attributes, body) = fragment.split_once('>')?;
            body.split_once("</text>")?
                .0
                .contains("[P] POINT_08")
                .then_some(attributes)
        })
        .expect("selected tree row is absent from the actual SVG viewport");
    assert!(
        selected.contains("fill=\"#303030\"") && selected.contains("font-weight=\"bold\""),
        "clicked tree row has no visible selection: {selected}"
    );
    fixture.finish(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_field_pointer_edits_the_clicked_field_at_normal_and_narrow_sizes() {
    for (cols, rows) in [(150, 30), (55, 16)] {
        for (label, key, before) in [
            ("Objective:", "objective", "OBJECTIVE_00"),
            ("Preconditions:", "preconditions", "PRE_00"),
            ("Expected:", "expected_outcomes", "EXPECTED_00"),
            ("Title:", "title", "POINT_00"),
            ("Hierarchy:", "hierarchy_path", "Control"),
        ] {
            let fixture = Fixture::new(&format!("field-{key}-{cols}"), cols, rows);
            let terminal = fixture.launch();
            terminal.click_text(label, 1000).unwrap();
            terminal.write("Z").unwrap();
            terminal.key("s").unwrap();
            fixture.wait(&terminal, "clicked-field-saved", || {
                let point = fixture.point(0);
                if key == "hierarchy_path" {
                    point[key] == json!([format!("{before}Z")])
                } else {
                    point[key] == format!("{before}Z")
                }
            });
            let point = fixture.point(0);
            for (other, expected) in [
                ("title", "POINT_00"),
                ("objective", "OBJECTIVE_00"),
                ("preconditions", "PRE_00"),
                ("expected_outcomes", "EXPECTED_00"),
            ] {
                if other != key {
                    assert_eq!(point[other], expected, "click edited another field");
                }
            }
            fixture.finish(&terminal);
        }
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_excerpt_and_realized_scenario_rows_select_before_navigation() {
    let fixture = Fixture::new("excerpt-scenario", 150, 30);
    let terminal = fixture.launch();
    terminal.resize(95, 24).unwrap();
    terminal.scroll_at(80, 8, false, false).unwrap();
    fixture.capture(&terminal, "three-pane-resized-and-excerpts-scrolled");
    terminal.click_text("SOURCE_05", 1000).unwrap();
    fixture.capture(&terminal, "excerpt-selected");
    terminal.key("Enter").unwrap();
    fixture.wait(&terminal, "excerpt-opened", || {
        terminal.screen().unwrap().contains("# SOURCE_05")
    });
    terminal.resize(150, 30).unwrap();
    terminal.click_text("Test Points [5]", 1000).unwrap();
    terminal.expect_text("REFERENCE_05", true, 1000).unwrap();
    terminal.scroll_at(50, 8, false, false).unwrap();
    fixture.capture(&terminal, "scenario-pane-scrolled");
    terminal.click_text("REFERENCE_05", 1000).unwrap();
    fixture.capture(&terminal, "scenario-selected");
    terminal.key("o").unwrap();
    fixture.wait(&terminal, "scenario-opened", || {
        terminal
            .screen()
            .unwrap()
            .contains("Given SCENARIO_SENTINEL_05")
    });
    fixture.finish(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_review_filter_batch_and_save_buttons_persist_their_targets() {
    let fixture = Fixture::new("review-controls", 150, 30);
    let terminal = fixture.launch();
    click_footer(&terminal, "Approve");
    fixture.wait(&terminal, "approve", || {
        fixture.point(0)["review_state"] == "approved"
    });
    terminal.click_text("POINT_01", 1000).unwrap();
    click_footer(&terminal, "Reject");
    fixture.wait(&terminal, "reject", || {
        fixture.point(1)["review_state"] == "rejected"
    });
    click_footer(&terminal, "Filter");
    fixture.wait(&terminal, "filter-proposed", || {
        terminal
            .screen()
            .unwrap()
            .contains("Test points (proposed)")
    });
    assert!(!pane(&terminal.screen().unwrap(), 0, 30).contains("POINT_01"));
    click_footer(&terminal, "Batch");
    fixture.wait(&terminal, "batch", || {
        fixture.point(2)["review_state"] == "approved"
    });
    assert_eq!(fixture.point(1)["review_state"], "rejected");
    assert_eq!(fixture.point(40)["review_state"], "proposed");
    terminal.click_text("POINT_40", 1000).unwrap();
    click_footer(&terminal, "Approve");
    fixture.capture(&terminal, "stale-approve-denied");
    assert_eq!(fixture.point(40)["review_state"], "proposed");
    terminal.click_text("Objective:", 1000).unwrap();
    terminal.write("Z").unwrap();
    click_footer(&terminal, "Save");
    fixture.wait(&terminal, "save", || {
        fixture.point(40)["objective"] == "OBJECTIVE_40Z"
    });
    let before = fixture.records();
    click_footer(&terminal, "Continue");
    fixture.capture(&terminal, "continue-without-active-generation");
    assert_eq!(fixture.records(), before);
    fixture.finish(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_narrow_review_controls_resize_and_modal_input_own_the_pointer() {
    let fixture = Fixture::new("narrow-modal", 150, 30);
    let terminal = fixture.launch();
    terminal.resize(55, 16).unwrap();
    fixture.capture(&terminal, "narrow");
    for control in ["Approve", "Reject", "Batch", "Continue", "Filter", "Save"] {
        let screen = terminal.screen().unwrap();
        assert!(
            screen.lines().last().unwrap().contains(control),
            "narrow footer missing {control}:\n{screen}"
        );
    }
    click_footer(&terminal, "Approve");
    fixture.wait(&terminal, "narrow-approve", || {
        fixture.point(0)["review_state"] == "approved"
    });
    click_footer(&terminal, "Filter");
    std::thread::sleep(Duration::from_millis(100));
    click_footer(&terminal, "Reject");
    fixture.wait(&terminal, "narrow-filter-reject", || {
        fixture.point(1)["review_state"] == "rejected"
    });
    for _ in 0..4 {
        click_footer(&terminal, "Filter");
        std::thread::sleep(Duration::from_millis(100));
    }
    click_footer(&terminal, "Batch");
    fixture.wait(&terminal, "narrow-batch", || {
        fixture.point(2)["review_state"] == "approved"
    });
    let batch = fixture.records();
    click_footer(&terminal, "Continue");
    fixture.capture(&terminal, "narrow-continue-without-active-generation");
    assert_eq!(fixture.records(), batch);
    terminal.click_text("Objective:", 1000).unwrap();
    terminal.write("Z").unwrap();
    click_footer(&terminal, "Save");
    fixture.wait(&terminal, "narrow-save-resets-reviewed-intent", || {
        fixture.point(0)["objective"] == "OBJECTIVE_00Z"
            && fixture.point(0)["review_state"] == "proposed"
    });
    terminal.resize(150, 30).unwrap();
    fixture.capture(&terminal, "resized-normal");
    terminal.click_text("Control", 1000).unwrap();
    let before = fixture.records();
    let tree_before = pane(&terminal.screen().unwrap(), 0, 30);
    terminal.key("m").unwrap();
    terminal.expect_text("Model Profiles", true, 1000).unwrap();
    terminal.key("a").unwrap();
    terminal
        .expect_text("Add Model Profile", true, 1000)
        .unwrap();
    terminal.write("147-pointer-input").unwrap();
    terminal.click_text("Test Points [5]", 1000).unwrap();
    terminal.scroll_at(5, 8, false, false).unwrap();
    fixture.capture(&terminal, "modal-input-owned");
    terminal
        .expect_text("Add Model Profile", true, 1000)
        .unwrap();
    terminal
        .expect_text("147-pointer-input", true, 1000)
        .unwrap();
    terminal.key("Escape").unwrap();
    terminal.key("Escape").unwrap();
    fixture.capture(&terminal, "modal-closed");
    assert_eq!(fixture.records(), before);
    assert_eq!(pane(&terminal.screen().unwrap(), 0, 30), tree_before);
    fixture.finish(&terminal);
}
