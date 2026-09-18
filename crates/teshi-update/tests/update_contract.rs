//! Contract tests using isolated installations and an injected GitHub transport.

use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Cursor,
    path::Path,
    sync::{Mutex, atomic::AtomicBool},
};
use teshi_core::version::{BUILD_TARGET, BuildIdentity, ReleaseChannel};
use teshi_update::{
    ErrorCode, Result, download,
    github::{Candidate, Clock, GithubSource, Http, HttpResponse},
    install::Installation,
    manifest::{
        BUNDLE_MANIFEST, BundleManifest, InstallKind, ManagedFile, ReleaseAsset, ReleaseManifest,
    },
    policy::{self, UpdateSettings},
    storage::{StateLock, write_json},
};

const TARGET: &str = "x86_64-pc-windows-msvc";
const API: &str = "https://api.github.com/repos/teshi-org/teshi/releases?per_page=100&page=1";
const LATEST: &str = "https://api.github.com/repos/teshi-org/teshi/releases/latest";
/// Keep in sync with `NIGHTLY_MANIFEST_LIMIT` in teshi-update's GitHub source.
const NIGHTLY_MANIFEST_LIMIT: usize = 8;

fn identity(sequence: u64, sha: char) -> BuildIdentity {
    BuildIdentity {
        semver: "0.7.10".into(),
        channel: ReleaseChannel::Nightly,
        git_sha: sha.to_string().repeat(40),
        build_timestamp: "2026-09-08T12:00:00Z".into(),
        build_sequence: sequence,
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> u64 {
        1000
    }
}
#[derive(Default)]
struct FakeHttp {
    pages: BTreeMap<String, String>,
    calls: Mutex<Vec<(String, Option<String>)>>,
    throttled: bool,
    not_found: bool,
}
impl Http for FakeHttp {
    fn get(&self, url: &str, etag: Option<&str>) -> Result<HttpResponse> {
        self.calls
            .lock()
            .unwrap()
            .push((url.into(), etag.map(str::to_owned)));
        Ok(HttpResponse {
            status: if self.throttled {
                429
            } else if self.not_found {
                404
            } else if etag.is_some() {
                304
            } else {
                200
            },
            etag: Some("test-etag".into()),
            retry_after: Some(60),
            body: Box::new(Cursor::new(
                self.pages
                    .get(url)
                    .expect("unexpected request")
                    .as_bytes()
                    .to_vec(),
            )),
        })
    }
}

fn add_release(
    http: &mut FakeHttp,
    id: u64,
    build: BuildIdentity,
    target: &str,
) -> serde_json::Value {
    add_dated_release(http, id, build, target, "2026-09-08T13:00:00Z")
}

fn add_dated_release(
    http: &mut FakeHttp,
    id: u64,
    build: BuildIdentity,
    target: &str,
    published_at: &str,
) -> serde_json::Value {
    let tag = match build.channel {
        ReleaseChannel::Nightly => {
            format!("v{}-nightly.20260908.{}", build.semver, &build.git_sha[..7])
        }
        ReleaseChannel::Stable => format!("v{}", build.semver),
        ReleaseChannel::Dev => format!("v{}-dev", build.semver),
    };
    let prerelease = build.channel == ReleaseChannel::Nightly;
    let asset = ReleaseAsset {
        name: format!("teshi-{tag}-{target}.zip"),
        target: target.into(),
        kind: InstallKind::Portable,
        size: 1,
        sha256: hash(b"x"),
    };
    let manifest = ReleaseManifest {
        schema: 1,
        minimum_updater: 1,
        identity: build,
        tag: tag.clone(),
        assets: vec![asset.clone()],
    };
    let raw = serde_json::to_string(&manifest).unwrap();
    let meta_url =
        format!("https://github.com/teshi-org/teshi/releases/download/{tag}/update-manifest.json");
    let sums_url = format!("https://github.com/teshi-org/teshi/releases/download/{tag}/SHA256SUMS");
    http.pages.insert(meta_url.clone(), raw.clone());
    http.pages.insert(
        sums_url.clone(),
        format!(
            "{}  update-manifest.json\n{}  {}\n",
            hash(raw.as_bytes()),
            asset.sha256,
            asset.name
        ),
    );
    serde_json::json!({"id":id,"tag_name":tag,"draft":false,"prerelease":prerelease,"published_at":published_at,"assets":[
        {"id":id*10,"name":"update-manifest.json","size":raw.len(),"browser_download_url":meta_url},
        {"id":id*10+1,"name":"SHA256SUMS","size":200,"browser_download_url":sums_url},
        {"id":id*10+2,"name":asset.name,"size":1,"browser_download_url":"https://github.com/payload"}]})
}

#[test]
fn github_selects_newest_sequence_not_listing_order_and_reuses_etags() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    let newer = add_release(&mut http, 2, identity(12, 'b'), TARGET);
    let older = add_release(&mut http, 3, identity(11, 'f'), TARGET);
    http.pages
        .insert(API.into(), serde_json::json!([newer, older]).to_string());
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    for _ in 0..2 {
        let candidate = source
            .check(
                &identity(10, 'a'),
                ReleaseChannel::Nightly,
                TARGET,
                InstallKind::Portable,
                false,
            )
            .unwrap()
            .unwrap();
        assert_eq!(candidate.manifest.identity.build_sequence, 12);
        assert_eq!(candidate.release_id, 2);
    }
    let calls = source.http.calls.lock().unwrap();
    assert!(calls.iter().any(|(_, etag)| etag.is_some()));
    assert!(calls.iter().all(|(url, _)| !url.ends_with(".zip")));
}

#[test]
fn rate_limit_is_persisted_and_blocks_repeated_requests() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp {
        throttled: true,
        ..Default::default()
    };
    http.pages.insert(API.into(), "[]".into());
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    for _ in 0..2 {
        assert_eq!(
            source
                .check(
                    &identity(10, 'a'),
                    ReleaseChannel::Nightly,
                    TARGET,
                    InstallKind::Portable,
                    false
                )
                .unwrap_err()
                .code,
            ErrorCode::RateLimited
        );
    }
    assert_eq!(source.http.calls.lock().unwrap().len(), 1);
}

#[test]
fn partial_newer_release_is_not_silently_skipped() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    let valid = add_release(&mut http, 2, identity(12, 'b'), TARGET);
    let incomplete = serde_json::json!({"id":3,"tag_name":"v0.7.10-nightly.20260908.ccccccc","draft":false,"prerelease":true,"published_at":"2026-09-08T13:30:00Z","assets":[]});
    http.pages.insert(
        API.into(),
        serde_json::json!([valid, incomplete]).to_string(),
    );
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    assert!(
        source
            .check(
                &identity(10, 'a'),
                ReleaseChannel::Nightly,
                TARGET,
                InstallKind::Portable,
                false
            )
            .unwrap_err()
            .message
            .contains("no update manifest")
    );
}

#[test]
fn same_day_legacy_release_before_current_build_does_not_block_updates() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    let valid = add_release(&mut http, 2, identity(12, 'b'), TARGET);
    let legacy = serde_json::json!({"id":1,"tag_name":"v0.7.10-nightly.20260908.ccccccc","draft":false,"prerelease":true,"published_at":"2026-09-08T01:00:00Z","assets":[]});
    http.pages
        .insert(API.into(), serde_json::json!([valid, legacy]).to_string());
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    assert!(
        source
            .check(
                &identity(10, 'a'),
                ReleaseChannel::Nightly,
                TARGET,
                InstallKind::Portable,
                false
            )
            .unwrap()
            .is_some()
    );
}

#[test]
fn check_lock_blocks_a_second_process() {
    let temp = tempfile::tempdir().unwrap();
    let _lock = StateLock::acquire(&temp.path().join("check.lock")).unwrap();
    let source = GithubSource {
        http: FakeHttp::default(),
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    assert_eq!(
        source
            .check(
                &identity(10, 'a'),
                ReleaseChannel::Nightly,
                TARGET,
                InstallKind::Portable,
                false
            )
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
}

fn candidate(bytes: &[u8]) -> Candidate {
    let build = identity(12, 'b');
    let tag = "v0.7.10-nightly.20260908.bbbbbbb".to_string();
    let asset = ReleaseAsset {
        name: format!("teshi-{tag}-{TARGET}.zip"),
        target: TARGET.into(),
        kind: InstallKind::Portable,
        size: bytes.len() as u64,
        sha256: hash(bytes),
    };
    Candidate {
        release_id: 1,
        asset_id: 2,
        manifest: ReleaseManifest {
            schema: 1,
            minimum_updater: 1,
            identity: build,
            tag,
            assets: vec![asset.clone()],
        },
        asset,
        download_url: "https://github.com/payload".into(),
        release_url: "https://github.com/release".into(),
    }
}

#[test]
fn truncated_download_and_cancellation_remove_partial_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    http.pages
        .insert("https://github.com/payload".into(), "short".into());
    let destination = temp.path().join("payload.zip");
    assert_eq!(
        download::download(
            &http,
            &candidate(b"long expected contents"),
            &destination,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .unwrap_err()
        .code,
        ErrorCode::Verification
    );
    assert!(!destination.exists());
    assert_eq!(
        download::download(
            &http,
            &candidate(b"short"),
            &destination,
            &AtomicBool::new(true),
            &mut |_| {}
        )
        .unwrap_err()
        .code,
        ErrorCode::Cancelled
    );
    assert!(!destination.exists());
}

#[test]
fn download_rejects_checksum_mismatch_before_install() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    http.pages
        .insert("https://github.com/payload".into(), "wrong".into());
    let destination = temp.path().join("payload.zip");
    let error = download::download(
        &http,
        &candidate(b"right"),
        &destination,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Verification);
    assert!(!destination.exists());
}

fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn extraction_validates_payload_inventory_and_refuses_traversal() {
    let temp = tempfile::tempdir().unwrap();
    let mut expected = candidate(b"");
    let prefix = format!("teshi-{}-{TARGET}", expected.manifest.tag);
    let files = vec![
        ManagedFile {
            path: "teshi.exe".into(),
            size: 3,
            sha256: hash(b"exe"),
            executable: true,
        },
        ManagedFile {
            path: "teshi-update-helper.exe".into(),
            size: 3,
            sha256: hash(b"exe"),
            executable: true,
        },
    ];
    let bundle = BundleManifest {
        schema: 1,
        identity: expected.manifest.identity.clone(),
        target: TARGET.into(),
        kind: InstallKind::Portable,
        layout: 1,
        update_explanation: None,
        files,
    };
    let raw = serde_json::to_vec(&bundle).unwrap();
    let bytes = archive(&[
        (&format!("{prefix}/{BUNDLE_MANIFEST}"), &raw),
        (&format!("{prefix}/teshi.exe"), b"exe"),
        (&format!("{prefix}/teshi-update-helper.exe"), b"exe"),
    ]);
    let path = temp.path().join("payload.zip");
    std::fs::write(&path, &bytes).unwrap();
    expected.asset.size = bytes.len() as u64;
    assert!(download::extract(&path, &temp.path().join("good"), &expected).is_ok());
    std::fs::write(&path, archive(&[(&format!("{prefix}/../escaped"), b"bad")])).unwrap();
    assert!(download::extract(&path, &temp.path().join("bad"), &expected).is_err());
    assert!(!temp.path().join("escaped").exists());
}

#[test]
fn github_paginates_to_the_newest_eligible_release() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    let newest = add_release(&mut http, 99, identity(20, 'c'), TARGET);
    let page1 = (0..100)
        .map(|i| {
            serde_json::json!({
                "id": 1000 + i,
                "tag_name": format!("v0.1.0-unused-{i}"),
                "draft": true,
                "prerelease": true,
                "published_at": "2026-01-01T00:00:00Z",
                "assets": []
            })
        })
        .collect::<Vec<_>>();
    http.pages
        .insert(API.into(), serde_json::json!(page1).to_string());
    http.pages.insert(
        "https://api.github.com/repos/teshi-org/teshi/releases?per_page=100&page=2".into(),
        serde_json::json!([newest]).to_string(),
    );
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    let candidate = source
        .check(
            &identity(10, 'a'),
            ReleaseChannel::Nightly,
            TARGET,
            InstallKind::Portable,
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(candidate.release_id, 99);
    assert_eq!(candidate.manifest.identity.build_sequence, 20);
}

#[test]
fn github_nightly_fetches_only_a_recent_manifest_window() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    // Oldest-first listing: without publish-date sorting the window would pick
    // sequence 9 instead of the newest sequence 21.
    let listed: Vec<_> = (2..=21)
        .map(|sequence| {
            add_dated_release(
                &mut http,
                sequence,
                nightly_build(sequence),
                TARGET,
                &format!("2026-09-08T{sequence:02}:00:00Z"),
            )
        })
        .collect();
    http.pages
        .insert(API.into(), serde_json::to_string(&listed).unwrap());
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    let candidate = source
        .check(
            &identity(1, 'a'),
            ReleaseChannel::Nightly,
            TARGET,
            InstallKind::Portable,
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(candidate.manifest.identity.build_sequence, 21);
    let calls = source.http.calls.lock().unwrap();
    let manifest_fetches = calls
        .iter()
        .filter(|(url, _)| url.ends_with("/update-manifest.json"))
        .count();
    assert_eq!(manifest_fetches, NIGHTLY_MANIFEST_LIMIT);
    let oldest_tag = format!(
        "/v0.7.10-nightly.20260908.{}/",
        &nightly_build(2).git_sha[..7]
    );
    assert!(!calls.iter().any(|(url, _)| url.contains(&oldest_tag)));
}

fn nightly_build(sequence: u64) -> BuildIdentity {
    BuildIdentity {
        semver: "0.7.10".into(),
        channel: ReleaseChannel::Nightly,
        git_sha: format!("c{sequence:03x}{}", "b".repeat(36)),
        build_timestamp: "2026-09-08T12:00:00Z".into(),
        build_sequence: sequence,
    }
}

#[test]
fn github_stable_uses_latest_release_endpoint() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    let mut current = identity(10, 'a');
    current.channel = ReleaseChannel::Stable;
    current.semver = "0.7.9".into();
    let mut latest_build = identity(11, 'b');
    latest_build.channel = ReleaseChannel::Stable;
    let latest = add_release(&mut http, 9, latest_build, TARGET);
    http.pages
        .insert(LATEST.into(), serde_json::to_string(&latest).unwrap());
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };
    let candidate = source
        .check(
            &current,
            ReleaseChannel::Stable,
            TARGET,
            InstallKind::Portable,
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(candidate.manifest.identity.semver, "0.7.10");
    let calls = source.http.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|(url, _)| url == LATEST || url.ends_with("/releases/latest"))
    );
    assert!(!calls.iter().any(|(url, _)| url == API));
}

#[test]
fn github_stable_falls_back_when_latest_is_not_newer() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp::default();
    let mut current = identity(10, 'a');
    current.channel = ReleaseChannel::Stable;
    current.semver = "0.7.10".into();
    let mut latest_build = identity(9, 'b');
    latest_build.channel = ReleaseChannel::Stable;
    latest_build.semver = "0.7.9".into();
    let latest = add_release(&mut http, 9, latest_build, TARGET);
    let mut highest_build = identity(12, 'c');
    highest_build.channel = ReleaseChannel::Stable;
    highest_build.semver = "0.7.11".into();
    let highest = add_release(&mut http, 10, highest_build, TARGET);
    http.pages
        .insert(LATEST.into(), serde_json::to_string(&latest).unwrap());
    http.pages.insert(
        API.into(),
        serde_json::to_string(&[latest, highest]).unwrap(),
    );
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };

    let candidate = source
        .check(
            &current,
            ReleaseChannel::Stable,
            TARGET,
            InstallKind::Portable,
            false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(candidate.manifest.identity.semver, "0.7.11");
    let calls = source.http.calls.lock().unwrap();
    assert!(calls.iter().any(|(url, _)| url == API));
}

#[test]
fn github_latest_404_means_no_stable_update() {
    let temp = tempfile::tempdir().unwrap();
    let mut http = FakeHttp {
        not_found: true,
        ..Default::default()
    };
    http.pages.insert(LATEST.into(), "{}".into());
    let mut current = identity(10, 'a');
    current.channel = ReleaseChannel::Stable;
    let source = GithubSource {
        http,
        clock: FixedClock,
        cache_dir: temp.path().into(),
    };

    assert!(
        source
            .check(
                &current,
                ReleaseChannel::Stable,
                TARGET,
                InstallKind::Portable,
                false,
            )
            .unwrap()
            .is_none()
    );
    let calls = source.http.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, LATEST);
}

#[test]
fn extraction_rejects_links_duplicates_and_wrong_target() {
    let temp = tempfile::tempdir().unwrap();
    let mut expected = candidate(b"");
    let prefix = format!("teshi-{}-{TARGET}", expected.manifest.tag);
    let files = vec![
        ManagedFile {
            path: "teshi.exe".into(),
            size: 3,
            sha256: hash(b"exe"),
            executable: true,
        },
        ManagedFile {
            path: "teshi-update-helper.exe".into(),
            size: 3,
            sha256: hash(b"exe"),
            executable: true,
        },
    ];
    let mut bundle = BundleManifest {
        schema: 1,
        identity: expected.manifest.identity.clone(),
        target: TARGET.into(),
        kind: InstallKind::Portable,
        layout: 1,
        update_explanation: None,
        files: files.clone(),
    };
    let raw = serde_json::to_vec(&bundle).unwrap();
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Cursor::new(Vec::new()),
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_cksum();
    tar.append_link(&mut header, format!("{prefix}/teshi.exe"), "elsewhere")
        .unwrap();
    let linked = tar.into_inner().unwrap().finish().unwrap().into_inner();
    expected.asset.name = expected.asset.name.replace(".zip", ".tar.gz");
    let path = temp.path().join("payload.tar.gz");
    std::fs::write(&path, &linked).unwrap();
    expected.asset.size = linked.len() as u64;
    assert!(
        download::extract(&path, &temp.path().join("link"), &expected).is_err(),
        "archive links must be rejected"
    );

    expected.asset.name = expected.asset.name.replace(".tar.gz", ".zip");
    let path = temp.path().join("payload.zip");
    let duplicate = archive(&[
        (&format!("{prefix}/{BUNDLE_MANIFEST}"), &raw),
        (&format!("{prefix}/teshi.exe"), b"exe"),
        (&format!("{prefix}/teshi-update-helper.exe"), b"exe"),
        (&format!("{prefix}/TESHI.EXE"), b"exe"),
    ]);
    std::fs::write(&path, &duplicate).unwrap();
    expected.asset.size = duplicate.len() as u64;
    assert!(download::extract(&path, &temp.path().join("dup"), &expected).is_err());

    bundle.target = "aarch64-apple-darwin".into();
    let wrong = serde_json::to_vec(&bundle).unwrap();
    let bytes = archive(&[
        (&format!("{prefix}/{BUNDLE_MANIFEST}"), &wrong),
        (&format!("{prefix}/teshi.exe"), b"exe"),
        (&format!("{prefix}/teshi-update-helper.exe"), b"exe"),
    ]);
    std::fs::write(&path, &bytes).unwrap();
    expected.asset.size = bytes.len() as u64;
    assert!(download::extract(&path, &temp.path().join("wrong"), &expected).is_err());
}

fn managed(path: &str, bytes: &[u8], executable: bool) -> ManagedFile {
    ManagedFile {
        path: path.into(),
        size: bytes.len() as u64,
        sha256: hash(bytes),
        executable,
    }
}

fn write_install(root: &Path, kind: InstallKind, identity: &BuildIdentity) -> std::path::PathBuf {
    let windows = BUILD_TARGET.contains("windows");
    let prefix = teshi_update::manifest::executable_prefix(kind);
    let suffix = if windows { ".exe" } else { "" };
    if prefix == "bin/" {
        std::fs::create_dir_all(root.join("bin")).unwrap();
    }
    let teshi = format!("{prefix}teshi{suffix}");
    let helper = format!("{prefix}teshi-update-helper{suffix}");
    std::fs::write(root.join(&teshi), b"cli").unwrap();
    std::fs::write(root.join(&helper), b"helper").unwrap();
    let bundle = BundleManifest {
        schema: 1,
        identity: identity.clone(),
        target: BUILD_TARGET.into(),
        kind,
        layout: 1,
        update_explanation: (kind == InstallKind::External)
            .then(|| "Use the package manager that installed Teshi".into()),
        files: vec![
            managed(&teshi, b"cli", true),
            managed(&helper, b"helper", true),
        ],
    };
    write_json(&root.join(BUNDLE_MANIFEST), &bundle).unwrap();
    root.join(teshi.replace('/', std::path::MAIN_SEPARATOR_STR))
}

#[test]
fn ownership_uses_manifest_not_package_manager_presence() {
    let temp = tempfile::tempdir().unwrap();
    let unmarked = temp.path().join("unmarked");
    std::fs::create_dir(&unmarked).unwrap();
    let exe = unmarked.join(if BUILD_TARGET.contains("windows") {
        "teshi.exe"
    } else {
        "teshi"
    });
    std::fs::write(&exe, b"cli").unwrap();
    std::fs::write(unmarked.join("winget.exe"), b"pm").unwrap();
    let unknown = Installation::detect(&exe, &identity(1, 'a')).unwrap();
    assert_eq!(unknown.kind, InstallKind::Unknown);
    assert!(!unknown.can_install());
    assert!(
        unknown
            .explanation
            .as_deref()
            .unwrap()
            .contains("unmarked/source")
    );

    let portable_root = temp.path().join("dedicated").join("teshi-0.7.9");
    std::fs::create_dir_all(&portable_root).unwrap();
    let portable_exe = write_install(&portable_root, InstallKind::Portable, &identity(1, 'a'));
    let portable = Installation::detect(&portable_exe, &identity(1, 'a')).unwrap();
    assert_eq!(portable.kind, InstallKind::Portable);
    assert!(!portable.can_install(), "{:?}", portable.explanation);
    assert!(
        portable
            .explanation
            .as_deref()
            .unwrap()
            .contains("cannot self-update")
    );

    let external_root = temp.path().join("dedicated").join("teshi-external");
    std::fs::create_dir_all(&external_root).unwrap();
    let external_exe = write_install(&external_root, InstallKind::External, &identity(1, 'a'));
    let external = Installation::detect(&external_exe, &identity(1, 'a')).unwrap();
    assert_eq!(external.kind, InstallKind::External);
    assert!(!external.can_install());
    assert!(
        external
            .explanation
            .as_deref()
            .unwrap()
            .contains("package manager")
    );

    let msi_root = temp.path().join("dedicated").join("teshi-msi");
    std::fs::create_dir_all(&msi_root).unwrap();
    let msi_exe = write_install(&msi_root, InstallKind::Msi, &identity(1, 'a'));
    let msi = Installation::detect(&msi_exe, &identity(1, 'a')).unwrap();
    assert_eq!(msi.kind, InstallKind::Msi);
    assert!(!msi.can_install());
    let msi_reason = msi.explanation.as_deref().unwrap();
    assert!(
        msi_reason.contains("setup") || msi_reason.contains("MSI"),
        "{msi_reason}"
    );

    if BUILD_TARGET.contains("windows") {
        let exe_root = temp.path().join("dedicated").join("teshi-exe");
        std::fs::create_dir_all(&exe_root).unwrap();
        let exe_path = write_install(&exe_root, InstallKind::Exe, &identity(1, 'a'));
        let exe = Installation::detect(&exe_path, &identity(1, 'a')).unwrap();
        assert_eq!(exe.kind, InstallKind::Exe);
        assert!(exe.can_install(), "{:?}", exe.explanation);
    }
}

#[test]
fn automatic_checks_skip_development_and_external_installations() {
    let temp = tempfile::tempdir().unwrap();
    let settings = UpdateSettings {
        auto_check: true,
        channel: None,
    };
    let unknown = Installation {
        root: None,
        kind: InstallKind::Unknown,
        bundle: None,
        explanation: Some("source build".into()),
    };
    assert!(!policy::claim_check(temp.path(), &unknown, &settings, 1_000).unwrap());

    let mut bundle = BundleManifest {
        schema: 1,
        identity: identity(1, 'a'),
        target: BUILD_TARGET.into(),
        kind: InstallKind::External,
        layout: 1,
        update_explanation: Some("apt".into()),
        files: vec![
            managed("teshi.exe", b"x", true),
            managed("teshi-update-helper.exe", b"x", true),
        ],
    };
    let external = Installation {
        root: Some(temp.path().into()),
        kind: InstallKind::External,
        bundle: Some(bundle.clone()),
        explanation: Some("apt".into()),
    };
    assert!(!policy::claim_check(temp.path(), &external, &settings, 1_000).unwrap());

    bundle.kind = InstallKind::Portable;
    bundle.identity.channel = ReleaseChannel::Dev;
    let development = Installation {
        root: Some(temp.path().into()),
        kind: InstallKind::Portable,
        bundle: Some(bundle),
        explanation: None,
    };
    assert!(!policy::claim_check(temp.path(), &development, &settings, 1_000).unwrap());
}

#[test]
fn manifest_rejects_case_alias_and_reserved_inventory_path() {
    let build = identity(1, 'a');
    let file = |path: &str| ManagedFile {
        path: path.into(),
        size: 1,
        sha256: hash(b"x"),
        executable: true,
    };
    let mut bundle = BundleManifest {
        schema: 1,
        identity: build,
        target: TARGET.into(),
        kind: InstallKind::Portable,
        layout: 1,
        update_explanation: None,
        files: vec![
            file("teshi.exe"),
            file("teshi-update-helper.exe"),
            file("TESHI.EXE"),
        ],
    };
    assert!(bundle.validate().is_err());
    bundle.files.pop();
    assert!(bundle.validate().is_ok());
    bundle.files.push(file(".teshi-update/journal.json"));
    assert!(bundle.validate().is_err());
}
