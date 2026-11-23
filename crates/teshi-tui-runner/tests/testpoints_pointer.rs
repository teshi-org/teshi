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
                "--recursive".into(),
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
            terminal
                .expect_text("Test Points MindMap", true, 10000)
                .unwrap();
        } else {
            terminal.click_text("Test Points [5]", 10000).unwrap();
            terminal.expect_text("Inspector", true, 10000).unwrap();
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
        terminal.key("ctrl+q").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        if terminal
            .screen()
            .unwrap_or_default()
            .contains("Quit Teshi?")
        {
            // The first Enter activates the default "Quit" choice.
            terminal.key("Enter").unwrap();
        }
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

/// Column where the right-hand stack starts, measured from the rendered pane
/// borders so the assertions follow the layout instead of a fixed geometry.
fn right_column(screen: &str) -> usize {
    let Some(line) = screen
        .lines()
        .find(|line| line.chars().filter(|&character| character == '╭').count() > 1)
    else {
        return 0;
    };
    let mut boxes = line
        .char_indices()
        .filter(|(_, character)| *character == '╭')
        .map(|(byte, _)| byte);
    boxes.next();
    boxes
        .next()
        .map(|byte| line[..byte].chars().count())
        .unwrap_or(0)
}

fn region(screen: &str, x0: usize, x1: usize, y0: usize, y1: usize) -> String {
    screen
        .lines()
        .skip(y0)
        .take(y1.saturating_sub(y0))
        .map(|line| {
            line.chars()
                .skip(x0)
                .take(x1.saturating_sub(x0))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click_action(terminal: &Terminal, text: &str) {
    let screen = terminal.screen().unwrap();
    let rows: Vec<&str> = screen.lines().collect();
    let (row, line) = rows
        .iter()
        .enumerate()
        .rev()
        .find(|(_, line)| line.contains(text))
        .unwrap_or_else(|| panic!("action control {text:?} is absent:\n{screen}"));
    let index = line.find(text).unwrap();
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
    let right = right_column(&before);
    let width = before.lines().next().map_or(0, |line| line.chars().count());
    let body = (3, before.lines().count().saturating_sub(1));
    let wheel = |column: usize, row: usize| {
        terminal
            .scroll_at(column as u16, row as u16, false, false)
            .unwrap()
    };
    // Wheel on the right Inspector pane leaves the 2D MindMap canvas untouched.
    wheel(right + 5, 8);
    fixture.capture(&terminal, "inspector-wheel");
    let inspector = terminal.screen().unwrap();
    assert_eq!(
        region(&before, 0, right, body.0, body.1),
        region(&inspector, 0, right, body.0, body.1)
    );
    assert_ne!(
        region(&before, right, width, body.0, body.1),
        region(&inspector, right, width, body.0, body.1)
    );
    // Wheel on the 2D MindMap canvas scrolls it while leaving the Inspector untouched.
    for _ in 0..8 {
        wheel(10, 8);
    }
    fixture.capture(&terminal, "canvas-wheel");
    let canvas = terminal.screen().unwrap();
    assert_eq!(
        region(&inspector, right, width, body.0, body.1),
        region(&canvas, right, width, body.0, body.1)
    );
    assert_ne!(
        region(&inspector, 0, right, body.0, body.1),
        region(&canvas, 0, right, body.0, body.1)
    );
    for _ in 0..20 {
        if terminal.screen().unwrap().contains("POINT_08") {
            break;
        }
        wheel(10, 8);
    }
    terminal.click_text("POINT_08", 1000).unwrap();
    fixture.wait(&terminal, "tree-row-selected", || {
        terminal.screen().unwrap().contains("Title: POINT_08")
    });
    let svg = fs::read_to_string(fixture.output.join("tree-row-selected.svg")).unwrap();
    assert!(
        svg.split("<text ").any(|fragment| {
            let Some((attributes, body)) = fragment.split_once('>') else {
                return false;
            };
            let Some((text, _)) = body.split_once("</text>") else {
                return false;
            };
            text.contains("POINT_08") && attributes.contains("font-weight=\"bold\"")
        }),
        "clicked canvas node has no visible bold selection in SVG"
    );
    fixture.finish(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_field_pointer_edits_the_clicked_field_at_normal_and_narrow_sizes() {
    for (cols, rows) in [(150, 30), (55, 16)] {
        for (label, key, before) in [
            ("Note:", "objective", "OBJECTIVE_00"),
            ("Preconditions:", "preconditions", "PRE_00"),
            ("Expected:", "expected_outcomes", "EXPECTED_00"),
            ("Title:", "title", "POINT_00"),
            ("Path:", "hierarchy_path", "Control"),
        ] {
            let fixture = Fixture::new(&format!("field-{key}-{cols}"), cols, rows);
            let terminal = fixture.launch();
            if cols < 60 {
                // Below 70 columns only the focused pane renders; switch to Inspector.
                terminal.key("Tab").unwrap();
                terminal.expect_text(label, true, 2000).unwrap();
            }
            terminal.click_text(label, 1000).unwrap();
            // In narrow mode `Tab` already focused Details with `Title` selected,
            // so clicking `Title:` enters INSERT immediately; other fields select first.
            if !(cols < 60 && key == "title") {
                terminal.key("i").unwrap();
            }
            terminal.expect_text("[INSERT]", true, 2000).unwrap();
            terminal.key("End").unwrap();
            terminal.write("Z").unwrap();
            terminal.key("Escape").unwrap();
            // Wait for the editor to leave INSERT before the NORMAL save key.
            terminal
                .expect_text("Inspector [NORMAL]", true, 2000)
                .unwrap();
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
fn testpoint_insert_keeps_command_text_and_saves_without_approving_or_switching_tabs() {
    for cols in [150, 55] {
        let fixture = Fixture::new(&format!("vim-input-{cols}"), cols, 30);
        let terminal = fixture.launch();
        if cols < 60 {
            terminal.key("Tab").unwrap();
            terminal.expect_text("Title:", true, 2000).unwrap();
        }
        terminal.click_text("Title:", 2000).unwrap();
        if cols >= 60 {
            terminal.key("i").unwrap();
        }
        terminal.expect_text("[INSERT]", true, 2000).unwrap();
        terminal.key("End").unwrap();
        let typed = " hjkl ars q12345 中文🙂尾";
        terminal.write(typed).unwrap();
        terminal.key("Left").unwrap();
        terminal.key("Backspace").unwrap();
        terminal.key("Delete").unwrap();
        terminal.write("中").unwrap();
        terminal.key("Ctrl+s").unwrap();
        let expected = "POINT_00 hjkl ars q12345 中文中";
        fixture.wait(&terminal, "insert-saved", || {
            fixture.point(0)["title"] == expected
        });
        assert_eq!(fixture.point(0)["review_state"], "proposed");
        terminal.expect_text("[INSERT]", true, 2000).unwrap();
        terminal.key("Escape").unwrap();
        terminal.expect_text("[NORMAL]", true, 2000).unwrap();
        // Step focus back to the 2D MindMap canvas and verify Normal-mode navigation.
        terminal.key("Shift+Tab").unwrap();
        terminal
            .expect_text("Test Points MindMap", true, 2000)
            .unwrap();
        if cols >= 60 {
            terminal.key("End").unwrap();
            fixture.wait(&terminal, "tree-last", || {
                terminal.screen().unwrap().contains("POINT_40")
            });
            terminal.key("Home").unwrap();
            fixture.wait(&terminal, "tree-first", || {
                let screen = terminal.screen().unwrap();
                screen.contains("Title:") && screen.contains(expected)
            });
        } else {
            terminal.key("j").unwrap();
            terminal.key("Tab").unwrap();
            fixture.wait(&terminal, "tree-next", || {
                terminal.screen().unwrap().contains("Title: POINT_01")
            });
            terminal.key("Shift+Tab").unwrap();
            terminal.key("k").unwrap();
            terminal.key("Tab").unwrap();
            fixture.wait(&terminal, "tree-first", || {
                let screen = terminal.screen().unwrap();
                screen.contains("Title:") && screen.contains(expected)
            });
        }
        fixture.finish(&terminal);
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_excerpt_and_realized_scenario_rows_select_before_navigation() {
    let fixture = Fixture::new("excerpt-scenario", 150, 30);
    let terminal = fixture.launch();
    terminal.resize(95, 24).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let resized = terminal.screen().unwrap();
    let inspector_x = (right_column(&resized) + 5) as u16;
    for _ in 0..30 {
        if terminal.screen().unwrap().contains("SOURCE_05") {
            break;
        }
        terminal.scroll_at(inspector_x, 10, false, false).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    fixture.capture(&terminal, "three-pane-resized-and-excerpts-scrolled");
    terminal.click_text("SOURCE_05", 1000).unwrap();
    fixture.capture(&terminal, "excerpt-selected");
    terminal.key("Enter").unwrap();
    fixture.wait(&terminal, "excerpt-opened", || {
        terminal.screen().unwrap().contains("# SOURCE_05")
    });
    terminal.resize(150, 30).unwrap();
    terminal.click_text("Test Points [5]", 1000).unwrap();
    terminal.expect_text("Inspector", true, 1000).unwrap();
    for _ in 0..20 {
        if terminal.screen().unwrap().contains("REFERENCE_05") {
            break;
        }
        terminal.scroll_at(120, 10, true, false).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    terminal.expect_text("REFERENCE_05", true, 1000).unwrap();
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
    click_action(&terminal, "Filter");
    fixture.wait(&terminal, "filter-uncovered", || {
        terminal
            .screen()
            .unwrap()
            .contains("Test Points MindMap (uncovered)")
    });
    assert!(!pane(&terminal.screen().unwrap(), 0, 90).contains("POINT_00"));
    for label in ["linked", "passed", "failed", "all"] {
        click_action(&terminal, "Filter");
        terminal
            .expect_text(&format!("Test Points MindMap ({label})"), true, 3000)
            .unwrap();
    }
    terminal.click_text("POINT_01", 1000).unwrap();
    terminal.expect_text("Title: POINT_01", true, 2000).unwrap();
    terminal.click_text("Note:", 1000).unwrap();
    terminal.key("i").unwrap();
    terminal.key("End").unwrap();
    terminal.write("Z").unwrap();
    terminal.key("Escape").unwrap();
    terminal
        .expect_text("Inspector [NORMAL]", true, 2000)
        .unwrap();
    terminal.key("s").unwrap();
    fixture.wait(&terminal, "save", || {
        fixture.point(1)["objective"] == "OBJECTIVE_01Z"
    });
    // Canvas `+Child` adds a node and `u` undoes it cleanly.
    terminal.key("Shift+Tab").unwrap();
    click_action(&terminal, "+Child");
    terminal.write("CHILD_NEW").unwrap();
    terminal.key("Enter").unwrap();
    fixture.wait(&terminal, "add-child", || {
        fixture.records()["test_points"]
            .as_array()
            .is_some_and(|items| items.len() == 42)
    });
    terminal.key("u").unwrap();
    fixture.wait(&terminal, "undo-child", || {
        fixture.records()["test_points"]
            .as_array()
            .is_some_and(|items| items.len() == 41)
    });
    fixture.finish(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn testpoint_narrow_review_controls_resize_and_modal_input_own_the_pointer() {
    let fixture = Fixture::new("narrow-modal", 150, 30);
    let terminal = fixture.launch();
    terminal.resize(55, 16).unwrap();
    terminal
        .expect_text("Test Points MindMap", true, 3000)
        .unwrap();
    fixture.capture(&terminal, "narrow");
    let screen = terminal.screen().unwrap();
    for control in ["+Child", "+Sib", "Edit", "Del"] {
        assert!(
            screen.contains(control),
            "narrow layout missing {control}:\n{screen}"
        );
    }
    for label in ["uncovered", "linked", "passed", "failed", "all"] {
        terminal.key("f").unwrap();
        terminal
            .expect_text(&format!("Test Points MindMap ({label})"), true, 3000)
            .unwrap();
    }
    // Re-select a leaf node from Root after empty filters reset selection to Root.
    terminal.key("Right").unwrap();
    terminal.key("Right").unwrap();
    // Below 70 columns only the focused pane is drawn; focus the Inspector.
    terminal.key("Tab").unwrap();
    terminal.expect_text("Note:", true, 3000).unwrap();
    let inspected_id = terminal
        .screen()
        .unwrap()
        .lines()
        .find_map(|line| {
            let (_, rest) = line.split_once("id: ")?;
            rest.split_whitespace().next().map(str::to_string)
        })
        .expect("inspected test point id missing");
    let inspected_idx: usize = inspected_id
        .strip_prefix("point-")
        .and_then(|s| s.parse().ok())
        .expect("numeric test point id");
    let expected_objective = format!("OBJECTIVE_{inspected_idx:02}Z");
    terminal.click_text("Note:", 1000).unwrap();
    terminal.key("i").unwrap();
    terminal.key("End").unwrap();
    terminal.write("Z").unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("[NORMAL]", true, 2000).unwrap();
    terminal.key("s").unwrap();
    fixture.wait(&terminal, "narrow-save-persisted", || {
        fixture.point(inspected_idx)["objective"] == expected_objective
    });
    terminal.resize(150, 30).unwrap();
    fixture.capture(&terminal, "resized-normal");
    let before = fixture.records();
    let canvas_before = pane(&terminal.screen().unwrap(), 0, 90);
    terminal.key("m").unwrap();
    terminal.expect_text("Model Profiles", true, 1000).unwrap();
    terminal.key("a").unwrap();
    terminal
        .expect_text("Add Model Profile", true, 1000)
        .unwrap();
    terminal.write("147-pointer-input").unwrap();
    terminal.scroll_at(5, 8, false, false).unwrap();
    fixture.capture(&terminal, "modal-input-owned");
    terminal
        .expect_text("Add Model Profile", true, 1000)
        .unwrap();
    terminal
        .expect_text("147-pointer-input", true, 1000)
        .unwrap();
    // Backdrop click on the tab bar cancels the Add form back to Model Profiles,
    // and a second backdrop click dismisses Model Profiles without switching tabs.
    terminal.click_text("Test Points [5]", 1000).unwrap();
    terminal
        .expect_text("Add Model Profile", false, 2000)
        .unwrap();
    terminal.expect_text("Model Profiles", true, 2000).unwrap();
    terminal.click_text("Test Points [5]", 1000).unwrap();
    terminal.expect_text("Model Profiles", false, 2000).unwrap();
    terminal
        .expect_text("Test Points MindMap", true, 2000)
        .unwrap();
    fixture.capture(&terminal, "modal-closed");
    assert_eq!(fixture.records(), before);
    assert_eq!(pane(&terminal.screen().unwrap(), 0, 90), canvas_before);
    fixture.finish(&terminal);
}
