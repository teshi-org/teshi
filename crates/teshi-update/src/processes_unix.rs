//! Unix process identity; Linux termination uses a pidfd to avoid PID reuse.

use super::*;

#[cfg(target_os = "linux")]
pub(super) fn inspect(pid: u32) -> Result<Option<ProcessIdentity>> {
    let root = PathBuf::from(format!("/proc/{pid}"));
    let stat = match fs::read_to_string(root.join("stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let fields = stat
        .rsplit_once(')')
        .ok_or_else(|| crate::invalid("Malformed process stat"))?
        .1;
    if fields.split_whitespace().next() == Some("Z") {
        return Ok(None);
    }
    let started = fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| crate::invalid("Missing process creation time"))?
        .to_string();
    let executable = fs::canonicalize(fs::read_link(root.join("exe"))?)?;
    Ok(Some(ProcessIdentity {
        pid,
        executable,
        started,
    }))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn inspect(pid: u32) -> Result<Option<ProcessIdentity>> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "lstart=", "-o", "comm="])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();
    if text.len() < 25 {
        return Err(crate::invalid("Malformed process identity from ps"));
    }
    let executable = fs::canonicalize(text[24..].trim())?;
    Ok(Some(ProcessIdentity {
        pid,
        executable,
        started: text[..24].to_string(),
    }))
}

pub(super) fn pids() -> Result<Vec<u32>> {
    #[cfg(target_os = "linux")]
    {
        Ok(fs::read_dir("/proc")?
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = std::process::Command::new("ps")
            .args(["-axo", "pid="])
            .output()?;
        if !output.status.success() {
            return Err(crate::invalid("Process enumeration failed"));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .filter_map(|pid| pid.parse().ok())
            .collect())
    }
}

pub(super) fn terminate(process: &ProcessIdentity) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, process.pid, 0) } as i32;
        if raw < 0 {
            if !is_running(process)? {
                return Ok(());
            }
            return Err(std::io::Error::last_os_error().into());
        }
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
        if !is_running(process)? {
            return Err(crate::invalid(
                "Process identity changed; refusing force close",
            ));
        }
        if unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if !wait_for_exit(process, Duration::from_secs(5))? {
            return Err(UpdateError::new(
                ErrorCode::Busy,
                "Process did not exit after force close",
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = process;
        Err(UpdateError::new(
            ErrorCode::Unsupported,
            "Safe force close is supported on Windows and Linux with pidfd support",
        ))
    }
}

pub(super) fn request_window_close(_: &ProcessIdentity) {}
