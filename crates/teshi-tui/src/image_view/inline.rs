//! Images drawn inside scrolling panels: the chat transcript and requirement
//! documents.
//!
//! Panels reserve a fixed box of cells for each image, so layout never depends
//! on whether or when the pixels arrive. Decoding and protocol encoding run on
//! a background thread; the UI thread only places finished images.
//!
//! Sixel and iTerm2 pixels live outside Ratatui's buffer. Each placement marks
//! its cells as hidden blanks; after the frame those marks prove nothing was
//! drawn on top, and when a placement moves or disappears its old cells are
//! written again from the frame buffer, which erases the stale pixels.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::SystemTime;

use ratatui::Frame;
use ratatui::backend::Backend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Rect, Size};
use ratatui::style::Modifier;
use ratatui_image::picker::Picker;

use super::pipeline::{self, Encoded, Prepared};
use super::viewport::{ViewState, Viewport};

/// Decoded images kept ready for placement.
const MAX_ENTRIES: usize = 48;
/// Largest source file decoded for an inline preview.
const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;

/// What a panel should show in a reserved image box this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineState {
    /// Pixels are placed (or will be once the visible rows are written).
    Shown,
    /// Still decoding in the background.
    Loading,
    /// The image cannot be shown; the reason is user-facing.
    Failed(String),
    /// The terminal has no pixel protocol, or only part of a Kitty image is visible.
    Placeholder,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
    cols: u16,
    rows: u16,
}

enum Entry {
    Loading,
    Ready(Arc<Prepared>),
    Failed(String),
}

/// One visible part of a placed image.
#[derive(Clone)]
struct Slot {
    key: Key,
    prepared: Arc<Prepared>,
    /// Screen cells showing image rows `skip..skip + area.height`.
    area: Rect,
    skip: u16,
}

impl Slot {
    fn same_place(&self, other: &Slot) -> bool {
        self.key == other.key && self.area == other.area && self.skip == other.skip
    }
}

struct Painted {
    slot: Slot,
    next_tile: usize,
}

struct Job {
    key: Key,
    picker: Picker,
}

/// Inline image placement and pixel output owned by the app.
#[derive(Default)]
pub struct InlineImages {
    picker: Option<Picker>,
    entries: HashMap<Key, (Entry, u64)>,
    frame: Vec<Slot>,
    painted: Vec<Painted>,
    frame_number: u64,
    screen: Option<Size>,
    jobs: Option<mpsc::Sender<Job>>,
    results: Option<mpsc::Receiver<(Key, Result<Prepared, String>)>>,
    pending: usize,
}

impl InlineImages {
    /// Use `picker` for pixel output; `None` keeps every image a placeholder.
    pub fn set_picker(&mut self, picker: Option<Picker>) {
        self.picker = picker;
        self.entries.clear();
    }

    /// Whether images draw as pixels rather than placeholders.
    pub fn has_pixels(&self) -> bool {
        self.picker.is_some()
    }

    /// Pixel size of one terminal cell, when images draw as pixels.
    pub fn font_size(&self) -> Option<(u16, u16)> {
        self.picker.as_ref().map(Picker::font_size)
    }

    /// Starts a frame; placements from the previous frame are forgotten.
    pub fn begin_frame(&mut self) {
        self.frame.clear();
        self.frame_number += 1;
    }

    /// Place the image at `path`, fitted to a `cols` x `rows` box whose rows
    /// `skip..` are visible at `area`.
    ///
    /// Call after the panel has drawn its text: the box cells are overwritten.
    /// Returns what the panel should draw over the box when nothing is shown.
    pub fn place(
        &mut self,
        frame: &mut Frame<'_>,
        path: &Path,
        (cols, rows): (u16, u16),
        area: Rect,
        skip: u16,
    ) -> InlineState {
        let Some(picker) = self.picker.clone() else {
            return InlineState::Placeholder;
        };
        let area = area.intersection(frame.area());
        if area.is_empty() || cols == 0 || rows == 0 {
            return InlineState::Placeholder;
        }
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return InlineState::Failed("not a file".into()),
            Err(_) => return InlineState::Failed("file not found".into()),
        };
        let key = Key {
            path: path.to_path_buf(),
            length: metadata.len(),
            modified: metadata.modified().ok(),
            cols,
            rows,
        };
        let prepared = match self.entries.get_mut(&key) {
            Some((entry, used)) => {
                *used = self.frame_number;
                match entry {
                    Entry::Ready(prepared) => prepared.clone(),
                    Entry::Loading => return InlineState::Loading,
                    Entry::Failed(reason) => return InlineState::Failed(reason.clone()),
                }
            }
            None => {
                self.submit(key, picker);
                return InlineState::Loading;
            }
        };
        let cells = prepared.cells;
        let visible = Rect::new(
            area.x,
            area.y,
            area.width.min(cells.width),
            area.height.min(cells.height.saturating_sub(skip)),
        );
        if visible.is_empty() {
            return InlineState::Shown;
        }
        match prepared.encoded.as_ref() {
            Encoded::Cells(protocol) => {
                // Kitty placements cannot be clipped by rows.
                if skip > 0 || visible.height < cells.height || visible.width < cells.width {
                    return InlineState::Placeholder;
                }
                frame.render_widget(ratatui_image::Image::new(protocol), visible);
            }
            Encoded::Pixels(_) => {
                mark(frame.buffer_mut(), visible);
                self.frame.push(Slot {
                    key,
                    prepared,
                    area: visible,
                    skip,
                });
            }
        }
        InlineState::Shown
    }

    fn submit(&mut self, key: Key, picker: Picker) {
        if self.jobs.is_none() {
            let (jobs, inbox) = mpsc::channel::<Job>();
            let (outbox, results) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("teshi-inline-images".into())
                .spawn(move || {
                    for job in inbox {
                        let result = prepare(&job).map_err(|error| format!("{error:#}"));
                        if outbox.send((job.key, result)).is_err() {
                            return;
                        }
                    }
                });
            if let Err(error) = spawned {
                self.entries
                    .insert(key, (Entry::Failed(error.to_string()), self.frame_number));
                return;
            }
            self.jobs = Some(jobs);
            self.results = Some(results);
        }
        let sent = self.jobs.as_ref().is_some_and(|jobs| {
            jobs.send(Job {
                key: key.clone(),
                picker,
            })
            .is_ok()
        });
        let entry = if sent {
            self.pending += 1;
            Entry::Loading
        } else {
            Entry::Failed("image worker stopped".into())
        };
        self.entries.insert(key, (entry, self.frame_number));
    }

    /// Collects finished background work without blocking.
    pub fn poll(&mut self) {
        let Some(results) = &self.results else {
            return;
        };
        while let Ok((key, result)) = results.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            let entry = match result {
                Ok(prepared) => Entry::Ready(Arc::new(prepared)),
                Err(reason) => Entry::Failed(reason),
            };
            self.entries.insert(key, (entry, self.frame_number));
        }
        if self.entries.len() > MAX_ENTRIES {
            let mut ages: Vec<(u64, Key)> = self
                .entries
                .iter()
                .filter(|(_, (entry, _))| !matches!(entry, Entry::Loading))
                .map(|(key, (_, used))| (*used, key.clone()))
                .collect();
            ages.sort_by_key(|(used, _)| *used);
            let excess = self.entries.len() - MAX_ENTRIES;
            for (_, key) in ages.into_iter().take(excess) {
                self.entries.remove(&key);
            }
        }
    }

    /// Whether decoding or pixel output still needs prompt loop iterations.
    pub fn is_busy(&self) -> bool {
        self.pending > 0
            || self
                .painted
                .iter()
                .any(|painted| painted.next_tile < tile_count(&painted.slot))
    }

    /// Whether [`Self::finish_frame`] has anything to place or erase, so the
    /// caller can skip copying the frame buffer.
    pub fn has_work(&self) -> bool {
        !self.frame.is_empty() || !self.painted.is_empty()
    }

    /// Forget what is on screen after the terminal was cleared.
    pub fn forget_painted(&mut self) {
        self.painted.clear();
    }

    /// Reconciles this frame's placements with what is on screen, then writes
    /// stale-cell repairs and a bounded amount of new pixel output.
    ///
    /// `buffer` is the frame just flushed. `suppressed` hides every image, for
    /// overlays that own the pixel layer.
    ///
    /// # Errors
    /// Returns terminal write errors.
    pub fn finish_frame<B: Backend + Write>(
        &mut self,
        backend: &mut B,
        buffer: &Buffer,
        suppressed: bool,
    ) -> io::Result<()> {
        let screen = buffer.area.as_size();
        if self.screen != Some(screen) {
            // Ratatui clears the screen on resize, so nothing old remains.
            self.screen = Some(screen);
            self.painted.clear();
        }
        let current: Vec<Slot> = if suppressed {
            Vec::new()
        } else {
            std::mem::take(&mut self.frame)
                .into_iter()
                .filter(|slot| still_marked(buffer, slot.area))
                .collect()
        };
        let mut stale = Vec::new();
        let mut kept = Vec::new();
        for painted in std::mem::take(&mut self.painted) {
            match current.iter().find(|slot| slot.same_place(&painted.slot)) {
                Some(_) => kept.push(painted),
                None => stale.push(painted.slot.area),
            }
        }
        for slot in current {
            if !kept.iter().any(|painted| painted.slot.same_place(&slot)) {
                kept.push(Painted { slot, next_tile: 0 });
            }
        }
        if !stale.is_empty() {
            let cells: Vec<(u16, u16, Cell)> = stale
                .iter()
                .flat_map(|area| area.positions())
                .filter_map(|position| {
                    buffer
                        .cell(position)
                        .map(|cell| (position.x, position.y, cell.clone()))
                })
                .collect();
            backend.write_all(b"\x1b7")?;
            backend.draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))?;
            backend.write_all(b"\x1b8")?;
            // Placements that overlap a repaired area must be drawn again.
            for painted in &mut kept {
                if stale
                    .iter()
                    .any(|area| !area.intersection(painted.slot.area).is_empty())
                {
                    painted.next_tile = 0;
                }
            }
        }
        let mut written = 0;
        for painted in &mut kept {
            written += write_tiles(backend, painted, pipeline::OUTPUT_BUDGET - written)?;
            if written >= pipeline::OUTPUT_BUDGET {
                break;
            }
        }
        if written > 0 || !stale.is_empty() {
            Write::flush(backend)?;
        }
        self.painted = kept;
        Ok(())
    }
}

/// Marks reserved cells as hidden blanks so later overdraw is detectable.
fn mark(buffer: &mut Buffer, area: Rect) {
    for position in area.positions() {
        if let Some(cell) = buffer.cell_mut(position) {
            cell.reset();
            cell.modifier = Modifier::HIDDEN;
        }
    }
}

/// Whether nothing was drawn over a marked area after it was placed.
fn still_marked(buffer: &Buffer, area: Rect) -> bool {
    area.positions().all(|position| {
        buffer
            .cell(position)
            .is_some_and(|cell| cell.symbol() == " " && cell.modifier.contains(Modifier::HIDDEN))
    })
}

fn tile_count(slot: &Slot) -> usize {
    match slot.prepared.encoded.as_ref() {
        Encoded::Pixels(tiles) => tiles.len(),
        Encoded::Cells(_) => 0,
    }
}

/// Writes the visible rows of one placement, resuming at `next_tile`, within `budget` bytes.
fn write_tiles(writer: &mut impl Write, painted: &mut Painted, budget: usize) -> io::Result<usize> {
    let slot = &painted.slot;
    let Encoded::Pixels(tiles) = slot.prepared.encoded.as_ref() else {
        return Ok(0);
    };
    let mut bytes = 0;
    while let Some(tile) = tiles.get(painted.next_tile) {
        let visible_row = tile.y >= slot.skip && tile.y - slot.skip < slot.area.height;
        if visible_row && tile.x < slot.area.width {
            let prefix = format!(
                "\x1b7\x1b[{};{}H",
                slot.area.y + (tile.y - slot.skip) + 1,
                slot.area.x + tile.x + 1
            );
            let wire_bytes = prefix.len() + tile.data.len() + 2;
            // Always make progress, even when one tile exceeds what is left.
            if bytes > 0 && bytes + wire_bytes > budget {
                break;
            }
            writer.write_all(prefix.as_bytes())?;
            writer.write_all(tile.data.as_bytes())?;
            writer.write_all(b"\x1b8")?;
            bytes += wire_bytes;
        }
        painted.next_tile += 1;
    }
    Ok(bytes)
}

fn prepare(job: &Job) -> anyhow::Result<Prepared> {
    anyhow::ensure!(
        job.key.length <= MAX_SOURCE_BYTES,
        "image is larger than {} MB",
        MAX_SOURCE_BYTES / (1024 * 1024)
    );
    let bytes = std::fs::read(&job.key.path)?;
    let image = crate::chat_images::decode(&bytes)?;
    pipeline::prepare_image(
        &image,
        &job.picker,
        ViewState::default(),
        Viewport::new(
            Size::new(job.key.cols, job.key.rows),
            job.picker.font_size(),
        ),
        &|| false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui_image::picker::ProtocolType;

    fn sixel_images(dir: &Path) -> (InlineImages, PathBuf) {
        let path = dir.join("shot.png");
        image::RgbaImage::from_pixel(80, 40, image::Rgba([200, 40, 40, 255]))
            .save(&path)
            .unwrap();
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let mut images = InlineImages::default();
        images.set_picker(Some(picker));
        (images, path)
    }

    /// Draws one frame placing `path` at `area`, and returns the reported state
    /// with everything written to the terminal after the frame.
    fn frame(
        images: &mut InlineImages,
        terminal: &mut ratatui::Terminal<TestBackend>,
        path: &Path,
        area: Option<Rect>,
        overdraw: bool,
    ) -> (InlineState, Vec<u8>) {
        images.begin_frame();
        let mut state = InlineState::Placeholder;
        let completed = terminal
            .draw(|frame| {
                if let Some(area) = area {
                    state = images.place(frame, path, (8, 2), area, 0);
                }
                if overdraw {
                    frame.render_widget(ratatui::widgets::Clear, frame.area());
                }
            })
            .unwrap();
        let buffer = completed.buffer.clone();
        let mut output = Vec::new();
        let mut sink = Sink {
            inner: TestBackend::new(buffer.area.width, buffer.area.height),
            output: &mut output,
        };
        images.finish_frame(&mut sink, &buffer, false).unwrap();
        (state, output)
    }

    fn wait_ready(images: &mut InlineImages) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while images.pending > 0 {
            assert!(std::time::Instant::now() < deadline, "decode timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
            images.poll();
        }
    }

    /// A backend that records raw writes, so tests can inspect pixel output.
    struct Sink<'a> {
        inner: TestBackend,
        output: &'a mut Vec<u8>,
    }

    impl Write for Sink<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Backend for Sink<'_> {
        fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            for (x, y, cell) in content {
                self.output
                    .extend_from_slice(format!("[{x},{y}]{}", cell.symbol()).as_bytes());
            }
            Ok(())
        }
        fn hide_cursor(&mut self) -> io::Result<()> {
            self.inner.hide_cursor()
        }
        fn show_cursor(&mut self) -> io::Result<()> {
            self.inner.show_cursor()
        }
        fn get_cursor_position(&mut self) -> io::Result<ratatui::layout::Position> {
            self.inner.get_cursor_position()
        }
        fn set_cursor_position<P: Into<ratatui::layout::Position>>(
            &mut self,
            position: P,
        ) -> io::Result<()> {
            self.inner.set_cursor_position(position)
        }
        fn clear(&mut self) -> io::Result<()> {
            self.inner.clear()
        }
        fn size(&self) -> io::Result<Size> {
            self.inner.size()
        }
        fn window_size(&mut self) -> io::Result<ratatui::backend::WindowSize> {
            self.inner.window_size()
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_placeholder_without_a_pixel_protocol() {
        let mut images = InlineImages::default();
        let mut terminal = ratatui::Terminal::new(TestBackend::new(20, 6)).unwrap();
        let (state, output) = frame(
            &mut images,
            &mut terminal,
            Path::new("any.png"),
            Some(Rect::new(0, 0, 8, 2)),
            false,
        );
        assert_eq!(state, InlineState::Placeholder);
        assert!(output.is_empty());
    }

    #[test]
    fn test_missing_file_reports_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (mut images, _) = sixel_images(dir.path());
        let mut terminal = ratatui::Terminal::new(TestBackend::new(20, 6)).unwrap();
        let (state, _) = frame(
            &mut images,
            &mut terminal,
            &dir.path().join("gone.png"),
            Some(Rect::new(0, 0, 8, 2)),
            false,
        );
        assert_eq!(state, InlineState::Failed("file not found".into()));
    }

    #[test]
    fn test_pixels_are_written_once_and_erased_when_the_placement_moves() {
        let dir = tempfile::tempdir().unwrap();
        let (mut images, path) = sixel_images(dir.path());
        let mut terminal = ratatui::Terminal::new(TestBackend::new(20, 6)).unwrap();
        let first = Some(Rect::new(1, 1, 8, 2));
        assert_eq!(
            frame(&mut images, &mut terminal, &path, first, false).0,
            InlineState::Loading
        );
        wait_ready(&mut images);
        let (state, output) = frame(&mut images, &mut terminal, &path, first, false);
        assert_eq!(state, InlineState::Shown);
        assert!(output.starts_with(b"\x1b7\x1b[2;2H\x1bP"), "{output:?}");
        let (_, idle) = frame(&mut images, &mut terminal, &path, first, false);
        assert!(idle.is_empty(), "an unchanged placement is not resent");
        let (_, moved) = frame(
            &mut images,
            &mut terminal,
            &path,
            Some(Rect::new(1, 3, 8, 2)),
            false,
        );
        let moved = String::from_utf8_lossy(&moved);
        assert!(moved.contains("[1,1] "), "old cells are repainted: {moved}");
        assert!(
            moved.contains("\x1b[4;2H"),
            "pixels follow the move: {moved}"
        );
        let (_, gone) = frame(&mut images, &mut terminal, &path, None, false);
        assert!(String::from_utf8_lossy(&gone).contains("[1,3] "));
        assert!(!images.is_busy());
    }

    #[test]
    fn test_overdrawn_placements_are_not_drawn() {
        let dir = tempfile::tempdir().unwrap();
        let (mut images, path) = sixel_images(dir.path());
        let mut terminal = ratatui::Terminal::new(TestBackend::new(20, 6)).unwrap();
        let area = Some(Rect::new(0, 0, 8, 2));
        frame(&mut images, &mut terminal, &path, area, false);
        wait_ready(&mut images);
        let (state, output) = frame(&mut images, &mut terminal, &path, area, true);
        assert_eq!(state, InlineState::Shown);
        assert!(output.is_empty(), "a popup over the image keeps its cells");
    }

    #[test]
    fn test_partially_visible_images_write_only_visible_rows() {
        let dir = tempfile::tempdir().unwrap();
        let (mut images, path) = sixel_images(dir.path());
        let mut terminal = ratatui::Terminal::new(TestBackend::new(20, 6)).unwrap();
        images.begin_frame();
        terminal
            .draw(|frame| {
                images.place(frame, &path, (8, 2), Rect::new(0, 0, 8, 1), 1);
            })
            .unwrap();
        wait_ready(&mut images);
        images.begin_frame();
        let completed = terminal
            .draw(|frame| {
                assert_eq!(
                    images.place(frame, &path, (8, 2), Rect::new(0, 0, 8, 1), 1),
                    InlineState::Shown
                );
            })
            .unwrap();
        let buffer = completed.buffer.clone();
        let mut output = Vec::new();
        let mut sink = Sink {
            inner: TestBackend::new(20, 6),
            output: &mut output,
        };
        images.finish_frame(&mut sink, &buffer, false).unwrap();
        let text = String::from_utf8_lossy(&output);
        assert_eq!(text.matches("\x1b[1;1H").count(), 1, "{text:?}");
        assert!(!text.contains("\x1b[2;1H"), "hidden row 0 is not drawn");
    }
}
