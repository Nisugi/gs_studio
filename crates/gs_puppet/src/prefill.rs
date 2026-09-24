//! Paint the pattern sheet from pictures of the creature: front, side and
//! back paintings, each drawn over its silhouette.
//!
//! The silhouettes are what the puppet was measured from, so a painting
//! made over one needs no fitting: a world point `(x, y)` is a pixel of the
//! front painting through the front silhouette's own [`Grid`]. For every
//! pixel of the sheet:
//!
//! 1. [`UvLayout::address`] gives the surface point it covers, which is put
//!    on the REST body (the pose the silhouettes show), with its outward
//!    normal.
//! 2. Each painting that can see it -- facing it, and not blocked by a
//!    nearer part of the body (a depth test against the rest body seen from
//!    that side) -- offers the colour at its projection.
//! 3. The offers are blended, each weighted by how squarely its view faces
//!    the surface, so the chest comes from the front, the flank from the
//!    side, and between them the two fade into each other.
//!
//! Views: front looks at the creature's face (+z toward the viewer); back
//! from behind, painted as seen from there (the front silhouette mirrored);
//! side as the side silhouette shows it, from the creature's right (-x),
//! and used again, as a mirror image, for the left.
//!
//! What no painting sees -- an armpit, the inside of a thigh -- is filled
//! from the nearest painted pixel of the same patch, so the sheet has no
//! holes. A pixel off every patch stays transparent.

use rig_bake::Grid;

use crate::math::{add, dot, norm_or, scale, V3};
use crate::puppet::Puppet;
use crate::uv::UvLayout;

/// How squarely a view must face a surface before it contributes.
const MIN_FACING: f32 = 0.15;
/// Sharpness of the hand-over between views.
const FACING_POWER: i32 = 3;
/// A surface point within this (feet) of the nearest surface a view sees
/// is that surface.
const DEPTH_TOL_FT: f32 = 0.08;
/// A projected point may miss its painting's paint by this many pixels;
/// the nearest painted pixel within reach is taken.
const REACH_PX: i32 = 6;
/// Ring samples when building the depth maps.
const AROUND: usize = 32;

/// One painting over a silhouette: straight RGBA8, any size with the
/// silhouette grid's aspect (it is scaled to match). Transparent = not
/// painted.
#[derive(Debug, Clone, Copy)]
pub struct Painting<'a> {
    pub rgba: &'a [u8],
    pub w: usize,
    pub h: usize,
}

/// The paintings a sheet is filled from, and the silhouette grids they were
/// drawn over. Any may be missing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Paintings<'a> {
    /// The front silhouette's grid; the back painting uses it mirrored.
    pub front_grid: Option<Grid>,
    pub side_grid: Option<Grid>,
    pub front: Option<Painting<'a>>,
    pub back: Option<Painting<'a>>,
    pub side: Option<Painting<'a>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Front,
    Back,
    /// From -x: the creature's right, as the side silhouette shows.
    Right,
    /// From +x: the side painting again, mirrored.
    Left,
}

impl Look {
    const ALL: [Look; 4] = [Look::Front, Look::Back, Look::Right, Look::Left];

    /// Unit vector toward the viewer.
    fn toward(self) -> V3 {
        match self {
            Look::Front => [0.0, 0.0, 1.0],
            Look::Back => [0.0, 0.0, -1.0],
            Look::Right => [-1.0, 0.0, 0.0],
            Look::Left => [1.0, 0.0, 0.0],
        }
    }

    /// World point -> (painting-space position in the grid's pixels, depth
    /// from the viewer; smaller = nearer).
    fn project(self, g: &Grid, p: V3) -> ([f32; 2], f32) {
        let row = (g.max_y - p[1]) * g.px_per_ft;
        match self {
            Look::Front => ([(p[0] - g.min_x) * g.px_per_ft, row], -p[2]),
            // Seen from behind the creature's left is on the viewer's left:
            // the front grid, mirrored.
            Look::Back => ([g.w as f32 - (p[0] - g.min_x) * g.px_per_ft, row], p[2]),
            // Side views put the creature's +z (its front) at +x on screen.
            Look::Right => ([(p[2] - g.min_x) * g.px_per_ft, row], p[0]),
            Look::Left => ([(p[2] - g.min_x) * g.px_per_ft, row], -p[0]),
        }
    }
}

/// The rest body seen from one side: nearest depth per grid pixel.
struct DepthMap {
    w: usize,
    h: usize,
    z: Vec<f32>,
}

impl DepthMap {
    fn new(p: &Puppet, look: Look, g: &Grid) -> DepthMap {
        let (w, h) = (g.w, g.h);
        let mut z = vec![f32::INFINITY; w * h];
        for s in &p.segments {
            let ring = |st: &crate::puppet::Station| -> Vec<([f32; 2], f32)> {
                (0..AROUND)
                    .map(|j| {
                        let (sn, cs) = (j as f32 / AROUND as f32 * std::f32::consts::TAU).sin_cos();
                        let q = add(st.c, add(scale(s.axis_w, st.w * cs), scale(s.axis_d, st.d * sn)));
                        look.project(g, q)
                    })
                    .collect()
            };
            let rings: Vec<_> = s.stations.iter().map(ring).collect();
            for pair in rings.windows(2) {
                for j in 0..AROUND {
                    let j1 = (j + 1) % AROUND;
                    let (a, b, c, d) = (pair[0][j], pair[0][j1], pair[1][j1], pair[1][j]);
                    for t in [[a, b, c], [a, c, d]] {
                        fill(&mut z, w, h, t);
                    }
                }
            }
        }
        DepthMap { w, h, z }
    }

    /// Nearest depth around a grid position (a 3x3 minimum: a point on a
    /// silhouette's edge is compared against its own surface).
    fn nearest(&self, at: [f32; 2]) -> f32 {
        let (cx, cy) = (at[0].floor() as isize, at[1].floor() as isize);
        let mut best = f32::INFINITY;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
                    best = best.min(self.z[y as usize * self.w + x as usize]);
                }
            }
        }
        best
    }
}

fn fill(z: &mut [f32], w: usize, h: usize, t: [([f32; 2], f32); 3]) {
    let p = t.map(|v| v.0);
    let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
    if area.abs() < 1e-6 || !area.is_finite() {
        return;
    }
    let x0 = p.iter().map(|v| v[0]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let y0 = p.iter().map(|v| v[1]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let x1 = (p.iter().map(|v| v[0]).fold(f32::MIN, f32::max).ceil() as isize).min(w as isize - 1);
    let y1 = (p.iter().map(|v| v[1]).fold(f32::MIN, f32::max).ceil() as isize).min(h as isize - 1);
    if x1 < x0 as isize || y1 < y0 as isize {
        return;
    }
    let edge = |a: [f32; 2], b: [f32; 2], q: [f32; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
    for y in y0..=y1 as usize {
        for x in x0..=x1 as usize {
            let q = [x as f32 + 0.5, y as f32 + 0.5];
            let w0 = edge(p[1], p[2], q) / area;
            let w1 = edge(p[2], p[0], q) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let d = w0 * t[0].1 + w1 * t[1].1 + w2 * t[2].1;
            let i = y * w + x;
            if d < z[i] {
                z[i] = d;
            }
        }
    }
}

impl Painting<'_> {
    /// The painted colour nearest `at` (grid pixels of `g`), within reach.
    fn sample(&self, g: &Grid, at: [f32; 2]) -> Option<[f32; 3]> {
        if self.w == 0 || self.h == 0 || self.rgba.len() < self.w * self.h * 4 {
            return None;
        }
        let (sx, sy) = (self.w as f32 / g.w as f32, self.h as f32 / g.h as f32);
        let (cx, cy) = ((at[0] * sx).floor() as i32, (at[1] * sy).floor() as i32);
        let reach = (REACH_PX as f32 * sx.max(sy)).ceil() as i32;
        let mut best: Option<(i32, [f32; 3])> = None;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let d2 = dx * dx + dy * dy;
                if d2 > reach * reach || best.is_some_and(|(b, _)| b <= d2) {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
                    continue;
                }
                let i = (y as usize * self.w + x as usize) * 4;
                if self.rgba[i + 3] >= 128 {
                    best = Some((d2, [0, 1, 2].map(|k| self.rgba[i + k] as f32)));
                }
            }
        }
        best.map(|(_, c)| c)
    }
}

/// The sheet painted from `paintings`: straight RGBA8, `layout.w` x
/// `layout.h`, transparent off the patches.
pub fn prefill(p: &Puppet, layout: &UvLayout, paintings: &Paintings) -> Vec<u8> {
    let views: Vec<(Look, Grid, Painting, DepthMap)> = Look::ALL
        .iter()
        .filter_map(|look| {
            let (grid, painting) = match look {
                Look::Front => (paintings.front_grid?, paintings.front?),
                Look::Back => (paintings.front_grid?, paintings.back?),
                Look::Right | Look::Left => (paintings.side_grid?, paintings.side?),
            };
            Some((*look, grid, painting, DepthMap::new(p, *look, &grid)))
        })
        .collect();

    let (w, h) = (layout.w, layout.h);
    let mut rgba = vec![0u8; w * h * 4];
    // 0 = off every patch, 1 = on a patch but unseen, 2 = painted.
    let mut state = vec![0u8; w * h];
    let mut owner = vec![usize::MAX; w * h];
    for y in 0..h {
        for x in 0..w {
            let Some((seg, station, theta)) = layout.address([x as f32 + 0.5, y as f32 + 0.5]) else { continue };
            let i = y * w + x;
            owner[i] = seg;
            state[i] = 1;
            let Some((point, normal)) = rest_point(p, seg, station, theta) else { continue };
            let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
            for (look, grid, painting, depth) in &views {
                let facing = dot(normal, look.toward());
                if facing < MIN_FACING {
                    continue;
                }
                let (at, d) = look.project(grid, point);
                if d > depth.nearest(at) + DEPTH_TOL_FT {
                    continue;
                }
                if let Some(c) = painting.sample(grid, at) {
                    let wt = facing.powi(FACING_POWER);
                    for k in 0..3 {
                        sum[k] += c[k] * wt;
                    }
                    weight += wt;
                }
            }
            if weight > 0.0 {
                let o = i * 4;
                rgba[o..o + 3].copy_from_slice(&sum.map(|v| (v / weight).round().clamp(0.0, 255.0) as u8));
                rgba[o + 3] = 255;
                state[i] = 2;
            }
        }
    }
    fill_holes(&mut rgba, &mut state, &owner, w, h);
    rgba
}

/// A surface address on the rest body: the point and its outward normal.
fn rest_point(p: &Puppet, seg: usize, station: f32, theta: f32) -> Option<(V3, V3)> {
    let s = p.segments.get(seg)?;
    let last = s.stations.len().checked_sub(1)?;
    let f = station.clamp(0.0, last as f32);
    let k = (f.floor() as usize).min(last);
    let (k1, t) = if k < last { (k + 1, f - k as f32) } else { (k, 0.0) };
    let (a, b) = (&s.stations[k], &s.stations[k1]);
    let c = add(a.c, scale(crate::math::sub(b.c, a.c), t));
    let (w, d) = (a.w + (b.w - a.w) * t, a.d + (b.d - a.d) * t);
    let (sn, cs) = theta.sin_cos();
    let point = add(c, add(scale(s.axis_w, w * cs), scale(s.axis_d, d * sn)));
    // An ellipse's normal: the radial direction scaled by the reciprocal
    // semi-axes, so a flat cross-section faces the right way.
    let normal = norm_or(add(scale(s.axis_w, cs / w.max(1e-3)), scale(s.axis_d, sn / d.max(1e-3))), [0.0, 0.0, 1.0]);
    Some((point, normal))
}

/// Grow painted pixels into their patch's unseen ones, a ring at a time,
/// never across into another patch.
fn fill_holes(rgba: &mut [u8], state: &mut [u8], owner: &[usize], w: usize, h: usize) {
    loop {
        let mut grew = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if state[i] != 1 {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                    let (qx, qy) = (x as isize + dx, y as isize + dy);
                    if qx < 0 || qy < 0 || qx as usize >= w || qy as usize >= h {
                        continue;
                    }
                    let j = qy as usize * w + qx as usize;
                    if state[j] == 2 && owner[j] == owner[i] {
                        for k in 0..3 {
                            sum[k] += rgba[j * 4 + k] as u32;
                        }
                        n += 1;
                    }
                }
                if n > 0 {
                    grew.push((i, sum.map(|v| (v / n) as u8)));
                }
            }
        }
        if grew.is_empty() {
            return;
        }
        for (i, c) in grew {
            rgba[i * 4..i * 4 + 3].copy_from_slice(&c);
            rgba[i * 4 + 3] = 255;
            state[i] = 2;
        }
    }
}
