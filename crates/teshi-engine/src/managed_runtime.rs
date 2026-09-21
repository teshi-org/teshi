//! Teshi-owned managed runtimes for Python-backed capabilities.
//!
//! This module intentionally solves the WinApp runtime first. Runtime state is
//! stored under Teshi app-data, never inside the tested project.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use fd_lock::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_data::app_data_dir;

pub const WINAPP_RUNTIME_ID: &str = "winapp";
/// Runtime v4 adds authenticated WinApp sidecar connections.
pub const WINAPP_RUNTIME_VERSION: u32 = 4;

#[cfg(all(windows, target_arch = "x86_64"))]
pub const WINAPP_RUNTIME_PLATFORM: &str = "windows-x86_64";
#[cfg(not(all(windows, target_arch = "x86_64")))]
pub const WINAPP_RUNTIME_PLATFORM: &str = "unsupported";

/// Build-time production metadata. CI/release may inject these with
/// `TESHI_WINAPP_RUNTIME_URL`, `TESHI_WINAPP_RUNTIME_SHA256`, and
/// `TESHI_WINAPP_RUNTIME_SIZE` at compile time. Tests and maintainers can use
/// the runtime override environment variables below without trusting project files.
const BUILT_IN_WINAPP_RUNTIME_URL: Option<&str> = option_env!("TESHI_WINAPP_RUNTIME_URL");
const BUILT_IN_WINAPP_RUNTIME_SHA256: Option<&str> = option_env!("TESHI_WINAPP_RUNTIME_SHA256");
const BUILT_IN_WINAPP_RUNTIME_SIZE: Option<&str> = option_env!("TESHI_WINAPP_RUNTIME_SIZE");

/// Installed runtime descriptor returned to sidecar startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedRuntime {
    pub root: PathBuf,
    pub python_exe: PathBuf,
    pub service_script: PathBuf,
    pub runtime_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeManifest {
    pub runtime: String,
    pub runtime_version: u32,
    pub platform: String,
    pub python_exe: String,
    pub service_script: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRequirement {
    pub runtime: &'static str,
    pub runtime_version: u32,
    pub platform: &'static str,
}

#[derive(Debug, Clone)]
struct RuntimeArtifact {
    url: String,
    sha256: String,
    size: Option<u64>,
}

pub fn winapp_runtime_requirement() -> RuntimeRequirement {
    RuntimeRequirement {
        runtime: WINAPP_RUNTIME_ID,
        runtime_version: WINAPP_RUNTIME_VERSION,
        platform: WINAPP_RUNTIME_PLATFORM,
    }
}

pub fn winapp_runtime_dir_in(app_data: &Path) -> PathBuf {
    app_data
        .join("runtimes")
        .join(WINAPP_RUNTIME_ID)
        .join(WINAPP_RUNTIME_VERSION.to_string())
}

pub fn ensure_winapp_runtime() -> Result<ManagedRuntime> {
    ensure_runtime(&winapp_runtime_requirement())
}

fn ensure_runtime(requirement: &RuntimeRequirement) -> Result<ManagedRuntime> {
    if requirement.platform == "unsupported" {
        return Err(anyhow!(
            "Teshi WinApp runtime is only supported on Windows x86_64."
        ));
    }

    if let Ok(dir) = std::env::var("TESHI_WINAPP_RUNTIME_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return validate_runtime_dir(Path::new(trimmed), requirement)
                .context("validate TESHI_WINAPP_RUNTIME_DIR");
        }
    }

    let app_data = app_data_dir()?;
    let final_dir = winapp_runtime_dir_in(&app_data);
    if let Ok(runtime) = validate_runtime_dir(&final_dir, requirement) {
        return Ok(runtime);
    }

    let lock_path = app_data
        .join("runtimes")
        .join(WINAPP_RUNTIME_ID)
        .join(format!("{}.install.lock", requirement.runtime_version));
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("open runtime install lock {}", lock_path.display()))?;
    let mut lock = RwLock::new(lock_file);
    let _guard = lock
        .write()
        .context("acquire WinApp runtime install lock")?;

    if let Ok(runtime) = validate_runtime_dir(&final_dir, requirement) {
        return Ok(runtime);
    }

    eprintln!(
        "Preparing Teshi WinApp runtime v{}...",
        requirement.runtime_version
    );
    install_runtime(requirement, &final_dir)?;
    validate_runtime_dir(&final_dir, requirement)
}

fn install_runtime(requirement: &RuntimeRequirement, final_dir: &Path) -> Result<()> {
    let artifact = runtime_artifact()?;
    let parent = final_dir
        .parent()
        .ok_or_else(|| anyhow!("invalid runtime directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let install_id = format!(
        ".installing-{}-{}",
        requirement.runtime_version,
        std::process::id()
    );
    let tmp_archive = parent.join(format!("{install_id}.zip"));
    let tmp_dir = parent.join(&install_id);
    fs::remove_file(&tmp_archive).ok();
    fs::remove_dir_all(&tmp_dir).ok();

    let install_result = (|| -> Result<()> {
        download_or_copy_artifact(&artifact, &tmp_archive)?;
        verify_file_sha256(&tmp_archive, &artifact.sha256)?;
        extract_zip(&tmp_archive, &tmp_dir)?;
        validate_runtime_dir(&tmp_dir, requirement)?;
        if final_dir.exists() {
            if validate_runtime_dir(final_dir, requirement).is_ok() {
                return Ok(());
            }
            return Err(anyhow!(
                "refusing to replace invalid existing Teshi WinApp runtime at {}",
                final_dir.display()
            ));
        }
        fs::rename(&tmp_dir, final_dir).with_context(|| {
            format!(
                "publish Teshi WinApp runtime {} -> {}",
                tmp_dir.display(),
                final_dir.display()
            )
        })?;
        Ok(())
    })();

    fs::remove_file(&tmp_archive).ok();
    if tmp_dir.exists() {
        fs::remove_dir_all(&tmp_dir).ok();
    }

    install_result.with_context(|| {
        format!(
            "Failed to prepare Teshi WinApp runtime. Required: WinApp runtime {}. The existing runtime was not modified.",
            requirement.runtime_version
        )
    })
}

fn runtime_artifact() -> Result<RuntimeArtifact> {
    if let Ok(path) = std::env::var("TESHI_WINAPP_RUNTIME_ARTIFACT") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            let sha256 = std::env::var("TESHI_WINAPP_RUNTIME_SHA256").context(
                "TESHI_WINAPP_RUNTIME_SHA256 is required with TESHI_WINAPP_RUNTIME_ARTIFACT",
            )?;
            return Ok(RuntimeArtifact {
                url: format!("file://{}", Path::new(trimmed).display()),
                sha256,
                size: None,
            });
        }
    }

    let url = BUILT_IN_WINAPP_RUNTIME_URL
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            anyhow!("Teshi WinApp runtime download metadata is not available in this build")
        })?;
    let sha256 = BUILT_IN_WINAPP_RUNTIME_SHA256
        .filter(|v| valid_sha256(v))
        .ok_or_else(|| {
            anyhow!("Teshi WinApp runtime checksum metadata is not available in this build")
        })?;
    let size = BUILT_IN_WINAPP_RUNTIME_SIZE.and_then(|v| v.parse::<u64>().ok());
    Ok(RuntimeArtifact {
        url: url.to_string(),
        sha256: sha256.to_string(),
        size,
    })
}

fn download_or_copy_artifact(artifact: &RuntimeArtifact, destination: &Path) -> Result<()> {
    if let Some(local) = artifact.url.strip_prefix("file://") {
        fs::copy(local, destination)
            .with_context(|| format!("copy runtime artifact from {local}"))?;
        return Ok(());
    }
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .context("create runtime download client")?
        .get(&artifact.url)
        .send()
        .context("download Teshi WinApp runtime")?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "runtime download returned HTTP {}",
            response.status()
        ));
    }
    if let Some(expected) = artifact.size {
        if response
            .content_length()
            .is_some_and(|actual| actual != expected)
        {
            return Err(anyhow!("runtime download size did not match metadata"));
        }
    }
    let bytes = response.bytes().context("read runtime download")?;
    if let Some(expected) = artifact.size {
        if bytes.len() as u64 != expected {
            return Err(anyhow!("runtime download length did not match metadata"));
        }
    }
    fs::write(destination, &bytes).with_context(|| format!("write {}", destination.display()))
}

fn verify_file_sha256(path: &Path, expected: &str) -> Result<()> {
    if !valid_sha256(expected) {
        return Err(anyhow!("invalid expected SHA-256 for runtime artifact"));
    }
    let actual = file_sha256(path)?;
    if actual != expected.to_ascii_lowercase() {
        return Err(anyhow!("runtime artifact checksum verification failed"));
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buf)?;
        if count == 0 {
            break;
        }
        hash.update(&buf[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| format!("create {}", destination.display()))?;
    let file = File::open(archive).with_context(|| format!("open {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("read runtime zip")?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).context("read runtime zip entry")?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("runtime archive contains unsafe path"))?
            .to_path_buf();
        let out = destination.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&out).with_context(|| format!("create {}", out.display()))?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            let mut output =
                File::create(&out).with_context(|| format!("create {}", out.display()))?;
            std::io::copy(&mut entry, &mut output)
                .with_context(|| format!("extract {}", out.display()))?;
            output.flush().ok();
        }
    }
    Ok(())
}

pub fn validate_runtime_dir(
    root: &Path,
    requirement: &RuntimeRequirement,
) -> Result<ManagedRuntime> {
    let manifest_path = root.join("runtime.json");
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: RuntimeManifest = serde_json::from_str(&text).context("parse runtime.json")?;
    validate_manifest(&manifest, requirement)?;
    let python_exe = root.join(&manifest.python_exe);
    if !python_exe.is_file() {
        return Err(anyhow!("Teshi WinApp runtime is missing its launcher"));
    }
    let service_script = root.join(&manifest.service_script);
    if !service_script.is_file() {
        return Err(anyhow!("Teshi WinApp runtime is missing WinApp service"));
    }
    Ok(ManagedRuntime {
        root: root.to_path_buf(),
        python_exe,
        service_script,
        runtime_version: manifest.runtime_version,
    })
}

pub fn validate_manifest(
    manifest: &RuntimeManifest,
    requirement: &RuntimeRequirement,
) -> Result<()> {
    if manifest.runtime != requirement.runtime {
        return Err(anyhow!("wrong runtime id"));
    }
    if manifest.runtime_version != requirement.runtime_version {
        return Err(anyhow!("wrong runtime version"));
    }
    if manifest.platform != requirement.platform {
        return Err(anyhow!("wrong runtime platform"));
    }
    if manifest.python_exe.trim().is_empty() || manifest.service_script.trim().is_empty() {
        return Err(anyhow!("runtime manifest is missing required paths"));
    }
    if Path::new(&manifest.python_exe).is_absolute()
        || Path::new(&manifest.service_script).is_absolute()
        || manifest.python_exe.contains("..")
        || manifest.service_script.contains("..")
    {
        return Err(anyhow!("runtime manifest contains unsafe paths"));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::sync::Mutex;
    use tempfile::tempdir;
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    use zip::write::FileOptions;

    fn req() -> RuntimeRequirement {
        RuntimeRequirement {
            runtime: WINAPP_RUNTIME_ID,
            runtime_version: 1,
            // Exercise the Windows artifact lifecycle on every CI host.
            platform: "windows-x86_64",
        }
    }

    fn write_runtime(root: &Path, manifest: RuntimeManifest) {
        fs::create_dir_all(root.join("python")).unwrap();
        fs::create_dir_all(root.join("resources")).unwrap();
        fs::write(root.join("python/python.exe"), b"python").unwrap();
        fs::write(root.join("resources/winapp_service.py"), b"print('ok')").unwrap();
        fs::write(
            root.join("runtime.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn good_manifest() -> RuntimeManifest {
        RuntimeManifest {
            runtime: WINAPP_RUNTIME_ID.into(),
            runtime_version: 1,
            platform: "windows-x86_64".into(),
            python_exe: "python/python.exe".into(),
            service_script: "resources/winapp_service.py".into(),
        }
    }

    #[test]
    fn runtime_requirement_is_versioned_independent_of_teshi_version() {
        let requirement = winapp_runtime_requirement();
        assert_eq!(requirement.runtime, "winapp");
        assert_eq!(requirement.runtime_version, 4);
    }

    #[cfg(not(all(windows, target_arch = "x86_64")))]
    #[test]
    fn production_winapp_runtime_remains_unsupported_on_other_platforms() {
        assert_eq!(winapp_runtime_requirement().platform, "unsupported");
        assert!(ensure_winapp_runtime().is_err());
    }

    #[test]
    fn runtime_installation_path_uses_app_data_not_project() {
        let app = PathBuf::from(r"C:\Users\u\AppData\Roaming\teshi");
        let path = winapp_runtime_dir_in(&app);
        assert!(path.ends_with(Path::new("runtimes").join("winapp").join("4")));
        assert!(!path.to_string_lossy().contains(".venv"));
        assert!(!path.to_string_lossy().contains(".teshi\\runtime"));
    }

    #[test]
    fn manifest_validation_rejects_wrong_identity_platform_and_version() {
        let mut manifest = good_manifest();
        validate_manifest(&manifest, &req()).unwrap();
        manifest.runtime = "browser".into();
        assert!(validate_manifest(&manifest, &req()).is_err());
        manifest = good_manifest();
        manifest.runtime_version = 2;
        assert!(validate_manifest(&manifest, &req()).is_err());
        manifest = good_manifest();
        manifest.platform = "windows-aarch64".into();
        assert!(validate_manifest(&manifest, &req()).is_err());
    }

    #[test]
    fn authenticated_runtime_rejects_installed_v3_runtime() {
        let mut requirement = req();
        requirement.runtime_version = WINAPP_RUNTIME_VERSION;
        let mut old_manifest = good_manifest();
        old_manifest.runtime_version = 3;
        assert!(validate_manifest(&old_manifest, &requirement).is_err());
        let installed = tempdir().unwrap();
        write_runtime(installed.path(), old_manifest);
        assert!(validate_runtime_dir(installed.path(), &requirement).is_err());
        let mut upgraded = good_manifest();
        upgraded.runtime_version = WINAPP_RUNTIME_VERSION;
        validate_manifest(&upgraded, &requirement).unwrap();
        assert!(include_str!("../../../scripts/build-winapp-runtime.ps1")
            .contains("$RuntimeVersion = 4"));
        assert!(
            include_str!("../../../.github/workflows/release.yml").contains("-RuntimeVersion 4")
        );
        assert!(include_str!("../../../.github/workflows/release.yml")
            .contains("winapp-runtime-windows-x86_64-v4.zip"));
        assert!(!include_str!("../../../.github/workflows/release.yml")
            .contains("winapp-runtime-windows-x86_64-v3"));
        assert!(!include_str!("../../../.github/workflows/release.yml")
            .contains("winapp-runtime-windows-x86_64-v2"));
        assert!(!include_str!("../../../.github/workflows/release.yml")
            .contains("winapp-runtime-windows-x86_64-v1"));
        assert!(include_str!("../../../scripts/build-winapp-runtime.ps1")
            .contains("resources/update_participant.py"));
    }

    #[test]
    fn malformed_manifest_and_missing_executable_are_rejected() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.path().join("runtime.json"), b"not json").unwrap();
        assert!(validate_runtime_dir(dir.path(), &req()).is_err());

        fs::write(
            dir.path().join("runtime.json"),
            serde_json::to_string(&good_manifest()).unwrap(),
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("resources")).unwrap();
        fs::write(dir.path().join("resources/winapp_service.py"), b"").unwrap();
        assert!(validate_runtime_dir(dir.path(), &req()).is_err());
    }

    #[test]
    fn complete_runtime_is_selected() {
        let dir = tempdir().unwrap();
        write_runtime(dir.path(), good_manifest());
        let runtime = validate_runtime_dir(dir.path(), &req()).unwrap();
        assert_eq!(runtime.runtime_version, 1);
        assert!(runtime.python_exe.ends_with("python/python.exe"));
    }

    fn runtime_zip() -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options: FileOptions<'_, ()> = FileOptions::default();
            zip.start_file("runtime.json", options).unwrap();
            zip.write_all(serde_json::to_string(&good_manifest()).unwrap().as_bytes())
                .unwrap();
            zip.start_file("python/python.exe", options).unwrap();
            zip.write_all(b"python").unwrap();
            zip.start_file("resources/winapp_service.py", options)
                .unwrap();
            zip.write_all(b"print('ok')").unwrap();
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("artifact.zip");
        fs::write(&file, b"payload").unwrap();
        assert!(verify_file_sha256(&file, &"0".repeat(64)).is_err());
    }

    #[test]
    fn missing_runtime_installs_transactionally_from_verified_archive() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let archive = dir.path().join("runtime.zip");
        let bytes = runtime_zip();
        fs::write(&archive, &bytes).unwrap();
        let sha = format!("{:x}", Sha256::digest(&bytes));
        std::env::set_var("TESHI_WINAPP_RUNTIME_ARTIFACT", &archive);
        std::env::set_var("TESHI_WINAPP_RUNTIME_SHA256", sha);

        let final_dir = dir.path().join("runtimes/winapp/1");
        install_runtime(&req(), &final_dir).unwrap();
        let runtime = validate_runtime_dir(&final_dir, &req()).unwrap();
        assert!(runtime.service_script.is_file());
        assert!(!final_dir.with_file_name(".installing-1").exists());

        std::env::remove_var("TESHI_WINAPP_RUNTIME_ARTIFACT");
        std::env::remove_var("TESHI_WINAPP_RUNTIME_SHA256");
    }

    #[test]
    fn existing_compatible_runtime_is_reused_without_download() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let final_dir = winapp_runtime_dir_in(dir.path());
        write_runtime(&final_dir, good_manifest());
        let original = fs::read(final_dir.join("resources/winapp_service.py")).unwrap();

        std::env::set_var("TESHI_APP_DATA_DIR", dir.path());
        let bad = dir.path().join("bad.zip");
        fs::write(&bad, b"not a zip").unwrap();
        std::env::set_var("TESHI_WINAPP_RUNTIME_ARTIFACT", &bad);
        std::env::set_var("TESHI_WINAPP_RUNTIME_SHA256", "0".repeat(64));
        ensure_runtime(&req()).unwrap();
        assert_eq!(
            fs::read(final_dir.join("resources/winapp_service.py")).unwrap(),
            original
        );

        std::env::remove_var("TESHI_WINAPP_RUNTIME_ARTIFACT");
        std::env::remove_var("TESHI_WINAPP_RUNTIME_SHA256");
        std::env::remove_var("TESHI_APP_DATA_DIR");
    }
}
