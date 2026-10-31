//! Pixel output for replay screenshots.
//!
//! Terminals with a graphics protocol get the capture at full color. Without
//! one the panel keeps the capture path and defers to the operating system's
//! viewer: half-block cells turn a UI screenshot into noise, so they are not
//! offered as a preview.

use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::{Picker, ProtocolType};
use serde_json::Value;
use teshi_core::gherkin::BddProject;

mod pipeline;
mod viewport;
#[cfg(windows)]
mod windows_probe;

use pipeline::{Encoded, Prepared, Request, Worker};
use viewport::Viewport;
pub use viewport::{SourceRegion, ViewAction, ViewState};

/// What this terminal can draw.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum GraphicsSupport {
    /// No pixel protocol; the panel offers an external viewer instead.
    #[default]
    None,
    /// Real pixels through the named terminal protocol.
    Pixels(ProtocolType),
}

impl GraphicsSupport {
    /// Whether a protocol can draw the capture inside the panel.
    pub fn is_pixels(&self) -> bool {
        matches!(self, Self::Pixels(_))
    }

    /// Short protocol name for status messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pixels(ProtocolType::Kitty) => "kitty",
            Self::Pixels(ProtocolType::Sixel) => "sixel",
            Self::Pixels(ProtocolType::Iterm2) => "iTerm2",
            Self::Pixels(ProtocolType::Halfblocks) => "half-blocks",
        }
    }
}

/// Whether the launch conditions rule out terminal graphics.
///
/// Images are off for redirected output, for `NO_COLOR`, and for the BDD
/// harness: probing a pseudo terminal stalls the launch for the query timeout
/// and any late reply reaches the recorded screen as keystrokes.
fn env_disables_images(env: &dyn Fn(&str) -> Option<String>, stdout_is_terminal: bool) -> bool {
    if env("TESHI_NO_IMAGE").is_some() || env("TESHI_TUI_RUNNER").is_some() {
        return true;
    }
    if env("NO_COLOR").is_some() || env("TERM").as_deref() == Some("dumb") {
        return true;
    }
    !stdout_is_terminal
}

/// Result of the one-time terminal graphics detection.
pub struct Detection {
    /// Picker, present only when the terminal can draw pixels.
    pub picker: Option<Picker>,
    /// What the terminal can draw.
    pub support: GraphicsSupport,
    /// Why images are off, or which override forced the protocol.
    pub detail: String,
}

impl Detection {
    fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            picker: None,
            support: GraphicsSupport::None,
            detail: detail.into(),
        }
    }

    /// Builds a picker for a protocol known ahead of time.
    fn forced(protocol: ProtocolType, detail: impl Into<String>) -> Self {
        let mut picker = Picker::from_fontsize(cell_size());
        picker.set_protocol_type(protocol);
        Self {
            picker: Some(picker),
            support: GraphicsSupport::Pixels(protocol),
            detail: detail.into(),
        }
    }
}

/// Cell size in pixels, from the terminal's window metrics when reported.
fn cell_size() -> (u16, u16) {
    crossterm::terminal::window_size()
        .ok()
        .and_then(|size| {
            (size.columns > 0 && size.rows > 0 && size.width > 0 && size.height > 0).then(|| {
                (
                    (size.width / size.columns).max(1),
                    (size.height / size.rows).max(1),
                )
            })
        })
        .unwrap_or((10, 20))
}

/// Protocol named by `TESHI_IMAGE_PROTOCOL`; `Ok(None)` disables images.
///
/// Half-blocks are accepted as a spelling of "off": a UI screenshot drawn in
/// half-block cells reads as noise, so the panel offers the system viewer.
fn override_protocol(value: &str) -> Result<Option<ProtocolType>, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "kitty" => Ok(Some(ProtocolType::Kitty)),
        "sixel" => Ok(Some(ProtocolType::Sixel)),
        "iterm2" => Ok(Some(ProtocolType::Iterm2)),
        "none" | "halfblocks" => Ok(None),
        other => Err(format!("TESHI_IMAGE_PROTOCOL={other} is unknown")),
    }
}

/// Protocol a terminal is known to support, from the markers it exports.
///
/// This is the fallback when the runtime query gets no usable answer: some
/// ConPTY versions and SSH clients cannot answer it, yet still draw images.
fn protocol_from_env(env: &dyn Fn(&str) -> Option<String>) -> Option<(ProtocolType, &'static str)> {
    let term = env("TERM").unwrap_or_default().to_ascii_lowercase();
    let program = env("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();

    if env("KITTY_WINDOW_ID").is_some() || term.contains("kitty") || program == "ghostty" {
        return Some((ProtocolType::Kitty, "kitty terminal markers"));
    }
    if program == "iterm.app" || program == "wezterm" || env("WEZTERM_PANE").is_some() {
        return Some((ProtocolType::Iterm2, "iTerm2 terminal markers"));
    }
    // WT_SESSION exists in older Windows Terminal versions too. Only the
    // console's device attributes establish Sixel support.
    if ["sixel", "mlterm", "foot", "contour", "rio"]
        .iter()
        .any(|name| term.contains(name))
    {
        return Some((ProtocolType::Sixel, "the terminal name implies sixel"));
    }
    None
}

/// Detects the terminal's pixel protocol, unless an override decides it.
///
/// The automatic probe must run after the alternate screen is entered and
/// before anything else reads stdin, because it consumes the terminal's reply.
pub fn init() -> Detection {
    let env = |key: &str| std::env::var(key).ok();

    if let Some(requested) = env("TESHI_IMAGE_PROTOCOL") {
        return match override_protocol(&requested) {
            Ok(Some(protocol)) => {
                let mut detail = format!("TESHI_IMAGE_PROTOCOL={requested}");
                if cfg!(windows) && env("ZELLIJ").is_some() {
                    detail.push_str("; Windows Zellij may filter pixel output; use direct Windows Terminal or open externally");
                }
                Detection::forced(protocol, detail)
            }
            Ok(None) => Detection::unavailable(format!("TESHI_IMAGE_PROTOCOL={requested}")),
            Err(message) => Detection::unavailable(message),
        };
    }
    if env_disables_images(&env, std::io::stdout().is_terminal()) {
        let reason = if env("TESHI_NO_IMAGE").is_some() {
            "TESHI_NO_IMAGE is set"
        } else if env("TESHI_TUI_RUNNER").is_some() {
            "the BDD harness sets TESHI_TUI_RUNNER"
        } else if env("NO_COLOR").is_some() {
            "NO_COLOR is set"
        } else if env("TERM").as_deref() == Some("dumb") {
            "TERM=dumb"
        } else {
            "stdout is not a terminal"
        };
        return Detection::unavailable(reason);
    }
    // The library's Windows query requires a cell-size reply and can leave
    // its stdin reader running after timeout. Use a bounded console probe
    // before the application's input reader starts instead.
    #[cfg(windows)]
    return windows_probe::detect(&env);

    #[cfg(not(windows))]
    detect_stdio(&env)
}

#[cfg(not(windows))]
fn detect_stdio(env: &dyn Fn(&str) -> Option<String>) -> Detection {
    let probed = Picker::from_query_stdio();
    if let Ok(picker) = &probed
        && picker.protocol_type() != ProtocolType::Halfblocks
    {
        return Detection {
            picker: Some(picker.clone()),
            support: GraphicsSupport::Pixels(picker.protocol_type()),
            detail: String::new(),
        };
    }
    let reason = match &probed {
        Ok(_) => "the terminal answered without a graphics protocol".to_string(),
        Err(error) => {
            tracing::debug!(%error, "terminal graphics query failed");
            format!("the graphics query failed: {error}")
        }
    };
    match protocol_from_env(env) {
        Some((protocol, marker)) => {
            Detection::forced(protocol, format!("{reason}; assumed {marker}"))
        }
        None => Detection::unavailable(reason),
    }
}

/// One captured step image belonging to the selected scenario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotRef {
    /// Run directory that owns the file.
    pub run_id: String,
    /// File name inside the run directory.
    pub file: String,
    /// Scenario the step belongs to, when the runner recorded one.
    pub scenario: String,
    /// Step text recorded with the capture.
    pub step_text: String,
    /// Source step line, when recorded.
    pub step_line: Option<u64>,
    /// RFC 3339 capture time.
    pub captured_at: String,
    /// `passed` or `failed` for the step that produced the capture.
    pub status: String,
}

/// An encoded viewport prepared off the UI thread.
pub struct Rendered {
    /// Source dimensions in pixels.
    pub pixels: (u32, u32),
    /// Region sampled from the original image.
    pub region: SourceRegion,
    prepared: Prepared,
    pending_area: Option<Rect>,
    painted_area: Option<Rect>,
    next_tile: usize,
}

impl Rendered {
    /// Whether encoded pixel tiles still await incremental terminal output.
    pub fn is_drawing(&self) -> bool {
        matches!(self.prepared.encoded.as_ref(), Encoded::Pixels(tiles) if self.next_tile < tiles.len())
    }

    fn new(prepared: Prepared) -> Self {
        Self {
            pixels: prepared.pixels,
            region: prepared.region,
            prepared,
            pending_area: None,
            painted_area: None,
            next_tile: 0,
        }
    }

    fn write_pixels(&mut self, writer: &mut impl Write) -> io::Result<usize> {
        let Some(area) = self.pending_area.take() else {
            return Ok(0);
        };
        if self.painted_area != Some(area) {
            self.next_tile = 0;
        }
        let Encoded::Pixels(tiles) = self.prepared.encoded.as_ref() else {
            return Ok(0);
        };
        let mut bytes = 0;
        // Bound each frame's pixel output so input can be drained between
        // chunks. The worker has already split any oversized tile.
        while let Some(tile) = tiles.get(self.next_tile) {
            let prefix = format!("\x1b7\x1b[{};{}H", area.y + tile.y + 1, area.x + tile.x + 1);
            let wire_bytes = prefix.len() + tile.data.len() + 2;
            if bytes + wire_bytes > pipeline::OUTPUT_BUDGET {
                break;
            }
            writer.write_all(prefix.as_bytes())?;
            writer.write_all(tile.data.as_bytes())?;
            writer.write_all(b"\x1b8")?;
            bytes += wire_bytes;
            self.next_tile += 1;
        }
        if bytes > 0 {
            writer.flush()?;
            self.painted_area = Some(area);
        }
        Ok(bytes)
    }
}

/// Screenshot panel state owned by the app.
#[derive(Default)]
pub struct ScreenshotPanel {
    /// Protocol picker; present only when the terminal can draw pixels.
    pub picker: Option<Picker>,
    /// What the terminal can draw, decided once at startup.
    pub support: GraphicsSupport,
    /// Why images are off, or which override forced the protocol.
    pub support_detail: String,
    /// Whether the viewer overlay is open.
    pub visible: bool,
    /// Captures of the selected scenario, in step order.
    pub refs: Vec<ScreenshotRef>,
    /// Selected capture index.
    pub index: usize,
    /// Decoded capture for the selected index.
    pub rendered: Option<Rendered>,
    /// Why the selected capture could not be shown.
    pub error: Option<String>,
    /// Whether a background request is still being prepared.
    pub loading: bool,
    /// Magnification and source-pixel center of the selected capture.
    pub view: ViewState,
    worker: Option<Worker>,
    source: Option<(PathBuf, ScreenshotRef)>,
    requested: Option<(Viewport, ViewState)>,
    last_viewport: Option<Viewport>,
    generation: u64,
    dimensions: Option<(u32, u32)>,
    pixel_screen_size: Option<ratatui::layout::Size>,
}

impl ScreenshotPanel {
    /// Selects a capture without reading or decoding it on the input thread.
    pub fn select(&mut self, project: &Path, shot: ScreenshotRef) {
        self.cancel();
        self.source = Some((project.to_path_buf(), shot));
        self.view = ViewState::default();
        self.dimensions = None;
        self.loading = self.picker.is_some();
    }

    /// Cancels queued work and invalidates any result still being decoded.
    pub fn cancel(&mut self) {
        if let Some(worker) = &self.worker {
            self.generation = worker.cancel();
        }
        self.requested = None;
        self.rendered = None;
        self.error = None;
        self.loading = false;
    }

    /// Closes the viewer promptly, leaving pixel cleanup to the next frame.
    pub fn close(&mut self) {
        self.visible = false;
        self.cancel();
        self.source = None;
    }

    /// Updates zoom or pan, cancelling superseded background requests.
    pub fn change_view(&mut self, action: ViewAction) {
        let viewport = self
            .last_viewport
            .unwrap_or_else(|| Viewport::new(Size::new(80, 24), (10, 20)));
        if self.view.apply(action, self.dimensions, viewport) {
            self.cancel();
            self.loading = self.picker.is_some();
        }
    }

    /// Queues at most one latest viewport request; this performs no image I/O.
    pub fn request_viewport(&mut self, area: Rect) {
        if !self.visible || area.is_empty() {
            return;
        }
        let (Some(picker), Some((project, shot))) = (self.picker.as_ref(), self.source.as_ref())
        else {
            return;
        };
        let viewport = Viewport::new(Size::new(area.width, area.height), picker.font_size());
        self.last_viewport = Some(viewport);
        if self.requested == Some((viewport, self.view)) {
            return;
        }
        self.requested = Some((viewport, self.view));
        let request = Request {
            project: project.clone(),
            shot: shot.clone(),
            picker: picker.clone(),
            view: self.view,
            viewport,
            prefetch: self.neighbors(),
            speculative: false,
        };
        if self.worker.is_none() {
            match Worker::start() {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.loading = false;
                    return;
                }
            }
        }
        if let Some(worker) = &self.worker {
            self.generation = worker.submit(request);
            self.rendered = None;
            self.error = None;
            self.loading = true;
        }
    }

    /// Captures that `j` / `k` reach next, which navigation wraps around to.
    fn neighbors(&self) -> Vec<ScreenshotRef> {
        let total = self.refs.len();
        if total < 2 {
            return Vec::new();
        }
        let next = (self.index + 1) % total;
        let previous = (self.index + total - 1) % total;
        let mut indices = vec![next];
        if previous != next {
            indices.push(previous);
        }
        indices
            .into_iter()
            .filter_map(|index| self.refs.get(index).cloned())
            .collect()
    }

    /// Whether a background result or pending pixel tiles still need event
    /// loop iterations, so the loop should not idle between them.
    pub fn is_busy(&self) -> bool {
        self.visible && (self.loading || self.rendered.as_ref().is_some_and(Rendered::is_drawing))
    }

    /// Publishes only the current request's completion, without waiting for it.
    pub fn poll(&mut self) {
        let Some((generation, result)) = self.worker.as_ref().and_then(Worker::poll) else {
            return;
        };
        if generation != self.generation || !self.visible {
            return;
        }
        self.loading = false;
        match result {
            Ok(prepared) => {
                tracing::debug!(
                    elapsed_ms = prepared.elapsed.as_millis(),
                    cache_hit = prepared.cache_hit,
                    width = prepared.pixels.0,
                    height = prepared.pixels.1,
                    "screenshot viewport prepared"
                );
                self.dimensions = Some(prepared.pixels);
                self.rendered = Some(Rendered::new(prepared));
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Erases stale terminal pixels when closing, replacing, or resizing a capture.
    ///
    /// # Errors
    /// Returns errors from querying or clearing the terminal.
    pub fn prepare_frame<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
    ) -> io::Result<()> {
        if let Some(previous_size) = self.pixel_screen_size {
            let resized = terminal.size()? != previous_size;
            let replaced = self
                .rendered
                .as_ref()
                .is_none_or(|image| image.painted_area.is_none());
            if !self.visible || replaced || resized {
                terminal.clear()?;
                self.pixel_screen_size = None;
                if let Some(image) = self.rendered.as_mut() {
                    image.painted_area = None;
                }
            }
        }
        if let Some(image) = self.rendered.as_mut() {
            image.pending_area = None;
        }
        Ok(())
    }

    /// Writes pixel payloads after Ratatui has flushed the cleared image region.
    ///
    /// # Errors
    /// Returns errors from writing the encoded image or querying terminal size.
    pub fn finish_frame<W: Write>(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<W>>,
    ) -> io::Result<()> {
        if self.visible
            && let Some(image) = self.rendered.as_mut()
            && image.write_pixels(terminal.backend_mut())? > 0
        {
            self.pixel_screen_size = Some(terminal.size()?);
        }
        Ok(())
    }
}

/// One scenario with saved captures, taken from the newest run that has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureEntry {
    /// Feature path relative to the project, with forward slashes.
    pub feature: String,
    /// Scenario name resolved from the run or from the recording step line.
    pub scenario: String,
    /// Captures in that run.
    pub count: usize,
    /// Start time of the run the captures came from.
    pub started_at: String,
}

/// Directory holding one project's replay history.
fn history_root(project: &Path) -> PathBuf {
    project.join(".teshi/logs/replay-screenshots")
}

/// The parts of a run manifest that describe captures.
///
/// Reading `run.json` directly keeps this off the evidence validation path:
/// `load_replay_run` re-reads and decodes every capture artifact, which is
/// far too slow to run while the UI waits for a keypress.
struct Manifest {
    id: String,
    started_at: String,
    events: Vec<Value>,
}

/// Newest manifests first, bounded by `max`.
fn manifests(project: &Path, max: usize) -> Vec<Manifest> {
    let root = history_root(project);
    let mut runs = Vec::new();
    if let Ok(bytes) = std::fs::read(root.join("index.json"))
        && let Ok(index) = serde_json::from_slice::<Value>(&bytes)
    {
        runs.push(legacy_manifest(&index));
    }
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let Ok(bytes) = std::fs::read(entry.path().join("run.json")) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            runs.push(Manifest {
                id: entry.file_name().to_string_lossy().to_string(),
                started_at: value["started_at"].as_str().unwrap_or_default().to_string(),
                events: value["events"].as_array().cloned().unwrap_or_default(),
            });
        }
    }
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    runs.truncate(max);
    runs
}

/// Historical captures recorded before per-run directories existed.
fn legacy_manifest(index: &Value) -> Manifest {
    let feature = index["feature"].clone();
    let mut events = Vec::new();
    for step in index["steps"].as_array().into_iter().flatten() {
        let mut event = step.clone();
        event["type"] = Value::from("step_screenshot");
        event["feature"] = feature.clone();
        event["capture_status"] = Value::from("available");
        event["status"] = Value::from("unknown");
        events.push(event);
    }
    Manifest {
        id: "legacy".to_string(),
        started_at: index["started_at"].as_str().unwrap_or_default().to_string(),
        events,
    }
}

/// Captures of one run that belong to `feature` and, when known, `scenario`.
fn run_screenshots(
    root: &Path,
    run_id: &str,
    events: &[Value],
    feature: &str,
    scenario: Option<&str>,
) -> Vec<ScreenshotRef> {
    events
        .iter()
        .filter_map(|event| {
            if event["type"] != "step_screenshot" || event["capture_status"] != "available" {
                return None;
            }
            let file = event["screenshot_file"].as_str()?.to_string();
            let recorded = normalize_feature(root, event["feature"].as_str().unwrap_or_default());
            if !recorded.is_empty() && recorded != feature && !recorded.ends_with(feature) {
                return None;
            }
            let recorded_scenario = event["scenario"].as_str().unwrap_or_default();
            if let Some(wanted) = scenario
                && !recorded_scenario.is_empty()
                && recorded_scenario != wanted
            {
                return None;
            }
            Some(ScreenshotRef {
                run_id: run_id.to_string(),
                file,
                scenario: recorded_scenario.to_string(),
                step_text: event["step_text"].as_str().unwrap_or_default().to_string(),
                step_line: event["step_line"].as_u64(),
                captured_at: event["captured_at"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                status: event["status"].as_str().unwrap_or("unknown").to_string(),
            })
        })
        .collect()
}

/// Captures of the newest run that recorded any for `feature`/\`scenario\`.
pub fn screenshots_for(
    project: &Path,
    feature: &str,
    scenario: Option<&str>,
) -> Vec<ScreenshotRef> {
    /// Older runs are searched only when newer ones hold no capture.
    const MAX_RUNS: usize = 50;

    for run in manifests(project, MAX_RUNS) {
        let shots = run_screenshots(project, &run.id, &run.events, feature, scenario);
        if !shots.is_empty() {
            return shots;
        }
    }
    Vec::new()
}

/// Which scenarios have captures, newest run first.
///
/// Only the newest run that recorded captures for a scenario contributes, so
/// the panel and the tree marker agree with what the viewer will show.
#[derive(Debug, Clone, Default)]
pub struct CaptureIndex {
    /// Scenarios with captures, newest run first.
    pub entries: Vec<CaptureEntry>,
}

impl CaptureIndex {
    /// Scans the project's replay history.
    ///
    /// Reading run manifests is bounded: a project with a long history still
    /// answers quickly, and older runs keep their captures on disk.
    pub fn scan(project: &BddProject) -> Self {
        /// Manifests are read one at a time; this caps the work per refresh.
        const MAX_RUNS: usize = 50;
        /// Scenarios kept for the fallback list and the tree marker.
        const MAX_ENTRIES: usize = 500;

        let root = &project.root_dir;
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut entries = Vec::new();
        for run in manifests(root, MAX_RUNS) {
            if entries.len() >= MAX_ENTRIES {
                break;
            }
            let mut per_scenario: HashMap<(String, String), usize> = HashMap::new();
            for event in &run.events {
                if event["type"] != "step_screenshot" || event["capture_status"] != "available" {
                    continue;
                }
                let feature =
                    normalize_feature(root, event["feature"].as_str().unwrap_or_default());
                if feature.is_empty() {
                    continue;
                }
                let scenario = match event["scenario"].as_str().filter(|name| !name.is_empty()) {
                    Some(name) => name.to_string(),
                    None => scenario_at_line(project, &feature, event["step_line"].as_u64()),
                };
                *per_scenario.entry((feature, scenario)).or_default() += 1;
            }
            for ((feature, scenario), count) in per_scenario {
                if seen.insert((feature.clone(), scenario.clone())) {
                    entries.push(CaptureEntry {
                        feature,
                        scenario,
                        count,
                        started_at: run.started_at.clone(),
                    });
                }
            }
        }
        entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Self { entries }
    }
}

/// Project-relative feature path from whatever the runner recorded.
///
/// Runners sometimes record an absolute path, including the Windows
/// extended-length prefix; both forms must line up with the parsed project.
fn normalize_feature(root: &Path, recorded: &str) -> String {
    let recorded = recorded.replace('\\', "/");
    let recorded = recorded
        .strip_prefix("//?/")
        .unwrap_or(&recorded)
        .to_string();
    let root = root.to_string_lossy().replace('\\', "/");
    let root = root
        .strip_prefix("//?/")
        .unwrap_or(&root)
        .trim_end_matches('/')
        .to_string();

    let same_prefix = recorded
        .get(..root.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&root));
    if same_prefix && recorded.as_bytes().get(root.len()) == Some(&b'/') {
        return recorded[root.len() + 1..].to_string();
    }
    recorded
}

/// Scenario containing `line`, for runs that recorded no scenario name.
fn scenario_at_line(project: &BddProject, feature: &str, line: Option<u64>) -> String {
    let Some(line) = line else {
        return String::new();
    };
    let Some(feature) = project
        .features
        .iter()
        .find(|item| relative_feature_path(&project.root_dir, &item.file_path) == feature)
    else {
        return String::new();
    };
    feature
        .all_scenarios()
        .into_iter()
        .rev()
        .find(|scenario| scenario.line_number as u64 <= line)
        .map(|scenario| scenario.name.clone())
        .unwrap_or_default()
}

/// Feature path as runners record it: project-relative with forward slashes.
pub(crate) fn relative_feature_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
fn load(picker: &Picker, project: &Path, shot: &ScreenshotRef, size: Size) -> Result<Rendered> {
    let request = Request {
        project: project.into(),
        shot: shot.clone(),
        picker: picker.clone(),
        viewport: Viewport::new(size, picker.font_size()),
        view: ViewState::default(),
        prefetch: Vec::new(),
        speculative: false,
    };
    pipeline::Engine::default()
        .prepare(&request, &|| false)
        .map(Rendered::new)
}

/// Read one saved capture, refusing names or paths outside its run.
fn capture_bytes(project: &Path, shot: &ScreenshotRef) -> Result<Vec<u8>> {
    const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
    let path = checked_capture_path(project, shot)?;
    let file = std::fs::File::open(&path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= MAX_FILE_BYTES,
        "screenshot exceeds the 64 MiB file limit; press o to open externally"
    );
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "screenshot exceeds the 64 MiB file limit; press o to open externally"
    );
    Ok(bytes)
}

fn checked_capture_path(project: &Path, shot: &ScreenshotRef) -> Result<PathBuf> {
    anyhow::ensure!(
        Path::new(&shot.file)
            .file_name()
            .and_then(|name| name.to_str())
            == Some(shot.file.as_str()),
        "invalid screenshot name"
    );
    let root = history_root(project)
        .canonicalize()
        .context("replay history unavailable")?;
    let parent = if shot.run_id == "legacy" {
        root.clone()
    } else {
        let parent = root
            .join(&shot.run_id)
            .canonicalize()
            .context("replay run unavailable")?;
        anyhow::ensure!(
            parent.parent() == Some(root.as_path()),
            "replay run escaped its history"
        );
        parent
    };
    let path = parent.join(&shot.file);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("saved screenshot is missing: {}", path.display()))?;
    anyhow::ensure!(
        canonical.parent() == Some(parent.as_path()),
        "screenshot escaped its run"
    );
    Ok(canonical)
}

/// Path of a saved capture inside its run directory.
///
/// Legacy captures recorded before per-run directories live directly in the
/// history root.
pub fn capture_path(project: &Path, shot: &ScreenshotRef) -> PathBuf {
    let root = project.join(".teshi/logs/replay-screenshots");
    if shot.run_id == "legacy" {
        return root.join(&shot.file);
    }
    root.join(&shot.run_id).join(&shot.file)
}

/// Hand a capture to the operating system's image viewer.
///
/// # Errors
/// Returns an error when no viewer could be started.
pub fn open_externally(project: &Path, shot: &ScreenshotRef) -> Result<()> {
    let path = capture_path(project, shot);
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        // `start` treats a quoted argument as a window title, so pass an empty one.
        command.args(["/C", "start", ""]);
        command
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    command
        .arg(&path)
        .spawn()
        .with_context(|| format!("cannot open {}", path.display()))?;
    Ok(())
}

/// Draw a capture inside `area`, keeping its aspect ratio.
pub fn render(frame: &mut Frame<'_>, area: Rect, rendered: &mut Rendered) {
    let cells = rendered.prepared.cells;
    if cells.width > area.width || cells.height > area.height {
        return;
    }
    let placement = Rect::new(
        area.x + (area.width - cells.width) / 2,
        area.y + (area.height - cells.height) / 2,
        cells.width,
        cells.height,
    );
    match rendered.prepared.encoded.as_ref() {
        Encoded::Pixels(_) => {
            frame.render_widget(ratatui::widgets::Clear, area);
            rendered.pending_area = Some(placement);
        }
        Encoded::Cells(protocol) => {
            frame.render_widget(ratatui_image::Image::new(protocol), placement)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn terminal_markers_identify_a_protocol() {
        let only = |key: &'static str, value: &'static str| {
            move |name: &str| (name == key).then(|| value.to_string())
        };

        assert_eq!(
            protocol_from_env(&only("WT_SESSION", "x")).map(|(protocol, _)| protocol),
            None
        );
        assert_eq!(
            protocol_from_env(&only("KITTY_WINDOW_ID", "1")).map(|(protocol, _)| protocol),
            Some(ProtocolType::Kitty)
        );
        assert_eq!(
            protocol_from_env(&only("TERM_PROGRAM", "ghostty")).map(|(protocol, _)| protocol),
            Some(ProtocolType::Kitty)
        );
        assert_eq!(
            protocol_from_env(&only("TERM_PROGRAM", "iTerm.app")).map(|(protocol, _)| protocol),
            Some(ProtocolType::Iterm2)
        );
        assert_eq!(
            protocol_from_env(&only("TERM", "xterm-foot")).map(|(protocol, _)| protocol),
            Some(ProtocolType::Sixel)
        );
        assert!(protocol_from_env(&no_env).is_none());
    }

    #[test]
    fn override_protocol_names_a_protocol_or_disables_images() {
        assert_eq!(override_protocol("sixel"), Ok(Some(ProtocolType::Sixel)));
        assert_eq!(override_protocol(" Kitty "), Ok(Some(ProtocolType::Kitty)));
        assert_eq!(override_protocol("iterm2"), Ok(Some(ProtocolType::Iterm2)));
        assert_eq!(override_protocol("none"), Ok(None));
        assert_eq!(override_protocol("halfblocks"), Ok(None));
        assert!(override_protocol("svg").is_err());
    }

    #[test]
    fn redirected_output_disables_images() {
        assert!(env_disables_images(&no_env, false));
        assert!(!env_disables_images(&no_env, true));
    }

    #[test]
    fn harness_and_user_overrides_disable_images() {
        for key in ["TESHI_NO_IMAGE", "TESHI_TUI_RUNNER", "NO_COLOR"] {
            let env = move |name: &str| (name == key).then(|| "1".to_string());
            assert!(env_disables_images(&env, true), "{key} must disable images");
        }
        let dumb = |name: &str| (name == "TERM").then(|| "dumb".to_string());
        assert!(env_disables_images(&dumb, true));
    }

    fn fixture_rendered(picker: &Picker, image: image::RgbaImage, size: Size) -> Rendered {
        let prepared = pipeline::prepare_image(
            &image::DynamicImage::ImageRgba8(image),
            picker,
            ViewState::default(),
            Viewport::new(size, picker.font_size()),
            &|| false,
        )
        .unwrap();
        Rendered::new(prepared)
    }

    #[test]
    fn pixel_overlay_clears_existing_text_before_displaying_pixels() {
        use ratatui::widgets::Paragraph;

        let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 40, 40, 255]));
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let mut rendered = fixture_rendered(&picker, image, Size::new(8, 3));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(12, 6)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("old text\nold text\nold text"), frame.area());
            })
            .unwrap();
        terminal
            .draw(|frame| {
                super::render(frame, Rect::new(0, 0, 8, 3), &mut rendered);
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(1, 0)].symbol(),
            " ",
            "image cells must erase the underlying text before pixel output"
        );
    }

    #[test]
    fn pixel_payload_follows_text_and_is_not_repeated_until_resized() {
        for protocol in [ProtocolType::Sixel, ProtocolType::Iterm2] {
            let image = image::RgbaImage::from_pixel(400, 200, image::Rgba([200, 40, 40, 255]));
            let mut picker = Picker::from_fontsize((10, 20));
            picker.set_protocol_type(protocol);
            let mut rendered = fixture_rendered(&picker, image, Size::new(8, 3));
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(12, 6)).unwrap();
            let draw = |terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
                        rendered: &mut Rendered,
                        area| {
                terminal
                    .draw(|frame| {
                        super::render(frame, area, rendered);
                        frame.render_widget(
                            ratatui::widgets::Paragraph::new("footer"),
                            Rect::new(0, 5, 12, 1),
                        );
                    })
                    .unwrap();
            };
            draw(&mut terminal, &mut rendered, Rect::new(1, 1, 8, 3));
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content()
                    .iter()
                    .all(|cell| !cell.symbol().contains('\x1b') && !cell.skip)
            );
            assert_eq!(terminal.backend().buffer()[(0, 5)].symbol(), "f");
            let mut output = b"text frame".to_vec();
            assert!(rendered.write_pixels(&mut output).unwrap() > 0);
            assert!(output.starts_with(b"text frame\x1b7\x1b[2;2H"));
            assert!(output.ends_with(b"\x1b8"));
            let first_len = output.len();
            draw(&mut terminal, &mut rendered, Rect::new(1, 1, 8, 3));
            assert_eq!(rendered.write_pixels(&mut output).unwrap(), 0);
            assert_eq!(
                output.len(),
                first_len,
                "idle frames must not resend the image"
            );
            rendered = fixture_rendered(
                &picker,
                image::RgbaImage::from_pixel(400, 200, image::Rgba([200, 40, 40, 255])),
                Size::new(6, 2),
            );
            draw(&mut terminal, &mut rendered, Rect::new(1, 1, 6, 2));
            assert!(rendered.write_pixels(&mut output).unwrap() > 0);
        }
    }

    #[test]
    fn pixel_layer_erases_stale_images_on_close_replace_and_resize() {
        for action in ["stable", "close", "replace", "resize"] {
            let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 40, 40, 255]));
            let mut picker = Picker::from_fontsize((10, 20));
            picker.set_protocol_type(ProtocolType::Sixel);
            let mut rendered = fixture_rendered(&picker, image, Size::new(8, 3));
            rendered.painted_area = Some(Rect::new(0, 0, 8, 3));
            let mut panel = ScreenshotPanel {
                visible: true,
                rendered: Some(rendered),
                pixel_screen_size: Some(ratatui::layout::Size::new(12, 6)),
                ..Default::default()
            };
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(12, 6)).unwrap();
            terminal
                .draw(|frame| {
                    frame.render_widget(ratatui::widgets::Paragraph::new("old"), frame.area())
                })
                .unwrap();
            match action {
                "close" => panel.visible = false,
                "replace" => panel.rendered.as_mut().unwrap().painted_area = None,
                "resize" => terminal.backend_mut().resize(10, 5),
                _ => {}
            }
            panel.prepare_frame(&mut terminal).unwrap();
            if action == "stable" {
                assert!(panel.pixel_screen_size.is_some());
                assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "o");
            } else {
                assert!(panel.pixel_screen_size.is_none(), "{action}");
                assert!(
                    panel.rendered.as_ref().unwrap().painted_area.is_none(),
                    "{action}"
                );
                assert_eq!(
                    terminal.backend().buffer()[(0, 0)].symbol(),
                    " ",
                    "{action}"
                );
            }
        }
    }

    #[test]
    fn pixel_output_yields_between_bounded_chunks_and_resumes_without_repeating_tiles() {
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let mut rendered = fixture_rendered(
            &picker,
            image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 0, 0, 255])),
            Size::new(8, 3),
        );
        rendered.prepared.encoded = std::sync::Arc::new(Encoded::Pixels(
            (0..3)
                .map(|y| pipeline::Tile {
                    x: 0,
                    y,
                    data: "x".repeat(32 * 1024),
                })
                .collect(),
        ));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(12, 6)).unwrap();
        let mut total = 0;
        for expected in [1, 2, 3, 3] {
            terminal
                .draw(|frame| super::render(frame, Rect::new(0, 0, 8, 3), &mut rendered))
                .unwrap();
            let mut output = Vec::new();
            let written = rendered.write_pixels(&mut output).unwrap();
            assert_eq!(written, output.len());
            assert!(written <= pipeline::OUTPUT_BUDGET);
            assert_eq!(rendered.next_tile, expected);
            total += output.iter().filter(|byte| **byte == b'x').count();
        }
        assert_eq!(total, 3 * 32 * 1024);
    }

    #[test]
    fn render_paints_the_capture_into_the_area() {
        let image = image::RgbaImage::from_pixel(400, 400, image::Rgba([200, 40, 40, 255]));
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        let mut rendered = fixture_rendered(&picker, image, Size::new(20, 10));

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 10)).unwrap();
        // The prepared protocol only paints cells during frame rendering.
        for _ in 0..3 {
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    super::render(frame, area, &mut rendered);
                })
                .unwrap();
        }

        // Half-blocks paint colored cells, not necessarily block glyphs: a
        // uniform area becomes spaces with a background color.
        let buffer = terminal.backend().buffer();
        let painted = buffer
            .content()
            .iter()
            .filter(|cell| {
                cell.bg != ratatui::style::Color::Reset || cell.fg != ratatui::style::Color::Reset
            })
            .count();
        assert!(painted > 0, "the capture must cover cells");
    }

    #[test]
    fn test_neighbors_wrap_and_skip_duplicates() {
        let shot = |file: &str| ScreenshotRef {
            run_id: "run".into(),
            file: file.into(),
            scenario: String::new(),
            step_text: String::new(),
            step_line: None,
            captured_at: String::new(),
            status: "passed".into(),
        };
        let files = |panel: &ScreenshotPanel| {
            panel
                .neighbors()
                .into_iter()
                .map(|shot| shot.file)
                .collect::<Vec<_>>()
        };
        let mut panel = ScreenshotPanel {
            refs: vec![shot("a.png")],
            ..Default::default()
        };
        assert!(files(&panel).is_empty(), "a single capture has no neighbor");
        panel.refs.push(shot("b.png"));
        assert_eq!(files(&panel), ["b.png"]);
        panel.refs.push(shot("c.png"));
        assert_eq!(files(&panel), ["b.png", "c.png"]);
        panel.index = 2;
        assert_eq!(files(&panel), ["a.png", "b.png"]);
    }

    #[test]
    fn relative_feature_path_normalizes_separators() {
        let root = Path::new("proj");
        assert_eq!(
            relative_feature_path(root, Path::new("proj/features/demo.feature")),
            "features/demo.feature"
        );
        assert_eq!(
            relative_feature_path(root, Path::new("proj/features\\demo.feature")),
            "features/demo.feature"
        );
        // A drive root is only a prefix once the platform treats `\` as a
        // separator, so the recorded Windows path can only be stripped there.
        #[cfg(windows)]
        assert_eq!(
            relative_feature_path(
                Path::new("C:/proj"),
                Path::new("C:/proj\\features\\demo.feature")
            ),
            "features/demo.feature"
        );
    }

    #[test]
    fn absolute_recorded_feature_paths_become_project_relative() {
        let root = Path::new("D:/Dev/teshi/fixture");
        assert_eq!(
            normalize_feature(root, "D:\\Dev\\teshi\\fixture\\demo.feature"),
            "demo.feature"
        );
        assert_eq!(
            normalize_feature(root, "\\\\?\\D:\\Dev\\teshi\\fixture\\demo.feature"),
            "demo.feature"
        );
        // A path outside the project keeps its own identity.
        assert_eq!(
            normalize_feature(root, "D:\\Dev\\other\\demo.feature"),
            "D:/Dev/other/demo.feature"
        );
    }

    #[test]
    fn capture_index_keeps_the_newest_run_per_scenario() {
        let dir = tempfile::tempdir().expect("temp project");
        std::fs::create_dir_all(dir.path().join(".teshi/logs/replay-screenshots")).unwrap();
        let feature_path = dir.path().join("demo.feature");
        std::fs::write(
            &feature_path,
            "Feature: Demo\n  Scenario: First\n    Given a\n",
        )
        .unwrap();
        let project = teshi_core::gherkin::parse_project(dir.path());
        let png = {
            let image = image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255]));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .expect("encode png");
            bytes.into_inner()
        };

        for _ in 0..2 {
            let mut run =
                teshi_engine::replay_evidence::ReplayEvidenceRun::create(dir.path()).unwrap();
            run.screenshot(
                &png,
                serde_json::json!({
                    "case_id":"case","step_id":"L3","feature":"demo.feature",
                    "step_text":"Given a","step_line":3,"status":"passed"
                }),
            )
            .expect("record capture");
        }

        let index = CaptureIndex::scan(&project);

        assert_eq!(index.entries.len(), 1);
        let entry = &index.entries[0];
        assert_eq!(entry.feature, "demo.feature");
        // The recording carried no scenario name, so it is resolved from the
        // step line instead.
        assert_eq!(entry.scenario, "First");
        assert_eq!(entry.count, 1);
    }

    #[test]
    fn run_screenshots_keeps_available_captures_of_the_selected_scenario() {
        let events: Vec<Value> = serde_json::json!([
            {"type":"step_screenshot","capture_status":"available","screenshot_file":"a.png",
             "feature":"features/demo.feature","scenario":"First","step_text":"Given a",
             "step_line":4,"status":"passed","captured_at":"2026-01-01T00:00:00Z"},
            {"type":"step_screenshot","capture_status":"failed","screenshot_file":"b.png",
             "feature":"features/demo.feature","scenario":"First"},
            {"type":"step_screenshot","capture_status":"available","screenshot_file":"c.png",
             "feature":"features/demo.feature","scenario":"Second"},
            {"type":"step_screenshot","capture_status":"available","screenshot_file":"d.png",
             "feature":"features/other.feature","scenario":"First"}
        ])
        .as_array()
        .cloned()
        .expect("fixture is an array");

        let shots = run_screenshots(
            Path::new("proj"),
            "run-1",
            &events,
            "features/demo.feature",
            Some("First"),
        );

        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].file, "a.png");
        assert_eq!(shots[0].run_id, "run-1");
        assert_eq!(shots[0].step_line, Some(4));
        assert_eq!(shots[0].status, "passed");
    }

    #[test]
    fn evidence_round_trip_resolves_the_capture_for_a_feature() {
        let dir = tempfile::tempdir().expect("temp project");
        std::fs::create_dir_all(dir.path().join(".teshi/logs/replay-screenshots")).unwrap();
        let png = {
            let image = image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .expect("encode png");
            bytes.into_inner()
        };
        let file = {
            let mut run =
                teshi_engine::replay_evidence::ReplayEvidenceRun::create(dir.path()).unwrap();
            let event = run
                .screenshot(
                    &png,
                    serde_json::json!({
                        "case_id":"case","step_id":"L4","feature":"features/demo.feature",
                        "step_text":"Given a","step_line":4,"status":"passed","scenario":"First"
                    }),
                )
                .expect("record capture");
            event["screenshot_file"].as_str().unwrap().to_string()
        };

        let shots = screenshots_for(dir.path(), "features/demo.feature", Some("First"));

        assert_eq!(shots.len(), 1);
        let shot = &shots[0];
        assert_eq!(shot.file, file);
        assert_eq!(
            capture_bytes(dir.path(), shot).expect("the capture is readable"),
            png
        );
    }

    #[test]
    fn capture_bytes_rejects_paths_outside_the_run() {
        let dir = tempfile::tempdir().expect("temp project");
        std::fs::create_dir_all(dir.path().join(".teshi/logs/replay-screenshots")).unwrap();
        let shot = ScreenshotRef {
            run_id: "6f0f5f38-0f2e-4b8f-9a3e-9f0f5f380f2e".into(),
            file: "../escape.png".into(),
            scenario: String::new(),
            step_text: String::new(),
            step_line: None,
            captured_at: String::new(),
            status: "passed".into(),
        };

        assert!(capture_bytes(dir.path(), &shot).is_err());
    }

    #[test]
    fn capture_path_stays_inside_its_run() {
        let shot = ScreenshotRef {
            run_id: "6f0f5f38-0f2e-4b8f-9a3e-9f0f5f380f2e".into(),
            file: "frame.png".into(),
            scenario: String::new(),
            step_text: String::new(),
            step_line: None,
            captured_at: String::new(),
            status: "passed".into(),
        };
        let path = capture_path(Path::new("proj"), &shot);
        assert_eq!(
            path,
            Path::new("proj/.teshi/logs/replay-screenshots")
                .join(&shot.run_id)
                .join("frame.png")
        );
    }
}
