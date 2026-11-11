//! Hosted Web UI launch for reviewing replay screenshots from the TUI.
//!
//! A terminal under ConPTY cannot draw screenshot pixels, so the viewer offers
//! a shortcut that opens the hosted UI in a browser on the same host. The first
//! activation starts the project daemon when none is listening; a later
//! activation reuses that process. Each activation still mints a new hosted
//! session. The URL is always returned as plain text as well, so a failed
//! browser launch never hides the address.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use teshi_engine::{
    DaemonManifest, DaemonManifestExt, remove_daemon_manifest_if_matches, spawn_daemon_background,
};

/// Canonical hosted Web UI entrypoint. Keep in sync with the daemon's
/// `HOSTED_WEB_ORIGIN` and its trusted-origin allowlist.
const HOSTED_WEB_ORIGIN: &str = "https://teshi-org.github.io/app/";

/// A freshly minted hosted Web UI address and the daemon port it targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewLaunch {
    /// Full launch URL, with the session token confined to the fragment.
    pub url: String,
    /// Loopback port read from the project's daemon manifest.
    pub port: u16,
    /// Whether this call spawned the daemon. False when an existing process was reused.
    pub started_daemon: bool,
}

/// One in-flight launch. The viewer keeps this until the daemon is ready so a
/// repeated click cannot start a second process.
pub(crate) struct PendingLaunch {
    rx: Receiver<Result<ReviewLaunch>>,
}

impl PendingLaunch {
    /// Returns the launch once ensure-and-mint finishes.
    pub(crate) fn poll(&self) -> Option<Result<ReviewLaunch>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("web review launch stopped")))
            }
        }
    }
}

/// Viewer-owned launch progress. The screenshot overlay covers the status bar,
/// so the notice is drawn inside the viewer as well.
#[derive(Default)]
pub(crate) struct LaunchState {
    pending: Option<PendingLaunch>,
    notice: Option<String>,
}

impl LaunchState {
    /// Whether a launch is still waiting for the daemon or its session.
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Progress or result text for the viewer footer.
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Drops a finished notice so the next time the viewer opens it starts clean.
    pub(crate) fn clear_notice(&mut self) {
        self.notice = None;
    }

    /// Starts a launch for `project_root`, or keeps the one already in flight.
    pub(crate) fn request(&mut self, project_root: &Path) {
        let root = project_root.to_path_buf();
        self.request_with(|| begin(&root));
    }

    /// Records `start` unless a launch is already running.
    ///
    /// Returns false when the existing launch was kept. The closure is not
    /// called in that case, so a second click cannot spawn another daemon.
    pub(crate) fn request_with(&mut self, start: impl FnOnce() -> PendingLaunch) -> bool {
        if self.pending.is_some() {
            self.notice = Some("Web review is already starting".to_string());
            return false;
        }
        self.notice = Some("Opening Web review...".to_string());
        self.pending = Some(start());
        true
    }

    /// Takes a finished launch, if the background work has completed.
    pub(crate) fn poll(&mut self) -> Option<Result<ReviewLaunch>> {
        let result = self.pending.as_ref()?.poll()?;
        self.pending = None;
        Some(result)
    }

    /// Replaces the viewer notice with the launch result.
    pub(crate) fn show(&mut self, notice: impl Into<String>) {
        self.notice = Some(notice.into());
    }
}

/// Starts ensure-and-mint off the UI thread.
///
/// The click handler returns immediately so the viewer can show progress, and
/// so a second click observes the in-flight launch instead of racing a spawn.
fn begin(project_root: &Path) -> PendingLaunch {
    let root = project_root.to_path_buf();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(prepare(&root));
    });
    PendingLaunch { rx }
}

/// Builds a launch URL for the project's daemon.
///
/// A live daemon is reused. When none is listening, this starts one and waits
/// until its manifest is reachable. A new hosted session supersedes any
/// previously opened hosted page.
///
/// # Errors
/// Returns an error when the daemon cannot be started, the session request
/// fails, or its response does not carry a well-formed token.
pub fn prepare(project_root: &Path) -> Result<ReviewLaunch> {
    let (port, started_daemon) = ensure_project_daemon(project_root)?;
    let token = mint_session(port)?;
    Ok(ReviewLaunch {
        url: hosted_url(port, &token),
        port,
        started_daemon,
    })
}

/// Reuses a listening project daemon, or starts one on loopback.
///
/// The child binds an OS-selected port and publishes it after the listener is
/// up, matching `teshi web`. The hosted page only dials `127.0.0.1`.
fn ensure_project_daemon(project_root: &Path) -> Result<(u16, bool)> {
    let root = project_root.to_path_buf();
    ensure_project_daemon_with(project_root, Duration::from_secs(15), move || {
        spawn_daemon_background(&root, 0, "127.0.0.1", None).context("start the project daemon")
    })
}

/// `start` runs only when no daemon is listening. The bool is true when this
/// call invoked `start` and then observed a live manifest.
fn ensure_project_daemon_with(
    project_root: &Path,
    timeout: Duration,
    start: impl FnOnce() -> Result<()>,
) -> Result<(u16, bool)> {
    if let Some(port) = live_daemon_port(project_root) {
        return Ok((port, false));
    }
    retire_stale_manifest(project_root)?;
    // A daemon published while the stale manifest was being cleared is reused.
    if let Some(port) = live_daemon_port(project_root) {
        return Ok((port, false));
    }
    start()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(port) = live_daemon_port(project_root) {
            return Ok((port, true));
        }
        // Leave a not-yet-listening manifest in place. Deleting it here would
        // race the child, which writes the file only after its socket is bound.
        if Instant::now() >= deadline {
            anyhow::bail!("daemon failed to start within {}s", timeout.as_secs());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        thread::sleep(Duration::from_millis(250).min(remaining));
    }
}

fn live_daemon_port(project_root: &Path) -> Option<u16> {
    let manifest = DaemonManifest::load_manifest(project_root)?;
    manifest.is_daemon_alive().then_some(manifest.port)
}

fn retire_stale_manifest(project_root: &Path) -> Result<()> {
    if let Some(manifest) = DaemonManifest::load_manifest(project_root)
        && !manifest.is_daemon_alive()
    {
        remove_daemon_manifest_if_matches(project_root, &manifest)?;
    }
    Ok(())
}

/// Composes the hosted launch URL; the token stays in the fragment so it is
/// never sent to the Pages origin as part of the request line.
fn hosted_url(port: u16, token: &str) -> String {
    format!("{HOSTED_WEB_ORIGIN}#port={port}&token={token}")
}

/// Asks the local daemon to mint a `hosted_web_ui` session.
///
/// A daemon that has just bound its port may not answer the first request, so
/// connection failures and server errors are retried until the deadline.
fn mint_session(port: u16) -> Result<String> {
    // Session/bootstrap traffic is loopback-only and must bypass any system
    // proxy, matching the daemon's own launcher.
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .context("create loopback HTTP client")?;
    let url = format!("http://127.0.0.1:{port}/api/v1/sessions");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last_error;
    loop {
        match client
            .post(&url)
            .json(&serde_json::json!({ "role": "hosted_web_ui" }))
            .send()
        {
            Ok(response) if response.status().is_success() => {
                let body: serde_json::Value = response.json().context("decode session response")?;
                let token = body
                    .get("token")
                    .and_then(serde_json::Value::as_str)
                    .context("session response carries no token")?;
                ensure!(is_safe_token(token), "session token is malformed");
                return Ok(token.to_owned());
            }
            Ok(response) if !response.status().is_server_error() => {
                anyhow::bail!("session request was rejected: {}", response.status());
            }
            Ok(response) => last_error = format!("daemon returned {}", response.status()),
            Err(error) => last_error = error.to_string(),
        }
        if Instant::now() >= deadline {
            anyhow::bail!("daemon did not become ready within 15s: {last_error}");
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// The hosted page only accepts URL-safe tokens, and the token is interpolated
/// into the launch URL, so reject everything else.
fn is_safe_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Opens `url` in the host's default browser.
///
/// This is best effort: callers must still expose the URL as plain text when
/// launching fails, which is the common case for headless SSH sessions.
///
/// # Errors
/// Returns an error when no browser launcher could be started.
pub fn open_in_browser(url: &str) -> Result<()> {
    ensure!(is_safe_url(url), "refusing to open an unexpected URL");
    if is_termux() {
        Command::new("termux-open-url")
            .arg(url)
            .spawn()
            .context("cannot start termux-open-url")?;
    } else {
        spawn_browser(url)?;
    }
    Ok(())
}

/// Only open an address this module produced. The exact
/// `#port=<digits>&token=<token>` shape keeps the token confined to the
/// fragment rather than relying on a permissive character allowlist.
fn is_safe_url(url: &str) -> bool {
    let Some(fragment) = url.strip_prefix(HOSTED_WEB_ORIGIN) else {
        return false;
    };
    let Some(rest) = fragment.strip_prefix("#port=") else {
        return false;
    };
    let Some((port, token)) = rest.split_once("&token=") else {
        return false;
    };
    !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) && is_safe_token(token)
}

fn is_termux() -> bool {
    std::env::var_os("TERMUX_VERSION").is_some()
        || std::env::var("PREFIX").is_ok_and(|prefix| prefix.contains("com.termux"))
}

#[cfg(windows)]
fn spawn_browser(url: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // `start` is a cmd builtin. `&` in the URL fragment would otherwise be
    // parsed as a command separator, dropping the token; raw_arg keeps the
    // whole URL quoted. `is_safe_url` bounds what can be interpolated.
    let mut command = Command::new("cmd");
    command.raw_arg(format!("/C start \"\" \"{url}\""));
    // A console the launcher shares is a console it can reset: cmd's own input
    // mode restore leaves teshi's VT reader reading a cooked handle, and the
    // console then echoes every key record onto the screen. Give it its own
    // console and no inherited std handles, so it cannot reach ours at all.
    command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().context("cannot start the browser")?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_browser(url: &str) -> Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(url)
        .spawn()
        .context("cannot start the browser")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hosted_url_keeps_token_inside_fragment() {
        let url = hosted_url(20253, "tk_abc123-XYZ");
        assert_eq!(
            url,
            "https://teshi-org.github.io/app/#port=20253&token=tk_abc123-XYZ"
        );
        assert!(!url[..url.find('#').unwrap()].contains("tk_abc123"));
    }

    #[test]
    fn test_hosted_url_uses_the_manifest_port() {
        assert!(hosted_url(43123, "tk_abc123456789").contains("#port=43123&"));
    }

    #[test]
    fn test_is_safe_token_rejects_shell_metacharacters() {
        for token in ["", "short&whoami", "a b", "a;b", "a\"b", "a|b", "a`b"] {
            assert!(!is_safe_token(token), "{token:?}");
        }
        assert!(is_safe_token("tk_AbC123-x_Y"));
    }

    #[test]
    fn test_is_safe_url_rejects_foreign_or_injected_targets() {
        for url in [
            "",
            "https://example.com/#port=1&token=abc",
            "https://teshi-org.github.io/app/#port=1&token=abc&whoami",
            "https://teshi-org.github.io/evil/#port=1&token=abc",
        ] {
            assert!(!is_safe_url(url), "{url:?}");
        }
        assert!(is_safe_url(&hosted_url(20253, "tk_abc123456789")));
    }

    #[test]
    fn test_ensure_reuses_a_listening_daemon_without_starting() {
        let root = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        write_manifest(root.path(), port);
        let mut starts = 0;
        let start = || {
            starts += 1;
            Ok(())
        };
        let (found, started) =
            ensure_project_daemon_with(root.path(), Duration::ZERO, start).unwrap();
        assert_eq!((found, started), (port, false));
        let (again, started_again) =
            ensure_project_daemon_with(root.path(), Duration::ZERO, || {
                starts += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!((again, started_again, starts), (port, false, 0));
    }

    #[test]
    fn test_ensure_starts_once_then_reuses_the_same_port() {
        let root = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stale = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stale_port = stale.local_addr().unwrap().port();
        drop(stale);
        write_manifest(root.path(), stale_port);
        let mut starts = 0;
        let (found, started) = ensure_project_daemon_with(root.path(), Duration::ZERO, || {
            starts += 1;
            write_manifest(root.path(), port);
            Ok(())
        })
        .unwrap();
        assert_eq!((found, started, starts), (port, true, 1));
        let (again, started_again) =
            ensure_project_daemon_with(root.path(), Duration::ZERO, || {
                starts += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!((again, started_again, starts), (port, false, 1));
    }

    #[test]
    fn test_ensure_reports_a_start_failure_without_waiting() {
        let root = tempfile::tempdir().unwrap();
        let error = ensure_project_daemon_with(root.path(), Duration::from_secs(30), || {
            anyhow::bail!("cannot spawn")
        })
        .unwrap_err();
        assert!(error.to_string().contains("cannot spawn"), "{error}");
    }

    #[test]
    fn test_ensure_times_out_when_the_daemon_never_listens() {
        let root = tempfile::tempdir().unwrap();
        let error = ensure_project_daemon_with(root.path(), Duration::ZERO, || Ok(())).unwrap_err();
        assert!(error.to_string().contains("failed to start"), "{error}");
    }

    #[test]
    fn test_second_request_does_not_start_another_launch() {
        let mut state = LaunchState::default();
        let (tx, rx) = mpsc::channel();
        assert!(state.request_with(|| PendingLaunch { rx }));
        assert_eq!(state.notice(), Some("Opening Web review..."));
        let mut starts = 0;
        assert!(!state.request_with(|| {
            starts += 1;
            let (_held, rx) = mpsc::channel();
            PendingLaunch { rx }
        }));
        assert_eq!(starts, 0);
        assert_eq!(state.notice(), Some("Web review is already starting"));
        assert!(state.poll().is_none());
        tx.send(Ok(ReviewLaunch {
            url: hosted_url(9, "tk_abc"),
            port: 9,
            started_daemon: true,
        }))
        .unwrap();
        let launch = state.poll().unwrap().unwrap();
        assert!(launch.started_daemon);
        assert!(state.poll().is_none());
        assert!(!state.is_pending());
    }

    fn write_manifest(root: &Path, port: u16) {
        let dir = root.join(".teshi");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("daemon.json"),
            format!(r#"{{"pid":1,"port":{port},"started":"2026-01-01T00:00:00Z"}}"#),
        )
        .unwrap();
    }
}
