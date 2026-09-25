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
/// A surface facing up at least this much that no painting sees (the crown
/// of the head: no view looks down on it) takes what the paintings show at
/// its spot -- the top edge of the head, its hair -- rather than growing
/// the nearest paint (the forehead) over it.
const UP_FACING: f32 = 0.4;
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

    /// A tangent-space normal as a painting over this view stores it (x to
    /// the picture's right, y up, z out of the picture toward the viewer,
    /// the usual convention) in REST world space.
    fn tangent_to_world(self, n: V3) -> V3 {
        match self {
            // The front picture's right is +x, and it faces +z.
            Look::Front => [n[0], n[1], n[2]],
            // Seen from behind, both are mirrored.
            Look::Back => [-n[0], n[1], -n[2]],
            // A side picture's right is the creature's front (+z); seen from
            // its right (-x), out of the picture is -x...
            Look::Right => [-n[2], n[1], n[0]],
            // ...and used again from the left, +x.
            Look::Left => [n[2], n[1], n[0]],
        }
    }

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
        DepthMap::of(p, look, g, &|_| true)
    }

    /// Only the segments `keep` accepts (hidden ones never).
    fn of(p: &Puppet, look: Look, g: &Grid, keep: &dyn Fn(&crate::puppet::Segment) -> bool) -> DepthMap {
        let (w, h) = (g.w, g.h);
        let mut z = vec![f32::INFINITY; w * h];
        for s in p.segments.iter().filter(|s| !s.hidden && keep(s)) {
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
            if weight == 0.0 && normal[1] > UP_FACING {
                for (look, grid, painting, _) in &views {
                    if let Some(c) = painting.sample(grid, look.project(grid, point).0) {
                        for k in 0..3 {
                            sum[k] += c[k];
                        }
                        weight += 1.0;
                    }
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

/// The NORMAL sheet from normal maps painted over the same silhouettes as
/// the paintings (see [`prefill`]): straight RGBA8, `layout.w` x `layout.h`,
/// transparent off the patches and where no map sees. Each map is a
/// tangent-space normal map of its view (RGB = xyz * 0.5 + 0.5, x to the
/// right, y up, z toward the viewer); what the sheet stores per pixel is
/// how far the map's normal, in rest space, leans away from the view's own
/// direction -- the DEVIATION, so a flat map is zero and the skin adds it
/// to the normal its own geometry gives (`Posed::skin_textured`). Views
/// blend as the paintings do, by how squarely each faces the surface.
pub fn prefill_normals(p: &Puppet, layout: &UvLayout, maps: &Paintings) -> Vec<u8> {
    let views: Vec<(Look, Grid, Painting, DepthMap)> = Look::ALL
        .iter()
        .filter_map(|look| {
            let (grid, map) = match look {
                Look::Front => (maps.front_grid?, maps.front?),
                Look::Back => (maps.front_grid?, maps.back?),
                Look::Right | Look::Left => (maps.side_grid?, maps.side?),
            };
            Some((*look, grid, map, DepthMap::new(p, *look, &grid)))
        })
        .collect();

    let (w, h) = (layout.w, layout.h);
    let mut rgba = vec![0u8; w * h * 4];
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
            for (look, grid, map, depth) in &views {
                let facing = dot(normal, look.toward());
                if facing < MIN_FACING {
                    continue;
                }
                let (at, d) = look.project(grid, point);
                if d > depth.nearest(at) + DEPTH_TOL_FT {
                    continue;
                }
                if let Some(c) = map.sample(grid, at) {
                    let tangent = norm_or(c.map(|v| v / 127.5 - 1.0), [0.0, 0.0, 1.0]);
                    let deviation = crate::math::sub(look.tangent_to_world(tangent), look.toward());
                    let wt = facing.powi(FACING_POWER);
                    for k in 0..3 {
                        sum[k] += deviation[k] * wt;
                    }
                    weight += wt;
                }
            }
            if weight > 0.0 {
                let o = i * 4;
                for k in 0..3 {
                    rgba[o + k] = crate::uv::encode_normal(sum[k] / weight);
                }
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

/// A silhouette with the rest puppet's garments ([`crate::outfit`]) added:
/// the outline a dressed creature is painted in. `mask` is the front or the
/// side silhouette (the back template mirrors the front); the grid stays
/// the same, so a painting over the dressed template projects exactly as
/// one over the bare one.
pub fn dressed_silhouette(p: &Puppet, mask: &rig_bake::Mask, side: bool) -> rig_bake::Mask {
    let look = if side { Look::Right } else { Look::Front };
    let worn = DepthMap::of(p, look, &mask.grid, &|s| s.kind.is_garment());
    let inside = mask.inside.iter().zip(&worn.z).map(|(&body, z)| body || z.is_finite()).collect();
    rig_bake::Mask { grid: mask.grid, inside }
}

/// Where the puppet's eyes are in a view's template, in the template's
/// pixels: `[x, y, radius]` for each eye that faces that view. A painting
/// should put the creature's eyes there -- on the stage a painted skin
/// wears no eye dots, so the face is wherever the painting put it.
pub fn puppet_eyes(p: &Puppet, grid: &Grid, view: PaintView) -> Vec<[f32; 3]> {
    let look = view.look();
    p.eyes
        .iter()
        .filter(|e| dot(e.normal, look.toward()) > 0.2)
        .map(|e| {
            let (at, _) = look.project(grid, e.pos);
            [at[0], at[1], e.radius * grid.px_per_ft]
        })
        .collect()
}

/// The three paint-over templates' views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintView {
    Front,
    /// The front silhouette mirrored, as seen from behind.
    Back,
    /// The side silhouette: the creature's right, facing the image's right.
    Side,
}

impl PaintView {
    fn look(self) -> Look {
        match self {
            PaintView::Front => Look::Front,
            PaintView::Back => Look::Back,
            PaintView::Side => Look::Right,
        }
    }
}

/// Grey of the farthest body surface in a depth guide (the nearest is 255,
/// outside the silhouette 0), so the body's far edge stays clear of the
/// background.
const GUIDE_FAR: f32 = 64.0;
/// A gap in the silhouette this many template pixels wide or less, with
/// body on both sides, is a seam of the outline (the humanoid's side view
/// draws its arm as its own shape a sliver off the body), not a gap in the
/// body: a depth guide fills it rather than drawing it as an edge.
const GUIDE_SEAM_PX: usize = 4;
/// ...and so is a hole in it, enclosed by body, of this many template
/// pixels or fewer (where the seam wanders around a hand).
const GUIDE_HOLE_PX: usize = 24;
/// The depth guide's blur radius, in template pixels.
const GUIDE_BLUR_PX: usize = 2;

/// A depth guide for an image generator (a ControlNet depth input): the REST
/// body seen from `view`, in exactly the frame of that view's paint-over
/// template -- `mask` is the silhouette it is cut from (the front one for
/// the back view) -- at `scale` times the template's size.
///
/// Nearest surface white, farthest [`GUIDE_FAR`] grey, outside the
/// silhouette black: the convention MiDaS-style depth models are trained on.
/// Silhouette the puppet does not reach (a fingertip past the end of its
/// limb) takes the nearest body depth, so the guide covers everything the
/// painting must; hairline seams of the outline are closed
/// ([`GUIDE_SEAM_PX`]); and a light blur softens where tubes meet.
/// Opaque RGBA8, `mask.grid.w * scale` x `mask.grid.h * scale`.
pub fn depth_guide(p: &Puppet, mask: &rig_bake::Mask, view: PaintView, scale: usize) -> Vec<u8> {
    let s = scale.max(1);
    let t = mask.grid;
    let g = Grid { w: t.w * s, h: t.h * s, px_per_ft: t.px_per_ft * s as f32, ..t };
    let (w, h) = (g.w, g.h);
    let body = seams_closed(mask);
    let inside: Vec<bool> = (0..w * h)
        .map(|i| {
            let (col, row) = (i % w / s, i / w / s);
            let col = if view == PaintView::Back { t.w - 1 - col } else { col };
            body[row * t.w + col]
        })
        .collect();
    let mut z = DepthMap::new(p, view.look(), &g).z;
    for (d, &ins) in z.iter_mut().zip(&inside) {
        if !ins {
            *d = f32::INFINITY;
        }
    }
    grow_depth(&mut z, &inside, w, h);
    blur_inside(&mut z, &inside, w, h, GUIDE_BLUR_PX * s);

    let (near, far) = z
        .iter()
        .filter(|d| d.is_finite())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &d| (a.min(d), b.max(d)));
    let span = (far - near).max(1e-4);
    let mut rgba = vec![0u8; w * h * 4];
    for (px, &d) in rgba.chunks_exact_mut(4).zip(&z) {
        let v = if d.is_finite() { GUIDE_FAR + (255.0 - GUIDE_FAR) * (far - d) / span } else { 0.0 };
        let v = v.round().clamp(0.0, 255.0) as u8;
        px.copy_from_slice(&[v, v, v, 255]);
    }
    rgba
}

/// The silhouette with its hairline seams filled: every run of outside
/// pixels at most [`GUIDE_SEAM_PX`] long with body at both ends, across or
/// down (whole runs, so a gap is either closed or left as it was), and every
/// enclosed hole of at most [`GUIDE_HOLE_PX`] pixels.
fn seams_closed(mask: &rig_bake::Mask) -> Vec<bool> {
    let (w, h) = (mask.grid.w, mask.grid.h);
    let mut body = mask.inside.clone();
    for (len, step, lines, line_step) in [(w, 1, h, w), (h, w, w, 1)] {
        for line in 0..lines {
            let at = |k: usize| line * line_step + k * step;
            let mut k = 0;
            while k < len {
                if mask.inside[at(k)] {
                    k += 1;
                    continue;
                }
                let start = k;
                while k < len && !mask.inside[at(k)] {
                    k += 1;
                }
                if start > 0 && k < len && k - start <= GUIDE_SEAM_PX {
                    (start..k).for_each(|j| body[at(j)] = true);
                }
            }
        }
    }
    // Holes: outside pixels the border cannot reach, a component at a time.
    let mut seen = vec![false; w * h];
    for first in 0..w * h {
        if body[first] || seen[first] {
            continue;
        }
        let (mut component, mut stack, mut open) = (Vec::new(), vec![first], false);
        seen[first] = true;
        while let Some(i) = stack.pop() {
            component.push(i);
            let (x, y) = (i % w, i / w);
            open |= x == 0 || y == 0 || x == w - 1 || y == h - 1;
            let next = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in next.into_iter().flatten() {
                if !body[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if !open && component.len() <= GUIDE_HOLE_PX {
            component.into_iter().for_each(|i| body[i] = true);
        }
    }
    body
}

/// Silhouette pixels no surface reached take the mean of their reached
/// neighbours, a ring at a time.
fn grow_depth(z: &mut [f32], inside: &[bool], w: usize, h: usize) {
    loop {
        let mut grew = Vec::new();
        for i in 0..w * h {
            if !inside[i] || z[i].is_finite() {
                continue;
            }
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let (mut sum, mut n) = (0.0f32, 0u32);
            for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                let (qx, qy) = (x + dx, y + dy);
                if qx >= 0 && qy >= 0 && (qx as usize) < w && (qy as usize) < h {
                    let d = z[qy as usize * w + qx as usize];
                    if d.is_finite() {
                        sum += d;
                        n += 1;
                    }
                }
            }
            if n > 0 {
                grew.push((i, sum / n as f32));
            }
        }
        if grew.is_empty() {
            return;
        }
        for (i, d) in grew {
            z[i] = d;
        }
    }
}

/// A box blur of radius `r` over the silhouette only (the background never
/// bleeds in), horizontal then vertical.
fn blur_inside(z: &mut [f32], inside: &[bool], w: usize, h: usize, r: usize) {
    let r = r as isize;
    for (len, step, lines, line_step) in [(w, 1, h, w), (h, w, w, 1)] {
        let src = z.to_vec();
        for line in 0..lines {
            for k in 0..len {
                let i = line * line_step + k * step;
                if !inside[i] || !src[i].is_finite() {
                    continue;
                }
                let (mut sum, mut n) = (0.0f32, 0.0f32);
                for o in -r..=r {
                    let q = k as isize + o;
                    if q < 0 || q as usize >= len {
                        continue;
                    }
                    let j = line * line_step + q as usize * step;
                    if inside[j] && src[j].is_finite() {
                        sum += src[j];
                        n += 1.0;
                    }
                }
                z[i] = sum / n;
            }
        }
    }
}

/// The rest body seen from one side: per pixel, the nearest surface's
/// depth, and the point and outward normal there.
struct SurfaceMap {
    z: Vec<f32>,
    pos: Vec<V3>,
    nrm: Vec<V3>,
}

impl SurfaceMap {
    fn new(p: &Puppet, look: Look, g: &Grid) -> SurfaceMap {
        let (w, h) = (g.w, g.h);
        let mut map = SurfaceMap { z: vec![f32::INFINITY; w * h], pos: vec![[0.0; 3]; w * h], nrm: vec![[0.0; 3]; w * h] };
        for s in p.segments.iter().filter(|s| !s.hidden) {
            let ring = |st: &crate::puppet::Station| -> Vec<Corner> {
                (0..AROUND)
                    .map(|j| {
                        let (sn, cs) = (j as f32 / AROUND as f32 * std::f32::consts::TAU).sin_cos();
                        let q = add(st.c, add(scale(s.axis_w, st.w * cs), scale(s.axis_d, st.d * sn)));
                        let n = norm_or(add(scale(s.axis_w, cs / st.w.max(1e-3)), scale(s.axis_d, sn / st.d.max(1e-3))), [0.0, 0.0, 1.0]);
                        let (at, d) = look.project(g, q);
                        Corner { at, d, q, n }
                    })
                    .collect()
            };
            let rings: Vec<_> = s.stations.iter().map(ring).collect();
            for pair in rings.windows(2) {
                for j in 0..AROUND {
                    let j1 = (j + 1) % AROUND;
                    let (a, b, c, d) = (pair[0][j], pair[0][j1], pair[1][j1], pair[1][j]);
                    for t in [[a, b, c], [a, c, d]] {
                        map.fill(w, h, t);
                    }
                }
            }
        }
        map
    }

    fn fill(&mut self, w: usize, h: usize, t: [Corner; 3]) {
        let p = t.map(|v| v.at);
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
                let d = w0 * t[0].d + w1 * t[1].d + w2 * t[2].d;
                let i = y * w + x;
                if d < self.z[i] {
                    self.z[i] = d;
                    let mix = |f: &dyn Fn(&Corner) -> V3| add(add(scale(f(&t[0]), w0), scale(f(&t[1]), w1)), scale(f(&t[2]), w2));
                    self.pos[i] = mix(&|c| c.q);
                    self.nrm[i] = norm_or(mix(&|c| c.n), [0.0, 0.0, 1.0]);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Corner {
    at: [f32; 2],
    d: f32,
    q: V3,
    n: V3,
}

/// Below this facing a view's paint counts as a smear, not paint (see
/// [`project_view`]).
pub const GRAZING: f32 = 0.35;

/// Projection-first (`docs/projection-first-export-spec.md`): what the
/// paintings made so far show of the REST body seen from `view`, in exactly
/// that view's template frame (`mask` is its silhouette -- the front one for
/// the back view) at `scale` times its size. Returns `(render, unpainted)`,
/// both RGBA8:
///
/// - `render`: the paint as [`prefill`] would put it there -- each painting
///   that sees the point, depth-tested, blended by how squarely it faces
///   it -- unlit, no eyes, and no hole filling: transparent wherever it is
///   unpainted and outside the silhouette;
/// - `unpainted`: white where the silhouette shows surface no painting
///   reached, or reached only at a grazing angle (the best view's facing
///   below `grazing`, [`GRAZING`] to start -- a front painting smeared
///   along the flank), black elsewhere.
///
/// A generator then inpaints only the white, so every view agrees with the
/// ones painted before it.
pub fn project_view(p: &Puppet, mask: &rig_bake::Mask, view: PaintView, scale: usize, paintings: &Paintings, grazing: f32) -> (Vec<u8>, Vec<u8>) {
    let s = scale.max(1);
    let t = mask.grid;
    let g = Grid { w: t.w * s, h: t.h * s, px_per_ft: t.px_per_ft * s as f32, ..t };
    let (w, h) = (g.w, g.h);
    let sources: Vec<(Look, Grid, Painting, DepthMap)> = Look::ALL
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
    let surface = SurfaceMap::new(p, view.look(), &g);
    let mut render = vec![0u8; w * h * 4];
    let mut unpainted = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let (col, row) = (i % w / s, i / w / s);
        let col = if view == PaintView::Back { t.w - 1 - col } else { col };
        unpainted[i * 4 + 3] = 255;
        if !mask.inside[row * t.w + col] {
            continue;
        }
        let mut colour = None;
        if surface.z[i].is_finite() {
            let (point, normal) = (surface.pos[i], surface.nrm[i]);
            let (mut sum, mut weight, mut best) = ([0.0f32; 3], 0.0f32, 0.0f32);
            for (look, grid, painting, depth) in &sources {
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
                    best = best.max(facing);
                }
            }
            if weight > 0.0 && best >= grazing {
                colour = Some(sum.map(|v| (v / weight).round().clamp(0.0, 255.0) as u8));
            }
        }
        match colour {
            Some(c) => {
                render[i * 4..i * 4 + 3].copy_from_slice(&c);
                render[i * 4 + 3] = 255;
            }
            None => unpainted[i * 4..i * 4 + 3].copy_from_slice(&[255, 255, 255]),
        }
    }
    (render, unpainted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MASK_PX;

    fn humanoid() -> (Puppet, rig_bake::Mask, rig_bake::Mask) {
        let data = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data");
        let text = std::fs::read_to_string(format!("{data}/rigs/humanoid.puppet.toml")).unwrap();
        let outlines = rig::Outlines::load(std::path::Path::new(&format!("{data}/outlines.json"))).unwrap();
        let mask = |v| rig_bake::rasterize(&outlines.get("humanoid", v).unwrap().outline(), MASK_PX);
        (Puppet::from_baked_toml(&text).unwrap(), mask(rig::View::Front), mask(rig::View::Side))
    }

    /// Each guide is its template scaled up: every silhouette pixel is body
    /// (at least the far grey), everything else black but the outline's
    /// hairline seams, and the nearest surface white. The back is the
    /// front's silhouette mirrored.
    #[test]
    fn a_depth_guide_fills_its_template_exactly() {
        let (p, front, side) = humanoid();
        for (mask, view) in [(&front, PaintView::Front), (&front, PaintView::Back), (&side, PaintView::Side)] {
            let closed = seams_closed(mask);
            let s = 2;
            let (tw, w) = (mask.grid.w, mask.grid.w * s);
            let rgba = depth_guide(&p, mask, view, s);
            assert_eq!(rgba.len(), w * mask.grid.h * s * 4, "{view:?}: the template's size, scaled");
            let mut brightest = 0u8;
            for (i, px) in rgba.chunks_exact(4).enumerate() {
                let (col, row) = (i % w / s, i / w / s);
                let col = if view == PaintView::Back { tw - 1 - col } else { col };
                assert_eq!(px[3], 255, "{view:?}: opaque");
                if mask.inside[row * tw + col] {
                    assert!(px[0] >= GUIDE_FAR as u8, "{view:?}: silhouette pixel {i} left dark ({})", px[0]);
                } else if !closed[row * tw + col] {
                    assert_eq!(px[0], 0, "{view:?}: pixel {i} outside the silhouette is not black");
                }
                brightest = brightest.max(px[0]);
            }
            assert_eq!(brightest, 255, "{view:?}: the nearest surface is white");
        }
    }

    /// The humanoid's side outline draws the arm a sliver off the body: the
    /// guide shows no edge there, and still keeps real gaps (between the
    /// legs from the front) black.
    #[test]
    fn a_depth_guide_closes_the_outlines_seams() {
        let (p, front, side) = humanoid();
        let seam = |m: &rig_bake::Mask| (0..m.inside.len()).filter(|&i| !m.inside[i] && seams_closed(m)[i]).count();
        assert!(seam(&side) > 300, "the side outline has its arm seam: {}", seam(&side));
        let rgba = depth_guide(&p, &side, PaintView::Side, 1);
        let closed = seams_closed(&side);
        for i in (0..side.inside.len()).filter(|&i| !side.inside[i] && closed[i]) {
            assert!(rgba[i * 4] >= GUIDE_FAR as u8, "seam pixel {i} drawn as background");
        }
        // From the front, the gap between the legs stays open.
        let g = front.grid;
        let row = g.h * 9 / 10;
        let centre = ((0.0 - g.min_x) * g.px_per_ft) as usize;
        let f = depth_guide(&p, &front, PaintView::Front, 1);
        assert_eq!(f[(row * g.w + centre) * 4], 0, "between the shins is background");
    }

    /// Near is bright: from the side the face is out ahead of the back of
    /// the head, so the side guide's head is brightest at its front edge.
    #[test]
    fn near_is_bright() {
        let (p, _, side) = humanoid();
        let rgba = depth_guide(&p, &side, PaintView::Side, 1);
        let g = side.grid;
        // A row through the head: its first and last silhouette pixels are
        // the back of the head and the face (the side view faces +x).
        let top = (0..g.h).find(|&row| (0..g.w).any(|c| side.inside[row * g.w + c])).unwrap();
        let row = top + g.h / 20;
        let cols: Vec<usize> = (0..g.w).filter(|&c| side.inside[row * g.w + c]).collect();
        let at = |c: usize| rgba[(row * g.w + c) * 4];
        let (back, face) = (at(cols[0]), at(*cols.last().unwrap()));
        // Seen from the creature's right, both edges of the head are about
        // equally near: what differs is front vs middle of the view axis.
        // So compare the head's middle (nearest, the cheek) with its rim.
        let mid = at(cols[cols.len() / 2]);
        assert!(mid > back && mid > face, "the cheek ({mid}) is nearer than the head's rim ({back}, {face})");
    }

    /// Projection-first: a front painted solid red shows red on whatever of
    /// the side view faces forward, leaves the flank (grazing) and the back
    /// unpainted, and never paints outside the silhouette.
    #[test]
    fn projecting_the_front_paints_what_it_saw_and_marks_the_rest() {
        let (p, front, side) = humanoid();
        let red: Vec<u8> = (0..front.grid.w * front.grid.h).flat_map(|i| if front.inside[i] { [210, 30, 30, 255] } else { [0; 4] }).collect();
        let paintings = Paintings {
            front_grid: Some(front.grid),
            side_grid: Some(side.grid),
            front: Some(Painting { rgba: &red, w: front.grid.w, h: front.grid.h }),
            ..Default::default()
        };
        let (render, unpainted) = project_view(&p, &side, PaintView::Side, 1, &paintings, GRAZING);
        let g = side.grid;
        let (mut painted, mut blank) = (0, 0);
        for i in 0..g.w * g.h {
            let (r, m) = (&render[i * 4..i * 4 + 4], unpainted[i * 4]);
            if !side.inside[i] {
                assert!(r[3] == 0 && m == 0, "pixel {i} outside the silhouette is neither painted nor to paint");
                continue;
            }
            if r[3] == 255 {
                assert!(r[0] > 150 && r[1] < 80 && m == 0, "painted pixels are the front's red and not to paint");
                painted += 1;
            } else {
                assert_eq!(m, 255, "an unpainted silhouette pixel is marked to paint");
                blank += 1;
            }
        }
        assert!(painted > 0 && blank > 0, "some of the side is the front's paint, some is left: {painted} / {blank}");
        // The front half of the side view (the creature faces +x there) is
        // where the paint landed.
        let centre = |want: bool| {
            let cols: Vec<usize> = (0..g.w * g.h).filter(|&i| side.inside[i] && (render[i * 4 + 3] == 255) == want).map(|i| i % g.w).collect();
            cols.iter().sum::<usize>() as f32 / cols.len() as f32
        };
        assert!(centre(true) > centre(false), "the paint is toward the front of the side view");
        // Nothing painted the back.
        let (back, back_blank) = project_view(&p, &front, PaintView::Back, 1, &paintings, GRAZING);
        let seen = back.chunks_exact(4).filter(|px| px[3] == 255).count();
        let inside = front.inside.iter().filter(|&&b| b).count();
        assert!(seen * 20 < inside, "the front painting barely reaches the back: {seen} of {inside}");
        assert!(back_blank.chunks_exact(4).filter(|px| px[0] == 255).count() * 10 > inside * 9);
    }
}
