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

/// [`ortho`] for a puppet: a figure drawn from above has its own view seen
/// from above (screen up = +z, nearer = higher), and its side view slid back
/// into the side silhouette's frame (`Puppet::side_shift`).
pub fn ortho_of(puppet: &Puppet, view: View, min_x: f32, max_y: f32, px_per_ft: f32) -> Box<dyn Fn(V3) -> ([f32; 2], f32)> {
    if puppet.from_above && view == View::Front {
        Box::new(move |p: V3| ([(p[0] - min_x) * px_per_ft, (max_y - p[2]) * px_per_ft], 100.0 - p[1]))
    } else if puppet.from_above {
        let side = ortho(view, min_x + puppet.side_shift, max_y, px_per_ft);
        Box::new(side)
    } else {
        Box::new(ortho(view, min_x, max_y, px_per_ft))
    }
}

/// The rest pose as flat primitives on `mask`'s grid. `unlifted` undoes the
/// grounding so the figure sits where the artist clicked it.
pub fn rest_prims(puppet: &Puppet, view: View, mask: &Mask, look: &Look) -> Vec<Prim> {
    let g = mask.grid;
    let posed = puppet.pose(&Pose::default());
    let lift = posed.lift;
    let project = ortho_of(puppet, view, g.min_x, g.max_y, g.px_per_ft);
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
///
/// The figure counts wherever it is, not only on the silhouette's grid:
/// rendered on that grid alone, whatever stood outside it was clipped
/// away, and a figure twice as wide as its outline scored 1.0.
pub fn rest_iou(puppet: &Puppet, view: View, mask: &Mask) -> f32 {
    let g = mask.grid;
    if g.w == 0 || g.h == 0 || !(g.px_per_ft > 0.0) {
        return 0.0;
    }
    let posed = puppet.pose(&Pose::default());
    let lift = posed.lift;
    // How far the figure reaches past the grid on each side, in pixels.
    let [lo, hi] = posed.bounds();
    let project = ortho_of(puppet, view, g.min_x, g.max_y, g.px_per_ft);
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for corner in 0..8 {
        let pick = |axis: usize| if corner & (1 << axis) == 0 { lo[axis] } else { hi[axis] };
        let (px, _) = project([pick(0), pick(1) - lift, pick(2)]);
        min = [min[0].min(px[0]), min[1].min(px[1])];
        max = [max[0].max(px[0]), max[1].max(px[1])];
    }
    // A pixel of slack for the rasterizer; never more than a few grids over
    // (a figure that blew up is wrong however far it reaches).
    let pad = |v: f32| (v + 1.0).max(0.0).ceil().min((4 * (g.w + g.h)) as f32) as usize;
    let (left, top) = (pad(-min[0]), pad(-min[1]));
    let (right, bottom) = (pad(max[0] - g.w as f32), pad(max[1] - g.h as f32));
    let (w, h) = (g.w + left + right, g.h + top + bottom);
    let look = Look { outline_px: 0.0, min_station_px: 0.0, ..Look::default() };
    let padded = ortho_of(puppet, view, g.min_x - left as f32 / g.px_per_ft, g.max_y + top as f32 / g.px_per_ft, g.px_per_ft);
    let mut canvas = Canvas::new(w, h, 1, [0, 0, 0]);
    canvas.draw(&posed.draw_list(&|p: V3| padded([p[0], p[1] - lift, p[2]]), &look));
    let figure: Vec<bool> = canvas.to_rgba8().chunks_exact(4).map(|px| px[0] != 0 || px[1] != 0 || px[2] != 0).collect();
    let (mut both, mut either) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let on_grid = (left..left + g.w).contains(&x) && (top..top + g.h).contains(&y);
            let inside = on_grid && mask.inside[(y - top) * g.w + (x - left)];
            let drawn = figure[y * w + x];
            both += usize::from(drawn && inside);
            either += usize::from(drawn || inside);
        }
    }
    if either == 0 {
        0.0
    } else {
        both as f32 / either as f32
    }
}
