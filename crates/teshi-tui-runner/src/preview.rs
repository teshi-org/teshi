//! Immutable SVG preview conversion, shared font preparation and stage timings.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;

/// Costs of converting one immutable source frame, in seconds.
#[derive(Debug, Serialize)]
pub struct PreviewTimings {
    /// Loading the shared font database, or retrieving its existing instance.
    pub fonts: f64,
    /// Reading and parsing the source SVG, including text shaping.
    pub parse: f64,
    /// Painting the parsed tree into a pixel buffer.
    pub render: f64,
    /// PNG encoding of the painted pixels.
    pub encode: f64,
    /// Persisting the completed PNG bytes.
    pub write: f64,
}

/// Converts one fresh SVG capture to PNG without caching the frame or its pixels.
///
/// Only immutable system-font preparation is shared inside the process. The
/// optional `TESHI_CAPTURE_TIMINGS` log records each stage and source filename.
///
/// # Errors
/// Returns input, parse, allocation, encoding, output or timing-log errors.
pub fn render_svg(svg: &Path, png: &Path) -> Result<PreviewTimings> {
    let started = Instant::now();
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    let options = resvg::usvg::Options {
        fontdb: FONTS
            .get_or_init(|| {
                let mut database = resvg::usvg::fontdb::Database::new();
                database.load_system_fonts();
                Arc::new(database)
            })
            .clone(),
        ..Default::default()
    };
    let fonts = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let tree = resvg::usvg::Tree::from_data(&std::fs::read(svg)?, &options)?;
    let parse = started.elapsed().as_secs_f64();
    let size = tree.size().to_int_size();
    anyhow::ensure!(
        size.width() <= 16384
            && size.height() <= 16384
            && u64::from(size.width()) * u64::from(size.height()) <= 32 * 1024 * 1024,
        "terminal preview exceeds the pixel budget"
    );
    let started = Instant::now();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .context("cannot allocate terminal preview")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    let render = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let bytes = pixmap.encode_png()?;
    let encode = started.elapsed().as_secs_f64();
    let started = Instant::now();
    std::fs::write(png, bytes)?;
    let timings = PreviewTimings {
        fonts,
        parse,
        render,
        encode,
        write: started.elapsed().as_secs_f64(),
    };
    if let Some(path) = std::env::var_os("TESHI_CAPTURE_TIMINGS") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(
            file,
            "{}",
            serde_json::json!({"svg":svg,"png":png,"pid":std::process::id(),"stages":timings})
        )?;
    }
    Ok(timings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_source_produces_fresh_pixels_and_oversized_input_fails() {
        let directory = tempfile::tempdir().unwrap();
        let svg = directory.path().join("frame.svg");
        let png = directory.path().join("frame.png");
        for (color, expected) in [("red", (255, 0, 0)), ("blue", (0, 0, 255))] {
            std::fs::write(&svg, format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="{color}"/></svg>"#)).unwrap();
            render_svg(&svg, &png).unwrap();
            let image = resvg::tiny_skia::Pixmap::load_png(&png).unwrap();
            let pixel = image.pixel(0, 0).unwrap();
            assert_eq!((pixel.red(), pixel.green(), pixel.blue()), expected);
        }
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="20000"/>"#,
        )
        .unwrap();
        assert!(
            render_svg(&svg, &png)
                .unwrap_err()
                .to_string()
                .contains("pixel budget")
        );
    }
}
