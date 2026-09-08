//! Installation provenance and managed-root validation.

use crate::{
    ErrorCode, Result, UpdateError,
    manifest::{BUNDLE_MANIFEST, BundleManifest, InstallKind},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use teshi_core::version::{BUILD_TARGET, BuildIdentity};

/// Resolved installation; unsupported roots remain useful for check-only output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installation {
    /// Canonical installation root, if recognized.
    pub root: Option<PathBuf>,
    /// Installation authority.
    pub kind: InstallKind,
    /// Validated bundle metadata when present.
    pub bundle: Option<BundleManifest>,
    /// Why direct installation is disabled.
    pub explanation: Option<String>,
}

impl Installation {
    /// Detects ownership from the actual executable, never from the working directory.
    ///
    /// # Errors
    /// Returns errors resolving the executable or reading malformed manifests.
    pub fn detect(executable: &Path, identity: &BuildIdentity) -> Result<Self> {
        let executable = fs::canonicalize(executable)?;
        let parent = executable
            .parent()
            .ok_or_else(|| UpdateError::new(ErrorCode::Unsupported, "Executable has no parent"))?;
        let roots = [
            Some(parent),
            parent
                .parent()
                .filter(|_| parent.file_name().is_some_and(|n| n == "bin")),
        ];
        for root in roots.into_iter().flatten() {
            let manifest_path = root.join(BUNDLE_MANIFEST);
            if !manifest_path.is_file() {
                continue;
            }
            reject_links(root, Path::new(BUNDLE_MANIFEST))?;
            let bytes = fs::read(&manifest_path)?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err(crate::invalid("Bundle manifest too large"));
            }
            let bundle: BundleManifest = serde_json::from_slice(&bytes)?;
            bundle.validate()?;
            if bundle.target != BUILD_TARGET || &bundle.identity != identity {
                return Err(UpdateError::new(
                    ErrorCode::Unsupported,
                    "Installed manifest does not match the running build",
                ));
            }
            let relative = executable
                .strip_prefix(root)
                .map_err(|_| crate::invalid("Executable outside install root"))?
                .to_string_lossy()
                .replace('\\', "/");
            if !bundle
                .files
                .iter()
                .any(|f| f.path == relative && f.executable)
            {
                return Err(crate::invalid(
                    "Running executable is not part of this bundle",
                ));
            }
            let registered = is_registered_msi(root);
            let explanation = match bundle.kind {
                InstallKind::External => Some(bundle.update_explanation.clone().unwrap_or_else(|| "Use the package manager that installed Teshi".into())),
                InstallKind::Msi if !registered => Some("MSI registration does not match this installation; reinstall using the official MSI or the Windows setup program".into()),
                InstallKind::Msi => Some("MSI and WinGet installs cannot self-update. Install the Windows setup program (teshi-*-x64-setup.exe) for in-app updates".into()),
                InstallKind::Exe if !cfg!(windows) => Some("Windows setup updates are not available on this operating system".into()),
                InstallKind::Exe if registered => Some("Windows Installer owns this directory; setup.exe replacement is disabled".into()),
                InstallKind::Exe => None,
                InstallKind::Portable if registered => Some("Windows Installer owns this directory; portable replacement is disabled".into()),
                InstallKind::Portable if cfg!(windows) => Some("Portable archives cannot self-update. Install the Windows setup program (teshi-*-x64-setup.exe) for in-app updates".into()),
                InstallKind::Portable => Some("Portable archives cannot self-update. Replace the extracted bundle from GitHub Releases manually".into()),
                InstallKind::Unknown => Some("Unrecognized installation".into()),
            };
            return Ok(Self {
                root: Some(root.to_path_buf()),
                kind: bundle.kind,
                bundle: Some(bundle),
                explanation,
            });
        }
        Ok(Self { root: None, kind: InstallKind::Unknown, bundle: None,
            explanation: Some("This is an unmarked/source installation. Install an updater-enabled release bundle manually first".into()) })
    }

    /// Whether native payload replacement is currently supported.
    pub fn can_install(&self) -> bool {
        self.explanation.is_none() && self.kind == InstallKind::Exe && cfg!(windows)
    }
}

/// Checks every existing path component for symbolic links and Windows reparse points.
///
/// # Errors
/// Returns an error for unsafe links or inaccessible components.
pub fn reject_links(root: &Path, relative: &Path) -> Result<()> {
    let mut path = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            path.push(component);
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                #[cfg(windows)]
                let link = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let link = metadata.file_type().is_symlink();
                if link {
                    return Err(crate::invalid(format!(
                        "Refusing linked installation path: {}",
                        path.display()
                    )));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn is_registered_msi(root: &Path) -> bool {
    use winreg::{
        RegKey,
        enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY},
    };
    let hive = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(uninstall) = hive.open_subkey_with_flags(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        KEY_READ | KEY_WOW64_64KEY,
    ) else {
        return false;
    };
    uninstall
        .enum_keys()
        .filter_map(std::result::Result::ok)
        .any(|name| {
            let Ok(key) = uninstall.open_subkey(name) else {
                return false;
            };
            let name: String = key.get_value("DisplayName").unwrap_or_default();
            let publisher: String = key.get_value("Publisher").unwrap_or_default();
            let installer: u32 = key.get_value("WindowsInstaller").unwrap_or_default();
            let location: String = key.get_value("InstallLocation").unwrap_or_default();
            name == "teshi"
                && publisher == "teshi-org"
                && installer == 1
                && fs::canonicalize(location).is_ok_and(|p| p == root)
        })
}

#[cfg(not(windows))]
pub(crate) fn is_registered_msi(_root: &Path) -> bool {
    false
}
