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
    /// # Errors
    /// Returns errors initializing the HTTP client.
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
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
            .build()
            .map_err(network)?;
        Ok(Self { client })
    }
}

fn network(error: reqwest::Error) -> UpdateError {
    UpdateError::new(ErrorCode::Network, error.to_string())
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
