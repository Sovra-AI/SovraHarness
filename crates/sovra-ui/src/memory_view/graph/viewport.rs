//! Pan and zoom for the knowledge map: maps layout thousandths to the drawing and back.
//!
//! Kept free of view state so the arithmetic is unit-tested without a window.

use gpui_kit::{Bounds, Pixels, Point, point};

/// Where the map is looking. `pan` is a screen offset; `zoom` scales about the centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Viewport {
    pub(crate) zoom: f32,
    pub(crate) pan: Point<Pixels>,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: point(Pixels::ZERO, Pixels::ZERO),
        }
    }
}

/// A measured drawing: its window bounds and the padding kept clear at the edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) pad: Pixels,
}

impl Frame {
    /// Where a layout point (thousandths of each axis) appears on screen at zoom 1.
    fn base(self, x: f32, y: f32) -> Point<Pixels> {
        let inner_w = (self.bounds.size.width - self.pad * 2.).max(Pixels::ZERO);
        let inner_h = (self.bounds.size.height - self.pad * 2.).max(Pixels::ZERO);
        point(
            self.bounds.origin.x + self.pad + inner_w * (x / 1000.),
            self.bounds.origin.y + self.pad + inner_h * (y / 1000.),
        )
    }

    pub(crate) fn to_screen(self, view: Viewport, x: f32, y: f32) -> Point<Pixels> {
        let centre = self.bounds.center();
        let base = self.base(x, y);
        point(
            centre.x + (base.x - centre.x) * view.zoom + view.pan.x,
            centre.y + (base.y - centre.y) * view.zoom + view.pan.y,
        )
    }

    /// The layout point under a screen position, used when a node is dragged.
    pub(crate) fn to_layout(self, view: Viewport, at: Point<Pixels>) -> (f32, f32) {
        let centre = self.bounds.center();
        let base_x = centre.x + (at.x - centre.x - view.pan.x) / view.zoom;
        let base_y = centre.y + (at.y - centre.y - view.pan.y) / view.zoom;
        let inner_w = (self.bounds.size.width - self.pad * 2.).max(Pixels::ZERO);
        let inner_h = (self.bounds.size.height - self.pad * 2.).max(Pixels::ZERO);
        let unit = |offset: Pixels, span: Pixels| {
            if span > Pixels::ZERO {
                (offset / span * 1000.).clamp(0., 1000.)
            } else {
                500.
            }
        };
        (
            unit(base_x - self.bounds.origin.x - self.pad, inner_w),
            unit(base_y - self.bounds.origin.y - self.pad, inner_h),
        )
    }

    /// Zooms by `factor` keeping the point under `anchor` still, as maps do.
    pub(crate) fn zoom_about(
        &self,
        view: Viewport,
        anchor: Point<Pixels>,
        factor: f32,
        limits: (f32, f32),
    ) -> Viewport {
        let zoom = (view.zoom * factor).clamp(limits.0, limits.1);
        let centre = self.bounds.center();
        let ratio = zoom / view.zoom;
        let pan = point(
            anchor.x - centre.x - (anchor.x - centre.x - view.pan.x) * ratio,
            anchor.y - centre.y - (anchor.y - centre.y - view.pan.y) * ratio,
        );
        Viewport { zoom, pan }
    }

    /// Width over height times 100, rounded to a quarter so small resizes keep the layout.
    pub(crate) fn aspect_percent(&self) -> u16 {
        let width = self.bounds.size.width - self.pad * 2.;
        let height = self.bounds.size.height - self.pad * 2.;
        if height <= Pixels::ZERO || width <= Pixels::ZERO {
            return sovra_core::MEMORY_GRAPH_SQUARE;
        }
        let quarters = (width / height * 4.).round().clamp(2., 16.);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 2..=16 quarters before conversion"
        )]
        {
            quarters as u16 * 25
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{px, size};

    fn frame() -> Frame {
        Frame {
            bounds: Bounds::new(point(px(100.), px(50.)), size(px(1000.), px(500.))),
            pad: px(20.),
        }
    }

    fn close(a: Point<Pixels>, b: Point<Pixels>) -> bool {
        (a.x - b.x).abs() < px(0.01) && (a.y - b.y).abs() < px(0.01)
    }

    #[test]
    fn layout_points_round_trip_through_any_pan_and_zoom() {
        let frame = frame();
        for view in [
            Viewport::default(),
            Viewport {
                zoom: 2.5,
                pan: point(px(-140.), px(60.)),
            },
        ] {
            for (x, y) in [(0., 0.), (500., 500.), (250., 900.), (1000., 1000.)] {
                let screen = frame.to_screen(view, x, y);
                let (bx, by) = frame.to_layout(view, screen);
                assert!((bx - x).abs() < 0.01 && (by - y).abs() < 0.01, "{x},{y}");
            }
        }
        assert!(close(
            frame.to_screen(Viewport::default(), 500., 500.),
            frame.bounds.center()
        ));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer_and_respects_limits() {
        let frame = frame();
        let anchor = point(px(300.), px(120.));
        let start = Viewport::default();
        let under = frame.to_layout(start, anchor);
        let zoomed = frame.zoom_about(start, anchor, 1.25, (0.5, 3.0));
        assert!((zoomed.zoom - 1.25).abs() < f32::EPSILON);
        assert!(close(frame.to_screen(zoomed, under.0, under.1), anchor));
        let capped = frame.zoom_about(zoomed, anchor, 100.0, (0.5, 3.0));
        assert!((capped.zoom - 3.0).abs() < f32::EPSILON);
        assert!(close(frame.to_screen(capped, under.0, under.1), anchor));
    }

    #[test]
    fn aspect_follows_the_pane_in_quarter_steps() {
        assert_eq!(frame().aspect_percent(), 200);
        let mut tall = frame();
        tall.bounds.size = size(px(300.), px(900.));
        assert_eq!(tall.aspect_percent(), 50);
        tall.bounds.size = size(px(0.), px(0.));
        assert_eq!(tall.aspect_percent(), sovra_core::MEMORY_GRAPH_SQUARE);
    }
}
