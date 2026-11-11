//! Hosted Web UI launch for reviewing replay screenshots from the TUI.
//!
//! A terminal under ConPTY cannot draw screenshot pixels, so the viewer offers
//! a shortcut that opens the hosted UI in a browser on the same host. The daemon
//! owns the evidence files; this module only mints a session and composes the
//! address. The URL is always returned as plain text as well, so a failed
//! browser launch never hides the address.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use teshi_engine::{DaemonManifest, DaemonManifestExt};

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
}

/// Builds a launch URL for the project's running daemon.
///
/// A new hosted session supersedes any previously opened hosted page, so each
/// call yields a token that invalidates the older one.
///
/// # Errors
/// Returns an error when no live daemon is reachable, the session request
/// fails, or its response does not carry a well-formed token.
pub fn prepare(project_root: &Path) -> Result<ReviewLaunch> {
    let manifest = DaemonManifest::load_manifest(project_root)
        .context("no daemon is running; start `teshi web --no-open` first")?;
    ensure!(
        manifest.is_daemon_alive(),
        "the daemon manifest is stale; restart `teshi web --no-open`"
    );
    let port = manifest.port;
    let token = mint_session(port)?;
    Ok(ReviewLaunch {
        url: hosted_url(port, &token),
        port,
    })
}

/// Composes the hosted launch URL; the token stays in the fragment so it is
/// never sent to the Pages origin as part of the request line.
fn hosted_url(port: u16, token: &str) -> String {
    format!("{HOSTED_WEB_ORIGIN}#port={port}&token={token}")
}

/// Asks the local daemon to mint a `hosted_web_ui` session.
fn mint_session(port: u16) -> Result<String> {
    // Session/bootstrap traffic is loopback-only and must bypass any system
    // proxy, matching the daemon's own launcher.
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .context("create loopback HTTP client")?;
    let response = client
        .post(format!("http://127.0.0.1:{port}/api/v1/sessions"))
        .json(&serde_json::json!({ "role": "hosted_web_ui" }))
        .send()
        .context("request hosted Web UI session")?;
    ensure!(
        response.status().is_success(),
        "session request was rejected: {}",
        response.status()
    );
    let body: serde_json::Value = response.json().context("decode session response")?;
    let token = body
        .get("token")
        .and_then(serde_json::Value::as_str)
        .context("session response carries no token")?;
    ensure!(is_safe_token(token), "session token is malformed");
    Ok(token.to_owned())
}

/// The hosted page only accepts URL-safe tokens, and the token is interpolated
/// into a Windows command line below, so reject everything else.
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

/// Only open an address this module produced. `cmd` receives the argument
/// unquoted by Rust on Windows, so the exact `#port=<digits>&token=<token>`
/// shape is required rather than a permissive character allowlist.
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
    // `start` is a cmd builtin. `&` in the URL fragment would otherwise be
    // parsed as a command separator, dropping the token; raw_arg keeps the
    // whole URL quoted. `is_safe_url` bounds what can be interpolated.
    let mut command = Command::new("cmd");
    command.raw_arg(format!("/C start \"\" \"{url}\""));
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
}
