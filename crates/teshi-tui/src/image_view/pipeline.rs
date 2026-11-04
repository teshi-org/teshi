//! A single latest-request worker with bounded decoded and encoded LRU caches.

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, ensure};
use icy_sixel::{EncodeOptions, QuantizeMethod};
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader, RgbaImage, imageops};
use ratatui::layout::{Rect, Size};
use ratatui_image::{
    Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};

use super::viewport::{SourceRegion, ViewState, Viewport};
use super::{ScreenshotRef, capture_bytes, checked_capture_path};

pub(super) const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;
const DECODED_CACHE_BYTES: usize = MAX_DECODED_BYTES as usize;
const ENCODED_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENCODED_BYTES: usize = 8 * 1024 * 1024;
pub(super) const OUTPUT_BUDGET: usize = 64 * 1024;
const MAX_TILE_BYTES: usize = OUTPUT_BUDGET - 32;
/// UI captures are mostly flat colors and text. A 256-color palette per tile
/// rarely bands, while dithering would add noise around glyphs and doubles
/// quantization time.
const SIXEL_OPTIONS: EncodeOptions = EncodeOptions {
    max_colors: 256,
    diffusion: 0.0,
    quantize_method: QuantizeMethod::Wu,
};

/// Cancellation predicate, shared with the threads that encode tile rows.
pub(super) type Cancel<'a> = dyn Fn() -> bool + Sync + 'a;

#[derive(Clone)]
pub(super) struct Request {
    pub project: PathBuf,
    pub shot: ScreenshotRef,
    pub picker: Picker,
    pub view: ViewState,
    pub viewport: Viewport,
    /// Captures warmed into the encoded cache, in fit view, after this request.
    pub prefetch: Vec<ScreenshotRef>,
    /// Set for prefetch work, which must not evict the decoded source that
    /// zooming the current capture depends on.
    pub speculative: bool,
}

pub(super) struct Tile {
    pub x: u16,
    pub y: u16,
    pub data: String,
}

pub(super) enum Encoded {
    Pixels(Vec<Tile>),
    Cells(Protocol),
}

#[derive(Clone)]
pub(super) struct Prepared {
    pub pixels: (u32, u32),
    /// Cells covered by the encoded image, never larger than the viewport.
    pub cells: Size,
    pub region: SourceRegion,
    pub encoded: Arc<Encoded>,
    pub elapsed: Duration,
    pub cache_hit: bool,
    bytes: usize,
}

struct Shared {
    generation: AtomicU64,
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    job: Option<(u64, Request)>,
    result: Option<(u64, Result<Prepared, String>)>,
    closed: bool,
}

pub(super) struct Worker {
    shared: Arc<Shared>,
}

impl Worker {
    pub fn start() -> Result<Self> {
        let mut engine = Engine::default();
        Self::start_with(move |request, cancelled| engine.prepare(request, cancelled))
    }

    fn start_with(
        process: impl FnMut(&Request, &Cancel<'_>) -> Result<Prepared> + Send + 'static,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let background = shared.clone();
        std::thread::Builder::new()
            .name("teshi-image-worker".into())
            .spawn(move || run(background, process))
            .context("cannot start image worker")?;
        Ok(Self { shared })
    }

    pub fn submit(&self, request: Request) -> u64 {
        let generation = self.cancel();
        if let Ok(mut state) = self.shared.state.lock() {
            state.job = Some((generation, request));
        }
        self.shared.wake.notify_one();
        generation
    }

    pub fn cancel(&self) -> u64 {
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut state) = self.shared.state.lock() {
            state.job = None;
            state.result = None;
        }
        generation
    }

    pub fn poll(&self) -> Option<(u64, Result<Prepared, String>)> {
        self.shared.state.try_lock().ok()?.result.take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
        }
        // Decoders cannot be interrupted mid-call. Never join them on the UI
        // thread; the bounded worker exits at its next cancellation boundary.
        self.shared.wake.notify_one();
    }
}

fn run(shared: Arc<Shared>, mut process: impl FnMut(&Request, &Cancel<'_>) -> Result<Prepared>) {
    loop {
        let (generation, request) = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            while state.job.is_none() && !state.closed {
                let Ok(next) = shared.wake.wait(state) else {
                    return;
                };
                state = next;
            }
            if state.closed {
                return;
            }
            let Some(job) = state.job.take() else {
                continue;
            };
            job
        };
        let cancelled = || shared.generation.load(Ordering::Acquire) != generation;
        let result = process(&request, &cancelled).map_err(|error| format!("{error:#}"));
        if cancelled() {
            continue;
        }
        if let Ok(mut state) = shared.state.lock()
            && !state.closed
            && !cancelled()
        {
            state.result = Some((generation, result));
        }
        // Any new submission bumps the generation, so prefetch yields to it
        // at the same boundaries as a superseded foreground request.
        for shot in &request.prefetch {
            if cancelled() {
                break;
            }
            let neighbor = Request {
                shot: shot.clone(),
                view: ViewState::default(),
                prefetch: Vec::new(),
                speculative: true,
                ..request.clone()
            };
            if let Err(error) = process(&neighbor, &cancelled) {
                tracing::debug!(file = %shot.file, error = %format!("{error:#}"), "screenshot prefetch skipped");
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct FileKey {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

#[derive(PartialEq)]
struct EncodedKey {
    file: FileKey,
    view: ViewState,
    viewport: Viewport,
    protocol: ProtocolType,
}

struct Cache<K, V> {
    items: VecDeque<(K, V, usize)>,
    bytes: usize,
    budget: usize,
}

impl<K: PartialEq, V: Clone> Cache<K, V> {
    fn new(budget: usize) -> Self {
        Self {
            items: VecDeque::new(),
            bytes: 0,
            budget,
        }
    }

    fn get(&mut self, key: &K) -> Option<V> {
        let index = self
            .items
            .iter()
            .position(|(candidate, _, _)| candidate == key)?;
        let item = self.items.remove(index)?;
        let value = item.1.clone();
        self.items.push_back(item);
        Some(value)
    }

    fn insert(&mut self, key: K, value: V, bytes: usize) {
        if bytes > self.budget {
            return;
        }
        if let Some(index) = self
            .items
            .iter()
            .position(|(candidate, _, _)| candidate == &key)
            && let Some((_, _, old_bytes)) = self.items.remove(index)
        {
            self.bytes -= old_bytes;
        }
        while self.bytes + bytes > self.budget {
            let Some((_, _, old_bytes)) = self.items.pop_front() else {
                break;
            };
            self.bytes -= old_bytes;
        }
        self.bytes += bytes;
        self.items.push_back((key, value, bytes));
    }
}

pub(super) struct Engine {
    decoded: Cache<FileKey, Arc<DynamicImage>>,
    encoded: Cache<EncodedKey, Prepared>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            decoded: Cache::new(DECODED_CACHE_BYTES),
            encoded: Cache::new(ENCODED_CACHE_BYTES),
        }
    }
}

impl Engine {
    pub fn prepare(&mut self, request: &Request, cancelled: &Cancel<'_>) -> Result<Prepared> {
        let started = Instant::now();
        ensure!(!cancelled(), "image request cancelled");
        let path = checked_capture_path(&request.project, &request.shot)?;
        let metadata = std::fs::metadata(&path)?;
        let file = FileKey {
            path,
            length: metadata.len(),
            modified: metadata.modified().ok(),
        };
        let key = EncodedKey {
            file: file.clone(),
            view: request.view,
            viewport: request.viewport,
            protocol: request.picker.protocol_type(),
        };
        if let Some(mut image) = self.encoded.get(&key) {
            image.cache_hit = true;
            image.elapsed = started.elapsed();
            return Ok(image);
        }
        let source = match self.decoded.get(&file) {
            Some(image) => image,
            None => {
                let bytes = capture_bytes(&request.project, &request.shot)?;
                ensure!(!cancelled(), "image request cancelled");
                let image = Arc::new(decode(&bytes)?);
                ensure!(!cancelled(), "image request cancelled");
                if !request.speculative {
                    self.decoded
                        .insert(file, image.clone(), image.as_bytes().len());
                }
                image
            }
        };
        let mut prepared = prepare_image(
            &source,
            &request.picker,
            request.view,
            request.viewport,
            cancelled,
        )?;
        prepared.elapsed = started.elapsed();
        ensure!(!cancelled(), "image request cancelled");
        self.encoded.insert(key, prepared.clone(), prepared.bytes);
        Ok(prepared)
    }
}

fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .context("screenshot is not a readable PNG or JPEG")?;
    ensure!(
        decoder.total_bytes() <= MAX_DECODED_BYTES,
        "image exceeds the 128 MiB decoded-image limit; press o to open externally"
    );
    let (w, h) = decoder.dimensions();
    ensure!(
        u64::from(w) * u64::from(h) <= MAX_DECODED_BYTES / 4,
        "image exceeds the 32-megapixel limit; press o to open externally"
    );
    DynamicImage::from_decoder(decoder)
        .context("cannot decode screenshot within the image memory limit")
}

pub(super) fn prepare_image(
    source: &DynamicImage,
    picker: &Picker,
    view: ViewState,
    viewport: Viewport,
    cancelled: &Cancel<'_>,
) -> Result<Prepared> {
    ensure!(!cancelled(), "image request cancelled");
    let pixels = source.dimensions();
    let region = view.region(pixels, viewport);
    // Only this source region is resampled. Zooming never magnifies a cached
    // fit-to-window thumbnail, and output allocations depend on the viewport.
    // Resize the native source format before converting to RGBA. In fit view
    // this avoids copying/converting every full-resolution source pixel.
    let resized = if (region.width, region.height) == pixels {
        if !view.is_zoomed() && region.width / region.output.0.max(1) >= 4 {
            source.thumbnail_exact(region.output.0, region.output.1)
        } else {
            source.resize_exact(
                region.output.0,
                region.output.1,
                imageops::FilterType::Triangle,
            )
        }
    } else {
        let crop = source.crop_imm(region.x, region.y, region.width, region.height);
        ensure!(!cancelled(), "image request cancelled");
        crop.resize_exact(
            region.output.0,
            region.output.1,
            imageops::FilterType::Triangle,
        )
    }
    .to_rgba8();
    ensure!(!cancelled(), "image request cancelled");
    // Encode only the cells the image covers. Letterbox padding stays plain
    // terminal background instead of black pixels every frame must quantize
    // and transmit.
    let cells = Size::new(
        cell_span(resized.width(), viewport.font.0, viewport.cells.width),
        cell_span(resized.height(), viewport.font.1, viewport.cells.height),
    );
    let (width, height) = Viewport { cells, ..viewport }.pixels();
    let (encoded, bytes) = if matches!(
        picker.protocol_type(),
        ProtocolType::Sixel | ProtocolType::Iterm2
    ) {
        let tiles = encode_tiles(&resized, picker, viewport.font, cells, cancelled)?;
        let retained = tiles.iter().map(|tile| tile.data.capacity()).sum::<usize>()
            + tiles.capacity() * std::mem::size_of::<Tile>();
        (Encoded::Pixels(tiles), retained)
    } else {
        let protocol = picker.new_protocol(
            DynamicImage::ImageRgba8(resized),
            Rect::new(0, 0, cells.width, cells.height),
            Resize::Fit(None),
        )?;
        // Account conservatively for Kitty's base64 pixels and per-cell
        // styles without depending on the protocols' private layouts.
        (
            Encoded::Cells(protocol),
            if picker.protocol_type() == ProtocolType::Halfblocks {
                usize::from(cells.width) * usize::from(cells.height) * 128
            } else {
                width as usize * height as usize * 16
                    + usize::from(cells.width) * usize::from(cells.height) * 64
                    + 65536
            },
        )
    };
    Ok(Prepared {
        pixels,
        cells,
        region,
        encoded: Arc::new(encoded),
        bytes,
        elapsed: Duration::ZERO,
        cache_hit: false,
    })
}

/// Encodes one-row tiles for every cell row of `image`, in row order.
///
/// Rows are split across a few scoped threads: tiles are quantized
/// independently, and quantization dominates once decoding is optimized.
fn encode_tiles(
    image: &RgbaImage,
    picker: &Picker,
    font: (u16, u16),
    cells: Size,
    cancelled: &Cancel<'_>,
) -> Result<Vec<Tile>> {
    const MAX_THREADS: usize = 8;
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .clamp(1, MAX_THREADS);
    let rows_per_band = usize::from(cells.height).div_ceil(threads).max(1);
    let tmux = inside_tmux();
    let bands: Vec<Result<Vec<Tile>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..cells.height)
            .step_by(rows_per_band)
            .map(|start| {
                let end = (usize::from(start) + rows_per_band).min(usize::from(cells.height));
                scope.spawn(move || {
                    let mut encoder = TileEncoder {
                        image,
                        picker,
                        font,
                        tmux,
                        cancelled,
                        tiles: Vec::new(),
                        bytes: 0,
                    };
                    for row in start..end as u16 {
                        let mut column = 0;
                        while column < cells.width {
                            let remaining = (cells.width - column).min(48);
                            encoder.encode(column, row, remaining)?;
                            column += remaining;
                        }
                    }
                    Ok(encoder.tiles)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("image tile encoder panicked")))
            })
            .collect()
    });
    let mut tiles = Vec::new();
    let mut bytes = 0;
    for band in bands {
        let band = band?;
        bytes += band.iter().map(|tile| tile.data.len()).sum::<usize>();
        ensure!(
            bytes <= MAX_ENCODED_BYTES,
            "encoded preview exceeds 8 MiB; use a smaller terminal view or press o"
        );
        tiles.extend(band);
    }
    Ok(tiles)
}

/// Whole cells needed for `pixels`, bounded by the viewport the output was
/// sized for.
fn cell_span(pixels: u32, font: u16, limit: u16) -> u16 {
    let cells = pixels.div_ceil(u32::from(font.max(1))).max(1);
    u16::try_from(cells).unwrap_or(u16::MAX).min(limit.max(1))
}

/// Mirrors ratatui-image's detection, so the Sixel tiles encoded here and the
/// protocols it encodes agree on tmux passthrough.
fn inside_tmux() -> bool {
    std::env::var("TERM").is_ok_and(|term| term.starts_with("tmux"))
        || std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "tmux")
}

/// Encodes one-row pixel tiles under a shared byte budget and cancellation
/// predicate.
struct TileEncoder<'a> {
    image: &'a RgbaImage,
    picker: &'a Picker,
    font: (u16, u16),
    tmux: bool,
    cancelled: &'a Cancel<'a>,
    tiles: Vec<Tile>,
    bytes: usize,
}

impl TileEncoder<'_> {
    /// Encodes `width` cells at (`x`, `y`), halving the span until each tile
    /// fits one bounded terminal write.
    fn encode(&mut self, x: u16, y: u16, width: u16) -> Result<()> {
        ensure!(!(self.cancelled)(), "image request cancelled");
        let tile = cell_tile(
            self.image,
            u32::from(x) * u32::from(self.font.0),
            u32::from(y) * u32::from(self.font.1),
            u32::from(width) * u32::from(self.font.0),
            u32::from(self.font.1),
        )?;
        let data = match self.picker.protocol_type() {
            ProtocolType::Sixel => sixel(&tile, self.tmux)?,
            _ => match self.picker.new_protocol(
                DynamicImage::ImageRgba8(tile),
                Rect::new(0, 0, width, 1),
                Resize::Fit(None),
            )? {
                Protocol::ITerm2(image) => image.data,
                _ => anyhow::bail!("unexpected pixel protocol"),
            },
        };
        if data.len() > MAX_TILE_BYTES && width > 1 {
            let half = width / 2;
            self.encode(x, y, half)?;
            return self.encode(x + half, y, width - half);
        }
        ensure!(
            data.len() <= MAX_TILE_BYTES,
            "terminal cell is too large for incremental image output"
        );
        self.bytes += data.len();
        ensure!(
            self.bytes <= MAX_ENCODED_BYTES,
            "encoded preview exceeds 8 MiB; use a smaller terminal view or press o"
        );
        self.tiles.push(Tile { x, y, data });
        Ok(())
    }
}

/// Copies a cell-aligned tile, padding past the image edge with opaque black.
///
/// Generic `imageops` helpers are monomorphized in this crate, so they stay
/// unoptimized in dev builds even when `image` itself is optimized. Row-wise
/// slice copies keep tile extraction fast in either profile.
fn cell_tile(image: &RgbaImage, left: u32, top: u32, width: u32, height: u32) -> Result<RgbaImage> {
    let mut raw = [0, 0, 0, 255].repeat(width as usize * height as usize);
    let stride = image.width() as usize * 4;
    let copied = width.min(image.width().saturating_sub(left)) as usize * 4;
    let rows = height.min(image.height().saturating_sub(top)) as usize;
    let source = image.as_raw();
    for row in 0..rows {
        let from = (top as usize + row) * stride + left as usize * 4;
        let to = row * width as usize * 4;
        raw[to..to + copied].copy_from_slice(&source[from..from + copied]);
    }
    RgbaImage::from_raw(width, height, raw).context("tile buffer does not match its size")
}

/// Encodes a tile with icy_sixel's Wu quantizer. ratatui-image 8 pins the
/// libsixel port with Stucki dithering, which costs several times as much
/// per tile.
fn sixel(tile: &RgbaImage, tmux: bool) -> Result<String> {
    let data = icy_sixel::sixel_encode(
        tile.as_raw(),
        tile.width() as usize,
        tile.height() as usize,
        &SIXEL_OPTIONS,
    )
    .context("cannot encode Sixel tile")?;
    if !tmux {
        return Ok(data);
    }
    // tmux forwards a DCS payload only when every inner ESC is doubled.
    Ok(format!(
        "\x1bPtmux;{}\x1b\\",
        data.replace('\x1b', "\x1b\x1b")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn request(project: &std::path::Path, file: &str) -> Request {
        let mut picker = Picker::from_fontsize((1, 2));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        Request {
            project: project.into(),
            shot: ScreenshotRef {
                run_id: "run".into(),
                file: file.into(),
                scenario: "Images".into(),
                step_text: "Given image".into(),
                step_line: Some(3),
                captured_at: String::new(),
                status: "passed".into(),
            },
            picker,
            view: ViewState::default(),
            viewport: Viewport::new(ratatui::layout::Size::new(20, 10), (1, 2)),
            prefetch: Vec::new(),
            speculative: false,
        }
    }

    fn prepared(request: &Request) -> Prepared {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 20, Rgba([200, 0, 0, 255])));
        prepare_image(
            &image,
            &request.picker,
            request.view,
            request.viewport,
            &|| false,
        )
        .unwrap()
    }

    #[test]
    fn latest_request_replaces_queued_work_and_rejects_late_results() {
        let gate = Arc::new(std::sync::Barrier::new(2));
        let background_gate = gate.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let background_seen = seen.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let worker = Worker::start_with(move |request, cancelled| {
            background_seen
                .lock()
                .unwrap()
                .push(request.shot.file.clone());
            if request.shot.file == "first.png" {
                started_tx.send(()).unwrap();
                background_gate.wait();
            }
            ensure!(!cancelled(), "cancelled");
            Ok(prepared(request))
        })
        .unwrap();
        worker.submit(request(std::path::Path::new("unused"), "first.png"));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        worker.submit(request(std::path::Path::new("unused"), "second.png"));
        let latest = worker.submit(request(std::path::Path::new("unused"), "third.png"));
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "input must not wait for the active decoder"
        );
        gate.wait();
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            if let Some(result) = worker.poll() {
                break result;
            }
            assert!(Instant::now() < deadline, "latest image did not complete");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(result.0, latest);
        assert!(result.1.is_ok());
        assert_eq!(*seen.lock().unwrap(), vec!["first.png", "third.png"]);
    }

    #[test]
    fn test_prefetch_runs_after_the_published_result_and_yields_to_new_requests() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let background_seen = seen.clone();
        let (prefetch_tx, prefetch_rx) = std::sync::mpsc::channel();
        let gate = Arc::new(std::sync::Barrier::new(2));
        let background_gate = gate.clone();
        let worker = Worker::start_with(move |request, cancelled| {
            background_seen
                .lock()
                .unwrap()
                .push((request.shot.file.clone(), request.speculative));
            if request.shot.file == "next.png" {
                prefetch_tx.send(()).unwrap();
                background_gate.wait();
            }
            ensure!(!cancelled(), "cancelled");
            Ok(prepared(request))
        })
        .unwrap();
        let mut first = request(std::path::Path::new("unused"), "current.png");
        first.prefetch = vec![first.shot.clone(), first.shot.clone()];
        first.prefetch[0].file = "next.png".into();
        first.prefetch[1].file = "previous.png".into();
        let generation = worker.submit(first);
        prefetch_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let result = worker.poll().expect("foreground result precedes prefetch");
        assert_eq!(result.0, generation);
        let latest = worker.submit(request(std::path::Path::new("unused"), "other.png"));
        gate.wait();
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            if let Some(result) = worker.poll() {
                break result;
            }
            assert!(Instant::now() < deadline, "latest image did not complete");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(result.0, latest);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                ("current.png".to_string(), false),
                ("next.png".to_string(), true),
                ("other.png".to_string(), false),
            ],
            "a new request must skip the remaining prefetch"
        );
    }

    #[test]
    fn test_speculative_prepare_caches_the_view_but_not_the_decoded_source() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join(".teshi/logs/replay-screenshots/run");
        std::fs::create_dir_all(&run).unwrap();
        RgbaImage::from_pixel(100, 50, Rgba([200, 0, 0, 255]))
            .save(run.join("next.png"))
            .unwrap();
        let mut warm = request(dir.path(), "next.png");
        warm.speculative = true;
        let mut engine = Engine::default();
        engine.prepare(&warm, &|| false).unwrap();
        assert_eq!(engine.decoded.bytes, 0, "prefetch must not evict sources");
        let shown = engine
            .prepare(&request(dir.path(), "next.png"), &|| false)
            .unwrap();
        assert!(shown.cache_hit, "the prefetched fit view is reused");
    }

    #[test]
    fn test_sixel_tiles_use_square_pixels_and_escape_tmux_passthrough() {
        let tile = RgbaImage::from_pixel(4, 6, Rgba([10, 20, 30, 255]));
        let plain = sixel(&tile, false).unwrap();
        assert!(plain.starts_with("\x1bP9;"), "{plain:?}");
        assert!(plain.ends_with("\x1b\\"));
        let wrapped = sixel(&tile, true).unwrap();
        assert!(wrapped.starts_with("\x1bPtmux;\x1b\x1bP9;"), "{wrapped:?}");
        assert!(wrapped.ends_with("\x1b\x1b\\\x1b\\"));
        assert_eq!(
            wrapped.matches('\x1b').count(),
            plain.matches('\x1b').count() * 2 + 2
        );
    }

    #[test]
    fn dropping_worker_does_not_wait_for_an_active_decoder() {
        let gate = Arc::new(std::sync::Barrier::new(2));
        let background_gate = gate.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = Worker::start_with(move |request, _| {
            started_tx.send(()).unwrap();
            background_gate.wait();
            done_tx.send(()).unwrap();
            Ok(prepared(request))
        })
        .unwrap();
        worker.submit(request(std::path::Path::new("unused"), "first.png"));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_millis(250));
        gate.wait();
        done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn cache_reuses_encoded_views_but_reloads_changed_source_files() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join(".teshi/logs/replay-screenshots/run");
        std::fs::create_dir_all(&run).unwrap();
        RgbaImage::from_pixel(100, 50, Rgba([200, 0, 0, 255]))
            .save(run.join("one.png"))
            .unwrap();
        let request = request(dir.path(), "one.png");
        let mut engine = Engine::default();
        let first = engine.prepare(&request, &|| false).unwrap();
        let cached = engine.prepare(&request, &|| false).unwrap();
        assert!(cached.cache_hit);
        assert!(Arc::ptr_eq(&first.encoded, &cached.encoded));
        RgbaImage::from_pixel(70, 100, Rgba([0, 200, 0, 255]))
            .save(run.join("one.png"))
            .unwrap();
        let changed = engine.prepare(&request, &|| false).unwrap();
        assert!(!changed.cache_hit);
        assert_eq!(changed.pixels, (70, 100));
        assert!(!Arc::ptr_eq(&first.encoded, &changed.encoded));
    }

    #[test]
    fn actual_size_preserves_source_stripes_instead_of_enlarging_the_fit_preview() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(1000, 1000, |x, _| {
            if x % 2 == 0 {
                Rgba([200, 0, 0, 255])
            } else {
                Rgba([0, 0, 200, 255])
            }
        }));
        let request = request(std::path::Path::new("unused"), "unused.png");
        let fit = prepare_image(
            &source,
            &request.picker,
            request.view,
            request.viewport,
            &|| false,
        )
        .unwrap();
        let mut actual = request.view;
        actual.apply(
            super::super::ViewAction::ActualSize,
            Some((1000, 1000)),
            request.viewport,
        );
        let zoom = prepare_image(&source, &request.picker, actual, request.viewport, &|| {
            false
        })
        .unwrap();
        assert_eq!((zoom.region.width, zoom.region.height), (20, 20));
        let colors = |prepared: &Prepared| {
            let Encoded::Cells(protocol) = prepared.encoded.as_ref() else {
                panic!("cell preview")
            };
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 10)).unwrap();
            terminal
                .draw(|frame| {
                    frame.render_widget(ratatui_image::Image::new(protocol), frame.area())
                })
                .unwrap();
            (
                terminal.backend().buffer()[(0, 0)].fg,
                terminal.backend().buffer()[(1, 0)].fg,
            )
        };
        let detail = colors(&zoom);
        assert_eq!(detail.0, ratatui::style::Color::Rgb(200, 0, 0));
        assert_eq!(detail.1, ratatui::style::Color::Rgb(0, 0, 200));
        assert_ne!(colors(&fit), detail);
    }

    #[test]
    fn oversized_png_is_rejected_before_pixel_decoding() {
        fn crc(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
                }
            }
            !crc
        }
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255])))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let checksum = crc(&png[12..29]);
        png[29..33].copy_from_slice(&checksum.to_be_bytes());
        let error = decode(&png).unwrap_err();
        assert!(
            format!("{error:#}").to_lowercase().contains("limit"),
            "{error:#}"
        );
    }

    #[test]
    #[ignore = "set TESHI_IMAGE_BENCH_PNG and TESHI_IMAGE_BENCH_OUTPUT for a cold/warm source benchmark"]
    fn large_image_cold_and_warm_preparation_benchmark() {
        let source = std::env::var_os("TESHI_IMAGE_BENCH_PNG").expect("source PNG");
        let output = std::env::var_os("TESHI_IMAGE_BENCH_OUTPUT").expect("benchmark output");
        let project = tempfile::tempdir().unwrap();
        let run = project.path().join(".teshi/logs/replay-screenshots/run");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::copy(source, run.join("large.png")).unwrap();
        let mut request = request(project.path(), "large.png");
        request.picker = Picker::from_fontsize((10, 20));
        request.picker.set_protocol_type(ProtocolType::Sixel);
        request.viewport = Viewport::new(ratatui::layout::Size::new(120, 30), (10, 20));
        let mut engine = Engine::default();
        let mut samples = Vec::new();
        for _ in 0..4 {
            let started = Instant::now();
            let prepared = engine.prepare(&request, &|| false).unwrap();
            samples.push(serde_json::json!({"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,
                "cache_hit":prepared.cache_hit,"source_pixels":prepared.pixels,
                "decoded_cache_bytes":engine.decoded.bytes,"encoded_cache_bytes":engine.encoded.bytes}));
        }
        assert!(!samples[0]["cache_hit"].as_bool().unwrap());
        assert!(
            samples
                .iter()
                .skip(1)
                .all(|sample| sample["cache_hit"] == true)
        );
        std::fs::write(output, serde_json::to_vec_pretty(&samples).unwrap()).unwrap();
    }

    #[test]
    fn lru_cache_keeps_recent_values_within_its_budget() {
        let mut cache = Cache::new(10);
        cache.insert(1, "one", 4);
        cache.insert(2, "two", 4);
        assert_eq!(cache.get(&1), Some("one"));
        cache.insert(3, "three", 4);
        assert!(cache.get(&2).is_none());
        assert!(cache.bytes <= 10);
        cache.insert(4, "too large", 11);
        assert!(cache.get(&4).is_none());
    }

    #[test]
    fn preview_tiles_are_bounded_and_cancellation_stops_encoding() {
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(400, 200, Rgba([200, 40, 40, 255])));
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Sixel);
        let viewport = Viewport::new(ratatui::layout::Size::new(100, 25), (10, 20));
        let result =
            prepare_image(&image, &picker, ViewState::default(), viewport, &|| false).unwrap();
        let Encoded::Pixels(tiles) = result.encoded.as_ref() else {
            panic!("pixel tiles")
        };
        assert_eq!(
            result.cells,
            ratatui::layout::Size::new(40, 10),
            "only cells covered by the unscaled image are encoded"
        );
        assert_eq!(tiles.len(), 10);
        assert!(tiles.iter().all(|tile| tile.data.len() <= OUTPUT_BUDGET));
        assert!(prepare_image(&image, &picker, ViewState::default(), viewport, &|| true).is_err());
    }
}
