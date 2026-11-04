//! Opt-in completed-render receipts for real-PTY test input synchronization.

use std::path::Path;

use anyhow::Result;
use ratatui::CompletedFrame;

/// Atomically publishes a receipt after the completed frame has been flushed.
pub(crate) fn publish(path: &Path, frame: &CompletedFrame<'_>) -> Result<()> {
    let width = usize::from(frame.area.width);
    if width == 0 {
        return Ok(());
    }
    let cells: Vec<Vec<&str>> = frame
        .buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let receipt = serde_json::json!({"pid":std::process::id(),"frame":frame.count,
        "cols":frame.area.width,"rows":frame.area.height,"cells":cells});
    let staging = path.with_extension("frame.tmp");
    std::fs::write(&staging, serde_json::to_vec(&receipt)?)?;
    std::fs::rename(staging, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};

    #[test]
    fn receipt_is_published_from_the_completed_frame_with_its_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("frame.json");
        let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
        let frame = terminal
            .draw(|frame| frame.render_widget(Paragraph::new("Ready 控件"), frame.area()))
            .unwrap();
        publish(&path, &frame).unwrap();
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(receipt["cols"], 20);
        assert_eq!(receipt["rows"], 4);
        assert_eq!(receipt["pid"], std::process::id());
        assert_eq!(receipt["cells"][0][0], "R");
        assert!(!path.with_extension("frame.tmp").exists());
    }
}
