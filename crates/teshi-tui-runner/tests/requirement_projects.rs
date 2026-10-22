//! Real terminal acceptance of project-specific requirement views.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use teshi_tui_runner::{Launch, Terminal};

fn project_linked(project: &Path, store: &Path, document_id: &str) -> bool {
    let Ok(project_bytes) = fs::read(project.join(".teshi/project.json")) else {
        return false;
    };
    let project: serde_json::Value = serde_json::from_slice(&project_bytes).unwrap();
    let project_id = project["id"].as_str().unwrap();
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(store.join("_teshi.json")).unwrap()).unwrap();
    index["documents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|doc| doc["id"] == document_id)
        .and_then(|doc| doc["projects"].get(project_id))
        .is_some()
}

fn wait_for_project_link(project: &Path, store: &Path, document_id: &str, linked: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while project_linked(project, store, document_id) != linked {
        assert!(
            Instant::now() < deadline,
            "project link did not become {linked}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_project_document(project: &Path, store: &Path, document_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if project_linked(project, store, document_id) {
            let project_bytes = fs::read(project.join(".teshi/project.json")).unwrap();
            let project_json: serde_json::Value = serde_json::from_slice(&project_bytes).unwrap();
            let index: serde_json::Value =
                serde_json::from_slice(&fs::read(store.join("_teshi.json")).unwrap()).unwrap();
            let doc = index["documents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|doc| doc["id"] == document_id)
                .unwrap();
            assert_eq!(
                doc["projects"][project_json["id"].as_str().unwrap()],
                "planned"
            );
            let body = fs::read_to_string(store.join(doc["path"].as_str().unwrap())).unwrap();
            assert!(
                body.starts_with("# Document 1"),
                "unexpected body: {body:?}"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "document was not saved for the project"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn project_views_link_existing_sources_and_create_project_owned_documents() {
    let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let project = root.join("project");
    let store = root.join("store");
    fs::create_dir(&project).unwrap();
    fs::create_dir(&store).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    fs::write(
        store.join("legacy.md"),
        "# Legacy source\n\nKeep original body\n",
    )
    .unwrap();
    fs::write(
        store.join("_teshi.json"),
        serde_json::json!({
            "version": 2, "store_id": "reqstore-project-pty", "documents": [
                {"id":"legacy", "path":"legacy.md", "title":"Legacy source", "revision":"r1"}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let terminal = Terminal::launch(Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            store.to_string_lossy().into_owned(),
        ],
        cwd: Some(root.to_string_lossy().into_owned()),
        env: [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "TESHI_APP_DATA_DIR",
        ]
        .into_iter()
        .map(|name| (name.into(), root.join(name).to_string_lossy().into_owned()))
        .collect(),
        cols: 240,
        rows: 32,
    })
    .unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("4").unwrap();
    terminal
        .expect_text("Current project", true, 10000)
        .unwrap();
    terminal
        .expect_text("No requirements in this scope", true, 10000)
        .unwrap();
    terminal.expect_text("Legacy source", false, 10000).unwrap();
    assert!(!project.join(".teshi/project.json").exists());
    terminal.key("o").unwrap();
    terminal.expect_text("All projects", true, 10000).unwrap();
    terminal
        .expect_text("# Legacy source", true, 10000)
        .unwrap();
    terminal.key("b").unwrap();
    wait_for_project_link(&project, &store, "legacy", true);
    terminal.key("o").unwrap();
    terminal.expect_text("No project", true, 10000).unwrap();
    terminal.expect_text("Legacy source", false, 10000).unwrap();
    terminal.key("o").unwrap();
    terminal
        .expect_text("Current project", true, 10000)
        .unwrap();
    terminal
        .expect_text("# Legacy source", true, 10000)
        .unwrap();
    terminal.key("b").unwrap();
    wait_for_project_link(&project, &store, "legacy", false);
    terminal.expect_text("Legacy source", false, 10000).unwrap();
    terminal.key("Ctrl+n").unwrap();
    terminal.expect_text("# Document 1", true, 10000).unwrap();
    terminal.key("Ctrl+s").unwrap();
    wait_for_project_document(&project, &store, "doc-1");
    terminal.expect_text("Document 1 *", false, 10000).unwrap();

    if let Some(artifacts) = std::env::var_os("TESHI_REQUIREMENT_PROJECT_ARTIFACTS") {
        let dir = std::path::PathBuf::from(artifacts);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("project-view.txt"), terminal.screen().unwrap()).unwrap();
        terminal.screenshot(&dir.join("project-view.svg")).unwrap();
        fs::copy(store.join("_teshi.json"), dir.join("store-index.json")).unwrap();
        fs::copy(
            project.join(".teshi/project.json"),
            dir.join("project.json"),
        )
        .unwrap();
    }
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    let identity: serde_json::Value =
        serde_json::from_slice(&fs::read(project.join(".teshi/project.json")).unwrap()).unwrap();
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(store.join("_teshi.json")).unwrap()).unwrap();
    let docs = index["documents"].as_array().unwrap();
    let created = docs.iter().find(|doc| doc["id"] == "doc-1").unwrap();
    let legacy = docs.iter().find(|doc| doc["id"] == "legacy").unwrap();
    assert_eq!(
        created["projects"][identity["id"].as_str().unwrap()],
        "planned"
    );
    assert!(legacy.get("projects").is_none());
    assert_eq!(
        fs::read_to_string(store.join("legacy.md")).unwrap(),
        "# Legacy source\n\nKeep original body\n"
    );
}
