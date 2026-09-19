//! A self-contained perspective camera. The draw list only ever asks for a
//! `Fn([f32; 3]) -> ([f32; 2], f32)`, so a host can pass its own projection.

use crate::math::{mul_mv, rot_x, rot_y, V3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Turns the FIGURE about the vertical axis. 0 = facing the camera, 90 =
    /// its front points screen-right (the side silhouette's orientation).
    pub yaw_deg: f32,
    /// Camera elevation: 0 = level, positive looks down on the figure.
    pub pitch_deg: f32,
    /// Feet from the camera to the look-at point. Small = strong perspective.
    pub distance: f32,
    /// Focal length in pixels: a foot at the look-at point covers
    /// `focal / distance` pixels.
    pub focal: f32,
    /// Look-at point, world feet.
    pub target: V3,
    /// Where the look-at point lands on screen, pixels.
    pub center: [f32; 2],
}

impl Camera {
    /// A camera that fits a figure `height_ft` tall into a viewport.
    pub fn framing(height_ft: f32, viewport: [f32; 2]) -> Camera {
        let distance = 16.0;
        let px_per_ft = viewport[1] * 0.72 / height_ft.max(0.5);
        Camera {
            yaw_deg: 0.0,
            pitch_deg: 12.0,
            distance,
            focal: px_per_ft * distance,
            target: [0.0, height_ft * 0.47, 0.0],
            center: [viewport[0] * 0.5, viewport[1] * 0.5],
        }
    }

    /// World point -> (screen pixels, depth in feet from the camera plane).
    pub fn project(&self, world: V3) -> ([f32; 2], f32) {
        let q = mul_mv(&rot_y(self.yaw_deg), crate::math::sub(world, self.target));
        let q = mul_mv(&rot_x(self.pitch_deg), q);
        let depth = (self.distance - q[2]).max(0.05);
        let s = self.focal / depth;
        ([self.center[0] + q[0] * s, self.center[1] - q[1] * s], depth)
    }

    pub fn projector(&self) -> impl Fn(V3) -> ([f32; 2], f32) + '_ {
        move |p| self.project(p)
    }
}
