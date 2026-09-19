//! How well the rest-pose puppet covers the silhouettes it came from: an
//! orthographic render of the puppet against the rasterized outline,
//! intersection over union. An editor shows this as the family's score.

use rig::View;
use rig_bake::Mask;

use crate::draw::{Look, Prim};
use crate::math::V3;
use crate::pose::Pose;
use crate::puppet::Puppet;
use crate::raster::Canvas;

/// Orthographic projection of the world onto `view`'s silhouette plane, in
/// pixels of a grid `px_per_ft` fine whose top-left corner is world
/// `(min_x, max_y)` (a `rig_bake::Grid`'s layout).
///
/// Front: the viewer stands in front of the creature (+z), so its left is
/// screen-right. Side: the viewer sees its RIGHT flank, front to the right.
pub fn ortho(view: View, min_x: f32, max_y: f32, px_per_ft: f32) -> impl Fn(V3) -> ([f32; 2], f32) {
    move |p: V3| match view {
        View::Front => ([(p[0] - min_x) * px_per_ft, (max_y - p[1]) * px_per_ft], 100.0 - p[2]),
        View::Side => ([(p[2] - min_x) * px_per_ft, (max_y - p[1]) * px_per_ft], 100.0 + p[0]),
    }
}

/// The rest pose as flat primitives on `mask`'s grid. `unlifted` undoes the
/// grounding so the figure sits where the artist clicked it.
pub fn rest_prims(puppet: &Puppet, view: View, mask: &Mask, look: &Look) -> Vec<Prim> {
    let g = mask.grid;
    let posed = puppet.pose(&Pose::default());
    let lift = posed.lift;
    let project = ortho(view, g.min_x, g.max_y, g.px_per_ft);
    posed.draw_list(&|p: V3| project([p[0], p[1] - lift, p[2]]), look)
}

/// Coverage of the rest-pose puppet, per pixel of `mask`'s grid.
pub fn rest_coverage(puppet: &Puppet, view: View, mask: &Mask) -> Vec<bool> {
    let g = mask.grid;
    let look = Look { outline_px: 0.0, min_station_px: 0.0, ..Look::default() };
    let mut canvas = Canvas::new(g.w, g.h, 1, [0, 0, 0]);
    canvas.draw(&rest_prims(puppet, view, mask, &look));
    canvas.to_rgba8().chunks_exact(4).map(|px| px[0] != 0 || px[1] != 0 || px[2] != 0).collect()
}

/// Intersection over union of the rest-pose puppet and the silhouette.
pub fn rest_iou(puppet: &Puppet, view: View, mask: &Mask) -> f32 {
    let cover = rest_coverage(puppet, view, mask);
    let (mut both, mut either) = (0usize, 0usize);
    for (a, b) in cover.iter().zip(&mask.inside) {
        both += usize::from(*a && *b);
        either += usize::from(*a || *b);
    }
    if either == 0 {
        0.0
    } else {
        both as f32 / either as f32
    }
}
