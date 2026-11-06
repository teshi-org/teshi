//! Images attached to AI chat prompts.
//!
//! A draft shows a pasted or dropped image as an `[Image #N]` placeholder
//! backed by [`DraftImages`]; a typed `@path` mention also attaches its image.
//! A sent message is stored with every placeholder expanded to an `@path`
//! mention, so the transcript text alone says what was attached and saved
//! sessions need no extra fields. The agent receives that text with the
//! mentions removed, plus one image block per mention. Clipboard bitmaps are
//! written to a user-level cache first, never into the project.
//!
//! Sending follows the same limits as iCode: an image goes as-is when it is a
//! format model APIs read and at most [`MAX_IMAGE_BYTES`]; anything else is
//! re-encoded as a JPEG within that size.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage, imageops::FilterType};
use sha2::{Digest, Sha256};

/// File extensions treated as images, lowercase.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
/// Largest image sent to an agent. Every image rides each request it is part
/// of, and gateways in front of model services may reject larger payloads.
pub const MAX_IMAGE_BYTES: usize = 256 * 1024;
/// Largest source file accepted at all.
const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;
/// Largest decoded image accepted, in pixels.
const MAX_PIXELS: u64 = 80_000_000;
/// Compression first fits the long edge within this, never enlarging.
const MAX_EDGE: u32 = 2048;
/// JPEG qualities tried in turn before the image is scaled down further.
const JPEG_QUALITIES: [u8; 3] = [88, 82, 76];
const MAX_COMPRESSION_PASSES: usize = 8;
/// Clipboard images kept per project cache directory.
const KEEP_COUNT: usize = 100;
const KEEP_BYTES: u64 = 512 * 1024 * 1024;
const CLIPBOARD_PREFIX: &str = "clipboard-image-";
/// Another process can briefly hold the Windows clipboard open.
const CLIPBOARD_ATTEMPTS: usize = 5;
const CLIPBOARD_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

/// Image bytes ready to be sent to an agent.
#[derive(Clone)]
pub struct PreparedImage {
    /// Encoded bytes in [`Self::media_type`].
    pub data: Arc<[u8]>,
    /// One of `image/png`, `image/jpeg`, `image/gif`, or `image/webp`.
    pub media_type: &'static str,
}

impl std::fmt::Debug for PreparedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedImage")
            .field("media_type", &self.media_type)
            .field("bytes", &self.data.len())
            .finish()
    }
}

/// Whether `path` names a file with an image extension.
pub fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// The media type model APIs accept for these bytes, judged by signature.
fn wire_media_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Load an image file and fit it to what an agent accepts.
///
/// # Errors
/// Returns a user-facing reason when the file cannot be read, is not a
/// supported image, is too large to process, or cannot be compressed within
/// [`MAX_IMAGE_BYTES`].
pub fn prepare_for_agent(path: &Path) -> Result<PreparedImage> {
    let name = display_name(path);
    let bytes = read_source(path).with_context(|| format!("Cannot read image {name}"))?;
    let (width, height) = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
        .with_context(|| format!("{name} is not a supported image; use PNG, JPEG, GIF, or WebP"))?;
    ensure_pixels(width, height).with_context(|| format!("{name} cannot be attached"))?;
    // A renamed BMP keeps an image extension but no model reads it directly.
    if bytes.len() <= MAX_IMAGE_BYTES
        && let Some(media_type) = wire_media_type(&bytes)
    {
        return Ok(PreparedImage {
            data: Arc::from(bytes),
            media_type,
        });
    }
    let image = decode(&bytes).with_context(|| format!("{name} could not be decoded"))?;
    let jpeg = compress_jpeg(&image, MAX_IMAGE_BYTES).with_context(|| {
        format!(
            "{name} is still larger than {} KB after compression; export a smaller copy",
            MAX_IMAGE_BYTES / 1024
        )
    })?;
    Ok(PreparedImage {
        data: Arc::from(jpeg),
        media_type: "image/jpeg",
    })
}

fn read_source(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    ensure!(
        length <= MAX_SOURCE_BYTES,
        "the file is {} MB, over the {} MB limit",
        length / (1024 * 1024),
        MAX_SOURCE_BYTES / (1024 * 1024)
    );
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or_default());
    file.take(MAX_SOURCE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_SOURCE_BYTES,
        "the file grew past the size limit"
    );
    Ok(bytes)
}

fn ensure_pixels(width: u32, height: u32) -> Result<()> {
    ensure!(width > 0 && height > 0, "the image has no pixels");
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "the image is {width}x{height}, over the {MAX_PIXELS}-pixel limit"
    );
    Ok(())
}

/// Decode with EXIF orientation applied, so a photo is sent upright.
pub(crate) fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4 + 64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    ensure_pixels(width, height)?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Encode `image` as a JPEG of at most `max_bytes`, or `None` when no size
/// within the bounded passes gets there.
fn compress_jpeg(image: &DynamicImage, max_bytes: usize) -> Option<Vec<u8>> {
    let fitted = DynamicImage::ImageRgb8(flatten_on_white(&fit_long_edge(image, MAX_EDGE)));
    let mut working = fitted.to_rgb8();
    for _ in 0..MAX_COMPRESSION_PASSES {
        let mut smallest = usize::MAX;
        for quality in JPEG_QUALITIES {
            let encoded = encode_jpeg(&working, quality)?;
            if encoded.len() <= max_bytes {
                return Some(encoded);
            }
            smallest = smallest.min(encoded.len());
        }
        // Bytes grow roughly with pixel count: aim a little under the limit,
        // shrinking each side by at least a tenth and to no less than a third.
        let ratio = ((max_bytes as f64 / smallest as f64).sqrt() * 0.96).clamp(0.35, 0.90);
        let size = (
            ((f64::from(working.width()) * ratio) as u32).max(1),
            ((f64::from(working.height()) * ratio) as u32).max(1),
        );
        if size == working.dimensions() {
            return None;
        }
        // Resample the fitted image, not the previous pass, to avoid compounding blur.
        working = fitted
            .resize_exact(size.0, size.1, FilterType::Lanczos3)
            .into_rgb8();
    }
    None
}

fn fit_long_edge(image: &DynamicImage, max_edge: u32) -> DynamicImage {
    if image.width().max(image.height()) <= max_edge {
        return image.clone();
    }
    image.resize(max_edge, max_edge, FilterType::Lanczos3)
}

/// JPEG has no alpha; transparent pixels become white rather than black.
///
/// Pixel loops here run on the UI thread and this crate is unoptimized in
/// local dev installs, so opaque images take `image`'s own conversion and
/// blending touches raw bytes only.
fn flatten_on_white(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    if rgba
        .as_raw()
        .chunks_exact(4)
        .all(|pixel| pixel[3] == u8::MAX)
    {
        return DynamicImage::ImageRgba8(rgba).to_rgb8();
    }
    let mut rgb = Vec::with_capacity(rgba.as_raw().len() / 4 * 3);
    for pixel in rgba.as_raw().chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        for &channel in &pixel[..3] {
            rgb.push(((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8);
        }
    }
    RgbImage::from_raw(rgba.width(), rgba.height(), rgb).unwrap_or_default()
}

fn encode_jpeg(image: &RgbImage, quality: u8) -> Option<Vec<u8>> {
    use image::ImageEncoder as _;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(out)
}

/// Per-project directory for clipboard images in the user cache.
///
/// Images stay out of the project tree so a paste never shows up in version
/// control, and each project gets its own bounded directory.
pub fn attachment_dir(project_root: &Path) -> PathBuf {
    let identity = std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.into());
    let digest = Sha256::digest(identity.to_string_lossy().as_bytes());
    let key: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("teshi")
        .join("attachments")
        .join(key)
}

/// Image files currently on the system clipboard.
///
/// Copied files arrive as a file list; a screenshot or copied picture arrives
/// as a bitmap, which is saved as PNG into `dir`. Returns an empty list when
/// the clipboard holds neither.
///
/// A bitmap's path is returned at once while `cache` writes the PNG in the
/// background; its chip and the next send wait for that write.
///
/// # Errors
/// Returns an error when the clipboard cannot be opened or read, or the
/// attachment folder cannot be created.
pub fn clipboard_images(dir: &Path, cache: &mut AttachmentCache) -> Result<Vec<PathBuf>> {
    let mut clipboard = arboard::Clipboard::new().context("Cannot open the clipboard")?;
    if let Ok(files) = clipboard.get().file_list() {
        let images: Vec<PathBuf> = files
            .into_iter()
            .filter(|path| has_image_extension(path) && path.is_file())
            .collect();
        if !images.is_empty() {
            return Ok(images);
        }
    }
    let mut attempt = 0;
    let bitmap = loop {
        attempt += 1;
        match clipboard.get_image() {
            Ok(image) => break image,
            Err(arboard::Error::ContentNotAvailable) => return Ok(Vec::new()),
            Err(arboard::Error::ClipboardOccupied) if attempt < CLIPBOARD_ATTEMPTS => {
                std::thread::sleep(CLIPBOARD_RETRY_DELAY);
            }
            Err(error) => return Err(error).context("Cannot read the clipboard image"),
        }
    };
    let width = u32::try_from(bitmap.width).context("clipboard image is too wide")?;
    let height = u32::try_from(bitmap.height).context("clipboard image is too tall")?;
    ensure_pixels(width, height)?;
    let rgba = image::RgbaImage::from_raw(width, height, bitmap.bytes.into_owned())
        .context("clipboard image has an unexpected pixel layout")?;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Cannot create the attachment folder {}", dir.display()))?;
    let path = clipboard_png_path(&rgba, dir);
    if path.is_file() {
        prune_clipboard_dir(dir, &path);
    } else {
        cache.save_in_background(rgba, dir, &path);
    }
    Ok(vec![path])
}

/// Where a clipboard bitmap is saved, named by content so a repeated paste
/// reuses one file.
fn clipboard_png_path(image: &image::RgbaImage, dir: &Path) -> PathBuf {
    let digest = Sha256::digest(image.as_raw());
    let key: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{CLIPBOARD_PREFIX}{key}.png"))
}

/// Save a clipboard bitmap as PNG at `path`, then prune old pastes.
fn save_clipboard_png(image: &image::RgbaImage, dir: &Path, path: &Path) -> Result<()> {
    if !path.is_file() {
        let partial = path.with_extension("png.partial");
        write_fast_png(image, &partial).context("Cannot save the clipboard image")?;
        std::fs::rename(&partial, path).context("Cannot save the clipboard image")?;
    }
    prune_clipboard_dir(dir, path);
    Ok(())
}

/// The file is a cache that is re-encoded before sending anyway, so speed
/// beats file size.
fn write_fast_png(image: &image::RgbaImage, path: &Path) -> Result<()> {
    use image::ImageEncoder as _;
    use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    PngEncoder::new_with_quality(&mut file, CompressionType::Fast, PngFilter::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )?;
    std::io::Write::flush(&mut file)?;
    Ok(())
}

/// Keep the newest clipboard images within the count and byte budgets.
fn prune_clipboard_dir(dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(CLIPBOARD_PREFIX) && name.ends_with(".png"))
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    let mut total = 0;
    for (index, (_, size, path)) in files.into_iter().enumerate() {
        total += size;
        if path != keep && (index >= KEEP_COUNT || total > KEEP_BYTES) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// One `@path` image mention in prompt text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageMention {
    /// Byte range of the mention in the text, including `@` and any quotes.
    pub range: Range<usize>,
    /// Path resolved against the base directory.
    pub path: PathBuf,
}

/// Sentence punctuation that may follow an unquoted mention without becoming
/// part of its path.
const MENTION_TRAILING: [char; 8] = [',', '.', ';', ':', '!', '?', ')', ']'];

/// The `@path` mention for `path`, quoted when it contains whitespace.
pub fn format_mention(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text.chars().any(char::is_whitespace) {
        format!("@\"{text}\"")
    } else {
        format!("@{text}")
    }
}

/// Image mentions in `text` whose path has an image extension.
///
/// A mention starts with `@` at the start of the text or after whitespace and
/// runs to the next whitespace, or to the closing quote of `@"..."`. Trailing
/// sentence punctuation is not part of the path. Relative paths resolve
/// against `base`. Existence is not checked here.
pub fn image_mentions(text: &str, base: &Path) -> Vec<ImageMention> {
    let mut mentions = Vec::new();
    let mut previous: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let at_boundary = previous.is_none_or(char::is_whitespace);
        previous = Some(ch);
        if ch != '@' || !at_boundary {
            continue;
        }
        let body = &text[start + 1..];
        let (path_text, consumed) = if let Some(quoted) = body.strip_prefix('"') {
            let Some(close) = quoted
                .find(['"', '\n'])
                .filter(|i| quoted[*i..].starts_with('"'))
            else {
                continue;
            };
            (&quoted[..close], close + 2)
        } else {
            let end = body.find(char::is_whitespace).unwrap_or(body.len());
            let token = body[..end].trim_end_matches(MENTION_TRAILING);
            (token, token.len())
        };
        let path = Path::new(path_text);
        if path_text.is_empty() || !has_image_extension(path) {
            continue;
        }
        let end = start + 1 + consumed;
        mentions.push(ImageMention {
            range: start..end,
            path: if path.is_absolute() {
                path.to_path_buf()
            } else {
                base.join(path)
            },
        });
        // Skip the consumed mention so an `@` inside the path is not rescanned.
        while chars.peek().is_some_and(|(index, _)| *index < end) {
            if let Some((_, skipped)) = chars.next() {
                previous = Some(skipped);
            }
        }
    }
    mentions
}

/// Draft text standing for the `number`-th image of a draft or message.
pub fn placeholder(number: usize) -> String {
    format!("[Image #{number}]")
}

/// Byte ranges and numbers of the `[Image #N]` placeholders in `text`.
pub fn placeholders(text: &str) -> Vec<(Range<usize>, usize)> {
    const OPEN: &str = "[Image #";
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find(OPEN) {
        let start = from + offset;
        let digits_start = start + OPEN.len();
        let digits = text[digits_start..]
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(text.len() - digits_start);
        let close = digits_start + digits;
        from = digits_start;
        if digits == 0 || !text[close..].starts_with(']') {
            continue;
        }
        if let Ok(number) = text[digits_start..close].parse() {
            found.push((start..close + 1, number));
            from = close + 1;
        }
    }
    found
}

/// Images attached to a draft, keyed by their placeholder number.
///
/// Numbers only grow within one draft, so deleting `[Image #1]` and pasting
/// again yields `[Image #2]` rather than reviving a stale number. An entry
/// whose placeholder was deleted from the text is simply not attached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftImages {
    slots: Vec<(usize, PathBuf)>,
}

impl DraftImages {
    /// Registers `path` and returns the placeholder to insert for it.
    pub fn attach(&mut self, path: PathBuf) -> String {
        let number = self.slots.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
        self.slots.push((number, path));
        placeholder(number)
    }

    /// The image behind placeholder `number`.
    pub fn path(&self, number: usize) -> Option<&Path> {
        self.slots
            .iter()
            .find(|(n, _)| *n == number)
            .map(|(_, path)| path.as_path())
    }

    /// Whether no image was ever attached.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// Every image attached to `text`, in text order: placeholders known to
/// `images` and typed `@path` mentions.
pub fn draft_attachments(text: &str, images: &DraftImages, base: &Path) -> Vec<ImageMention> {
    let mut attachments: Vec<ImageMention> = placeholders(text)
        .into_iter()
        .filter_map(|(range, number)| {
            images.path(number).map(|path| ImageMention {
                range,
                path: path.to_path_buf(),
            })
        })
        .chain(image_mentions(text, base))
        .collect();
    attachments.sort_by_key(|attachment| attachment.range.start);
    attachments
}

/// Range of `range` widened by one adjoining space, so removing it does not
/// leave a double space behind.
fn with_adjoining_space(text: &str, mut range: Range<usize>) -> Range<usize> {
    if text[range.end..].starts_with(' ') {
        range.end += 1;
    } else if range.start > 0 && text[..range.start].ends_with(' ') {
        range.start -= 1;
    }
    range
}

/// Remove the `index`-th draft attachment and one adjoining space from `text`.
///
/// Returns the byte offset where the attachment started, or `None` when there
/// is no such attachment.
pub fn remove_attachment(
    text: &mut String,
    images: &DraftImages,
    base: &Path,
    index: usize,
) -> Option<usize> {
    let attachment = draft_attachments(text, images, base)
        .into_iter()
        .nth(index)?;
    let range = with_adjoining_space(text, attachment.range);
    text.replace_range(range.clone(), "");
    Some(range.start)
}

/// The text to send and store for a draft: each known placeholder becomes an
/// `@path` mention, padded with spaces where it touches a word so it still
/// parses as a mention.
pub fn expand_placeholders(text: &str, images: &DraftImages) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, number) in placeholders(text) {
        let Some(path) = images.path(number) else {
            continue;
        };
        out.push_str(&text[last..range.start]);
        if out
            .chars()
            .next_back()
            .is_some_and(|ch| !ch.is_whitespace())
        {
            out.push(' ');
        }
        out.push_str(&format_mention(path));
        if text[range.end..]
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_whitespace() && !MENTION_TRAILING.contains(&ch))
        {
            out.push(' ');
        }
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

/// The draft form of stored message text: image mentions become numbered
/// placeholders, the inverse of [`expand_placeholders`].
///
/// Used to show a sent message and to put a message back into the composer.
pub fn collapse_mentions(text: &str, base: &Path) -> (String, DraftImages) {
    let mut images = DraftImages::default();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for mention in image_mentions(text, base) {
        out.push_str(&text[last..mention.range.start]);
        out.push_str(&images.attach(mention.path));
        last = mention.range.end;
    }
    out.push_str(&text[last..]);
    (out, images)
}

/// Stored message text with its image mentions removed, for the agent: the
/// images themselves travel as image blocks.
pub fn strip_image_mentions(text: &str, base: &Path) -> String {
    let mut out = text.to_string();
    for mention in image_mentions(text, base).into_iter().rev() {
        let range = with_adjoining_space(&out, mention.range);
        out.replace_range(range, "");
    }
    out.trim().to_string()
}

/// The image files named by a paste or drop that consists only of image paths.
///
/// Terminal drag-and-drop and copied file paths arrive as pasted text. Only a
/// payload whose every path names an existing image file qualifies; prose and
/// code return `None` and are pasted as text.
pub fn pasted_image_paths(text: &str, base: &Path) -> Option<Vec<PathBuf>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut groups: Vec<Vec<&str>> = vec![vec![trimmed]];
    let lines: Vec<&str> = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() > 1 {
        groups.push(lines);
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() > 1 {
        groups.push(tokens);
    }
    groups.into_iter().find_map(|group| {
        group
            .into_iter()
            .map(|candidate| pasted_image_path(candidate, base))
            .collect()
    })
}

/// An existing image file named by one pasted candidate.
fn pasted_image_path(candidate: &str, base: &Path) -> Option<PathBuf> {
    let mut text = candidate.trim();
    let quoted = text.len() >= 2
        && (text.starts_with('"') && text.ends_with('"')
            || text.starts_with('\'') && text.ends_with('\''));
    if quoted {
        text = &text[1..text.len() - 1];
    }
    let decoded;
    if let Some(rest) = text.strip_prefix("file://") {
        // `file:///C:/x.png` keeps a leading slash before the drive letter.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let rest = match rest.as_bytes() {
            [b'/', _, b':', ..] => &rest[1..],
            _ => rest,
        };
        decoded = percent_decode(rest)?;
        text = &decoded;
    }
    let path = Path::new(text);
    // A space inside an unquoted relative token is more likely prose.
    if text.chars().any(char::is_whitespace) && !quoted && !path.is_absolute() {
        return None;
    }
    if !has_image_extension(path) {
        return None;
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    path.is_file().then_some(path)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// File name shown for an attachment.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// What the attachment strip knows about one image file, read from its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    /// Pixel width and height.
    pub size: (u32, u32),
    /// File size in bytes; `None` while a pasted bitmap is still being saved.
    pub bytes: Option<u64>,
}

/// Header-only metadata for an attachment, or why it cannot be attached.
///
/// # Errors
/// Returns a short reason when the file is missing or not a readable image.
pub fn image_info(path: &Path) -> Result<ImageInfo> {
    let metadata = std::fs::metadata(path).context("not found")?;
    if !metadata.is_file() {
        bail!("not a file");
    }
    let size = image::image_dimensions(path).context("unreadable")?;
    Ok(ImageInfo {
        size,
        bytes: Some(metadata.len()),
    })
}

/// Identity of a file version: a changed file is read again.
type Stamp = (u64, Option<std::time::SystemTime>);

fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()))
}

/// Entries kept per map before the cache starts over.
const CACHE_LIMIT: usize = 64;

enum JobEvent {
    /// A pasted bitmap's PNG is on disk as this file version, or why not.
    Saved(Result<Stamp, String>),
    /// Compression finished. Failures are not kept: a send redoes the work
    /// and reports the reason.
    Prepared(Result<PreparedImage>),
}

/// Background work for one attachment file.
struct Job {
    /// Pixel size of a pasted bitmap whose PNG is still being written.
    saving: Option<(u32, u32)>,
    /// File version being compressed; unknown until a pasted PNG is written.
    stamp: Option<Stamp>,
    events: std::sync::mpsc::Receiver<JobEvent>,
}

/// Per-file results reused across frames and sends.
///
/// Chips are drawn every frame and a draft is often sent more than once after
/// an error, so header reads and compression are cached until the file changes.
/// Saving a pasted bitmap and compressing a screenshot each take hundreds of
/// milliseconds, so both run in the background as soon as the image is
/// attached; only a send that arrives before they finish waits for them.
#[derive(Default)]
pub struct AttachmentCache {
    info: HashMap<PathBuf, (Option<Stamp>, Result<ImageInfo, String>)>,
    prepared: HashMap<PathBuf, (Stamp, PreparedImage)>,
    jobs: HashMap<PathBuf, Job>,
}

impl AttachmentCache {
    /// Header metadata for a chip, or a short reason it cannot be attached.
    ///
    /// The first successful read of a file version also starts compressing it
    /// for the agent in the background.
    pub fn info(&mut self, path: &Path) -> Result<ImageInfo, String> {
        self.poll(path);
        if let Some(size) = self.jobs.get(path).and_then(|job| job.saving) {
            return Ok(ImageInfo { size, bytes: None });
        }
        let current = stamp(path);
        if let Some((cached, result)) = self.info.get(path)
            && *cached == current
        {
            return result.clone();
        }
        if self.info.len() >= CACHE_LIMIT {
            self.info.clear();
        }
        let result = image_info(path).map_err(|error| format!("{error:#}"));
        self.info
            .insert(path.to_path_buf(), (current, result.clone()));
        if let (Ok(_), Some(current)) = (&result, current) {
            self.warm(path, current);
        }
        result
    }

    /// Writes a pasted bitmap to `path` and then compresses it, off the UI thread.
    fn save_in_background(&mut self, image: image::RgbaImage, dir: &Path, path: &Path) {
        if self.jobs.get(path).is_some_and(|job| job.saving.is_some()) {
            return;
        }
        let size = image.dimensions();
        let (dir, owned) = (dir.to_path_buf(), path.to_path_buf());
        self.spawn(path, Some(size), None, move |events| {
            let saved = save_clipboard_png(&image, &dir, &owned)
                .map_err(|error| format!("{error:#}"))
                .and_then(|()| stamp(&owned).ok_or_else(|| "not found".to_string()));
            drop(image);
            let saved_ok = saved.is_ok();
            if events.send(JobEvent::Saved(saved)).is_ok() && saved_ok {
                let _ = events.send(JobEvent::Prepared(prepare_for_agent(&owned)));
            }
        });
    }

    fn warm(&mut self, path: &Path, current: Stamp) {
        let cached = self.prepared.get(path).map(|(stamp, _)| *stamp);
        let running = self.jobs.get(path).and_then(|job| job.stamp);
        if cached == Some(current) || running == Some(current) {
            return;
        }
        let owned = path.to_path_buf();
        self.spawn(path, None, Some(current), move |events| {
            let _ = events.send(JobEvent::Prepared(prepare_for_agent(&owned)));
        });
    }

    fn spawn(
        &mut self,
        path: &Path,
        saving: Option<(u32, u32)>,
        stamp: Option<Stamp>,
        work: impl FnOnce(std::sync::mpsc::Sender<JobEvent>) + Send + 'static,
    ) {
        if self.jobs.len() >= CACHE_LIMIT {
            self.jobs.clear();
        }
        let (sender, events) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("teshi-attachment".into())
            .spawn(move || work(sender));
        match spawned {
            Ok(_) => {
                self.jobs.insert(
                    path.to_path_buf(),
                    Job {
                        saving,
                        stamp,
                        events,
                    },
                );
            }
            // Compression falls back to the send itself; a paste has nothing
            // on disk, so its chip says why.
            Err(error) if saving.is_some() => {
                self.info.insert(
                    path.to_path_buf(),
                    (
                        None,
                        Err(format!("Cannot save the clipboard image: {error}")),
                    ),
                );
            }
            Err(_) => {}
        }
    }

    /// Applies finished background work for `path` without blocking.
    fn poll(&mut self, path: &Path) {
        while let Some(job) = self.jobs.get(path) {
            match job.events.try_recv() {
                Ok(event) => self.apply(path, event),
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.jobs.remove(path);
                }
            }
        }
    }

    /// Blocks until background work for `path` is done.
    fn wait(&mut self, path: &Path) {
        while let Some(job) = self.jobs.get(path) {
            match job.events.recv() {
                Ok(event) => self.apply(path, event),
                Err(_) => {
                    self.jobs.remove(path);
                }
            }
        }
    }

    fn apply(&mut self, path: &Path, event: JobEvent) {
        match event {
            JobEvent::Saved(Ok(saved)) => {
                if let Some(job) = self.jobs.get_mut(path) {
                    job.saving = None;
                    job.stamp = Some(saved);
                }
            }
            JobEvent::Saved(Err(reason)) => {
                self.jobs.remove(path);
                self.info
                    .insert(path.to_path_buf(), (stamp(path), Err(reason)));
            }
            JobEvent::Prepared(result) => {
                let started = self.jobs.remove(path).and_then(|job| job.stamp);
                if let (Ok(prepared), Some(started)) = (result, started) {
                    if self.prepared.len() >= CACHE_LIMIT {
                        self.prepared.clear();
                    }
                    self.prepared
                        .insert(path.to_path_buf(), (started, prepared));
                }
            }
        }
    }

    /// [`prepare_for_agent`], reusing the last result for an unchanged file.
    ///
    /// Waits for a background save or compression of the file to finish
    /// rather than starting over.
    ///
    /// # Errors
    /// Returns the user-facing reason from [`prepare_for_agent`].
    pub fn prepare(&mut self, path: &Path) -> Result<PreparedImage> {
        self.wait(path);
        let current = stamp(path);
        if let (Some(current), Some((cached, prepared))) = (current, self.prepared.get(path))
            && *cached == current
        {
            return Ok(prepared.clone());
        }
        let prepared = prepare_for_agent(path)?;
        if let Some(current) = current {
            if self.prepared.len() >= CACHE_LIMIT {
                self.prepared.clear();
            }
            self.prepared
                .insert(path.to_path_buf(), (current, prepared.clone()));
        }
        Ok(prepared)
    }
}

/// Short chip label: `name W×H 120 KB`.
pub fn chip_label(path: &Path, info: &Result<ImageInfo, String>) -> String {
    let name = display_name(path);
    match info {
        Ok(info) => {
            let (width, height) = info.size;
            match info.bytes {
                Some(bytes) if bytes >= 1024 * 1024 => format!(
                    "{name} {width}×{height} {:.1} MB",
                    bytes as f64 / (1024.0 * 1024.0)
                ),
                Some(bytes) => format!("{name} {width}×{height} {} KB", bytes.div_ceil(1024)),
                None => format!("{name} {width}×{height}"),
            }
        }
        Err(reason) => format!("{name} {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attachment_cache_rereads_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        let mut cache = AttachmentCache::default();
        assert!(cache.info(&path).is_err(), "a missing file has a reason");
        write_png(&path, 4, 2);
        assert_eq!(cache.info(&path).map(|info| info.size), Ok((4, 2)));
        assert_eq!(
            chip_label(&path, &cache.info(&path)),
            "a.png 4×2 1 KB",
            "small files round up to one KB"
        );
        assert_eq!(cache.prepare(&path).unwrap().media_type, "image/png");
    }

    fn write_png(path: &Path, width: u32, height: u32) {
        image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x * 7) as u8, (y * 13) as u8, ((x ^ y) * 3) as u8, 255])
        })
        .save(path)
        .unwrap();
    }

    #[test]
    fn test_image_mentions_table() {
        let base = Path::new("/project");
        let cases: &[(&str, &[&str])] = &[
            ("look at @shot.png please", &["@shot.png"]),
            ("@a.PNG, @b.jpg.", &["@a.PNG", "@b.jpg"]),
            ("mail me@x.png or @notes.txt", &[]),
            ("@\"my shot.webp\" ok", &["@\"my shot.webp\""]),
            ("@\"unterminated.png", &[]),
            ("(@c.gif)", &[]),
            ("line\n@d.bmp", &["@d.bmp"]),
        ];
        for (text, expected) in cases {
            let found: Vec<&str> = image_mentions(text, base)
                .iter()
                .map(|mention| &text[mention.range.clone()])
                .collect();
            assert_eq!(&found, expected, "mentions in {text:?}");
        }
    }

    #[test]
    fn test_relative_mentions_resolve_against_the_base() {
        let mentions = image_mentions("@img/a.png @/abs/b.png", Path::new("/root"));
        assert_eq!(mentions[0].path, Path::new("/root/img/a.png"));
        assert_eq!(mentions[1].path, Path::new("/abs/b.png"));
    }

    #[test]
    fn test_remove_attachment_drops_one_adjoining_space() {
        let base = Path::new("/p");
        let none = DraftImages::default();
        let mut text = String::from("see @a.png @b.png now");
        assert_eq!(remove_attachment(&mut text, &none, base, 0), Some(4));
        assert_eq!(text, "see @b.png now");
        let mut text = String::from("tail @a.png");
        remove_attachment(&mut text, &none, base, 0);
        assert_eq!(text, "tail");
        assert_eq!(remove_attachment(&mut text, &none, base, 0), None);
    }

    #[test]
    fn test_placeholders_table() {
        let cases: &[(&str, &[usize])] = &[
            ("[Image #1]", &[1]),
            ("a [Image #2] b [Image #10]", &[2, 10]),
            ("[Image #] [Image #x] [Image #3", &[]),
            ("[Image [Image #4]", &[4]),
        ];
        for (text, expected) in cases {
            let numbers: Vec<usize> = placeholders(text).into_iter().map(|(_, n)| n).collect();
            assert_eq!(&numbers, expected, "placeholders in {text:?}");
        }
    }

    #[test]
    fn test_draft_numbers_only_grow_and_unknown_placeholders_stay_text() {
        let mut images = DraftImages::default();
        assert_eq!(images.attach("/p/a.png".into()), "[Image #1]");
        assert_eq!(images.attach("/p/b.png".into()), "[Image #2]");
        let text = "x [Image #2] [Image #9] @/p/c.png";
        let found: Vec<PathBuf> = draft_attachments(text, &images, Path::new("/p"))
            .into_iter()
            .map(|attachment| attachment.path)
            .collect();
        assert_eq!(
            found,
            [PathBuf::from("/p/b.png"), PathBuf::from("/p/c.png")],
            "#9 has no image and #1 was deleted from the text"
        );
        assert_eq!(images.attach("/p/d.png".into()), "[Image #3]");
    }

    #[test]
    fn test_expand_and_collapse_round_trip() {
        let base = Path::new("/p");
        let mut images = DraftImages::default();
        let first = images.attach(PathBuf::from("/p/a b.png"));
        let second = images.attach(PathBuf::from("/p/c.png"));
        let draft = format!("look{first}and {second}, ok");
        let stored = expand_placeholders(&draft, &images);
        assert_eq!(stored, "look @\"/p/a b.png\" and @/p/c.png, ok");
        let (shown, restored) = collapse_mentions(&stored, base);
        assert_eq!(shown, "look [Image #1] and [Image #2], ok");
        assert_eq!(restored.path(1), Some(Path::new("/p/a b.png")));
        assert_eq!(strip_image_mentions(&stored, base), "look and, ok");
        assert_eq!(strip_image_mentions("@/p/c.png", base), "");
    }

    #[test]
    fn test_format_mention_quotes_paths_with_spaces() {
        assert_eq!(format_mention(Path::new("a b.png")), "@\"a b.png\"");
        let text = format!("{} x", format_mention(Path::new("/tmp/a b.png")));
        assert_eq!(
            image_mentions(&text, Path::new("/"))
                .first()
                .map(|m| m.path.clone()),
            Some(PathBuf::from("/tmp/a b.png"))
        );
    }

    #[test]
    fn test_paste_of_existing_image_paths_is_recognized() {
        let dir = tempfile::tempdir().unwrap();
        let spaced = dir.path().join("with space.png");
        let plain = dir.path().join("plain.png");
        write_png(&spaced, 2, 2);
        write_png(&plain, 2, 2);
        let quoted = format!("\"{}\"", spaced.display());
        assert_eq!(
            pasted_image_paths(&quoted, dir.path()),
            Some(vec![spaced.clone()])
        );
        assert_eq!(
            pasted_image_paths(&format!("{}\nplain.png", spaced.display()), dir.path()),
            Some(vec![spaced, plain])
        );
        assert_eq!(pasted_image_paths("plain.png and prose", dir.path()), None);
        assert_eq!(pasted_image_paths("missing.png", dir.path()), None);
        assert_eq!(pasted_image_paths("", dir.path()), None);
    }

    #[test]
    fn test_file_urls_are_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a b.png");
        write_png(&path, 1, 1);
        let url = format!(
            "file://{}{}",
            if cfg!(windows) { "/" } else { "" },
            path.display()
                .to_string()
                .replace('\\', "/")
                .replace(' ', "%20")
        );
        assert!(pasted_image_paths(&url, dir.path()).is_some(), "{url}");
    }

    #[test]
    fn test_small_png_is_sent_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.png");
        write_png(&path, 8, 8);
        let prepared = prepare_for_agent(&path).unwrap();
        assert_eq!(prepared.media_type, "image/png");
        assert_eq!(&*prepared.data, std::fs::read(&path).unwrap().as_slice());
    }

    #[test]
    fn test_unsupported_wire_format_is_converted_to_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.bmp");
        image::RgbImage::from_pixel(16, 8, image::Rgb([10, 200, 30]))
            .save(&path)
            .unwrap();
        let prepared = prepare_for_agent(&path).unwrap();
        assert_eq!(prepared.media_type, "image/jpeg");
        assert_eq!(wire_media_type(&prepared.data), Some("image/jpeg"));
    }

    #[test]
    fn test_oversized_image_is_compressed_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noise.png");
        // Noise defeats PNG compression, so the source is far over 256 KiB.
        let mut state = 0x1234_5678_u32;
        image::RgbImage::from_fn(1200, 900, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let [a, b, c, _] = state.to_le_bytes();
            image::Rgb([a, b, c])
        })
        .save(&path)
        .unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > MAX_IMAGE_BYTES as u64);
        let prepared = prepare_for_agent(&path).unwrap();
        assert_eq!(prepared.media_type, "image/jpeg");
        assert!(prepared.data.len() <= MAX_IMAGE_BYTES);
    }

    #[test]
    fn test_unreadable_files_report_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake.png");
        std::fs::write(&fake, b"not an image").unwrap();
        let error = format!("{:#}", prepare_for_agent(&fake).unwrap_err());
        assert!(error.contains("fake.png"), "{error}");
        let missing = format!(
            "{:#}",
            prepare_for_agent(&dir.path().join("gone.png")).unwrap_err()
        );
        assert!(missing.contains("Cannot read image gone.png"), "{missing}");
    }

    #[test]
    fn test_clipboard_pngs_are_deduplicated_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::RgbaImage::from_pixel(3, 3, image::Rgba([1, 2, 3, 255]));
        let first = clipboard_png_path(&image, dir.path());
        save_clipboard_png(&image, dir.path(), &first).unwrap();
        assert_eq!(clipboard_png_path(&image, dir.path()), first);
        for index in 0..KEEP_COUNT + 5 {
            let other = image::RgbaImage::from_pixel(2, 2, image::Rgba([index as u8, 0, 9, 255]));
            let path = clipboard_png_path(&other, dir.path());
            save_clipboard_png(&other, dir.path(), &path).unwrap();
        }
        let kept = std::fs::read_dir(dir.path()).unwrap().count();
        assert!(kept <= KEEP_COUNT, "{kept} files kept");
    }

    #[test]
    fn test_background_paste_is_saved_and_prepared_before_send() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::RgbaImage::from_pixel(5, 4, image::Rgba([9, 8, 7, 255]));
        let path = clipboard_png_path(&image, dir.path());
        let mut cache = AttachmentCache::default();
        cache.save_in_background(image, dir.path(), &path);
        let info = cache.info(&path).expect("a pending paste has a chip");
        assert_eq!(info.size, (5, 4));
        let prepared = cache.prepare(&path).expect("send waits for the save");
        assert_eq!(prepared.media_type, "image/png");
        assert!(path.is_file(), "the PNG is on disk after the send");
        assert!(cache.jobs.is_empty(), "finished work is collected");
        assert_eq!(
            cache.info(&path).unwrap().bytes.map(|bytes| bytes > 0),
            Some(true)
        );
    }

    #[test]
    fn test_chip_label_omits_size_while_saving() {
        let path = Path::new("/c/clip.png");
        let saving = Ok(ImageInfo {
            size: (3, 2),
            bytes: None,
        });
        assert_eq!(chip_label(path, &saving), "clip.png 3×2");
    }

    #[test]
    fn test_attachment_dir_is_outside_the_project() {
        let project = tempfile::tempdir().unwrap();
        let dir = attachment_dir(project.path());
        assert!(!dir.starts_with(project.path()));
        assert_eq!(dir, attachment_dir(project.path()));
    }
}
