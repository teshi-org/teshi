//! Bounded payload download and archive extraction.

use crate::{
    ErrorCode, Result, UpdateError, UpdateEvent, UpdateStatus,
    github::{Candidate, Http},
    install::reject_links,
    manifest::{BUNDLE_MANIFEST, BundleManifest, validate_path},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// Streams a payload, resuming an existing partial file when supported.
///
/// The caller must hold an exclusive reservation for the destination. Network
/// interruptions and cancellation retain partial bytes; integrity errors remove
/// them. Every completed download is checked against the full size and SHA256.
///
/// # Errors
/// Returns cancellation, network, space, length or checksum errors.
pub fn download(
    http: &impl Http,
    candidate: &Candidate,
    destination: &Path,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(UpdateEvent),
) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| crate::invalid("Missing download directory"))?;
    crate::install::reject_links(
        parent,
        destination
            .strip_prefix(parent)
            .map_err(|_| crate::invalid("Invalid payload path"))?,
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Err(UpdateError::new(ErrorCode::Cancelled, "Download cancelled"));
    }
    let mut size = match destination.metadata() {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => return Err(crate::invalid("Payload is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    if size >= candidate.asset.size && destination.exists() {
        if size == candidate.asset.size && file_hash(destination)? == candidate.asset.sha256 {
            return Ok(());
        }
        fs::remove_file(destination)?;
        size = 0;
    }
    if fs2::available_space(parent)? < candidate.asset.size.saturating_sub(size) {
        return Err(UpdateError::new(
            ErrorCode::Io,
            "Insufficient free space to download update",
        ));
    }
    let result = (|| {
        let mut response = if size > 0 {
            http.get_range(&candidate.download_url, size)?
        } else {
            http.get(&candidate.download_url, None)?
        };
        if size > 0 && response.status == 416 {
            response = http.get(&candidate.download_url, None)?;
        }
        match response.status {
            200 => size = 0,
            206 if size > 0
                && valid_range(
                    response.content_range.as_deref(),
                    size,
                    candidate.asset.size,
                ) => {}
            206 => {
                return Err(UpdateError::new(
                    ErrorCode::Verification,
                    "Invalid Content-Range for resumed payload",
                ));
            }
            status => {
                return Err(UpdateError::new(
                    ErrorCode::Network,
                    format!("Download returned HTTP {status}"),
                ));
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(size == 0)
            .append(size > 0)
            .write(true)
            .open(destination)?;
        let mut body = response.body;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(UpdateError::new(ErrorCode::Cancelled, "Download cancelled"));
            }
            let count = body.read(&mut buffer).map_err(|error| {
                UpdateError::new(
                    ErrorCode::Network,
                    format!("Payload transfer interrupted: {error}"),
                )
            })?;
            if count == 0 {
                break;
            }
            size += count as u64;
            if size > candidate.asset.size {
                return Err(UpdateError::new(
                    ErrorCode::Verification,
                    "Download exceeds declared size",
                ));
            }
            file.write_all(&buffer[..count])?;
            emit(UpdateEvent {
                status: UpdateStatus::Downloading,
                progress: Some(size as f32 / candidate.asset.size as f32),
                detail: candidate.asset.name.clone(),
            });
        }
        file.sync_all()?;
        drop(file);
        if size != candidate.asset.size {
            return Err(UpdateError::new(
                ErrorCode::Network,
                "Payload transfer ended before declared size; partial download retained",
            ));
        }
        emit(UpdateEvent {
            status: UpdateStatus::Verifying,
            progress: None,
            detail: candidate.asset.name.clone(),
        });
        if file_hash(destination)? != candidate.asset.sha256 {
            return Err(UpdateError::new(
                ErrorCode::Verification,
                "Downloaded payload failed SHA256 validation",
            ));
        }
        Ok(())
    })();
    if result
        .as_ref()
        .is_err_and(|error| error.code == ErrorCode::Verification)
    {
        fs::remove_file(destination).or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        })?;
    }
    result
}

fn valid_range(header: Option<&str>, start: u64, total: u64) -> bool {
    header == Some(format!("bytes {start}-{}/{total}", total.saturating_sub(1)).as_str())
}

/// Hashes a regular file with bounded memory.
///
/// # Errors
/// Returns filesystem/read errors.
pub fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Verifies all inventoried files without accepting links.
///
/// # Errors
/// Returns manifest or integrity errors for missing/changed files.
pub fn verify_bundle(root: &Path, manifest: &BundleManifest) -> Result<()> {
    manifest.validate()?;
    for entry in &manifest.files {
        reject_links(root, Path::new(&entry.path))?;
        let path = root.join(&entry.path);
        if !path.is_file()
            || path.metadata()?.len() != entry.size
            || file_hash(&path)? != entry.sha256
        {
            return Err(UpdateError::new(
                ErrorCode::Verification,
                format!("Bundle file verification failed: {}", entry.path),
            ));
        }
    }
    Ok(())
}

struct Extractor<'a> {
    root: &'a Path,
    prefix: String,
    names: BTreeSet<String>,
    total: u64,
}
impl Extractor<'_> {
    fn entry(
        &mut self,
        name: &str,
        size: u64,
        directory: bool,
        reader: &mut impl Read,
    ) -> Result<()> {
        let name = name.trim_end_matches('/');
        validate_path(name)?;
        if name == self.prefix && directory {
            return Ok(());
        }
        let relative = name
            .strip_prefix(&format!("{}/", self.prefix))
            .ok_or_else(|| crate::invalid("Unexpected archive root"))?;
        validate_path(relative)?;
        if !self.names.insert(relative.to_ascii_lowercase()) || self.names.len() > 60_000 {
            return Err(crate::invalid(
                "Duplicate archive entry or too many entries",
            ));
        }
        self.total = self
            .total
            .checked_add(size)
            .ok_or_else(|| crate::invalid("Archive size overflow"))?;
        if self.total > 8 * 1024 * 1024 * 1024
            || (relative == BUNDLE_MANIFEST && size > 8 * 1024 * 1024)
        {
            return Err(crate::invalid("Archive exceeds expansion limit"));
        }
        let path = self.root.join(relative);
        if directory {
            fs::create_dir_all(path)?;
            return Ok(());
        }
        if fs2::available_space(self.root)? < size.saturating_add(64 * 1024 * 1024) {
            return Err(UpdateError::new(
                ErrorCode::Io,
                "Insufficient space for extraction",
            ));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
        let written = std::io::copy(&mut reader.take(size + 1), &mut file)?;
        if written != size {
            return Err(crate::invalid("Archive entry size mismatch"));
        }
        file.sync_all()?;
        Ok(())
    }
}

/// Extracts a verified archive into a fresh directory and validates its full inventory.
///
/// # Errors
/// Refuses malformed archives, links, extra payloads or mismatched identity/target.
pub fn extract(
    archive: &Path,
    destination: &Path,
    candidate: &Candidate,
) -> Result<BundleManifest> {
    fs::create_dir(destination)?;
    let prefix = format!(
        "teshi-{}-{}",
        candidate.manifest.tag, candidate.asset.target
    );
    let mut extractor = Extractor {
        root: destination,
        prefix,
        names: BTreeSet::new(),
        total: 0,
    };
    if candidate.asset.name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(File::open(archive)?)
            .map_err(|e| crate::invalid(e.to_string()))?;
        for index in 0..zip.len() {
            let mut file = zip
                .by_index(index)
                .map_err(|e| crate::invalid(e.to_string()))?;
            if file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                return Err(crate::invalid("Archive links are not supported"));
            }
            let name = file.name().to_owned();
            extractor.entry(&name, file.size(), file.is_dir(), &mut file)?;
        }
    } else if candidate.asset.name.ends_with(".tar.gz") {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
        for entry in tar.entries()? {
            let mut entry = entry?;
            let kind = entry.header().entry_type();
            if !kind.is_file() && !kind.is_dir() {
                return Err(crate::invalid(
                    "Archive links/special entries are not supported",
                ));
            }
            let path = entry.path()?.to_string_lossy().into_owned();
            extractor.entry(&path, entry.size(), kind.is_dir(), &mut entry)?;
        }
    } else {
        return Err(crate::invalid("Expected a portable archive"));
    }
    let manifest: BundleManifest =
        serde_json::from_slice(&fs::read(destination.join(BUNDLE_MANIFEST))?)?;
    if manifest.identity != candidate.manifest.identity
        || manifest.target != candidate.asset.target
        || manifest.kind != candidate.asset.kind
    {
        return Err(crate::invalid(
            "Archive manifest does not match selected release",
        ));
    }
    verify_bundle(destination, &manifest)?;
    let expected: BTreeSet<_> = manifest
        .files
        .iter()
        .map(|f| f.path.to_ascii_lowercase())
        .chain(std::iter::once(BUNDLE_MANIFEST.into()))
        .collect();
    for name in &extractor.names {
        if !expected.contains(name) && !expected.iter().any(|f| f.starts_with(&format!("{name}/")))
        {
            return Err(crate::invalid(format!("Unmanaged archive entry: {name}")));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in &manifest.files {
            fs::set_permissions(
                destination.join(&file.path),
                fs::Permissions::from_mode(if file.executable { 0o755 } else { 0o644 }),
            )?;
        }
    }
    Ok(manifest)
}
