//! CLI acceptance of a shared library with independent project associations.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(root: &Path, project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teshi"))
        .current_dir(project)
        .env("TESHI_APP_DATA_DIR", root.join("app-data"))
        .arg("--requirements-root")
        .arg(root.join("store"))
        .arg("requirements")
        .args(args)
        .output()
        .unwrap()
}

fn json(root: &Path, project: &Path, args: &[&str]) -> serde_json::Value {
    let output = run(root, project, args);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn project_membership_status_and_snapshot_roundtrip_through_cli() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let a = root.join("a");
    let b = root.join("b");
    let store = root.join("store");
    for path in [&a, &b, &store] {
        fs::create_dir(path).unwrap();
    }
    fs::write(store.join("one.md"), "# One\n\nShared source\n").unwrap();
    fs::write(store.join("two.md"), "# Two\n\nUnassigned source\n").unwrap();
    fs::write(
        store.join("_teshi.json"),
        serde_json::json!({
            "version": 2, "store_id": "reqstore-cli-projects", "documents": [
                {"id":"doc-1", "path":"one.md", "title":"One", "revision":"r1"},
                {"id":"doc-2", "path":"two.md", "title":"Two", "revision":"r2"}
            ]
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        json(root, &a, &["list", "--json"])["documents"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(!a.join(".teshi/project.json").exists());
    assert_eq!(
        json(root, &a, &["list", "--all-projects", "--json"])["documents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    json(root, &a, &["link-project", "doc-1", "--json"]);
    json(root, &b, &["link-project", "doc-1", "--json"]);
    json(
        root,
        &a,
        &[
            "set-project-status",
            "doc-1",
            "--status",
            "verified",
            "--json",
        ],
    );
    let a_id: serde_json::Value =
        serde_json::from_slice(&fs::read(a.join(".teshi/project.json")).unwrap()).unwrap();
    let b_id: serde_json::Value =
        serde_json::from_slice(&fs::read(b.join(".teshi/project.json")).unwrap()).unwrap();
    let listed = json(root, &a, &["list", "--json"]);
    assert_eq!(listed["documents"].as_array().unwrap().len(), 1);
    let projects = &listed["documents"][0]["projects"];
    assert_eq!(projects[a_id["id"].as_str().unwrap()], "verified");
    assert_eq!(projects[b_id["id"].as_str().unwrap()], "planned");
    assert_eq!(
        json(root, &b, &["list", "--unassigned-project", "--json"])["documents"][0]["id"],
        "doc-2"
    );
    let before = fs::read(store.join("_teshi.json")).unwrap();
    let failed = run(root, &a, &["link-project", "doc-2", "missing", "--json"]);
    assert!(!failed.status.success());
    assert_eq!(before, fs::read(store.join("_teshi.json")).unwrap());
    assert!(
        !run(
            root,
            &a,
            &[
                "set-project-status",
                "doc-2",
                "--status",
                "verified",
                "--json"
            ]
        )
        .status
        .success()
    );

    assert!(
        run(root, &a, &["export-project", "--output", "snapshot.json"])
            .status
            .success()
    );
    let exported = fs::read(a.join("snapshot.json")).unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&exported).unwrap();
    assert_eq!(snapshot["store_id"], "reqstore-cli-projects");
    assert_eq!(snapshot["documents"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["documents"][0]["body"], "# One\n\nShared source\n");
    assert_eq!(
        snapshot["documents"][0]["meta"]["projects"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !run(root, &a, &["export-project", "--output", "snapshot.json"])
            .status
            .success()
    );
    assert_eq!(exported, fs::read(a.join("snapshot.json")).unwrap());

    json(root, &a, &["unlink-project", "doc-1", "--json"]);
    assert!(
        json(root, &a, &["list", "--json"])["documents"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        json(root, &b, &["list", "--json"])["documents"][0]["id"],
        "doc-1"
    );
    let moved = root.join("moved-b");
    fs::rename(&b, &moved).unwrap();
    assert_eq!(
        json(root, &moved, &["list", "--json"])["documents"][0]["id"],
        "doc-1"
    );
    assert_eq!(
        fs::read_to_string(store.join("one.md")).unwrap(),
        "# One\n\nShared source\n"
    );
    assert!(!a.join("testpoints").exists());
}
