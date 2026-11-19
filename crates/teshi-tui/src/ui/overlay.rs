//! Overlay sizing shared by every modal card.
//!
//! iCode gives each dialog two constraints per axis: a preferred size and a
//! share of the viewport it never exceeds (`max-width: 90%`), plus an absolute
//! cap on large terminals (`max-width: 120`). This module is the one place that
//! resolves those rules for the cards migrated so far; dialogs keep their own
//! width-tier ladder and the remaining overlays migrate incrementally.
//!
//! Rendering stays with the caller: resolve the card with [`OverlaySpec::rect`],
//! then draw chrome, body and actions inside it.

use ratatui::layout::Rect;

/// Share of the viewport an overlay may use unless the caller says otherwise.
///
/// The remaining tenth keeps the backdrop visible, as iCode's `max-width: 90%`
/// does for confirmations, forms and pickers.
pub(crate) const DEFAULT_SHARE: u8 = 90;
/// Share used by form cards, matching iCode's `max-height: 85%`.
pub(crate) const FORM_SHARE: u8 = 85;

/// How one axis prefers to be sized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizeRule {
    /// Fixed columns or rows, like iCode's `width: 60`.
    Cells(u16),
    /// A share of the viewport, like iCode's `width: 92%`.
    Percent(u8),
    /// Measured from the content by the caller.
    Content,
}

/// One axis of an overlay: preferred size, floor, absolute cap and viewport share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Axis {
    pub rule: SizeRule,
    /// Smallest useful size; ignored when the screen cannot seat it.
    pub min: u16,
    /// Absolute cap so a wide terminal does not stretch the card.
    pub max: u16,
    /// Viewport share this axis never exceeds. `100` means "no share cap".
    pub share: u8,
}

impl Axis {
    /// Fixed size, like iCode's `width: 60`.
    pub(crate) const fn cells(cells: u16, min: u16, share: u8) -> Self {
        Self {
            rule: SizeRule::Cells(cells),
            min,
            max: cells,
            share,
        }
    }

    /// Viewport share, like iCode's `height: 70%`.
    pub(crate) const fn percent(percent: u8, min: u16, max: u16, share: u8) -> Self {
        Self {
            rule: SizeRule::Percent(percent),
            min,
            max,
            share,
        }
    }

    /// Content-driven size, capped by `max` and `share`.
    pub(crate) const fn content(min: u16, max: u16, share: u8) -> Self {
        Self {
            rule: SizeRule::Content,
            min,
            max,
            share,
        }
    }

    /// Resolve this axis against `available` cells and the caller's measurement.
    ///
    /// `content` is ignored by [`SizeRule::Cells`] and [`SizeRule::Percent`].
    pub(crate) fn resolve(self, available: u16, content: u16) -> u16 {
        if available == 0 {
            return 0;
        }
        let preferred = match self.rule {
            SizeRule::Cells(cells) => cells,
            SizeRule::Percent(percent) => (u32::from(available) * u32::from(percent) / 100) as u16,
            SizeRule::Content => content,
        };
        let cap = self
            .max
            .min(share_cap(available, self.share))
            .min(available);
        // A floor above the cap would be meaningless (`clamp` would panic), so
        // the screen wins: a tiny terminal loses the margin, never the card.
        preferred.clamp(self.min.min(cap), cap)
    }
}

/// iCode-style box rule for one overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverlaySpec {
    pub width: Axis,
    pub height: Axis,
}

impl OverlaySpec {
    /// Small prompt or picker: fixed width, content height, 90% caps.
    pub(crate) const fn prompt(width: u16, min_h: u16, max_h: u16) -> Self {
        Self {
            width: Axis::cells(width, 8, DEFAULT_SHARE),
            height: Axis::content(min_h, max_h, DEFAULT_SHARE),
        }
    }

    /// Form card: fixed width, content height capped at 85%, like iCode's
    /// `max-height: 85%` on the model, auth and agent configuration cards.
    pub(crate) const fn form(width: u16, min_h: u16, max_h: u16) -> Self {
        Self {
            width: Axis::cells(width, 8, DEFAULT_SHARE),
            height: Axis::content(min_h, max_h, FORM_SHARE),
        }
    }

    /// Content-driven width between `min_w` and `max_w`, content height: review
    /// prompts whose card should hug a short diff instead of filling the screen.
    pub(crate) const fn content(min_w: u16, max_w: u16, min_h: u16, max_h: u16) -> Self {
        Self {
            width: Axis::content(min_w, max_w, DEFAULT_SHARE),
            height: Axis::content(min_h, max_h, DEFAULT_SHARE),
        }
    }

    /// Large viewer: percentage width and height with absolute caps, matching
    /// iCode's `width: 92%; max-width: 120; height: 85%; max-height: 45`.
    pub(crate) const fn viewer(
        width_pct: u8,
        min_w: u16,
        max_w: u16,
        height_pct: u8,
        min_h: u16,
        max_h: u16,
    ) -> Self {
        Self {
            width: Axis::percent(width_pct, min_w, max_w, 100),
            height: Axis::percent(height_pct, min_h, max_h, 100),
        }
    }

    /// Resolve to a centered rectangle.
    ///
    /// `content` is `(columns, rows)` measured without the card's border and
    /// action strip; it only matters for [`SizeRule::Content`] axes.
    pub(crate) fn rect(self, area: Rect, content: (u16, u16)) -> Rect {
        let width = self.width.resolve(area.width, content.0);
        let height = self.height.resolve(area.height, content.1);
        centered(area, width, height)
    }
}

/// Center `width × height` in `area`, never exceeding it.
pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// Cells that `share` percent of `available` allows.
pub(crate) fn share_cap(available: u16, share: u8) -> u16 {
    (u32::from(available) * u32::from(share) / 100) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terminal sizes every overlay must survive: wide, ordinary, small, narrow.
    const VIEWPORTS: [(u16, u16); 4] = [(200, 60), (120, 36), (80, 24), (40, 20)];

    fn rect(spec: OverlaySpec, width: u16, height: u16, content: (u16, u16)) -> Rect {
        spec.rect(Rect::new(0, 0, width, height), content)
    }

    #[test]
    fn fixed_width_keeps_its_columns_until_the_screen_share_bites() {
        let spec = OverlaySpec::prompt(60, 8, 20);
        let widths: Vec<u16> = VIEWPORTS
            .iter()
            .map(|(width, height)| rect(spec, *width, *height, (30, 12)).width)
            .collect();
        assert_eq!(widths, vec![60, 60, 60, 36], "fixed width with 90% cap");
    }

    #[test]
    fn content_height_is_driven_by_the_body_and_bounded_by_floor_and_cap() {
        let spec = OverlaySpec::prompt(60, 8, 20);
        let heights: Vec<u16> = [(30, 3), (30, 12), (30, 40)]
            .iter()
            .map(|(columns, rows)| rect(spec, 120, 36, (*columns, *rows)).height)
            .collect();
        assert_eq!(heights, vec![8, 12, 20]);
    }

    #[test]
    fn content_width_grows_with_its_content_and_stops_at_the_absolute_cap() {
        let spec = OverlaySpec::content(20, 100, 10, 22);
        assert_eq!(rect(spec, 200, 60, (40, 12)).width, 40);
        assert_eq!(rect(spec, 200, 60, (200, 12)).width, 100);
        assert_eq!(rect(spec, 80, 24, (200, 12)).width, 72);
        assert_eq!(rect(spec, 200, 60, (5, 12)).width, 20, "minimum width");
    }

    #[test]
    fn form_height_keeps_fifteen_percent_of_the_screen_visible() {
        let spec = OverlaySpec::form(60, 8, 40);
        assert_eq!(rect(spec, 120, 40, (30, 100)).height, 34, "85% of 40 rows");
        assert_eq!(rect(spec, 120, 20, (30, 100)).height, 17, "85% of 20 rows");
        assert_eq!(
            rect(spec, 120, 20, (30, 6)).height,
            8,
            "content below the floor"
        );
    }

    #[test]
    fn a_tiny_screen_spends_the_margin_instead_of_overflowing() {
        let spec = OverlaySpec::prompt(60, 8, 20);
        let placed = rect(spec, 8, 6, (30, 12));
        assert_eq!(placed.width, 7, "90% of 8 columns, floor does not win");
        assert_eq!(placed.height, 5, "90% of 6 rows");
        assert!(placed.right() <= 8 && placed.bottom() <= 6);
    }

    #[test]
    fn percent_axis_keeps_the_share_and_absolute_caps() {
        let axis = Axis::percent(70, 20, 100, DEFAULT_SHARE);
        assert_eq!(axis.resolve(200, 0), 100, "absolute cap");
        assert_eq!(axis.resolve(110, 0), 77, "70% of the viewport");
        assert_eq!(axis.resolve(40, 0), 28, "narrow viewport");
        assert_eq!(axis.resolve(0, 0), 0, "no screen, no card");
    }

    #[test]
    fn centered_honours_the_area_origin_and_clamps_oversized_cards() {
        let area = Rect::new(5, 2, 10, 6);
        assert_eq!(centered(area, 4, 2), Rect::new(8, 4, 4, 2));
        assert_eq!(centered(area, 20, 20), area, "never exceeds the area");
    }

    #[test]
    fn viewer_keeps_the_percentage_and_the_absolute_caps() {
        let spec = OverlaySpec::viewer(75, 20, 120, 70, 10, 45);
        let sizes: Vec<(u16, u16)> = VIEWPORTS
            .iter()
            .map(|(width, height)| {
                let placed = rect(spec, *width, *height, (0, 0));
                (placed.width, placed.height)
            })
            .collect();
        assert_eq!(
            sizes,
            vec![(120, 42), (90, 25), (60, 16), (30, 14)],
            "75%/70% of the viewport, capped at 120x45"
        );
    }

    #[test]
    fn cards_stay_centered_and_inside_the_viewport() {
        let spec = OverlaySpec::prompt(60, 8, 20);
        for (width, height) in VIEWPORTS {
            let area = Rect::new(0, 0, width, height);
            let placed = spec.rect(area, (30, 12));
            assert!(placed.width <= width && placed.height <= height);
            assert!(placed.right() <= width && placed.bottom() <= height);
            let left = placed.x;
            let right = width.saturating_sub(placed.right());
            assert!(
                left.abs_diff(right) <= 1,
                "{spec:?} at {width}x{height}: margins {left}/{right}"
            );
            let top = placed.y;
            let bottom = height.saturating_sub(placed.bottom());
            assert!(
                top.abs_diff(bottom) <= 1,
                "{spec:?} at {width}x{height}: margins {top}/{bottom}"
            );
        }
    }
}
