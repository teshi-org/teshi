//! Handle-bound Windows process inspection and termination.

use super::*;
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
        },
    },
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE},
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn open(pid: u32, terminate: bool) -> Result<Option<Handle>> {
    let rights = PROCESS_QUERY_LIMITED_INFORMATION
        | PROCESS_SYNCHRONIZE
        | if terminate { PROCESS_TERMINATE } else { 0 };
    let handle = unsafe { OpenProcess(rights, 0, pid) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(87) {
            return Ok(None);
        }
        return Err(error.into());
    }
    Ok(Some(Handle(handle)))
}

fn identity(handle: &Handle, pid: u32) -> Result<Option<ProcessIdentity>> {
    if unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_OBJECT_0 {
        return Ok(None);
    }
    let mut image = vec![0u16; 32768];
    let mut length = image.len() as u32;
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    // The executable path and creation time come from the same process handle.
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, image.as_mut_ptr(), &mut length) } == 0
        || unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user) }
            == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let executable = fs::canonicalize(PathBuf::from(String::from_utf16_lossy(
        &image[..length as usize],
    )))?;
    let started =
        ((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64).to_string();
    Ok(Some(ProcessIdentity {
        pid,
        executable,
        started,
    }))
}

pub(super) fn inspect(pid: u32) -> Result<Option<ProcessIdentity>> {
    match open(pid, false)? {
        Some(handle) => identity(&handle, pid),
        None => Ok(None),
    }
}

pub(super) fn pids() -> Result<Vec<u32>> {
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    let snapshot = Handle(raw);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    let mut pids = Vec::new();
    while found {
        pids.push(entry.th32ProcessID);
        found = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    Ok(pids)
}

pub(super) fn terminate(process: &ProcessIdentity) -> Result<()> {
    let Some(handle) = open(process.pid, true)? else {
        return Ok(());
    };
    match identity(&handle, process.pid)? {
        None => return Ok(()),
        Some(actual) if actual != *process => {
            return Err(crate::invalid(
                "Process identity changed; refusing force close",
            ));
        }
        _ => {}
    }
    // Keep this handle through termination and wait, eliminating the PID reuse race.
    if unsafe { TerminateProcess(handle.0, 1) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { WaitForSingleObject(handle.0, 5000) } != WAIT_OBJECT_0 {
        return Err(UpdateError::new(
            ErrorCode::Busy,
            format!("Process {} did not exit after force close", process.pid),
        ));
    }
    Ok(())
}

pub(super) fn request_window_close(process: &ProcessIdentity) {
    unsafe extern "system" fn close_window(
        window: windows_sys::Win32::Foundation::HWND,
        data: isize,
    ) -> i32 {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
        }
        if pid == data as u32 {
            unsafe {
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
        }
        1
    }
    if let Ok(Some(handle)) = open(process.pid, false)
        && identity(&handle, process.pid).ok().flatten().as_ref() == Some(process)
    {
        // Retain the process handle until enumeration finishes, so its PID
        // cannot be recycled while matching windows to this process.
        unsafe {
            EnumWindows(Some(close_window), process.pid as isize);
        }
    }
}
