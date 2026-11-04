//! Minimal real-pointer regressions for existing management controls.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use teshi_tui_runner::{Launch, Terminal};

struct Fixture {
    _root: tempfile::TempDir,
    launch: Launch,
    profiles: PathBuf,
    source: PathBuf,
    artifacts: PathBuf,
}

#[derive(Clone, Copy)]
enum PointerResult<'a> {
    Visible(&'a str),
    Hidden(&'a str),
    Selected(&'a str),
}

impl Fixture {
    fn new(name: &str, narrow: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("management-project");
        fs::create_dir(&project).unwrap();
        let source = project.join("sample.feature");
        fs::write(&source, "Feature: Management fixture\n").unwrap();
        let profiles = root.path().join("TESHI_APP_DATA_DIR/model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::write(profiles.join("Alpha.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "id":"Alpha", "name":"Existing Alpha", "provider":"openai", "model_id":"fixture-alpha", "api_key":"", "base_url":"http://127.0.0.1:1/v1"
        })).unwrap()).unwrap();
        fs::write(profiles.join("active"), "Alpha").unwrap();
        for (id, label) in [
            ("pointer-alpha", "Agent Alpha"),
            ("pointer-beta", "Agent Beta"),
        ] {
            let directory = project.join(".teshi/agents").join(id);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("agent.yaml"), format!("id: {id}\nname: {label}\ndescription: {} desc\nsystem_prompt: system.md\nmodel_ref: ''\n", if id.ends_with("alpha") { "Alpha" } else { "Beta" })).unwrap();
            fs::write(directory.join("system.md"), "Local pointer fixture.").unwrap();
        }
        let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
        let launch = Launch {
            program: binary.to_string_lossy().into_owned(),
            args: vec![
                "--recursive".into(),
                project.to_string_lossy().into_owned(),
                "--requirements-root".into(),
                root.path()
                    .join("requirements")
                    .to_string_lossy()
                    .into_owned(),
            ],
            cwd: Some(root.path().to_string_lossy().into_owned()),
            env: [
                ("TERM".into(), "xterm-256color".into()),
                ("TESHI_AGENT_BACKEND".into(), "native".into()),
                (
                    "TESHI_TEST_FRAME_PATH".into(),
                    root.path()
                        .join("completed-frame.json")
                        .to_string_lossy()
                        .into_owned(),
                ),
            ]
            .into_iter()
            .chain(
                [
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
                }),
            )
            .collect(),
            cols: if narrow { 55 } else { 150 },
            rows: if narrow { 16 } else { 30 },
        };
        let artifacts =
            PathBuf::from(std::env::var_os("TESHI_MANAGEMENT_POINTER_ARTIFACTS").unwrap()).join(
                format!("{name}-{}", if narrow { "narrow" } else { "normal" }),
            );
        fs::create_dir_all(&artifacts).unwrap();
        Self {
            _root: root,
            launch,
            profiles,
            source,
            artifacts,
        }
    }

    fn launch(&self) -> Terminal {
        let terminal = Terminal::launch(self.launch.clone()).unwrap();
        terminal.expect_text("sample", true, 10000).unwrap();
        terminal
    }

    fn capture(&self, terminal: &Terminal, name: &str) {
        // Business readiness belongs to the operation's explicit postcondition.
        let receipt = self._root.path().join("completed-frame.json");
        if let Ok(bytes) = fs::read(receipt) {
            fs::write(
                self.artifacts.join(format!("{name}-completed-frame.json")),
                bytes,
            )
            .unwrap();
        }
        fs::write(
            self.artifacts.join(format!("{name}.txt")),
            terminal.screen().unwrap(),
        )
        .unwrap();
        terminal
            .screenshot(&self.artifacts.join(format!("{name}.svg")))
            .unwrap();
    }

    fn click(&self, terminal: &Terminal, labels: &[&str], name: &str, result: PointerResult<'_>) {
        self.capture(terminal, &format!("{name}-before"));
        // A panel paints its frame before its controls, so wait for the label
        // instead of reading the first screen that shows the panel title.
        let deadline = Instant::now() + Duration::from_secs(5);
        let label = loop {
            let screen = terminal.screen().unwrap();
            if let Some(label) = labels.iter().find(|label| screen.contains(**label)) {
                break (*label).to_string();
            }
            assert!(
                Instant::now() < deadline,
                "missing control {labels:?}:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        terminal.click_text(&label, 1500).unwrap();
        let outcome = terminal.expect_screen(name, 20000, |screen| match result {
            PointerResult::Visible(text) => screen.contains(text),
            PointerResult::Hidden(text) => !screen.contains(text),
            PointerResult::Selected(text) => screen
                .lines()
                .any(|line| line.contains(text) && line.contains('▶')),
        });
        self.capture(terminal, &format!("{name}-after"));
        outcome.unwrap();
    }

    fn finish(&self, terminal: Terminal) {
        assert_eq!(
            fs::read_to_string(&self.source).unwrap(),
            "Feature: Management fixture\n"
        );
        terminal.key("Escape").unwrap();
        terminal.key("Escape").unwrap();
        terminal.key("ctrl+q").unwrap();
        terminal.expect_exit(0, 10000).unwrap();
    }

    fn profile_count(&self) -> usize {
        fs::read_dir(&self.profiles)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .count()
    }

    fn seed_installed(&self) -> PathBuf {
        let store = self._root.path().join("TESHI_ACP_STORE_DIR");
        fs::create_dir_all(&store).unwrap();
        let provider = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/wrapped_chat_acp.py")
            .canonicalize()
            .unwrap();
        fs::write(store.join("installed.json"), serde_json::to_vec_pretty(&serde_json::json!([
            {"id":"installed-alpha", "name":"ACP Alpha", "program":"python", "args":[provider], "env":{}},
            {"id":"installed-beta", "name":"ACP Beta", "program":"python", "args":[provider], "env":{}}
        ])).unwrap()).unwrap();
        store
    }
}

struct OwnedRegistry {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl OwnedRegistry {
    fn new() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let request_log = Arc::clone(&requests);
        let stop_worker = Arc::clone(&stop);
        let payload = serde_json::to_vec(&serde_json::json!({"agents":[
            {"id":"installed-alpha", "name":"ACP Alpha", "description":"Registry alpha description", "distribution":{"binary":{"windows-x86_64":{"archive":format!("http://{address}/archive/alpha.zip"),"cmd":"agent.exe"}}}},
            {"id":"available-beta", "name":"Registry Beta", "description":"Registry beta description", "distribution":{"binary":{"windows-x86_64":{"archive":format!("http://{address}/archive/beta.zip"),"cmd":"agent.exe"}}}}
        ]})).unwrap();
        let worker = std::thread::spawn(move || {
            while !stop_worker.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut connection, _)) => {
                        connection
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request_bytes = Vec::new();
                        let mut buffer = [0u8; 1024];
                        while request_bytes.len() < 8192 {
                            match connection.read(&mut buffer) {
                                Ok(0) | Err(_) => break,
                                Ok(count) => request_bytes.extend_from_slice(&buffer[..count]),
                            }
                            if request_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                        let request = String::from_utf8_lossy(&request_bytes);
                        let first_line = request.lines().next().unwrap_or("").to_string();
                        request_log.lock().unwrap().push(first_line.clone());
                        let (status, body) = if first_line.starts_with("GET /registry.json ") {
                            ("200 OK", payload.as_slice())
                        } else {
                            (
                                "404 Not Found",
                                b"unexpected registry fixture request".as_slice(),
                            )
                        };
                        let headers = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = connection.write_all(headers.as_bytes());
                        let _ = connection.write_all(body);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("owned registry accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for OwnedRegistry {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn command(terminal: &Terminal, command: &str) {
    terminal.key("3").unwrap();
    terminal.expect_text("AI Chat", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.write(command).unwrap();
    terminal.expect_text(command, true, 5000).unwrap();
    terminal.key("Enter").unwrap();
}

fn click_field(terminal: &Terminal, name: &str) {
    let screen = terminal.screen().unwrap();
    let (row, col) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            if line.contains("Profile") {
                return None;
            }
            line.find(name)
                .map(|col| (row + 1, line[..col].chars().count() + 1))
        })
        .unwrap_or_else(|| panic!("missing field {name}:\n{screen}"));
    terminal
        .write(&format!("\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m"))
        .unwrap();
    let screen = terminal
        .expect_screen("clicked field is focused", 3000, |screen| {
            screen
                .lines()
                .any(|line| line.contains(name) && line.contains('▸'))
        })
        .unwrap();
    assert!(
        screen
            .lines()
            .any(|line| line.contains(name) && line.contains('▸')),
        "click did not focus {name}:\n{screen}"
    );
}

fn model_list_add_control(narrow: bool) {
    let fixture = Fixture::new("model-add", narrow);
    fs::write(fixture.profiles.join("Beta.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "id":"Beta", "name":"Existing Beta", "provider":"openai", "model_id":"fixture-beta", "api_key":"", "base_url":"http://127.0.0.1:1/v1"
    })).unwrap()).unwrap();
    fs::write(fixture.profiles.join("active"), "Beta").unwrap();
    let terminal = fixture.launch();
    terminal.key("m").unwrap();
    terminal.expect_text("Model Profiles", true, 1500).unwrap();
    fixture.click(
        &terminal,
        &["Add [a]", "Add[a]", "a add"],
        "add-pointer",
        PointerResult::Visible("Add Model Profile"),
    );
    terminal
        .expect_text("Add Model Profile", true, 1500)
        .unwrap();
    fixture.click(
        &terminal,
        &["Cancel [Esc]"],
        "cancel-add",
        PointerResult::Hidden("Add Model Profile"),
    );
    terminal
        .expect_text("Add Model Profile", false, 1500)
        .unwrap();
    assert_eq!(fixture.profile_count(), 2);
    fixture.click(
        &terminal,
        &["Edit [e]", "Edit[e]"],
        "edit-alpha",
        PointerResult::Visible("Edit Model Profile"),
    );
    terminal
        .expect_text("Edit Model Profile", true, 1500)
        .unwrap();
    click_field(&terminal, "Name");
    terminal.write(" edited").unwrap();
    fixture.click(
        &terminal,
        &["Save [Enter]"],
        "save-edited-alpha",
        PointerResult::Hidden("Edit Model Profile"),
    );
    terminal
        .expect_text("Edit Model Profile", false, 1500)
        .unwrap();
    let profile: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.profiles.join("Alpha.json")).unwrap()).unwrap();
    assert_eq!(profile["name"], "Existing Alpha edited");
    assert_eq!(profile["id"], "Alpha");
    fixture.click(
        &terminal,
        &["Use [Enter]", "Use[Enter]"],
        "use-edited-alpha",
        PointerResult::Hidden("Model Profiles"),
    );
    terminal.expect_text("Model Profiles", false, 1500).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.profiles.join("active"))
            .unwrap()
            .trim(),
        "Alpha"
    );
    terminal.key("m").unwrap();
    terminal.expect_text("Model Profiles", true, 1500).unwrap();
    fixture.click(
        &terminal,
        &["Del [dd/Del]"],
        "delete-edited-alpha",
        PointerResult::Visible("Confirm deletion"),
    );
    terminal
        .expect_text("Confirm deletion", true, 2000)
        .unwrap();
    assert!(fixture.profiles.join("Alpha.json").exists());
    fixture.click(
        &terminal,
        &["Cancel [Esc/n]"],
        "cancel-delete-alpha",
        PointerResult::Hidden("Confirm deletion"),
    );
    assert!(fixture.profiles.join("Alpha.json").exists());
    fixture.click(
        &terminal,
        &["Del [dd/Del]"],
        "request-delete-alpha-again",
        PointerResult::Visible("Confirm deletion"),
    );
    fixture.click(
        &terminal,
        &["Delete [Enter/y]"],
        "confirm-delete-alpha",
        PointerResult::Hidden("Confirm deletion"),
    );
    assert!(!fixture.profiles.join("Alpha.json").exists());
    assert!(fixture.profiles.join("Beta.json").is_file());
    assert_eq!(fixture.profile_count(), 1);
    assert_eq!(
        fs::read_to_string(fixture.profiles.join("active"))
            .unwrap()
            .trim(),
        "Beta"
    );
    fixture.click(
        &terminal,
        &["Close [Esc]", "Close[Esc]"],
        "close-model-list",
        PointerResult::Hidden("Model Profiles"),
    );
    terminal.expect_text("Model Profiles", false, 1500).unwrap();
    fixture.finish(terminal);
}

fn model_form_fields_save_cancel(narrow: bool) {
    let fixture = Fixture::new("model-form", narrow);
    let terminal = fixture.launch();
    terminal.key("m").unwrap();
    terminal.key("a").unwrap();
    terminal
        .expect_text("Add Model Profile", true, 1500)
        .unwrap();
    fixture.capture(&terminal, "form-open");
    click_field(&terminal, "Provider");
    terminal.write("openai").unwrap();
    click_field(&terminal, "Name");
    terminal.write("Pointer model").unwrap();
    click_field(&terminal, "Model");
    terminal.write("fixture-model").unwrap();
    click_field(&terminal, "Base URL");
    terminal.write("http://127.0.0.1:1/v1").unwrap();
    click_field(&terminal, "API Key");
    click_field(&terminal, "Max Tokens");
    for _ in "4096".chars() {
        terminal.key("Backspace").unwrap();
    }
    terminal.write("3072").unwrap();
    click_field(&terminal, "Temperature");
    for _ in "0.7".chars() {
        terminal.key("Backspace").unwrap();
    }
    terminal.write("0.42").unwrap();
    fixture.capture(&terminal, "fields-focused");
    fixture.click(
        &terminal,
        &["Save [Enter]"],
        "save-profile",
        PointerResult::Hidden("Add Model Profile"),
    );
    terminal
        .expect_text("Add Model Profile", false, 1500)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records: Vec<serde_json::Value> = fs::read_dir(&fixture.profiles)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path
                    .extension()
                    .is_some_and(|extension| extension == "json"))
                .then(|| serde_json::from_slice(&fs::read(path).unwrap()).unwrap())
            })
            .collect();
        if let Some(profile) = records
            .iter()
            .find(|profile| profile["name"] == "Pointer model")
        {
            assert_eq!(profile["provider"], "openai");
            assert_eq!(profile["model_id"], "fixture-model");
            assert_eq!(profile["base_url"], "http://127.0.0.1:1/v1");
            assert_eq!(profile["api_key"], "");
            assert_eq!(profile["max_output_tokens"], 3072);
            assert!((profile["chat_options"]["temperature"].as_f64().unwrap() - 0.42).abs() < 1e-6);
            fs::write(
                fixture.artifacts.join("saved-profile.json"),
                serde_json::to_vec_pretty(profile).unwrap(),
            )
            .unwrap();
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pointer save did not persist a profile"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    terminal.key("a").unwrap();
    terminal
        .expect_text("Add Model Profile", true, 1500)
        .unwrap();
    fixture.click(
        &terminal,
        &["Cancel [Esc]"],
        "cancel-second-add",
        PointerResult::Hidden("Add Model Profile"),
    );
    terminal
        .expect_text("Add Model Profile", false, 1500)
        .unwrap();
    assert_eq!(fixture.profile_count(), 2);
    fixture.finish(terminal);
}

fn auth_overview_close(narrow: bool) {
    let fixture = Fixture::new("auth-close", narrow);
    let terminal = fixture.launch();
    command(&terminal, "/auth");
    terminal.expect_text("Auth Management", true, 1500).unwrap();
    fixture.click(
        &terminal,
        &["Close [Esc]", "Esc to close"],
        "close-auth",
        PointerResult::Hidden("Auth Management"),
    );
    terminal
        .expect_text("Auth Management", false, 1500)
        .unwrap();
    fixture.finish(terminal);
}

fn click_sidebar(terminal: &Terminal, text: &str) {
    // The panel opens before its sidebar rows are listed, so wait for the row
    // instead of reading the first screen that shows the title.
    terminal
        .expect_text(text, true, 5000)
        .unwrap_or_else(|error| panic!("missing sidebar text {text}: {error}"));
    let screen = terminal.screen().unwrap();
    let (col, row) = screen
        .lines()
        .enumerate()
        .flat_map(|(row, line)| {
            line.match_indices(text)
                .map(move |(col, _)| (line[..col].chars().count() + 1, row + 1))
        })
        .min()
        .unwrap_or_else(|| panic!("missing sidebar text {text}:\n{screen}"));
    terminal
        .write(&format!("\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m"))
        .unwrap();
}

fn agent_profile_sidebar_apply(narrow: bool) {
    let fixture = Fixture::new("agent-profile", narrow);
    let terminal = fixture.launch();
    command(&terminal, "/agent-profile");
    terminal
        .expect_text("Agent Configuration", true, 1500)
        .unwrap();
    click_sidebar(&terminal, "Agent Beta");
    terminal
        .expect_text("Name: Agent Beta", true, 1500)
        .unwrap();
    fixture.capture(&terminal, "beta-name-selected");
    click_sidebar(&terminal, "Alpha desc");
    terminal
        .expect_text("Name: Agent Alpha", true, 1500)
        .unwrap();
    fixture.capture(&terminal, "alpha-description-selected");
    click_sidebar(&terminal, "Beta desc");
    terminal
        .expect_text("Name: Agent Beta", true, 1500)
        .unwrap();
    fixture.click(
        &terminal,
        &["Apply [Enter]", "Enter apply"],
        "apply-beta-profile",
        PointerResult::Hidden("Agent Configuration"),
    );
    terminal
        .expect_text("Agent Configuration", false, 1500)
        .unwrap();
    terminal.write("/agent-profile").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Name: Agent Beta", true, 1500)
        .unwrap();
    fixture.capture(&terminal, "reopened-active-profile");
    fixture.finish(terminal);
}

fn picker_row_selected(terminal: &Terminal, name: &str) -> bool {
    terminal
        .screen()
        .unwrap()
        .lines()
        .any(|line| line.contains(name) && line.contains('▶'))
}

fn backend_picker(narrow: bool) {
    let fixture = Fixture::new("backend-picker", narrow);
    let store = fixture.seed_installed();
    let installed = fs::read(store.join("installed.json")).unwrap();
    let terminal = fixture.launch();
    command(&terminal, "/agent");
    terminal
        .expect_text("Select Agent for this session", true, 1500)
        .unwrap();
    fixture.click(
        &terminal,
        &["ACP Beta"],
        "select-acp-beta",
        PointerResult::Selected("ACP Beta"),
    );
    assert!(picker_row_selected(&terminal, "ACP Beta"));
    fixture.click(
        &terminal,
        &["Use [Enter]"],
        "use-acp-beta",
        PointerResult::Hidden("Select Agent for this session"),
    );
    terminal
        .expect_text("Select Agent for this session", false, 1500)
        .unwrap();
    terminal.write("/agent").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Select Agent for this session", true, 1500)
        .unwrap();
    assert!(
        terminal
            .screen()
            .unwrap()
            .lines()
            .any(|line| line.contains("ACP Beta") && line.contains("[selected]"))
    );
    fixture.click(
        &terminal,
        &["Teshi Native"],
        "select-native",
        PointerResult::Selected("Teshi Native"),
    );
    assert!(picker_row_selected(&terminal, "Teshi Native"));
    fixture.click(
        &terminal,
        &["Use [Enter]"],
        "use-native",
        PointerResult::Hidden("Select Agent for this session"),
    );
    terminal
        .expect_text("Select Agent for this session", false, 1500)
        .unwrap();
    assert_eq!(fs::read(store.join("installed.json")).unwrap(), installed);
    assert!(!store.join("packages").exists());
    fixture.finish(terminal);
}

fn installed_picker(narrow: bool) {
    let fixture = Fixture::new("installed-picker", narrow);
    let store = fixture.seed_installed();
    let installed = fs::read(store.join("installed.json")).unwrap();
    let terminal = fixture.launch();
    command(&terminal, "/acp");
    terminal
        .expect_text("Installed ACP Agents", true, 1500)
        .unwrap();
    fixture.click(
        &terminal,
        &["ACP Beta"],
        "installed-select-beta",
        PointerResult::Selected("ACP Beta"),
    );
    assert!(picker_row_selected(&terminal, "ACP Beta"));
    fixture.click(
        &terminal,
        &["Close [Esc]"],
        "installed-close",
        PointerResult::Hidden("Installed ACP Agents"),
    );
    terminal
        .expect_text("Installed ACP Agents", false, 1500)
        .unwrap();
    assert_eq!(fs::read(store.join("installed.json")).unwrap(), installed);
    assert!(!store.join("packages").exists());
    fixture.finish(terminal);
}

fn registry_picker(narrow: bool) {
    let registry = OwnedRegistry::new();
    let mut fixture = Fixture::new("registry-picker", narrow);
    let store = fixture.seed_installed();
    let installed = fs::read(store.join("installed.json")).unwrap();
    fixture.launch.env.extend([
        (
            "TESHI_ACP_REGISTRY_URL".into(),
            format!("http://{}/registry.json", registry.address),
        ),
        ("NO_PROXY".into(), "127.0.0.1,localhost,::1".into()),
    ]);
    let terminal = fixture.launch();
    command(&terminal, "/acp");
    terminal
        .expect_text("Installed ACP Agents", true, 1500)
        .unwrap();
    fixture.click(
        &terminal,
        &["Registry [i]"],
        "open-registry",
        PointerResult::Visible("Registry Beta"),
    );
    // The picker waits on a real registry fetch; a loaded runner needs slack.
    let registry_visible = terminal.expect_text("Registry Beta", true, 20000);
    fs::write(
        fixture.artifacts.join("registry-fetch-requests.json"),
        serde_json::to_vec_pretty(&registry.requests.lock().unwrap().clone()).unwrap(),
    )
    .unwrap();
    fixture.capture(&terminal, "registry-fetch-result");
    registry_visible.unwrap();
    fixture.click(
        &terminal,
        &["Registry Beta"],
        "registry-select-beta",
        PointerResult::Selected("Registry Beta"),
    );
    assert!(picker_row_selected(&terminal, "Registry Beta"));
    fixture.click(
        &terminal,
        &["Registry beta description"],
        "registry-description-select",
        PointerResult::Selected("Registry Beta"),
    );
    assert!(picker_row_selected(&terminal, "Registry Beta"));
    fixture.click(
        &terminal,
        &["Close [Esc]"],
        "registry-close",
        PointerResult::Hidden("Official ACP Registry"),
    );
    terminal
        .expect_text("Official ACP Registry", false, 1500)
        .unwrap();
    assert_eq!(fs::read(store.join("installed.json")).unwrap(), installed);
    assert!(!store.join("packages").exists());
    let requests = registry.requests.lock().unwrap().clone();
    assert!(
        !requests.is_empty(),
        "the registry must use the owned fixture URL"
    );
    assert!(
        requests
            .iter()
            .all(|request| request == "GET /registry.json HTTP/1.1"),
        "registry row selection unexpectedly requested a download: {requests:?}"
    );
    fs::write(
        fixture.artifacts.join("registry-requests.json"),
        serde_json::to_vec_pretty(&requests).unwrap(),
    )
    .unwrap();
    fixture.finish(terminal);
}

macro_rules! acceptance {
    ($name:ident, $function:ident, $narrow:expr) => {
        #[test]
        #[ignore = "requires TESHI_SELF_TEST_BIN and TESHI_MANAGEMENT_POINTER_ARTIFACTS"]
        fn $name() {
            $function($narrow);
        }
    };
}

fn vim_model_input_and_delete_confirmation(narrow: bool) {
    let fixture = Fixture::new("vim-model-input", narrow);
    fs::write(fixture.profiles.join("Beta.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "id":"Beta", "name":"Second Beta", "provider":"openai", "model_id":"fixture-beta", "api_key":"", "base_url":"http://127.0.0.1:1/v1"
    })).unwrap()).unwrap();
    let terminal = fixture.launch();
    terminal.key("m").unwrap();
    terminal.expect_text("Model Profiles", true, 2000).unwrap();
    terminal.key("e").unwrap();
    terminal
        .expect_text("Edit Model Profile", true, 2000)
        .unwrap();
    terminal.key("Home").unwrap();
    terminal.write("jk中🙂").unwrap();
    terminal.key("Left").unwrap();
    terminal.key("Delete").unwrap();
    terminal.key("Backspace").unwrap();
    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Edit Model Profile", false, 3000)
        .unwrap();
    let profile: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.profiles.join("Alpha.json")).unwrap()).unwrap();
    assert_eq!(profile["name"], "jkExisting Alpha");
    fixture.capture(&terminal, "unicode-cursor-saved");
    // The rename changes sort order; select the captured target explicitly.
    terminal.key("End").unwrap();
    terminal.key("d").unwrap();
    terminal.key("d").unwrap();
    terminal
        .expect_text("Confirm deletion", true, 2000)
        .unwrap();
    terminal.key("Down").unwrap();
    terminal.key("1").unwrap();
    terminal.key("Escape").unwrap();
    terminal
        .expect_text("Confirm deletion", false, 2000)
        .unwrap();
    assert!(fixture.profiles.join("Alpha.json").exists());
    fixture.capture(&terminal, "deletion-cancelled");
    terminal.key("Delete").unwrap();
    terminal
        .expect_text("Confirm deletion", true, 2000)
        .unwrap();
    terminal.key("y").unwrap();
    terminal
        .expect_text("Confirm deletion", false, 2000)
        .unwrap();
    fixture.capture(&terminal, "deletion-confirmed");
    assert!(!fixture.profiles.join("Alpha.json").exists());
    assert!(fixture.profiles.join("Beta.json").exists());
    terminal.key("Delete").unwrap();
    terminal
        .expect_text("Confirm deletion", true, 2000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("cannot delete the last remaining", true, 2000)
        .unwrap();
    assert!(fixture.profiles.join("Beta.json").exists());
    fixture.capture(&terminal, "last-profile-preserved");
    terminal.key("Escape").unwrap();
    terminal.key("Escape").unwrap();
    fixture.finish(terminal);
}

acceptance!(
    vim_model_input_normal,
    vim_model_input_and_delete_confirmation,
    false
);
acceptance!(
    vim_model_input_narrow,
    vim_model_input_and_delete_confirmation,
    true
);

acceptance!(model_list_add_normal, model_list_add_control, false);
acceptance!(backend_picker_normal, backend_picker, false);
acceptance!(backend_picker_narrow, backend_picker, true);
acceptance!(installed_picker_normal, installed_picker, false);
acceptance!(installed_picker_narrow, installed_picker, true);
acceptance!(registry_picker_normal, registry_picker, false);
acceptance!(registry_picker_narrow, registry_picker, true);
acceptance!(model_list_add_narrow, model_list_add_control, true);
acceptance!(
    model_form_fields_normal,
    model_form_fields_save_cancel,
    false
);
acceptance!(
    model_form_fields_narrow,
    model_form_fields_save_cancel,
    true
);
acceptance!(auth_close_normal, auth_overview_close, false);
acceptance!(auth_close_narrow, auth_overview_close, true);
acceptance!(
    agent_profile_apply_normal,
    agent_profile_sidebar_apply,
    false
);
acceptance!(
    agent_profile_apply_narrow,
    agent_profile_sidebar_apply,
    true
);
