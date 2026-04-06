use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

#[cfg(all(windows, feature = "winpty-backend"))]
use std::fs::OpenOptions;
#[cfg(all(windows, feature = "winpty-backend"))]
use std::path::PathBuf;
#[cfg(all(windows, feature = "winpty-backend"))]
use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
#[cfg(all(windows, feature = "winpty-backend"))]
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, TerminateProcess, WaitForSingleObject,
};
#[cfg(all(windows, feature = "winpty-backend"))]
use winpty::{Config, ConfigFlags, SpawnConfig, SpawnFlags, Winpty};

pub struct TuiDriver {
    child: ChildKind,
    writer: Box<dyn Write + Send>,
    buffer: Arc<Mutex<String>>,
}

enum ChildKind {
    Portable(Box<dyn portable_pty::Child + Send>),
    #[cfg(all(windows, feature = "winpty-backend"))]
    Winpty(WinptyChild),
}

impl TuiDriver {
    pub fn spawn(teshi_bin: &Path, args: &[&str], cwd: &Path) -> Result<Self> {
        let (child, mut reader, writer) = if use_winpty() {
            spawn_with_winpty(teshi_bin, args, cwd)?
        } else {
            spawn_with_conpty(teshi_bin, args, cwd)?
        };

        let buffer = Arc::new(Mutex::new(String::new()));
        let buffer_clone = buffer.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        let mut guard = buffer_clone.lock().unwrap();
                        guard.push_str(&text);
                        if guard.len() > 200_000 {
                            let drain = guard.len() - 200_000;
                            guard.drain(0..drain);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            writer,
            buffer,
        })
    }

    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn send_key_down(&mut self) -> Result<()> {
        self.send_text("\x1b[B")
    }

    pub fn snapshot(&self) -> String {
        self.buffer.lock().unwrap().clone()
    }

    pub fn wait_for_contains(&mut self, needle: &str, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if self.snapshot().contains(needle) {
                return Ok(());
            }
            if let Some(status) = self.try_wait()? {
                anyhow::bail!("teshi exited early: {status:?}");
            }
            if start.elapsed() > timeout {
                let snap = self.snapshot();
                let tail = tail_snippet(&snap, 2000);
                anyhow::bail!("timeout waiting for screen to contain '{needle}'. tail:\n{tail}");
            }
            thread::sleep(Duration::from_millis(30));
        }
    }

    pub fn wait_for_change(&mut self, before: &str, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if self.snapshot() != before {
                return Ok(());
            }
            if let Some(status) = self.try_wait()? {
                anyhow::bail!("teshi exited early: {status:?}");
            }
            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for screen change");
            }
            thread::sleep(Duration::from_millis(30));
        }
    }

    pub fn wait_for_output(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if !self.snapshot().is_empty() {
                return Ok(());
            }
            if let Some(status) = self.try_wait()? {
                anyhow::bail!("teshi exited early: {status:?}");
            }
            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for any screen output");
            }
            thread::sleep(Duration::from_millis(30));
        }
    }

    pub fn shutdown(mut self, timeout: Duration) -> Result<()> {
        let _ = self.send_text("q");
        let start = Instant::now();
        loop {
            if start.elapsed() > timeout {
                let _ = self.child.kill();
                break;
            }
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        Ok(())
    }

    fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>> {
        self.child.try_wait()
    }
}

impl ChildKind {
    fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>> {
        match self {
            ChildKind::Portable(child) => Ok(child.try_wait()?),
            #[cfg(all(windows, feature = "winpty-backend"))]
            ChildKind::Winpty(child) => child.try_wait(),
        }
    }

    fn kill(&mut self) -> Result<()> {
        match self {
            ChildKind::Portable(child) => Ok(child.kill()?),
            #[cfg(all(windows, feature = "winpty-backend"))]
            ChildKind::Winpty(child) => child.kill(),
        }
    }
}

fn spawn_with_conpty(
    teshi_bin: &Path,
    args: &[&str],
    cwd: &Path,
) -> Result<(ChildKind, Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("openpty failed")?;

    let mut cmd = CommandBuilder::new(teshi_bin);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.env("TERM", "xterm-256color");
    propagate_env_flag(&mut cmd, "TESHI_PTY_NO_RAW", "TESHI_NO_RAW");
    propagate_env_flag(&mut cmd, "TESHI_PTY_NO_ALT", "TESHI_NO_ALT");
    propagate_diag_path(&mut cmd, cwd);
    cmd.cwd(cwd);

    let child = pair
        .slave
        .spawn_command(cmd)
        .context("spawn teshi failed")?;

    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;

    Ok((ChildKind::Portable(child), reader, writer))
}

#[cfg(all(windows, feature = "winpty-backend"))]
fn spawn_with_winpty(
    teshi_bin: &Path,
    args: &[&str],
    cwd: &Path,
) -> Result<(ChildKind, Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    let mut cfg = Config::new(ConfigFlags::empty()).context("winpty config")?;
    cfg.set_initial_size(120, 40);
    let mut winpty = Winpty::open(&cfg).context("winpty open")?;

    let cmdline = build_cmdline(teshi_bin, args);
    let spawn_cfg = SpawnConfig::new(SpawnFlags::empty(), None, Some(&cmdline), Some(cwd), None)
        .context("winpty spawn config")?;
    let handles = winpty.spawn(&spawn_cfg).context("winpty spawn")?;

    let conout = open_pipe_with_retry(winpty.conout_name(), false)?;
    let conin = open_pipe_with_retry(winpty.conin_name(), true)?;

    let child = WinptyChild {
        winpty,
        process: handles.process,
        thread: handles.thread,
    };

    Ok((ChildKind::Winpty(child), Box::new(conout), Box::new(conin)))
}

#[cfg(any(not(windows), not(feature = "winpty-backend")))]
fn spawn_with_winpty(
    _teshi_bin: &Path,
    _args: &[&str],
    _cwd: &Path,
) -> Result<(ChildKind, Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    anyhow::bail!("winpty backend requires Windows and the `winpty-backend` feature");
}

fn use_winpty() -> bool {
    cfg!(all(windows, feature = "winpty-backend")) && std::env::var_os("TESHI_USE_WINPTY").is_some()
}

#[cfg(all(windows, feature = "winpty-backend"))]
fn build_cmdline(teshi_bin: &Path, args: &[&str]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_arg(&teshi_bin.display().to_string()));
    for arg in args {
        parts.push(quote_arg(arg));
    }
    parts.join(" ")
}

#[cfg(all(windows, feature = "winpty-backend"))]
fn quote_arg(arg: &str) -> String {
    if !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    for ch in arg.chars() {
        if ch == '"' {
            out.push_str("\\\"");
        } else {
            out.push(ch);
        }
    }
    out.push('"');
    out
}

#[cfg(all(windows, feature = "winpty-backend"))]
fn open_pipe_with_retry(path: PathBuf, write: bool) -> Result<std::fs::File> {
    let start = Instant::now();
    loop {
        let mut opts = OpenOptions::new();
        if write {
            opts.write(true);
        } else {
            opts.read(true);
        }
        match opts.open(&path) {
            Ok(file) => return Ok(file),
            Err(err) => {
                if start.elapsed() > Duration::from_secs(3) {
                    return Err(err)
                        .with_context(|| format!("failed to open winpty pipe {}", path.display()));
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[cfg(all(windows, feature = "winpty-backend"))]
struct WinptyChild {
    winpty: Winpty,
    process: isize,
    thread: isize,
}

#[cfg(all(windows, feature = "winpty-backend"))]
impl WinptyChild {
    fn try_wait(&mut self) -> Result<Option<portable_pty::ExitStatus>> {
        unsafe {
            let res = WaitForSingleObject(self.process as _, 0);
            if res == WAIT_TIMEOUT {
                return Ok(None);
            }
            if res != WAIT_OBJECT_0 {
                anyhow::bail!("WaitForSingleObject failed: {res}");
            }
            let mut code: u32 = 0;
            if GetExitCodeProcess(self.process as _, &mut code) == 0 {
                anyhow::bail!("GetExitCodeProcess failed");
            }
            Ok(Some(portable_pty::ExitStatus::with_exit_code(code)))
        }
    }

    fn kill(&mut self) -> Result<()> {
        unsafe {
            let _ = TerminateProcess(self.process as _, 1);
        }
        Ok(())
    }
}

#[cfg(all(windows, feature = "winpty-backend"))]
impl Drop for WinptyChild {
    fn drop(&mut self) {
        unsafe {
            if self.thread != 0 {
                let _ = CloseHandle(self.thread as _);
            }
            if self.process != 0 {
                let _ = CloseHandle(self.process as _);
            }
        }
    }
}

fn propagate_env_flag(cmd: &mut CommandBuilder, host_var: &str, child_var: &str) {
    if std::env::var_os(host_var).is_some() {
        cmd.env(child_var, "1");
    }
}

fn propagate_diag_path(cmd: &mut CommandBuilder, cwd: &Path) {
    if std::env::var_os("TESHI_PTY_DIAG").is_none() {
        return;
    }
    let path = cwd.join("teshi_pty_diag.log");
    cmd.env("TESHI_DIAG_PATH", path);
}

fn tail_snippet(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    text[text.len() - max..].to_string()
}
