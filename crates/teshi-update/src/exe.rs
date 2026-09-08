//! Per-user Windows setup.exe updates. Inno stages files; the helper journals them.

use crate::{Result, github::Candidate, invalid, manifest::InstallKind};
#[cfg(windows)]
use std::process::Command;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Directory Inno writes during `/update=true` so running binaries stay locked in place.
pub const STAGED_DIR: &str = "install";

/// Silent Inno arguments used for in-app upgrades. `/DIR=` is supplied separately.
pub fn silent_update_args() -> &'static [&'static str] {
    &["/VERYSILENT", "/NORESTART", "/CURRENTUSER", "/update=true"]
}

/// Ensures the candidate is the Windows setup payload, never ZIP/MSI.
///
/// # Errors
/// Returns [`crate::ErrorCode::Unsupported`] for any other asset kind or name.
pub fn require_exe_payload(candidate: &Candidate) -> Result<()> {
    if candidate.asset.kind != InstallKind::Exe || !candidate.asset.name.ends_with("-setup.exe") {
        return Err(crate::UpdateError::new(
            crate::ErrorCode::Unsupported,
            "EXE installations never apply ZIP, tar.gz or MSI payloads",
        ));
    }
    Ok(())
}

/// Copies a verified Inno staging tree, refusing links.
///
/// # Errors
/// Returns I/O or invalid-path errors when the tree cannot be duplicated.
pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        crate::install::reject_links(from, Path::new(&entry.file_name()))?;
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), dest)?;
        } else {
            return Err(invalid(format!(
                "Refusing non-file update payload: {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// Absolute `{app}\\install` path produced by a silent Inno update.
pub fn staged_payload(root: &Path) -> PathBuf {
    root.join(STAGED_DIR)
}

/// Runs the downloaded setup.exe to stage files beside the live install.
///
/// # Errors
/// Returns installation errors when Inno is missing, fails, or this OS is not Windows.
pub fn run_silent_setup(package: &Path, app_dir: &Path) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = (package, app_dir);
        return Err(crate::UpdateError::new(
            crate::ErrorCode::Unsupported,
            "Windows setup updates are not available on this operating system",
        ));
    }
    #[cfg(windows)]
    {
        let mut command = Command::new(package);
        command.args(silent_update_args()).arg(format!(
            "/DIR={}",
            app_dir
                .to_str()
                .ok_or_else(|| invalid("Install path is not valid Unicode"))?
        ));
        let status = command.status().map_err(|error| {
            crate::UpdateError::new(
                crate::ErrorCode::Installation,
                format!("Failed to start setup.exe: {error}"),
            )
        })?;
        if !status.success() {
            return Err(crate::UpdateError::new(
                crate::ErrorCode::Installation,
                format!("Setup.exe exited with {}", status.code().unwrap_or(-1)),
            ));
        }
        let staged = staged_payload(app_dir);
        if !staged.join(crate::manifest::BUNDLE_MANIFEST).is_file() {
            return Err(invalid(
                "Setup.exe did not stage teshi-bundle.json under install/",
            ));
        }
        Ok(())
    }
}

/// Best-effort removal of the Inno staging directory after a committed journal.
pub fn remove_staged_payload(root: &Path) {
    let staged = staged_payload(root);
    if staged.is_dir() {
        let _ = remove_dir_all_best_effort(&staged);
    }
}

fn remove_dir_all_best_effort(path: &Path) -> io::Result<()> {
    fs::remove_dir_all(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ReleaseAsset, ReleaseManifest};
    use teshi_core::version::{BuildIdentity, ReleaseChannel};

    #[test]
    fn silent_args_are_unattended_and_per_user() {
        let args = silent_update_args();
        assert!(args.contains(&"/VERYSILENT"));
        assert!(args.contains(&"/update=true"));
        assert!(args.contains(&"/CURRENTUSER"));
        assert!(!args.iter().any(|a| a.eq_ignore_ascii_case("/forcerestart")));
    }

    #[test]
    fn rejects_portable_or_msi_payloads() {
        let identity = BuildIdentity {
            semver: "0.7.10".into(),
            channel: ReleaseChannel::Stable,
            git_sha: "a".repeat(40),
            build_timestamp: "2026-09-08T00:00:00Z".into(),
            build_sequence: 1,
        };
        let candidate = |kind, name: &str| Candidate {
            release_id: 1,
            asset_id: 1,
            manifest: ReleaseManifest {
                schema: 1,
                minimum_updater: 1,
                identity: identity.clone(),
                tag: "v0.7.10".into(),
                assets: vec![],
            },
            asset: ReleaseAsset {
                name: name.into(),
                target: "x86_64-pc-windows-msvc".into(),
                kind,
                size: 10,
                sha256: "b".repeat(64),
            },
            download_url: "https://example.invalid/a".into(),
            release_url: "https://example.invalid/r".into(),
        };
        assert!(
            require_exe_payload(&candidate(
                InstallKind::Portable,
                "teshi-v0.7.10-x86_64-pc-windows-msvc.zip"
            ))
            .is_err()
        );
        assert!(
            require_exe_payload(&candidate(InstallKind::Msi, "teshi-v0.7.10-x64.msi")).is_err()
        );
        assert!(
            require_exe_payload(&candidate(InstallKind::Exe, "teshi-v0.7.10-x64-setup.exe"))
                .is_ok()
        );
    }
}
