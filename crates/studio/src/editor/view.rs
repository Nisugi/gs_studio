//! World (feet, y up) <-> screen (pixels, y down) transform for the canvas.

use rig::Pt;

/// The canvas rectangle on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    pub min: Pt,
    pub size: Pt,
}

impl Canvas {
    pub fn centre(&self) -> Pt {
        [self.min[0] + self.size[0] * 0.5, self.min[1] + self.size[1] * 0.5]
    }
}

pub const MIN_ZOOM: f32 = 2.0;
pub const MAX_ZOOM: f32 = 4000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    /// World point shown at the canvas centre.
    pub centre: Pt,
    /// Screen pixels per foot.
    pub zoom: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        ViewTransform { centre: [0.0, 3.0], zoom: 80.0 }
    }
}

impl ViewTransform {
    pub fn world_to_screen(&self, canvas: &Canvas, p: Pt) -> Pt {
        let c = canvas.centre();
        [c[0] + (p[0] - self.centre[0]) * self.zoom, c[1] - (p[1] - self.centre[1]) * self.zoom]
    }

    pub fn screen_to_world(&self, canvas: &Canvas, s: Pt) -> Pt {
        let c = canvas.centre();
        [self.centre[0] + (s[0] - c[0]) / self.zoom, self.centre[1] - (s[1] - c[1]) / self.zoom]
    }

    /// Zoom by `factor`, keeping the world point under `anchor` (a screen
    /// position, normally the cursor) where it is.
    pub fn zoom_at(&mut self, canvas: &Canvas, anchor: Pt, factor: f32) {
        let before = self.screen_to_world(canvas, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.screen_to_world(canvas, anchor);
        self.centre = [self.centre[0] + before[0] - after[0], self.centre[1] + before[1] - after[1]];
    }

    /// Pan by a screen-space drag delta: the content follows the pointer.
    pub fn pan(&mut self, delta_px: Pt) {
        self.centre = [self.centre[0] - delta_px[0] / self.zoom, self.centre[1] + delta_px[1] / self.zoom];
    }

    /// Show all of `bounds` (min_x, min_y, max_x, max_y in feet) with
    /// `margin_px` to spare on every side.
    pub fn fit(&mut self, canvas: &Canvas, bounds: [f32; 4], margin_px: f32) {
        let (w, h) = ((bounds[2] - bounds[0]).max(1e-3), (bounds[3] - bounds[1]).max(1e-3));
        let avail = [(canvas.size[0] - 2.0 * margin_px).max(16.0), (canvas.size[1] - 2.0 * margin_px).max(16.0)];
        self.zoom = (avail[0] / w).min(avail[1] / h).clamp(MIN_ZOOM, MAX_ZOOM);
        self.centre = [(bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: Canvas = Canvas { min: [100.0, 50.0], size: [800.0, 600.0] };

    fn close(a: Pt, b: Pt) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn y_flips_and_round_trips() {
        let v = ViewTransform { centre: [0.0, 3.0], zoom: 100.0 };
        assert!(close(v.world_to_screen(&CANVAS, [0.0, 3.0]), [500.0, 350.0]));
        // Up in the world is up (smaller y) on screen; +x is screen-right.
        assert!(close(v.world_to_screen(&CANVAS, [1.0, 4.0]), [600.0, 250.0]));
        let p = [-0.73, 5.2];
        assert!(close(v.screen_to_world(&CANVAS, v.world_to_screen(&CANVAS, p)), p));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut v = ViewTransform::default();
        let cursor = [220.0, 480.0];
        let before = v.screen_to_world(&CANVAS, cursor);
        v.zoom_at(&CANVAS, cursor, 1.7);
        assert!(close(v.screen_to_world(&CANVAS, cursor), before));
        assert!((v.zoom - 136.0).abs() < 1e-3);
        v.zoom_at(&CANVAS, cursor, 1e9);
        assert_eq!(v.zoom, MAX_ZOOM);
        v.zoom_at(&CANVAS, cursor, 0.0);
        assert_eq!(v.zoom, MIN_ZOOM);
    }

    #[test]
    fn pan_drags_the_content_with_the_pointer() {
        let mut v = ViewTransform { centre: [0.0, 0.0], zoom: 50.0 };
        let grabbed = v.screen_to_world(&CANVAS, [300.0, 300.0]);
        v.pan([40.0, -25.0]);
        assert!(close(v.screen_to_world(&CANVAS, [340.0, 275.0]), grabbed));
    }

    #[test]
    fn fit_shows_the_whole_bounds_centred() {
        let mut v = ViewTransform::default();
        let bounds = [-1.1, 0.0, 1.1, 6.0];
        v.fit(&CANVAS, bounds, 20.0);
        assert!((v.zoom - 560.0 / 6.0).abs() < 1e-3); // height-limited
        let top_left = v.world_to_screen(&CANVAS, [bounds[0], bounds[3]]);
        let bottom_right = v.world_to_screen(&CANVAS, [bounds[2], bounds[1]]);
        assert!(top_left[0] >= 120.0 && (top_left[1] - 70.0).abs() < 1e-2);
        assert!(bottom_right[0] <= 880.0 && (bottom_right[1] - 630.0).abs() < 1e-2);
        assert!(close(v.world_to_screen(&CANVAS, [0.0, 3.0]), CANVAS.centre()));
    }
}
