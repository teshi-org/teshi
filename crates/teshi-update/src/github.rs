//! GitHub release resolution with bounded metadata and conditional requests.

use crate::{
    ErrorCode, Result, UpdateError, invalid,
    manifest::{InstallKind, ReleaseAsset, ReleaseManifest},
    release::is_update,
    storage::{StateLock, private_directory, write_json},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use teshi_core::version::{BuildIdentity, ReleaseChannel};

const API: &str = "https://api.github.com/repos/teshi-org/teshi/releases";
const METADATA_LIMIT: u64 = 8 * 1024 * 1024;

/// Clock injected into polling/cache logic.
pub trait Clock {
    /// Unix timestamp in seconds.
    fn now(&self) -> u64;
}

/// Production wall clock.
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// Streaming response supplied by the transport abstraction.
pub struct HttpResponse {
    /// HTTP status.
    pub status: u16,
    /// Cache validator, if present.
    pub etag: Option<String>,
    /// Retry delay in seconds, if throttled.
    pub retry_after: Option<u64>,
    /// Response stream, bounded by the caller.
    pub body: Box<dyn Read + Send>,
}

/// HTTP transport for release lookup and payload downloads.
pub trait Http {
    /// Requests an HTTPS resource with an optional cache validator.
    ///
    /// # Errors
    /// Returns network or policy errors.
    fn get(&self, url: &str, etag: Option<&str>) -> Result<HttpResponse>;
}

/// Production HTTPS transport; has no token or arbitrary repository configuration.
pub struct GithubHttp {
    client: reqwest::blocking::Client,
}

/// Accepts only GitHub API and release asset download origins over HTTPS.
pub fn allowed_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

impl GithubHttp {
    /// Builds a bounded-time GitHub client with a constrained redirect policy.
    ///
    /// The client follows `HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY` and, with reqwest's
    /// `system-proxy` feature, Windows/macOS static system proxy settings. On
    /// Windows it verifies TLS with a snapshot of the system root store so
    /// online CRL/OCSP fetches cannot block update traffic.
    ///
    /// # Errors
    /// Returns errors initializing the HTTP client or loading Windows roots.
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: github_http_client()?,
        })
    }
}

fn github_http_client() -> Result<reqwest::blocking::Client> {
    apply_env_all_proxy(apply_update_tls(github_http_client_builder())?)?
        .build()
        .map_err(network)
}

fn github_http_client_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("teshi-update/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && allowed_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Disallowed release redirect")
            }
        }))
}

fn apply_update_tls(
    builder: reqwest::blocking::ClientBuilder,
) -> Result<reqwest::blocking::ClientBuilder> {
    #[cfg(windows)]
    {
        Ok(builder.tls_certs_only(windows_update_root_certificates()?))
    }
    #[cfg(not(windows))]
    {
        Ok(builder)
    }
}

fn apply_env_all_proxy(
    builder: reqwest::blocking::ClientBuilder,
) -> Result<reqwest::blocking::ClientBuilder> {
    // reqwest copies the Windows/macOS system proxy into the HTTPS slot when
    // HTTPS_PROXY is unset, which would hide a configured ALL_PROXY. GitHub
    // traffic is HTTPS-only, so prefer an explicit ALL_PROXY in that case.
    if !first_proxy_env(&["HTTPS_PROXY", "https_proxy"]).is_empty() {
        return Ok(builder);
    }
    let all = first_proxy_env(&["ALL_PROXY", "all_proxy"]);
    if all.is_empty() {
        return Ok(builder);
    }
    let proxy = reqwest::Proxy::all(&all)
        .map_err(network)?
        .no_proxy(reqwest::NoProxy::from_env());
    Ok(builder.proxy(proxy))
}

fn first_proxy_env(names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_default()
}

#[cfg(windows)]
fn windows_update_root_certificates() -> Result<Vec<reqwest::Certificate>> {
    let loaded = rustls_native_certs::load_native_certs();
    let load_errors = loaded.errors.iter().map(ToString::to_string).collect();
    certificates_from_ders(loaded.certs.iter().map(AsRef::as_ref), load_errors)
}

/// Converts DER roots into reqwest certificates, skipping unusable blobs.
///
/// # Errors
/// Returns a network error when no usable root remains.
#[cfg(any(windows, test))]
pub(crate) fn certificates_from_ders<'a>(
    ders: impl IntoIterator<Item = &'a [u8]>,
    load_errors: Vec<String>,
) -> Result<Vec<reqwest::Certificate>> {
    let mut certs = Vec::new();
    for der in ders {
        if !is_plausible_cert_der(der) {
            continue;
        }
        if let Ok(cert) = reqwest::Certificate::from_der(der) {
            certs.push(cert);
        }
    }
    if certs.is_empty() {
        let detail = if load_errors.is_empty() {
            "the certificate store returned no usable roots".to_string()
        } else {
            load_errors.join("; ")
        };
        return Err(UpdateError::new(
            ErrorCode::Network,
            format!("Unable to load Windows root certificates for update TLS: {detail}"),
        ));
    }
    Ok(certs)
}

#[cfg(any(windows, test))]
fn is_plausible_cert_der(der: &[u8]) -> bool {
    // X.509 certificates are DER SEQUENCEs; skip empty or truncated blobs so one
    // unreadable store entry cannot disable the entire update client.
    der.len() >= 64 && der[0] == 0x30
}

fn network(error: reqwest::Error) -> UpdateError {
    UpdateError::new(ErrorCode::Network, redact_proxy_secrets(&error.to_string()))
}

fn redact_proxy_secrets(message: &str) -> String {
    let mut out = String::new();
    let mut rest = message;
    while let Some(scheme) = rest.find("://") {
        out.push_str(&rest[..scheme + 3]);
        rest = &rest[scheme + 3..];
        let end = rest.find([' ', '\'', '"', '\n']).unwrap_or(rest.len());
        let authority = &rest[..end];
        if let Some(at) = authority.rfind('@') {
            out.push_str("***:***@");
            out.push_str(&authority[at + 1..]);
        } else {
            out.push_str(authority);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

impl Http for GithubHttp {
    fn get(&self, url: &str, etag: Option<&str>) -> Result<HttpResponse> {
        let parsed = reqwest::Url::parse(url).map_err(|e| invalid(e.to_string()))?;
        if !allowed_url(&parsed) {
            return Err(invalid("Disallowed release URL"));
        }
        let mut request = self
            .client
            .get(parsed)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let response = request.send().map_err(network)?;
        let etag = response
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse().ok())
            .or_else(|| {
                response
                    .headers()
                    .get("x-ratelimit-reset")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.parse::<u64>().ok())
                    .map(|t| t.saturating_sub(SystemClock.now()))
            });
        Ok(HttpResponse {
            status: response.status().as_u16(),
            etag,
            retry_after,
            body: Box::new(response),
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    until: u64,
    entries: BTreeMap<String, Cached>,
}
#[derive(Serialize, Deserialize)]
struct Cached {
    etag: Option<String>,
    body: String,
}

/// Release metadata pinned to GitHub release and asset IDs for installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    /// GitHub release identity.
    pub release_id: u64,
    /// GitHub payload identity.
    pub asset_id: u64,
    /// Validated release manifest.
    pub manifest: ReleaseManifest,
    /// Exact selected asset.
    pub asset: ReleaseAsset,
    /// Pinned release download URL.
    pub download_url: String,
    /// Human-readable release page.
    pub release_url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    assets: Vec<GithubAsset>,
}
#[derive(Deserialize)]
struct GithubAsset {
    id: u64,
    name: String,
    size: u64,
    browser_download_url: String,
}

/// A GitHub source with injected transport, clock and isolated cache directory.
pub struct GithubSource<H, C> {
    /// HTTPS transport.
    pub http: H,
    /// Polling clock.
    pub clock: C,
    /// User state directory, never project data.
    pub cache_dir: PathBuf,
}

impl<H: Http, C: Clock> GithubSource<H, C> {
    fn fetch(&self, cache: &mut Cache, url: &str) -> Result<String> {
        let previous = cache.entries.get(url);
        let response = self
            .http
            .get(url, previous.and_then(|v| v.etag.as_deref()))?;
        if response.status == 304 {
            return previous
                .map(|v| v.body.clone())
                .ok_or_else(|| invalid("304 without cached metadata"));
        }
        if response.status == 403 || response.status == 429 {
            cache.until = self
                .clock
                .now()
                .saturating_add(response.retry_after.unwrap_or(3600).max(60));
            return Err(UpdateError::new(
                ErrorCode::RateLimited,
                format!(
                    "GitHub rate limit; retry after {} seconds",
                    cache.until.saturating_sub(self.clock.now())
                ),
            ));
        }
        if response.status != 200 {
            return Err(UpdateError::new(
                ErrorCode::Network,
                format!("GitHub metadata request returned HTTP {}", response.status),
            ));
        }
        let mut body = String::new();
        response
            .body
            .take(METADATA_LIMIT + 1)
            .read_to_string(&mut body)?;
        if body.len() as u64 > METADATA_LIMIT {
            return Err(invalid("Metadata exceeds size limit"));
        }
        cache.entries.insert(
            url.into(),
            Cached {
                etag: response.etag,
                body: body.clone(),
            },
        );
        Ok(body)
    }

    /// Resolves the newest eligible release without downloading any payload archive.
    ///
    /// # Errors
    /// Returns network, rate-limit, compatibility or malformed-metadata errors.
    pub fn check(
        &self,
        current: &BuildIdentity,
        channel: ReleaseChannel,
        target: &str,
        kind: InstallKind,
        explicit: bool,
    ) -> Result<Option<Candidate>> {
        private_directory(&self.cache_dir)?;
        let _lock = StateLock::acquire(&self.cache_dir.join("check.lock"))?;
        let path = self.cache_dir.join("github.json");
        let mut cache: Cache = std::fs::read(&path)
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .unwrap_or_default();
        if cache.until > self.clock.now() {
            return Err(UpdateError::new(
                ErrorCode::RateLimited,
                format!(
                    "Next GitHub check allowed in {} seconds",
                    cache.until - self.clock.now()
                ),
            ));
        }
        let result = self.resolve(&mut cache, current, channel, target, kind, explicit);
        // Persist throttling even when the request failed. Bound old release entries.
        if cache.entries.len() > 2000 {
            cache.entries.clear();
        }
        write_json(&path, &cache)?;
        result
    }

    fn resolve(
        &self,
        cache: &mut Cache,
        current: &BuildIdentity,
        channel: ReleaseChannel,
        target: &str,
        kind: InstallKind,
        explicit: bool,
    ) -> Result<Option<Candidate>> {
        if channel == ReleaseChannel::Dev {
            return Err(invalid("Select stable or nightly"));
        }
        let mut releases = Vec::new();
        for page in 1..=100 {
            let body = self.fetch(cache, &format!("{API}?per_page=100&page={page}"))?;
            let batch: Vec<GithubRelease> = serde_json::from_str(&body)?;
            let finished = batch.len() < 100;
            releases.extend(
                batch
                    .into_iter()
                    .filter(|r| !r.draft && r.prerelease == (channel == ReleaseChannel::Nightly)),
            );
            if finished {
                break;
            }
            if page == 100 {
                return Err(invalid(
                    "Release pagination limit exceeded; refusing incomplete selection",
                ));
            }
        }
        let current_version =
            semver::Version::parse(&current.semver).map_err(|e| invalid(e.to_string()))?;
        let mut possible = Vec::new();
        for release in releases {
            let Some(tag) = release.tag_name.strip_prefix('v') else {
                continue;
            };
            let base = tag.split("-nightly.").next().unwrap_or(tag);
            let Ok(version) = semver::Version::parse(base) else {
                continue;
            };
            if version < current_version || !version.pre.is_empty() {
                continue;
            }
            if channel == ReleaseChannel::Stable && tag != base {
                continue;
            }
            if channel == ReleaseChannel::Nightly && !tag.contains("-nightly.") {
                continue;
            }
            if channel == ReleaseChannel::Stable
                && current.channel == channel
                && version == current_version
            {
                continue;
            }
            possible.push((version, release));
        }
        // Older stable releases cannot outrank the highest SemVer even if published later.
        if channel == ReleaseChannel::Stable
            && let Some(highest) = possible.iter().map(|(v, _)| v).max().cloned()
        {
            possible.retain(|(v, _)| *v == highest);
        }
        let mut best: Option<Candidate> = None;
        for (_, release) in possible {
            let meta = release
                .assets
                .iter()
                .find(|a| a.name == "update-manifest.json");
            let Some(meta) = meta else {
                // The first updater release is manually installed. Legacy snapshots
                // published before its build began cannot supersede that installation.
                let older = channel == ReleaseChannel::Nightly
                    && current.channel == channel
                    && release
                        .published_at
                        .as_deref()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .zip(chrono::DateTime::parse_from_rfc3339(&current.build_timestamp).ok())
                        .is_some_and(|(published, built)| published < built);
                if older {
                    continue;
                }
                return Err(invalid(format!(
                    "Release {} has no update manifest; install it manually",
                    release.tag_name
                )));
            };
            let sums_asset = release
                .assets
                .iter()
                .find(|a| a.name == "SHA256SUMS")
                .ok_or_else(|| invalid("Release has no SHA256SUMS"))?;
            let raw = self.fetch(cache, &meta.browser_download_url)?;
            let sums = self.fetch(cache, &sums_asset.browser_download_url)?;
            verify_checksum_text(&sums, "update-manifest.json", raw.as_bytes())?;
            let manifest: ReleaseManifest = serde_json::from_str(&raw)?;
            manifest.validate()?;
            if manifest.tag != release.tag_name || manifest.identity.channel != channel {
                return Err(invalid("GitHub release and manifest disagree"));
            }
            if !is_update(current, &manifest.identity, explicit)? {
                continue;
            }
            if best.as_ref().is_some_and(|b| {
                b.manifest.identity.build_sequence > manifest.identity.build_sequence
            }) && channel == ReleaseChannel::Nightly
            {
                continue;
            }
            let asset = manifest
                .assets
                .iter()
                .find(|a| a.target == target && a.kind == kind)
                .ok_or_else(|| {
                    invalid(format!(
                        "Release {} does not support {target}/{kind:?}",
                        manifest.tag
                    ))
                })?
                .clone();
            let remote = release
                .assets
                .iter()
                .find(|a| a.name == asset.name)
                .ok_or_else(|| invalid("Release payload missing"))?;
            if remote.size != asset.size || checksum_entry(&sums, &asset.name)? != asset.sha256 {
                return Err(invalid("Payload metadata disagrees with GitHub/SHA256SUMS"));
            }
            let release_url = format!(
                "https://github.com/teshi-org/teshi/releases/tag/{}",
                manifest.tag
            );
            best = Some(Candidate {
                release_id: release.id,
                asset_id: remote.id,
                manifest,
                asset,
                download_url: remote.browser_download_url.clone(),
                release_url,
            });
        }
        Ok(best)
    }
}

/// Finds exactly one checksum entry, rejecting duplicates and malformed hashes.
///
/// # Errors
/// Returns a verification error for absent or ambiguous entries.
pub fn checksum_entry<'a>(sums: &'a str, name: &str) -> Result<&'a str> {
    let mut found = None;
    for line in sums.lines() {
        if let Some((hash, filename)) = line.split_once(' ')
            && filename.trim_start_matches([' ', '*']) == name
        {
            if found.is_some() || !crate::manifest::valid_hash(hash) {
                return Err(invalid("Ambiguous checksum entry"));
            }
            found = Some(hash);
        }
    }
    found.ok_or_else(|| invalid(format!("Missing checksum for {name}")))
}

/// Verifies small metadata bytes against SHA256SUMS.
///
/// # Errors
/// Returns a verification error when bytes disagree.
pub fn verify_checksum_text(sums: &str, name: &str, bytes: &[u8]) -> Result<()> {
    use sha2::{Digest, Sha256};
    if checksum_entry(sums, name)? != format!("{:x}", Sha256::digest(bytes)) {
        return Err(UpdateError::new(
            ErrorCode::Verification,
            format!("Checksum mismatch for {name}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cert_der() -> &'static [u8] {
        include_bytes!("../tests/fixtures/teshi-update_test.crt.der")
    }

    #[test]
    fn allowed_url_accepts_github_https_origins_only() {
        for url in [
            "https://api.github.com/repos/teshi-org/teshi/releases",
            "https://github.com/teshi-org/teshi/releases/download/v1/a.zip",
            "https://release-assets.githubusercontent.com/a",
            "https://objects.githubusercontent.com/a",
        ] {
            assert!(allowed_url(&reqwest::Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            "http://api.github.com/repos/teshi-org/teshi/releases",
            "https://api.github.com:8443/repos/teshi-org/teshi/releases",
            "https://evil.example/payload",
            "https://user:pass@api.github.com/repos/teshi-org/teshi/releases",
            "http://127.0.0.1:8888",
        ] {
            assert!(!allowed_url(&reqwest::Url::parse(url).unwrap()), "{url}");
        }
    }

    #[test]
    fn github_http_rejects_disallowed_urls_before_network() {
        let http = GithubHttp::new().expect("github client");
        let Err(error) = http.get("https://evil.example/payload", None) else {
            panic!("policy");
        };
        assert_eq!(error.message, "Disallowed release URL");
    }

    #[test]
    fn github_http_constructs_a_strict_tls_client() {
        GithubHttp::new().expect("github client should construct");
    }

    #[test]
    #[ignore = "optional live GitHub network check"]
    fn live_github_tls_succeeds_without_online_revocation() {
        let http = GithubHttp::new().expect("github client");
        let result = http.get(
            "https://api.github.com/repos/teshi-org/teshi/releases?per_page=1",
            None,
        );
        match result {
            Ok(response) => assert!(
                matches!(response.status, 200 | 403 | 429),
                "unexpected GitHub status {}",
                response.status
            ),
            Err(error) => panic!("GitHub TLS/proxy request failed: {error}"),
        }
    }

    #[test]
    fn certificates_from_ders_skips_invalid_blobs_and_keeps_usable_roots() {
        let certs = certificates_from_ders(
            [test_cert_der(), b"", b"not-a-cert", &[0xff; 80]],
            Vec::new(),
        )
        .expect("usable root");
        assert_eq!(certs.len(), 1);
    }

    #[test]
    fn certificates_from_ders_fail_when_no_usable_roots_remain() {
        let error = certificates_from_ders(
            [b"".as_slice(), b"not-a-cert", &[0xff; 80]],
            vec!["store read failed".into()],
        )
        .expect_err("empty roots");
        assert_eq!(error.code, ErrorCode::Network);
        assert!(
            error
                .message
                .contains("Unable to load Windows root certificates for update TLS")
        );
        assert!(error.message.contains("store read failed"));
    }

    #[test]
    fn certificates_from_ders_fail_without_load_errors_when_store_is_empty() {
        let error = certificates_from_ders(std::iter::empty(), Vec::new()).expect_err("empty");
        assert!(error.message.contains("no usable roots"));
    }

    #[test]
    fn proxy_credentials_are_redacted_from_network_errors() {
        assert_eq!(
            redact_proxy_secrets(
                "error sending request for url (https://user:secret@proxy.local:8080/)"
            ),
            "error sending request for url (https://***:***@proxy.local:8080/)"
        );
        assert_eq!(
            redact_proxy_secrets("connection failed for https://github.com"),
            "connection failed for https://github.com"
        );
    }
}
