//! Bounded Windows console graphics detection, before the input reader starts.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
    ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    FlushConsoleInputBuffer, GetConsoleMode, GetNumberOfConsoleInputEvents, GetStdHandle,
    INPUT_RECORD, KEY_EVENT, ReadConsoleInputW, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    SetConsoleMode,
};

use super::{Detection, ProtocolType, protocol_from_env};

/// Retains both modes so every exit path restores the caller's console.
struct ConsoleModes {
    input: HANDLE,
    output: HANDLE,
    input_mode: u32,
    output_mode: u32,
}

impl ConsoleModes {
    fn enter() -> io::Result<Self> {
        // SAFETY: Standard console handles are borrowed for this startup-only
        // exchange. No other application input reader has started yet.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if input.is_null()
                || input == INVALID_HANDLE_VALUE
                || output.is_null()
                || output == INVALID_HANDLE_VALUE
            {
                return Err(io::Error::last_os_error());
            }
            let mut input_mode = 0;
            let mut output_mode = 0;
            if GetConsoleMode(input, &mut input_mode) == 0
                || GetConsoleMode(output, &mut output_mode) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let modes = Self {
                input,
                output,
                input_mode,
                output_mode,
            };
            if SetConsoleMode(
                output,
                modes.output_mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            ) == 0
                || SetConsoleMode(input, ENABLE_VIRTUAL_TERMINAL_INPUT) == 0
                || FlushConsoleInputBuffer(input) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(modes)
        }
    }
}

impl Drop for ConsoleModes {
    fn drop(&mut self) {
        // SAFETY: These handles remain owned by the process throughout the
        // exchange; restoring modes does not close either handle.
        unsafe {
            SetConsoleMode(self.input, self.input_mode);
            SetConsoleMode(self.output, self.output_mode);
        }
    }
}

/// A complete primary device-attributes reply, or None while incomplete.
fn attributes(reply: &str) -> Option<Vec<u16>> {
    let (_, body) = reply.rsplit_once("\x1b[?")?;
    let (body, _) = body.split_once('c')?;
    body.split(';')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()
}

fn query() -> io::Result<Option<Vec<u16>>> {
    let modes = ConsoleModes::enter()?;
    let mut stdout = io::stdout().lock();
    // DA1 is answered by ConPTY itself on older Windows versions. Do not
    // batch cell-size queries: those replies can arrive after DA1, as keys.
    stdout.write_all(b"\x1b[c")?;
    stdout.flush()?;
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut reply = String::new();
    while Instant::now() < deadline && reply.len() < 4096 {
        let mut count = 0;
        // SAFETY: The startup probe is the sole input reader. Reading exactly
        // one queued record cannot block or consume keys after the final reply.
        unsafe {
            if GetNumberOfConsoleInputEvents(modes.input, &mut count) == 0 {
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let mut record: INPUT_RECORD = std::mem::zeroed();
            if ReadConsoleInputW(modes.input, &mut record, 1, &mut count) == 0 {
                return Err(io::Error::last_os_error());
            }
            if count > 0 && record.EventType == KEY_EVENT as u16 {
                let key = record.Event.KeyEvent;
                if key.bKeyDown != 0
                    && let Some(ch) = char::from_u32(key.uChar.UnicodeChar as u32)
                    && ch != '\0'
                {
                    reply.push(ch);
                }
            }
        }
        if let Some(attributes) = attributes(&reply) {
            return Ok(Some(attributes));
        }
    }
    Ok(None)
}

/// Selects Sixel from DA1 even without a font-size reply or WT_SESSION.
fn from_attributes(attributes: &[u16], env: &dyn Fn(&str) -> Option<String>) -> Detection {
    if let Some((protocol, marker)) = protocol_from_env(env) {
        return Detection::forced(protocol, marker);
    }
    if attributes.contains(&4) {
        // Windows Terminal exposes a virtual 10x20 pixel grid for Sixel,
        // independent of the display font or DPI; no font-size query is needed.
        return Detection::forced(ProtocolType::Sixel, "console DA1 reports Sixel support");
    }
    Detection::unavailable("the console answered without Sixel support")
}

pub(super) fn detect(env: &dyn Fn(&str) -> Option<String>) -> Detection {
    let mut detection = match query() {
        Ok(Some(attributes)) => from_attributes(&attributes, env),
        result => {
            let reason = match result {
                Ok(None) => "the console graphics query timed out".to_string(),
                Err(error) => format!("the console graphics query failed: {error}"),
                Ok(Some(_)) => unreachable!(),
            };
            if let Some((protocol, marker)) = protocol_from_env(env) {
                Detection::forced(protocol, format!("{reason}; assumed {marker}"))
            } else {
                Detection::unavailable(reason)
            }
        }
    };
    if !detection.support.is_pixels() && env("ZELLIJ").is_some() {
        detection.detail.push_str("; Windows Zellij may filter pixel output; use direct Windows Terminal or open externally");
    }
    detection
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_attributes_need_a_complete_numeric_reply() {
        for reply in ["", "\x1b[?61;4", "\x1b[?61;no;c", "\x1b[61;4c"] {
            assert!(attributes(reply).is_none(), "{reply:?}");
        }
        assert_eq!(attributes("\x1b[?61;4;6;22c"), Some(vec![61, 4, 6, 22]));
        assert_eq!(attributes("noise\x1b[?1;0c"), Some(vec![1, 0]));
    }

    #[test]
    fn sixel_does_not_require_cell_size_or_terminal_environment() {
        let detection = from_attributes(&[61, 4, 6, 22], &|_| None);
        assert_eq!(
            detection.support,
            super::super::GraphicsSupport::Pixels(ProtocolType::Sixel)
        );
        assert_eq!(detection.picker.unwrap().font_size(), (10, 20));
    }

    #[test]
    fn old_windows_terminal_marker_does_not_enable_sixel() {
        let detection =
            from_attributes(&[1, 0], &|name| (name == "WT_SESSION").then(|| "x".into()));
        assert!(!detection.support.is_pixels());
    }

    #[test]
    #[ignore = "run in Windows Terminal with TESHI_GRAPHICS_FIXTURE_OUTPUT and TESHI_GRAPHICS_FIXTURE_PROJECT"]
    fn windows_terminal_graphics_fixture() {
        let output = std::env::var_os("TESHI_GRAPHICS_FIXTURE_OUTPUT").expect("artifact path");
        let project = std::path::PathBuf::from(
            std::env::var_os("TESHI_GRAPHICS_FIXTURE_PROJECT").expect("project path"),
        );
        let _guard = crate::TerminalGuard::setup().unwrap();
        let mode = || unsafe {
            let mut input = 0;
            let mut output = 0;
            assert_ne!(
                GetConsoleMode(GetStdHandle(STD_INPUT_HANDLE), &mut input),
                0
            );
            assert_ne!(
                GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mut output),
                0
            );
            (input, output)
        };
        let original = mode();
        let mut samples = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let detection = super::super::init();
            samples.push(serde_json::json!({
                "protocol": detection.support.label(), "detail": detection.detail,
                "font_size": detection.picker.as_ref().map(|picker| picker.font_size()),
                "elapsed_ms": started.elapsed().as_millis(), "modes_restored": mode() == original
            }));
        }
        std::fs::write(&output, serde_json::to_vec_pretty(&samples).unwrap()).unwrap();
        for sample in &samples {
            assert_eq!(sample["protocol"], "sixel", "{sample}");
            assert_eq!(sample["modes_restored"], true);
        }

        let detection = super::super::init();
        let shots = super::super::screenshots_for(&project, "completed_image.feature", None);
        assert!(!shots.is_empty());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout())).unwrap();
        let mut rendered = super::super::load(
            &detection.picker.unwrap(),
            &project,
            &shots[0],
            terminal.size().unwrap(),
        )
        .unwrap();
        let mut payload_bytes = 0;
        loop {
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    super::super::render(frame, area, &mut rendered);
                })
                .unwrap();
            let written = rendered.write_pixels(terminal.backend_mut()).unwrap();
            if written == 0 {
                break;
            }
            payload_bytes += written;
        }
        assert!(
            payload_bytes > 100,
            "Sixel payload must reach the terminal backend"
        );
        samples.push(serde_json::json!({
            "source": super::super::capture_path(&project, &shots[0]),
            "pixels": rendered.pixels, "sixel_bytes": payload_bytes
        }));
        std::fs::write(&output, serde_json::to_vec_pretty(&samples).unwrap()).unwrap();
    }
}
