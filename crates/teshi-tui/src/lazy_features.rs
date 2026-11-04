//! Directory browsing without retaining the project's complete AST.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use teshi_core::gherkin::{BddFeature, parse_feature};

const CHANGED_DURING_LOAD: &str = "Feature changed while loading; select it again to retry";

/// A bounded single-file working set, independent of the directory catalog.
pub(crate) struct LazyFeatures {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) revision: u64,
    pub(crate) ready: bool,
    pub(crate) selected: Option<PathBuf>,
    pub(crate) error: Option<String>,
    pub(crate) scan_error: Option<String>,
    root: PathBuf,
    recursive: bool,
    catalog_rx: Option<Receiver<Result<Vec<PathBuf>, String>>>,
    content_rx: Option<Receiver<Result<LoadedContent, String>>>,
    requested: Option<PathBuf>,
    selection_revision: u64,
    requested_revision: u64,
    last_scan: Instant,
    refresh_requested: bool,
    retries: u8,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

// Each receiver belongs to one project instance; dropping it isolates project switches.
struct LoadedContent {
    feature: BddFeature,
    source: String,
    stamp: ContentStamp,
}

#[derive(PartialEq, Eq)]
struct ContentStamp {
    canonical_path: PathBuf,
    length: u64,
    modified: std::time::SystemTime,
    created: Option<std::time::SystemTime>,
}

impl ContentStamp {
    fn capture(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
        Ok(Self {
            canonical_path: std::fs::canonicalize(path).map_err(|e| e.to_string())?,
            length: metadata.len(),
            modified: metadata.modified().map_err(|e| e.to_string())?,
            created: metadata.created().ok(),
        })
    }
}

fn read_versioned_content(path: &Path) -> Result<LoadedContent, String> {
    let stamp = ContentStamp::capture(path)?;
    let (feature, source) = read_content(path)?;
    verify_content(path, &source, &stamp)?;
    Ok(LoadedContent {
        feature,
        source,
        stamp,
    })
}

// Compare bytes in the worker after parsing. Metadata alone misses equal-length
// writes whose timestamps were restored. The extra read remains source-bounded.
fn verify_content(path: &Path, source: &str, stamp: &ContentStamp) -> Result<(), String> {
    if ContentStamp::capture(path)? != *stamp
        || read_source(path)? != source
        || ContentStamp::capture(path)? != *stamp
    {
        return Err(CHANGED_DURING_LOAD.into());
    }
    Ok(())
}

impl LazyFeatures {
    /// Starts path-only discovery; no Feature bodies are read at startup.
    pub(crate) fn new(root: PathBuf, recursive: bool) -> Self {
        let mut state = Self {
            paths: Vec::new(),
            revision: 0,
            ready: false,
            selected: None,
            error: None,
            scan_error: None,
            root,
            recursive,
            catalog_rx: None,
            content_rx: None,
            requested: None,
            selection_revision: 0,
            requested_revision: 0,
            last_scan: Instant::now(),
            refresh_requested: false,
            retries: 0,
            cancelled: Default::default(),
        };
        state.scan();
        state
    }

    fn scan(&mut self) {
        self.refresh_requested = false;
        let root = self.root.clone();
        let recursive = self.recursive;
        let cancelled = self.cancelled.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("feature-catalog".into())
            .spawn(move || {
                let result = if root.exists() {
                    discover_bounded(&root, recursive, &cancelled)
                } else {
                    Ok(Vec::new())
                };
                let _ = tx.send(result);
            }) {
            Ok(_) => self.catalog_rx = Some(rx),
            Err(error) => self.scan_error = Some(error.to_string()),
        }
        self.last_scan = Instant::now();
    }

    /// Applies a completed catalog without replacing it on a failed scan.
    pub(crate) fn poll_catalog(&mut self) {
        if let Some(rx) = &self.catalog_rx {
            match rx.try_recv() {
                Ok(Ok(paths)) => {
                    self.ready = true;
                    if paths != self.paths {
                        self.paths = paths;
                        self.revision += 1;
                    }
                    self.last_scan = Instant::now();
                    self.catalog_rx = None;
                    self.scan_error = None;
                }
                Ok(Err(error)) => {
                    self.scan_error = Some(error);
                    self.last_scan = Instant::now();
                    self.catalog_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.scan_error = Some("Feature catalog worker disconnected".into());
                    self.last_scan = Instant::now();
                    self.catalog_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        } else if self.refresh_requested || self.last_scan.elapsed() >= Duration::from_secs(5) {
            self.scan();
        }
    }

    /// Requests another scan, including when the current snapshot is still in flight.
    pub(crate) fn request_refresh(&mut self) {
        self.refresh_requested = true;
    }

    /// Whether catalog or selected content is being loaded.
    pub(crate) fn is_loading(&self) -> bool {
        self.catalog_rx.is_some() || self.content_rx.is_some()
    }

    /// Queues the latest selection. Only one parser and one result can exist.
    pub(crate) fn select(&mut self, path: PathBuf) {
        self.selection_revision = self.selection_revision.wrapping_add(1);
        self.selected = Some(path);
        self.error = None;
        self.retries = 0;
        self.start_content();
    }

    /// Supersedes an asynchronous request with content installed by an explicit tool.
    pub(crate) fn cancel_selection(&mut self, path: PathBuf) {
        self.content_rx = None;
        self.requested = None;
        self.selected = Some(path);
        self.error = None;
        self.retries = 0;
    }

    fn start_content(&mut self) {
        if self.content_rx.is_some() {
            return;
        }
        let Some(path) = self.selected.clone() else {
            return;
        };
        self.requested = Some(path.clone());
        self.requested_revision = self.selection_revision;
        let (tx, rx) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("feature-content".into())
            .spawn(move || {
                let result = read_versioned_content(&path);
                let _ = tx.send(result);
            }) {
            Ok(_) => self.content_rx = Some(rx),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Returns content only for the latest selection; stale results are discarded.
    pub(crate) fn poll_content(
        &mut self,
    ) -> Option<(BddFeature, String, std::time::SystemTime, u64)> {
        let result = match self.content_rx.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("Feature parser disconnected".into()),
        };
        self.content_rx = None;
        if self.requested != self.selected || self.requested_revision != self.selection_revision {
            self.start_content();
            return None;
        }
        match result {
            Ok(content) => match ContentStamp::capture(&content.feature.file_path) {
                Ok(stamp) if stamp == content.stamp => Some((
                    content.feature,
                    content.source,
                    content.stamp.modified,
                    content.stamp.length,
                )),
                _ => self.retry_content(),
            },
            Err(error) if error == CHANGED_DURING_LOAD => self.retry_content(),
            Err(error) => {
                if self.selected.as_ref().is_some_and(|path| !path.exists()) {
                    // The selection disappeared while loading; drop it so the
                    // caller can move on instead of reporting a missing file.
                    self.selected = None;
                    self.requested = None;
                    self.request_refresh();
                    return None;
                }
                self.error = Some(error);
                None
            }
        }
    }

    /// A clean selection whose file changed mid-read is reread; a file that keeps
    /// changing still surfaces the explicit retry message after a bounded number
    /// of attempts instead of blocking the window.
    fn retry_content(&mut self) -> Option<(BddFeature, String, std::time::SystemTime, u64)> {
        const MAX_RETRIES: u8 = 2;
        if self.retries < MAX_RETRIES {
            self.retries += 1;
            self.start_content();
            return None;
        }
        self.error =
            Some("Feature changed before loading completed; select it again to retry".into());
        None
    }
}

/// Reads one bounded Feature source and its AST.
pub(crate) fn read_content(path: &Path) -> Result<(BddFeature, String), String> {
    let source = read_source(path)?;
    let feature = parse_feature(&source, path.to_path_buf());
    Ok((feature, source))
}

fn read_source(path: &Path) -> Result<String, String> {
    use std::io::Read;
    // Enforce the limit on the read itself, including files growing during IO.
    const MAX_SOURCE: u64 = 2 * 1024 * 1024;
    let mut source = String::new();
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(MAX_SOURCE + 1)
        .read_to_string(&mut source)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if source.len() as u64 > MAX_SOURCE {
        return Err(
            "Feature exceeds the 2 MiB interactive loading budget; split the file to browse it"
                .into(),
        );
    }
    Ok(source)
}

/// Validates a replacement before discarding dirty edits, retaining one bounded source at a time.
pub(crate) fn validate_replacement(root: &Path) -> Result<(), String> {
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    for path in discover_bounded(root, true, &cancelled)? {
        let _ = read_source(&path)?;
    }
    Ok(())
}

impl Drop for LazyFeatures {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn discover_bounded(
    root: &Path,
    recursive: bool,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Vec<PathBuf>, String> {
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;
    // This budget includes paths and directory-cycle identities. There is no
    // unbounded queue of directory entries or parsed Feature bodies.
    const PATH_BUDGET: usize = 32 * 1024 * 1024;
    let mut retained = 0usize;
    let mut visited = HashSet::new();
    visited.insert(std::fs::canonicalize(root).map_err(|e| e.to_string())?);
    let mut stack = vec![std::fs::read_dir(root).map_err(|e| e.to_string())?];
    let mut paths = Vec::new();
    while let Some(entries) = stack.last_mut() {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Feature scan cancelled".into());
        }
        let Some(entry) = entries.next() else {
            stack.pop();
            continue;
        };
        let path = entry.map_err(|e| e.to_string())?.path();
        let metadata = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if metadata.is_dir() && recursive {
            let identity = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
            if visited.contains(&identity) {
                continue;
            }
            retained = retained.saturating_add(identity.as_os_str().len() * 4 + 128);
            if retained > PATH_BUDGET {
                return Err(
                    "Feature directory exceeds the catalog memory budget; open a smaller directory"
                        .into(),
                );
            }
            visited.insert(identity);
            stack.push(std::fs::read_dir(&path).map_err(|e| e.to_string())?);
        } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == "feature") {
            retained = retained.saturating_add(path.as_os_str().len() * 4 + 128);
            if retained > PATH_BUDGET {
                return Err(
                    "Feature directory exceeds the catalog memory budget; open a smaller directory"
                        .into(),
                );
            }
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_catalog(state: &mut LazyFeatures) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.catalog_rx.is_some() {
            state.poll_catalog();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn catalog_does_not_read_bodies_and_respects_scope() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("broken.feature"), [0xff]).unwrap();
        std::fs::write(root.path().join("ignored.txt"), "Feature: Not a file").unwrap();
        std::fs::write(root.path().join("nested/deep.feature"), "Feature: Deep").unwrap();
        let mut state = LazyFeatures::new(root.path().into(), false);
        wait_catalog(&mut state);
        assert!(state.error.is_none());
        assert_eq!(state.paths.len(), 1);
        assert!(read_content(&state.paths[0]).is_err());
        let mut recursive = LazyFeatures::new(root.path().into(), true);
        wait_catalog(&mut recursive);
        assert_eq!(recursive.paths.len(), 2);
    }

    #[test]
    fn stale_content_cannot_replace_latest_selection() {
        let root = tempfile::tempdir().unwrap();
        let latest = root.path().join("latest.feature");
        std::fs::write(&latest, "Feature: Latest").unwrap();
        let mut state = LazyFeatures::new(root.path().into(), true);
        wait_catalog(&mut state);
        let (tx, rx) = mpsc::channel();
        tx.send(Err("obsolete request".into())).unwrap();
        state.content_rx = Some(rx);
        state.requested = Some("old.feature".into());
        state.selected = Some(latest);
        assert!(state.poll_content().is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((feature, _, _, _)) = state.poll_content() {
                assert_eq!(feature.name, "Latest");
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn changed_file_result_is_reread_before_installation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("changing.feature");
        std::fs::write(&path, "Feature: Old").unwrap();
        let content = read_versioned_content(&path).unwrap();
        std::fs::write(&path, "Feature: New content").unwrap();
        let mut state = LazyFeatures::new(root.path().into(), true);
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(content)).unwrap();
        state.content_rx = Some(rx);
        state.requested = Some(path.clone());
        state.selected = Some(path);
        // The stale snapshot is never installed; the reread carries the new body.
        assert!(state.poll_content().is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((feature, _, _, _)) = state.poll_content() {
                assert_eq!(feature.name, "New content");
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(state.error.is_none());
    }

    #[test]
    fn equal_length_rewrite_with_restored_timestamp_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("changing.feature");
        std::fs::write(&path, "Feature: Old").unwrap();
        let stamp = ContentStamp::capture(&path).unwrap();
        std::fs::write(&path, "Feature: New").unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(stamp.modified))
            .unwrap();
        assert!(
            verify_content(&path, "Feature: Old", &stamp)
                .unwrap_err()
                .contains("changed")
        );
    }

    #[test]
    fn repeated_selection_discards_earlier_request_for_same_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("same.feature");
        std::fs::write(&path, "Feature: Same").unwrap();
        let mut state = LazyFeatures::new(root.path().into(), true);
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(read_versioned_content(&path).unwrap())).unwrap();
        state.content_rx = Some(rx);
        state.requested = Some(path.clone());
        state.selected = Some(path);
        state.selection_revision = 2;
        assert!(state.poll_content().is_none());
        assert!(state.content_rx.is_some());
        assert_eq!(state.requested_revision, 2);
    }

    #[test]
    fn oversized_file_is_rejected_before_parsing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("huge.feature");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(2 * 1024 * 1024 + 1).unwrap();
        assert!(read_content(&path).unwrap_err().contains("budget"));
    }

    #[test]
    #[ignore = "dense performance baseline; creates 10,000 files and parses 3,000,000 steps"]
    fn dense_loading_baseline() {
        let root = tempfile::tempdir().unwrap();
        let mut source = String::from("Feature: Dense benchmark\n");
        for scenario in 0..10 {
            source.push_str(&format!("  Scenario: Case {scenario}\n"));
            for step in 0..30 {
                source.push_str(&format!("    Given benchmark step {step}\n"));
            }
        }
        let fixture_started = Instant::now();
        for file in 0..10_000 {
            let directory = root.path().join(format!("d{}/a/b/c/d/e/f/g", file / 100));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{file:05}.feature")), &source).unwrap();
        }
        eprintln!("fixture_ms={}", fixture_started.elapsed().as_millis());
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let started = Instant::now();
        let paths = discover_bounded(root.path(), true, &cancelled).unwrap();
        assert_eq!(paths.len(), 10_000);
        eprintln!("scan_ms={}", started.elapsed().as_millis());
        let started = Instant::now();
        let mut scenarios = 0;
        let mut steps = 0;
        for path in &paths {
            let content = read_versioned_content(path).unwrap();
            for scenario in content.feature.all_scenarios() {
                scenarios += 1;
                steps += scenario.steps.len();
            }
        }
        assert_eq!((scenarios, steps), (100_000, 3_000_000));
        eprintln!("parse_and_verify_ms={}", started.elapsed().as_millis());
        let started = Instant::now();
        for cycle in 0..3 {
            for path in paths.iter().take(100) {
                let content = read_versioned_content(path).unwrap();
                assert_eq!(content.feature.all_scenarios().len(), 10);
            }
            eprintln!(
                "browse_cycle={} elapsed_ms={}",
                cycle + 1,
                started.elapsed().as_millis()
            );
        }
    }

    #[test]
    fn cancelled_scan_returns_no_partial_catalog() {
        let root = tempfile::tempdir().unwrap();
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        assert!(
            discover_bounded(root.path(), true, &cancelled)
                .unwrap_err()
                .contains("cancelled")
        );
    }
}
