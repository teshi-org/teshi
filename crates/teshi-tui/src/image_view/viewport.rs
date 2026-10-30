//! Source-pixel view transforms, independent of image decoding and terminal I/O.

use ratatui::layout::Size;

pub(super) const MAX_OUTPUT_PIXELS: u64 = 2 * 1024 * 1024;
const CENTER_SCALE: i64 = 1_000_000;

/// A view operation owned by the screenshot viewer.
#[derive(Clone, Copy, Debug)]
pub enum ViewAction {
    /// Increase magnification around the current center.
    ZoomIn,
    /// Decrease magnification around the current center.
    ZoomOut,
    /// Show source pixels at 100 percent magnification.
    ActualSize,
    /// Fit the whole image and reset its center.
    Fit,
    /// Move by a quarter of the visible source region.
    Pan(i32, i32),
}

/// A source-pixel scale and normalized center, preserved across terminal resizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewState {
    percent: Option<u16>,
    center_x: i64,
    center_y: i64,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            percent: None,
            center_x: CENTER_SCALE / 2,
            center_y: CENTER_SCALE / 2,
        }
    }
}

/// A bounded output canvas in terminal cells and virtual terminal pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Viewport {
    pub cells: Size,
    pub font: (u16, u16),
}

impl Viewport {
    pub fn new(size: Size, font: (u16, u16)) -> Self {
        let font = (font.0.clamp(1, 512), font.1.clamp(1, 512));
        let pixels =
            u64::from(size.width) * u64::from(size.height) * u64::from(font.0) * u64::from(font.1);
        let scale = (MAX_OUTPUT_PIXELS as f64 / pixels.max(1) as f64)
            .sqrt()
            .min(1.0);
        let mut width = ((f64::from(size.width) * scale).floor() as u16).max(1);
        let mut height = ((f64::from(size.height) * scale).floor() as u16).max(1);
        let max_cells = MAX_OUTPUT_PIXELS / (u64::from(font.0) * u64::from(font.1));
        // A very thin viewport can round one dimension up to one cell. Clamp
        // the other dimension again so that rounding cannot exceed the cap.
        if u64::from(width) * u64::from(height) > max_cells {
            if width >= height {
                width = (max_cells / u64::from(height)).max(1) as u16;
            } else {
                height = (max_cells / u64::from(width)).max(1) as u16;
            }
        }
        Self {
            cells: Size::new(width, height),
            font,
        }
    }

    pub fn pixels(self) -> (u32, u32) {
        (
            u32::from(self.cells.width) * u32::from(self.font.0),
            u32::from(self.cells.height) * u32::from(self.font.1),
        )
    }
}

/// A rectangle read from the original decoded image and its output dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRegion {
    /// Left source-pixel coordinate.
    pub x: u32,
    /// Top source-pixel coordinate.
    pub y: u32,
    /// Width in original pixels.
    pub width: u32,
    /// Height in original pixels.
    pub height: u32,
    pub(super) output: (u32, u32),
}

impl ViewState {
    /// Returns a compact magnification label for the viewer.
    pub fn label(self) -> String {
        self.percent
            .map_or_else(|| "Fit".into(), |percent| format!("{percent}%"))
    }

    /// Whether arrow keys can move a zoomed source region.
    pub fn is_zoomed(self) -> bool {
        self.percent.is_some()
    }

    pub(super) fn apply(
        &mut self,
        action: ViewAction,
        dimensions: Option<(u32, u32)>,
        viewport: Viewport,
    ) -> bool {
        let old = *self;
        match action {
            ViewAction::Fit => *self = Self::default(),
            ViewAction::ActualSize => self.percent = Some(100),
            ViewAction::ZoomIn | ViewAction::ZoomOut => {
                let (width, height) = viewport.pixels();
                let fit = dimensions.map_or(100.0, |(w, h)| {
                    (f64::from(width) / f64::from(w.max(1)))
                        .min(f64::from(height) / f64::from(h.max(1)))
                        .min(1.0)
                        * 100.0
                });
                let current = self.percent.map_or(fit, f64::from);
                let scale = if matches!(action, ViewAction::ZoomIn) {
                    1.25
                } else {
                    0.8
                };
                self.percent = Some((current * scale).round().clamp(1.0, 1600.0) as u16);
            }
            ViewAction::Pan(dx, dy) => {
                if let (Some(percent), Some((w, h))) = (self.percent, dimensions) {
                    let (width, height) = viewport.pixels();
                    let step = |pixels: u32, extent: u32| {
                        ((i64::from(pixels) * 100 / i64::from(percent) / 4).max(1) * CENTER_SCALE
                            / i64::from(extent.max(1)))
                        .max(1)
                    };
                    self.center_x =
                        (self.center_x + i64::from(dx) * step(width, w)).clamp(0, CENTER_SCALE);
                    self.center_y =
                        (self.center_y + i64::from(dy) * step(height, h)).clamp(0, CENTER_SCALE);
                }
            }
        }
        old != *self
    }

    pub(super) fn region(self, source: (u32, u32), viewport: Viewport) -> SourceRegion {
        let (w, h) = (source.0.max(1), source.1.max(1));
        let (out_w, out_h) = viewport.pixels();
        match self.percent {
            None => {
                let scale = (f64::from(out_w) / f64::from(w))
                    .min(f64::from(out_h) / f64::from(h))
                    .min(1.0);
                SourceRegion {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                    output: (
                        (f64::from(w) * scale).round().max(1.0) as u32,
                        (f64::from(h) * scale).round().max(1.0) as u32,
                    ),
                }
            }
            Some(percent) => {
                let width =
                    (u64::from(out_w) * 100 / u64::from(percent)).clamp(1, u64::from(w)) as u32;
                let height =
                    (u64::from(out_h) * 100 / u64::from(percent)).clamp(1, u64::from(h)) as u32;
                let x = ((self.center_x * i64::from(w) / CENTER_SCALE) as u32)
                    .saturating_sub(width / 2)
                    .min(w - width);
                let y = ((self.center_y * i64::from(h) / CENTER_SCALE) as u32)
                    .saturating_sub(height / 2)
                    .min(h - height);
                SourceRegion {
                    x,
                    y,
                    width,
                    height,
                    output: (
                        ((u64::from(width) * u64::from(percent) / 100) as u32)
                            .min(out_w)
                            .max(1),
                        ((u64::from(height) * u64::from(percent) / 100) as u32)
                            .min(out_h)
                            .max(1),
                    ),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_canvas_is_bounded_even_for_a_huge_terminal() {
        for size in [
            Size::new(65535, 65535),
            Size::new(65535, 1),
            Size::new(1, 65535),
        ] {
            let viewport = Viewport::new(size, (512, 512));
            let (w, h) = viewport.pixels();
            assert!(u64::from(w) * u64::from(h) <= MAX_OUTPUT_PIXELS);
        }
    }

    #[test]
    fn actual_size_and_pan_read_original_pixels_and_clamp_to_edges() {
        let viewport = Viewport::new(Size::new(20, 10), (10, 20));
        let mut view = ViewState::default();
        assert_eq!(view.region((4000, 2000), viewport).output, (200, 100));
        view.apply(ViewAction::ActualSize, Some((4000, 2000)), viewport);
        let center = view.region((4000, 2000), viewport);
        assert_eq!((center.width, center.height), (200, 200));
        view.apply(ViewAction::Pan(1, 0), Some((4000, 2000)), viewport);
        assert!(view.region((4000, 2000), viewport).x > center.x);
        for _ in 0..100 {
            view.apply(ViewAction::Pan(100, 100), Some((4000, 2000)), viewport);
        }
        let end = view.region((4000, 2000), viewport);
        assert_eq!((end.x + end.width, end.y + end.height), (4000, 2000));
        view.apply(ViewAction::Fit, Some((4000, 2000)), viewport);
        assert_eq!(view, ViewState::default());
    }
}
