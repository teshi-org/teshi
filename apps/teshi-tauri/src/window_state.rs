//! Window geometry helpers for teshi-desktop persistence policy.
//!
//! Position is intentionally not persisted; windowed mode is centered after restore.
//! Size is clamped to the current monitor work area and application minimums.

use std::sync::Mutex;

use anyhow::{Context, Result};
use tauri::{LogicalSize, WebviewWindow};

use teshi_engine::{
    load_settings, save_settings, validated_window_size, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
};

/// State flags persisted by the window-state plugin (position excluded).
pub const PERSISTED_STATE_FLAGS: tauri_plugin_window_state::StateFlags =
    tauri_plugin_window_state::StateFlags::SIZE
        .union(tauri_plugin_window_state::StateFlags::MAXIMIZED)
        .union(tauri_plugin_window_state::StateFlags::FULLSCREEN)
        .union(tauri_plugin_window_state::StateFlags::VISIBLE);

/// One-shot legacy size taken from `settings.json` during startup migration.
pub struct PendingLegacyWindowSize(pub Mutex<Option<(u32, u32)>>);

/// Removes legacy `window_width` / `window_height` from settings and returns them once.
pub fn take_legacy_window_size_from_settings() -> Result<Option<(u32, u32)>> {
    let mut settings = load_settings()?;
    let legacy = match (settings.window_width, settings.window_height) {
        (Some(w), Some(h)) => validated_window_size(w, h),
        _ => None,
    };
    if legacy.is_some() {
        settings.window_width = None;
        settings.window_height = None;
        save_settings(&settings)?;
    }
    Ok(legacy)
}

/// Clamps logical width/height to the monitor work area and configured minimums.
///
/// When the work area is smaller than the minimum, the work area wins so the window
/// remains openable on small displays.
pub fn fit_logical_size_to_work_area(
    width: f64,
    height: f64,
    work_area_width: f64,
    work_area_height: f64,
    min_width: u32,
    min_height: u32,
) -> (f64, f64) {
    let min_w = min_width as f64;
    let min_h = min_height as f64;
    let cap_w = work_area_width.max(1.0);
    let cap_h = work_area_height.max(1.0);

    let fit_w = width.min(cap_w);
    let fit_h = height.min(cap_h);

    let w = fit_w.max(min_w.min(cap_w));
    let h = fit_h.max(min_h.min(cap_h));
    (w, h)
}

/// Applies legacy size, clamps to the work area, and centers windowed mode after plugin restore.
pub fn finalize_main_window(
    window: &WebviewWindow,
    pending_legacy: &PendingLegacyWindowSize,
) -> Result<()> {
    let maximized = window.is_maximized().context("query maximized")?;
    let fullscreen = window.is_fullscreen().context("query fullscreen")?;
    if maximized || fullscreen {
        let _ = window.show();
        return Ok(());
    }

    if let Some((legacy_w, legacy_h)) = pending_legacy.0.lock().ok().and_then(|mut g| g.take()) {
        window.set_size(LogicalSize::new(legacy_w as f64, legacy_h as f64))?;
    }

    let scale = window.scale_factor().context("query scale factor")?;
    let inner = window.inner_size().context("query inner size")?;
    let logical_w = inner.width as f64 / scale;
    let logical_h = inner.height as f64 / scale;

    let monitor = window
        .current_monitor()
        .context("query current monitor")?
        .or_else(|| window.primary_monitor().ok().flatten())
        .context("no monitor available")?;

    let work = monitor.work_area();
    let work_logical_w = work.size.width as f64 / scale;
    let work_logical_h = work.size.height as f64 / scale;

    let (clamped_w, clamped_h) = fit_logical_size_to_work_area(
        logical_w,
        logical_h,
        work_logical_w,
        work_logical_h,
        MIN_WINDOW_WIDTH,
        MIN_WINDOW_HEIGHT,
    );

    if (clamped_w - logical_w).abs() > f64::EPSILON || (clamped_h - logical_h).abs() > f64::EPSILON
    {
        window.set_size(LogicalSize::new(clamped_w, clamped_h))?;
    }

    window.center().context("center window")?;
    let _ = window.show();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_size_clamps_to_work_area() {
        let (w, h) = fit_logical_size_to_work_area(
            1920.0,
            1080.0,
            1366.0,
            768.0,
            MIN_WINDOW_WIDTH,
            MIN_WINDOW_HEIGHT,
        );
        assert_eq!(w, 1366.0);
        assert_eq!(h, 768.0);
    }

    #[test]
    fn fit_size_enforces_minimum_when_work_area_allows() {
        let (w, h) = fit_logical_size_to_work_area(
            800.0,
            600.0,
            1920.0,
            1080.0,
            MIN_WINDOW_WIDTH,
            MIN_WINDOW_HEIGHT,
        );
        assert_eq!(w, MIN_WINDOW_WIDTH as f64);
        assert_eq!(h, MIN_WINDOW_HEIGHT as f64);
    }

    #[test]
    fn fit_size_caps_minimum_to_work_area_on_small_display() {
        let (w, h) = fit_logical_size_to_work_area(
            1600.0,
            900.0,
            1024.0,
            700.0,
            MIN_WINDOW_WIDTH,
            MIN_WINDOW_HEIGHT,
        );
        assert_eq!(w, 1024.0);
        assert_eq!(h, 700.0);
    }
}
