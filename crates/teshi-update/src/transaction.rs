//! Journaled managed-file replacement and process participation.

use crate::{
    ErrorCode, Result, UpdateError, UpdateStatus,
    download::{file_hash, verify_bundle},
    install::reject_links,
    manifest::{BUNDLE_MANIFEST, BundleManifest, InstallKind},
    storage::{StateLock, private_directory, write_json},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const STATE: &str = ".teshi-update";

/// Persistent outcome displayed on the next invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResult {
    /// Transaction directory identifier.
    pub transaction_id: String,
    /// Final or pending phase.
    pub status: UpdateStatus,
    /// Human-readable result.
    pub detail: String,
    /// True only if Windows Installer requests an OS reboot.
    pub reboot_required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    schema: u32,
    old: BundleManifest,
    new: BundleManifest,
    paths: Vec<String>,
    // Every original is backed up before this list is advanced. An intent is
    // flushed before changing a path; recovery can safely repeat restoration.
    attempted: Vec<String>,
    committed: bool,
    restart: Option<Restart>,
}

/// Approved native desktop restart context. Executable selection is fixed by the helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Restart {
    /// Original desktop arguments, forwarded without a shell.
    pub arguments: Vec<String>,
    /// Original project working directory.
    pub working_directory: std::path::PathBuf,
}

/// A running application holds this lock until its process exits.
pub struct Participant {
    _file: tempfile::NamedTempFile,
}

/// Whether this process should register as an install participant.
///
/// Inspect-only commands must not take the installation gate, so
/// `teshi update --status` and `--check` can run while a helper holds it.
pub fn registers_install_participant<S: AsRef<str>>(args: &[S]) -> bool {
    let rest: Vec<&str> = args.iter().skip(1).map(AsRef::as_ref).collect();
    if rest.first().is_some_and(|arg| {
        matches!(
            *arg,
            "--update-identity" | "--help" | "-h" | "--version" | "-V"
        )
    }) {
        return false;
    }
    if rest.first() == Some(&"update") {
        return !rest.iter().any(|arg| {
            matches!(
                *arg,
                "--status" | "--check" | "--help" | "-h" | "--version" | "-V"
            )
        });
    }
    true
}

/// Registers a process in a marked installation, recovering interrupted work first.
///
/// # Errors
/// Blocks new application startup while an update holds the installation gate.
pub fn participate(executable: &Path) -> Result<Option<Participant>> {
    let executable = fs::canonicalize(executable)?;
    let Some(parent) = executable.parent() else {
        return Ok(None);
    };
    let root = if parent.join(BUNDLE_MANIFEST).is_file() {
        parent
    } else if parent.file_name().is_some_and(|s| s == "bin")
        && parent
            .parent()
            .is_some_and(|p| p.join(BUNDLE_MANIFEST).is_file())
    {
        parent
            .parent()
            .ok_or_else(|| crate::invalid("Missing install root"))?
    } else {
        return Ok(None);
    };
    let installed: BundleManifest = serde_json::from_slice(&fs::read(root.join(BUNDLE_MANIFEST))?)?;
    installed.validate()?;
    let state = match installed.kind {
        InstallKind::Portable | InstallKind::Exe => {
            reject_links(root, Path::new(STATE))?;
            root.join(STATE)
        }
        InstallKind::Msi | InstallKind::External | InstallKind::Unknown => return Ok(None),
    };
    private_directory(&state)?;
    let _gate = StateLock::acquire(&state.join("gate.lock"))?;
    recover_locked(root)?;
    let participants = state.join("participants");
    private_directory(&participants)?;
    let file = tempfile::Builder::new()
        .prefix("process-")
        .tempfile_in(participants)?;
    fs2::FileExt::try_lock_exclusive(file.as_file())?;
    Ok(Some(Participant { _file: file }))
}

/// Reads the last outcome without starting services or changing installation files.
pub fn last_result(root: &Path) -> Option<TransactionResult> {
    fs::read(root.join(STATE).join("result.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn outcome(
    root: &Path,
    id: &str,
    status: UpdateStatus,
    detail: impl Into<String>,
) -> Result<TransactionResult> {
    let value = TransactionResult {
        transaction_id: id.into(),
        status,
        detail: detail.into(),
        reboot_required: false,
    };
    write_json(&root.join(STATE).join("result.json"), &value)?;
    Ok(value)
}

/// Creates same-volume staging while reserving the single update slot.
pub struct Preparation {
    /// Directory for the downloaded payload and extracted stage.
    pub directory: tempfile::TempDir,
    /// Exclusive reservation retained until helper acknowledgement.
    pub reservation: StateLock,
}

impl Preparation {
    /// Reserves a marked portable installation for staging.
    ///
    /// # Errors
    /// Returns Busy for another update or errors creating same-volume state.
    pub fn new(root: &Path) -> Result<Self> {
        reject_links(root, Path::new(STATE))?;
        private_directory(&root.join(STATE))?;
        let reservation = StateLock::acquire(&root.join(STATE).join("prepare.lock"))?;
        let _gate = StateLock::acquire(&root.join(STATE).join("gate.lock"))?;
        if root.join(STATE).join("pending.json").exists() {
            return Err(UpdateError::new(
                ErrorCode::Busy,
                "An earlier update needs recovery before staging another",
            ));
        }
        let directory = tempfile::Builder::new()
            .prefix("transaction-")
            .tempdir_in(root.join(STATE))?;
        Ok(Self {
            directory,
            reservation,
        })
    }

    /// Starts the bundled helper and waits for a gate-held acknowledgement.
    ///
    /// # Errors
    /// Fails before handoff if the helper cannot acquire the installation gate.
    pub fn handoff(
        self,
        root: &Path,
        old: &BundleManifest,
        new: &BundleManifest,
        restart: Option<Restart>,
    ) -> Result<TransactionResult> {
        old.validate()?;
        new.validate()?;
        verify_bundle(&self.directory.path().join("stage"), new)?;
        let helper_name = if cfg!(windows) {
            "teshi-update-helper.exe"
        } else {
            "teshi-update-helper"
        };
        let helper_rel = format!(
            "{}{helper_name}",
            crate::manifest::executable_prefix(old.kind)
        );
        let helper_source = crate::manifest::shipped_executable(root, old.kind, helper_name);
        let expected = old
            .files
            .iter()
            .find(|f| f.path == helper_rel)
            .ok_or_else(|| crate::invalid("Installed helper missing from inventory"))?;
        let helper_relative =
            std::path::PathBuf::from(helper_rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        reject_links(root, &helper_relative)?;
        if file_hash(&helper_source)? != expected.sha256 {
            return Err(crate::invalid(
                "Installed helper checksum mismatch; reinstall Teshi",
            ));
        }
        let helper = self.directory.path().join(helper_name);
        fs::copy(helper_source, &helper)?;
        let paths = old
            .files
            .iter()
            .chain(&new.files)
            .map(|f| f.path.clone())
            .chain(std::iter::once(BUNDLE_MANIFEST.into()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let journal = Journal {
            schema: 1,
            old: old.clone(),
            new: new.clone(),
            paths,
            attempted: Vec::new(),
            committed: false,
            restart,
        };
        write_json(&self.directory.path().join("journal.json"), &journal)?;
        let journal_hash = file_hash(&self.directory.path().join("journal.json"))?;
        let mut command = Command::new(&helper);
        command
            .arg("--transaction")
            .arg(self.directory.path())
            .arg("--plan-sha256")
            .arg(journal_hash)
            .current_dir(&self.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        let started = Instant::now();
        while !self.directory.path().join("accepted").exists() {
            if child.try_wait()?.is_some() || started.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                // A helper can die after publishing recovery intent but before
                // acknowledging. Never delete the journal needed by next startup.
                if root.join(STATE).join("pending.json").exists() {
                    let _directory = self.directory.keep();
                }
                return Err(UpdateError::new(
                    ErrorCode::Installation,
                    "Update helper did not accept the transaction",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let id = self
            .directory
            .path()
            .file_name()
            .ok_or_else(|| crate::invalid("Missing transaction ID"))?
            .to_string_lossy()
            .into_owned();
        // The helper holds gate.lock before acknowledging, so new participants
        // cannot enter between dropping this reservation and exiting the CLI.
        let result = outcome(
            root,
            &id,
            UpdateStatus::WaitingForExit,
            "Helper accepted the update; waiting for Teshi processes to exit",
        )?;
        let _directory = self.directory.keep();
        drop(self.reservation);
        Ok(result)
    }
}

fn participant_wait() -> Duration {
    std::env::var("TESHI_UPDATE_QUIESCE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|seconds: &u64| (1..=120).contains(seconds))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30))
}

pub(crate) fn wait_for_participants(state: &Path) -> Result<Vec<File>> {
    let directory = state.join("participants");
    let deadline = Instant::now() + participant_wait();
    loop {
        let mut locks = Vec::new();
        let mut busy = false;
        if directory.is_dir() {
            for entry in fs::read_dir(&directory)? {
                let path = entry?.path();
                reject_links(
                    &directory,
                    path.strip_prefix(&directory)
                        .map_err(|_| crate::invalid("Invalid participant path"))?,
                )?;
                let file = match OpenOptions::new().read(true).write(true).open(&path) {
                    Ok(file) => file,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e.into()),
                };
                if fs2::FileExt::try_lock_exclusive(&file).is_err() {
                    busy = true;
                    break;
                }
                locks.push(file);
            }
        }
        if !busy {
            return Ok(locks);
        }
        if Instant::now() >= deadline {
            return Err(UpdateError::new(
                ErrorCode::Busy,
                "Teshi processes are still running. Close desktop, daemons and active runs, then retry",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn validate_journal(journal: &Journal) -> Result<()> {
    journal.old.validate()?;
    journal.new.validate()?;
    if journal.schema != 1
        || journal.old.kind != journal.new.kind
        || !matches!(journal.old.kind, InstallKind::Portable | InstallKind::Exe)
        || journal.old.target != journal.new.target
    {
        return Err(crate::invalid("Invalid transaction protocol/layout"));
    }
    if let Some(restart) = &journal.restart {
        let executable = {
            let name = if journal.new.target.contains("windows") {
                "teshi-desktop.exe"
            } else {
                "teshi-desktop"
            };
            format!(
                "{}{name}",
                crate::manifest::executable_prefix(journal.new.kind)
            )
        };
        if !restart.working_directory.is_absolute()
            || !journal
                .new
                .files
                .iter()
                .any(|f| f.path == executable && f.executable)
        {
            return Err(crate::invalid(
                "Restart requires a packaged native desktop and absolute project directory",
            ));
        }
    }
    let expected: BTreeSet<_> = journal
        .old
        .files
        .iter()
        .chain(&journal.new.files)
        .map(|f| f.path.as_str())
        .chain(std::iter::once(BUNDLE_MANIFEST))
        .collect();
    let actual: BTreeSet<_> = journal.paths.iter().map(String::as_str).collect();
    let folded: BTreeSet<_> = actual.iter().map(|p| p.to_ascii_lowercase()).collect();
    if folded.len() != actual.len()
        || folded.iter().any(|name| {
            let mut parent = name.as_str();
            while let Some((head, _)) = parent.rsplit_once('/') {
                if folded.contains(head) {
                    return true;
                }
                parent = head;
            }
            false
        })
    {
        return Err(crate::invalid(
            "Unsupported file/directory or case-only transition",
        ));
    }
    if actual != expected
        || actual.len() != journal.paths.len()
        || journal
            .attempted
            .iter()
            .any(|p| !actual.contains(p.as_str()))
    {
        return Err(crate::invalid(
            "Transaction paths do not match managed inventory",
        ));
    }
    Ok(())
}

fn atomic_copy(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| crate::invalid("Missing target parent"))?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut File::open(source)?, &mut temporary)?;
    temporary
        .as_file()
        .set_permissions(source.metadata()?.permissions())?;
    temporary.as_file().sync_all()?;
    let start = Instant::now();
    loop {
        match temporary.persist(target) {
            Ok(_) => break,
            Err(error) if start.elapsed() < Duration::from_secs(2) => {
                temporary = error.file;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn rollback(root: &Path, directory: &Path, journal: &Journal) -> Result<()> {
    validate_journal(journal)?;
    for name in journal.attempted.iter().rev() {
        reject_links(root, Path::new(name))?;
        let backup = directory.join("backup").join(name);
        reject_links(directory, &Path::new("backup").join(name))?;
        let target = root.join(name);
        let was_managed =
            name == BUNDLE_MANIFEST || journal.old.files.iter().any(|f| &f.path == name);
        if was_managed {
            atomic_copy(&backup, &target)?;
        } else if target.exists() {
            fs::remove_file(&target)?;
        }
    }
    Ok(())
}

fn recover_locked(root: &Path) -> Result<()> {
    let pending = root.join(STATE).join("pending.json");
    if !pending.is_file() {
        return Ok(());
    }
    let id: String = serde_json::from_slice(&fs::read(&pending)?)?;
    if !id.starts_with("transaction-") || id.contains(['/', '\\', ':']) {
        return Err(crate::invalid("Invalid pending transaction ID"));
    }
    let directory = root.join(STATE).join(&id);
    reject_links(&root.join(STATE), Path::new(&id))?;
    let journal: Journal = serde_json::from_slice(&fs::read(directory.join("journal.json"))?)?;
    validate_journal(&journal)?;
    if !journal.committed {
        let _participants = wait_for_participants(&root.join(STATE))?;
        rollback(root, &directory, &journal)?;
        outcome(
            root,
            &id,
            UpdateStatus::Errored,
            "Recovered an interrupted update; previous version restored",
        )?;
    }
    fs::remove_file(pending)?;
    Ok(())
}

/// Applies an accepted local transaction. Only the standalone helper calls this.
///
/// # Errors
/// Leaves a recoverable journal if replacement or rollback fails.
pub fn run_helper(directory: &Path, expected_plan_hash: &str) -> Result<()> {
    let directory = fs::canonicalize(directory)?;
    let state = directory
        .parent()
        .ok_or_else(|| crate::invalid("Missing update state"))?;
    if state.file_name().is_none_or(|p| p != STATE) {
        return Err(crate::invalid(
            "Helper transaction must be inside installation update state",
        ));
    }
    let root = state
        .parent()
        .ok_or_else(|| crate::invalid("Missing installation root"))?;
    let id = directory
        .file_name()
        .ok_or_else(|| crate::invalid("Missing transaction name"))?
        .to_string_lossy()
        .into_owned();
    if !id.starts_with("transaction-") {
        return Err(crate::invalid("Invalid transaction name"));
    }
    if file_hash(&directory.join("journal.json"))? != expected_plan_hash {
        return Err(crate::invalid("Helper plan changed before handoff"));
    }
    let mut journal: Journal = serde_json::from_slice(&fs::read(directory.join("journal.json"))?)?;
    validate_journal(&journal)?;
    if !journal.attempted.is_empty() || journal.committed {
        return Err(crate::invalid("Cannot replay an existing transaction"));
    }
    let _gate = StateLock::acquire(&state.join("gate.lock"))?;
    recover_locked(root)?;
    // The current inventory must still be the installation used for preparation.
    let installed: BundleManifest = serde_json::from_slice(&fs::read(root.join(BUNDLE_MANIFEST))?)?;
    if serde_json::to_value(&installed)? != serde_json::to_value(&journal.old)? {
        return Err(crate::invalid(
            "Installed inventory changed during preparation",
        ));
    }
    verify_bundle(&directory.join("stage"), &journal.new)?;
    write_json(&state.join("pending.json"), &id)?;
    File::create(directory.join("accepted"))?.sync_all()?;
    let result = (|| {
        // Parent publishes Pending before releasing its reservation. Wait for
        // that release so a fast helper cannot have Completed overwritten by Pending.
        let deadline = Instant::now() + Duration::from_secs(10);
        let _reservation = loop {
            match StateLock::acquire(&state.join("prepare.lock")) {
                Ok(lock) => break lock,
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        };
        let _participants = wait_for_participants(state)?;
        outcome(
            root,
            &id,
            UpdateStatus::Installing,
            "Installing verified bundle",
        )?;
        let expected = journal.new.clone();
        apply_files(root, &directory, &mut journal, || {
            smoke_check(root, &expected)
        })?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            outcome(
                root,
                &id,
                UpdateStatus::Completed,
                "Update installed successfully",
            )?;
            fs::remove_file(state.join("pending.json"))?;
            crate::exe::remove_staged_payload(root);
        }
        Err(error) => {
            rollback(root, &directory, &journal)?;
            outcome(
                root,
                &id,
                UpdateStatus::Errored,
                format!("Update failed; previous bundle retained: {error}"),
            )?;
            fs::remove_file(state.join("pending.json"))?;
            return Err(error);
        }
    }
    // New application startup must be able to acquire the gate.
    drop(_gate);
    if let Some(restart) = journal.restart {
        let name = if cfg!(windows) {
            "teshi-desktop.exe"
        } else {
            "teshi-desktop"
        };
        let mut command = Command::new(crate::manifest::shipped_executable(
            root,
            journal.new.kind,
            name,
        ));
        command
            .args(restart.arguments)
            .current_dir(restart.working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        if let Err(error) = command.spawn() {
            outcome(
                root,
                &id,
                UpdateStatus::Completed,
                format!("Update installed, but desktop restart failed: {error}"),
            )?;
        }
    }
    Ok(())
}

fn apply_files(
    root: &Path,
    directory: &Path,
    journal: &mut Journal,
    verify: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let old: BTreeSet<_> = journal
        .old
        .files
        .iter()
        .map(|f| f.path.as_str())
        .chain(std::iter::once(BUNDLE_MANIFEST))
        .collect();
    let new: BTreeSet<_> = journal
        .new
        .files
        .iter()
        .map(|f| f.path.as_str())
        .chain(std::iter::once(BUNDLE_MANIFEST))
        .collect();
    let required_space: u64 = journal.old.files.iter().map(|f| f.size).sum();
    if fs2::available_space(root)? < required_space.saturating_add(64 * 1024 * 1024) {
        return Err(UpdateError::new(
            ErrorCode::Io,
            "Insufficient rollback space",
        ));
    }
    for name in &journal.paths {
        reject_links(root, Path::new(name))?;
        let target = root.join(name);
        if target.exists() && !old.contains(name.as_str()) {
            return Err(crate::invalid(format!(
                "New payload would overwrite an unmanaged file: {name}"
            )));
        }
        if old.contains(name.as_str()) {
            atomic_copy(&target, &directory.join("backup").join(name))?;
        }
    }
    for name in journal.paths.clone() {
        journal.attempted.push(name.clone());
        write_json(&directory.join("journal.json"), journal)?;
        if new.contains(name.as_str()) {
            atomic_copy(&directory.join("stage").join(&name), &root.join(&name))?;
        } else {
            fs::remove_file(root.join(&name))?;
        }
    }
    verify()?;
    journal.committed = true;
    write_json(&directory.join("journal.json"), journal)?;
    Ok(())
}

fn smoke_check(root: &Path, manifest: &BundleManifest) -> Result<()> {
    let name = if cfg!(windows) { "teshi.exe" } else { "teshi" };
    let mut command = Command::new(crate::manifest::shipped_executable(
        root,
        manifest.kind,
        name,
    ));
    command
        .arg("--update-identity")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(UpdateError::new(
                ErrorCode::Installation,
                "New binary identity check timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output()?;
    let actual: teshi_core::version::BuildIdentity = serde_json::from_slice(&output.stdout)?;
    if !output.status.success() || actual != manifest.identity {
        return Err(crate::invalid("New binary identity check failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ManagedFile;
    use sha2::{Digest, Sha256};
    use teshi_core::version::{BUILD_TARGET, BuildIdentity, ReleaseChannel};

    fn write_bundle(root: &Path, sequence: u64, names: &[&str]) -> BundleManifest {
        fs::create_dir_all(root).unwrap();
        let bytes = format!("fixture {sequence}").into_bytes();
        let files = names
            .iter()
            .map(|name| {
                let path = root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, &bytes).unwrap();
                ManagedFile {
                    path: (*name).into(),
                    size: bytes.len() as u64,
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    executable: !name.contains('/'),
                }
            })
            .collect();
        let manifest = BundleManifest {
            schema: 1,
            identity: BuildIdentity {
                semver: "0.7.10".into(),
                channel: ReleaseChannel::Nightly,
                git_sha: format!("{sequence:040x}"),
                build_timestamp: "2026-09-08T12:00:00Z".into(),
                build_sequence: sequence,
            },
            target: BUILD_TARGET.into(),
            kind: InstallKind::Portable,
            layout: 1,
            update_explanation: None,
            files,
        };
        write_json(&root.join(BUNDLE_MANIFEST), &manifest).unwrap();
        manifest
    }

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Journal) {
        let root = tempfile::tempdir().unwrap();
        let cli = if cfg!(windows) { "teshi.exe" } else { "teshi" };
        let helper = if cfg!(windows) {
            "teshi-update-helper.exe"
        } else {
            "teshi-update-helper"
        };
        let old = write_bundle(root.path(), 1, &[cli, helper, "share/obsolete"]);
        let directory = root.path().join(STATE).join("transaction-test");
        fs::create_dir_all(&directory).unwrap();
        let new = write_bundle(&directory.join("stage"), 2, &[cli, helper, "share/new"]);
        let paths = old
            .files
            .iter()
            .chain(&new.files)
            .map(|f| f.path.clone())
            .chain(std::iter::once(BUNDLE_MANIFEST.into()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let journal = Journal {
            schema: 1,
            old,
            new,
            paths,
            attempted: Vec::new(),
            committed: false,
            restart: None,
        };
        (root, directory, journal)
    }

    #[test]
    fn successful_transaction_removes_obsolete_and_preserves_unknown_files() {
        let (root, directory, mut journal) = fixture();
        fs::write(root.path().join("user-notes.txt"), "preserve").unwrap();
        apply_files(root.path(), &directory, &mut journal, || Ok(())).unwrap();
        assert!(journal.committed);
        assert!(!root.path().join("share/obsolete").exists());
        assert_eq!(
            fs::read_to_string(root.path().join("share/new")).unwrap(),
            "fixture 2"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("user-notes.txt")).unwrap(),
            "preserve"
        );
        verify_bundle(root.path(), &journal.new).unwrap();
    }

    #[test]
    fn smoke_failure_rolls_back_all_changes_and_can_repeat_recovery() {
        let (root, directory, mut journal) = fixture();
        let result = apply_files(root.path(), &directory, &mut journal, || {
            Err(crate::invalid("injected smoke failure"))
        });
        assert!(result.is_err());
        assert!(!journal.committed);
        rollback(root.path(), &directory, &journal).unwrap();
        rollback(root.path(), &directory, &journal).unwrap();
        verify_bundle(root.path(), &journal.old).unwrap();
        assert!(!root.path().join("share/new").exists());
    }

    #[test]
    fn unknown_collision_fails_before_changing_any_managed_file() {
        let (root, directory, mut journal) = fixture();
        fs::write(root.path().join("share/new"), "user content").unwrap();
        assert!(apply_files(root.path(), &directory, &mut journal, || Ok(())).is_err());
        assert!(journal.attempted.is_empty());
        verify_bundle(root.path(), &journal.old).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join("share/new")).unwrap(),
            "user content"
        );
    }

    #[test]
    fn startup_recovers_durable_intent_before_registering_process() {
        let (root, directory, mut journal) = fixture();
        assert!(
            apply_files(root.path(), &directory, &mut journal, || Err(
                crate::invalid("power loss")
            ))
            .is_err()
        );
        write_json(
            &root.path().join(STATE).join("pending.json"),
            &"transaction-test",
        )
        .unwrap();
        let cli = root
            .path()
            .join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
        let _participant = participate(&cli).unwrap();
        verify_bundle(root.path(), &journal.old).unwrap();
        assert!(!root.path().join(STATE).join("pending.json").exists());
        assert_eq!(
            last_result(root.path()).unwrap().status,
            UpdateStatus::Errored
        );
    }

    #[test]
    fn inspect_commands_skip_install_participant() {
        assert!(!registers_install_participant(&[
            "teshi",
            "--update-identity"
        ]));
        assert!(!registers_install_participant(&["teshi", "--help"]));
        assert!(!registers_install_participant(&[
            "teshi", "update", "--status", "--json"
        ]));
        assert!(!registers_install_participant(&[
            "teshi", "update", "--check"
        ]));
        assert!(registers_install_participant(&["teshi", "update", "--yes"]));
        assert!(registers_install_participant(&["teshi", "."]));
    }

    #[test]
    fn last_result_is_readable_while_the_install_gate_is_held() {
        let (root, _, _) = fixture();
        outcome(
            root.path(),
            "transaction-status",
            UpdateStatus::WaitingForExit,
            "helper accepted",
        )
        .unwrap();
        let _gate = StateLock::acquire(&root.path().join(STATE).join("gate.lock")).unwrap();
        let previous = last_result(root.path()).unwrap();
        assert_eq!(previous.status, UpdateStatus::WaitingForExit);
        assert_eq!(previous.detail, "helper accepted");
        let cli = root
            .path()
            .join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
        assert!(participate(&cli).is_err());
    }

    #[test]
    fn gate_blocks_startup_and_second_preparation() {
        let (root, _, _) = fixture();
        let _gate = StateLock::acquire(&root.path().join(STATE).join("gate.lock")).unwrap();
        let cli = root
            .path()
            .join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
        assert!(participate(&cli).is_err());
        assert!(Preparation::new(root.path()).is_err());
    }

    #[test]
    fn modified_plan_hash_is_rejected_without_replacement() {
        let (root, directory, journal) = fixture();
        write_json(&directory.join("journal.json"), &journal).unwrap();
        assert!(run_helper(&directory, &"0".repeat(64)).is_err());
        verify_bundle(root.path(), &journal.old).unwrap();
    }
}
