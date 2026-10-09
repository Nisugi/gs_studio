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
//! The side is painted over the figure with any arm drawn raised HUNG
//! ([`Puppet::with_arms_hung`], [`side_silhouette`]): from the side an
//! A-pose's arm lines up with the body and its sheet shows no arm, but a
//! painter draws one hanging down the flank anyway -- and projected through
//! the raised arm, that arm and its hand landed on the hip and thigh. So
//! the side view's template, depth guide and projection all use the hung
//! figure; the front and back keep the figure as drawn.
//!
//! What no painting sees -- an armpit, the inside of a thigh, a neck hidden
//! under the head -- takes the colour of the painted surface nearest it on
//! the body, in 3D and across patches ([`fill_unseen`]), so the sheet has
//! no holes. A pixel off every patch stays transparent.

use std::borrow::Cow;

use rig_bake::Grid;

use crate::math::{add, dot, mul_mv, norm_or, scale, transpose, Xf, V3};
use crate::model::Puppet;
use crate::uv::{rest_point, UvLayout};

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
/// the nearest painted pixel within reach is taken. For marking what is
/// left to paint ([`project_view`]).
const REACH_PX: i32 = 6;
/// ...and for the sheet ([`prefill`]) only this many: a miss there is
/// paint the painting does not have -- a part painted narrower than its
/// template, a background pixel cut out -- and taking its nearest pixel
/// stretched one edge column (a grey band over the back of the head)
/// across the band that projects past it. Seen squarely nearby, the
/// surface's own paint fills it instead ([`fill_unseen`]).
const SHEET_REACH_PX: i32 = 1;
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

    fn is_side(self) -> bool {
        matches!(self, Look::Right | Look::Left)
    }
}

/// The bodies the views are painted over: the figure as drawn for the front
/// and the back, and from the side the same with any raised arm hung
/// ([`Puppet::with_arms_hung`]), with each segment's move from the one to
/// the other. The two share their segments and stations, so one surface
/// address is a point on each.
struct Bodies<'a> {
    rest: &'a Puppet,
    side: Option<(Puppet, Vec<Xf>)>,
}

impl<'a> Bodies<'a> {
    fn new(p: &'a Puppet) -> Bodies<'a> {
        Bodies { rest: p, side: p.with_arms_hung() }
    }

    /// The body `look` sees.
    fn of(&self, look: Look) -> &Puppet {
        match &self.side {
            Some((hung, _)) if look.is_side() => hung,
            _ => self.rest,
        }
    }

    /// A direction on `look`'s body, turned back into rest space.
    fn to_rest(&self, look: Look, segment: usize, v: V3) -> V3 {
        match &self.side {
            Some((_, moves)) if look.is_side() => mul_mv(&transpose(&moves[segment].r), v),
            _ => v,
        }
    }
}

/// The body `look` sees, alone (see [`Bodies`]).
fn body_for(p: &Puppet, look: Look) -> Cow<'_, Puppet> {
    match p.with_arms_hung() {
        Some((hung, _)) if look.is_side() => Cow::Owned(hung),
        _ => Cow::Borrowed(p),
    }
}

/// A body seen from one side: nearest depth per grid pixel.
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
    fn of(p: &Puppet, look: Look, g: &Grid, keep: &dyn Fn(&crate::model::Segment) -> bool) -> DepthMap {
        let (w, h) = (g.w, g.h);
        let mut z = vec![f32::INFINITY; w * h];
        for s in p.segments.iter().filter(|s| !s.hidden && keep(s)) {
            let ring = |st: &crate::model::Station| -> Vec<([f32; 2], f32)> {
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
        self.sample_within(g, at, REACH_PX)
    }

    /// [`Painting::sample`] within `reach_px` (the grid's pixels).
    fn sample_within(&self, g: &Grid, at: [f32; 2], reach_px: i32) -> Option<[f32; 3]> {
        if self.w == 0 || self.h == 0 || self.rgba.len() < self.w * self.h * 4 {
            return None;
        }
        let (sx, sy) = (self.w as f32 / g.w as f32, self.h as f32 / g.h as f32);
        let (cx, cy) = ((at[0] * sx).floor() as i32, (at[1] * sy).floor() as i32);
        let reach = (reach_px as f32 * sx.max(sy)).ceil() as i32;
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
    let bodies = Bodies::new(p);
    let views: Vec<(Look, Grid, Painting, DepthMap)> = Look::ALL
        .iter()
        .filter_map(|look| {
            let (grid, painting) = match look {
                Look::Front => (paintings.front_grid?, paintings.front?),
                Look::Back => (paintings.front_grid?, paintings.back?),
                Look::Right | Look::Left => (paintings.side_grid?, paintings.side?),
            };
            Some((*look, grid, painting, DepthMap::new(bodies.of(*look), *look, &grid)))
        })
        .collect();

    let (w, h) = (layout.w, layout.h);
    let mut rgba = vec![0u8; w * h * 4];
    // 0 = off every patch, 1 = on a patch but unseen, 2 = painted.
    let mut state = vec![0u8; w * h];
    let mut owner = vec![usize::MAX; w * h];
    let mut points: Vec<Option<V3>> = vec![None; w * h];
    for y in 0..h {
        for x in 0..w {
            let Some((seg, station, theta)) = layout.address([x as f32 + 0.5, y as f32 + 0.5]) else { continue };
            let i = y * w + x;
            owner[i] = seg;
            state[i] = 1;
            let Some(rest) = rest_point(p, seg, station, theta) else { continue };
            let (point, normal) = rest;
            points[i] = Some(point);
            // the same address on the side's body (arms hung)
            let side = bodies.side.as_ref().and_then(|(hung, _)| rest_point(hung, seg, station, theta)).unwrap_or(rest);
            let on = |look: &Look| if look.is_side() { side } else { rest };
            let (mut sum, mut weight, mut best) = ([0.0f32; 3], 0.0f32, 0.0f32);
            for (look, grid, painting, depth) in &views {
                let (point, normal) = on(look);
                let facing = dot(normal, look.toward());
                if facing < MIN_FACING {
                    continue;
                }
                let (at, d) = look.project(grid, point);
                if d > depth.nearest(at) + DEPTH_TOL_FT {
                    continue;
                }
                if let Some(c) = painting.sample_within(grid, at, SHEET_REACH_PX) {
                    let wt = facing.powi(FACING_POWER);
                    for k in 0..3 {
                        sum[k] += c[k] * wt;
                    }
                    weight += wt;
                    best = best.max(facing);
                }
            }
            // Seen only at a grazing angle, a few of the painting's pixels
            // would be smeared over the whole surface -- its edge, where a
            // painter's background or a part painted narrower than the
            // template shows (a grey band over the back of the head, grey
            // down the inside of a leg). Paint seen squarely nearby fills it
            // instead ([`fill_unseen`]).
            if best < GRAZING {
                (sum, weight) = ([0.0; 3], 0.0);
            }
            if weight == 0.0 && normal[1] > UP_FACING {
                for (look, grid, painting, _) in &views {
                    if let Some(c) = painting.sample(grid, look.project(grid, on(look).0).0) {
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
    fill_unseen(&mut rgba, &mut state, &points, &owner, w, h);
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
    let bodies = Bodies::new(p);
    let views: Vec<(Look, Grid, Painting, DepthMap)> = Look::ALL
        .iter()
        .filter_map(|look| {
            let (grid, map) = match look {
                Look::Front => (maps.front_grid?, maps.front?),
                Look::Back => (maps.front_grid?, maps.back?),
                Look::Right | Look::Left => (maps.side_grid?, maps.side?),
            };
            Some((*look, grid, map, DepthMap::new(bodies.of(*look), *look, &grid)))
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
            let Some(rest) = rest_point(p, seg, station, theta) else { continue };
            let side = bodies.side.as_ref().and_then(|(hung, _)| rest_point(hung, seg, station, theta)).unwrap_or(rest);
            let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
            for (look, grid, map, depth) in &views {
                let (point, normal) = if look.is_side() { side } else { rest };
                let facing = dot(normal, look.toward());
                if facing < MIN_FACING {
                    continue;
                }
                let (at, d) = look.project(grid, point);
                if d > depth.nearest(at) + DEPTH_TOL_FT {
                    continue;
                }
                if let Some(c) = map.sample_within(grid, at, SHEET_REACH_PX) {
                    let tangent = norm_or(c.map(|v| v / 127.5 - 1.0), [0.0, 0.0, 1.0]);
                    // on the side's body (a hung arm turned from rest): back to rest
                    let deviation = bodies.to_rest(*look, seg, crate::math::sub(look.tangent_to_world(tangent), look.toward()));
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


/// Colour the unseen pixels from the painted surface nearest them ON THE
/// BODY, not on the sheet: every painted pixel is binned by its rest point
/// into cubes of [`FILL_LEVELS`] sizes, and an unseen one takes the
/// distance-weighted mean colour of the cubes round it at the finest size
/// that has any. Near paint that is the colour just beside it -- across a
/// patch's edge too, so a neck no view sees takes the jaw's and the
/// chest's -- and far from any, the broad mean of the skin round it, where
/// growing across the sheet drew streaks of whatever edge was nearest.
/// A few smoothing passes over the filled pixels soften the steps where one
/// cube size hands over to the next. What this leaves (a pixel with no
/// surface point) is for [`fill_holes`].
fn fill_unseen(rgba: &mut [u8], state: &mut [u8], points: &[Option<V3>], owner: &[usize], w: usize, h: usize) {
    #[derive(Default, Clone, Copy)]
    struct Cube {
        rgb: [f32; 3],
        at: V3,
        n: f32,
    }
    let cube_of = |p: V3, size: f32| -> [i32; 3] { p.map(|v| (v / size).floor() as i32) };
    // 21 bits a coordinate: a million cubes of the finest size each way.
    let key = |c: [i32; 3]| -> u64 { c.iter().fold(0u64, |k, &v| (k << 21) | (v as u32 as u64 & 0x1f_ffff)) };
    let size = |level: usize| FILL_CELL_FT * (1u32 << level) as f32;
    let mut levels: Vec<Cubes<Cube>> = (0..FILL_LEVELS).map(|_| Cubes::default()).collect();
    for i in 0..w * h {
        let (2, Some(p)) = (state[i], points[i]) else { continue };
        let c = [rgba[i * 4] as f32, rgba[i * 4 + 1] as f32, rgba[i * 4 + 2] as f32];
        for (level, cubes) in levels.iter_mut().enumerate() {
            let q = cubes.entry(key(cube_of(p, size(level)))).or_default();
            q.rgb = std::array::from_fn(|k| q.rgb[k] + c[k]);
            q.at = add(q.at, p);
            q.n += 1.0;
        }
    }
    let mut filled = Vec::new();
    for i in 0..w * h {
        let (1, Some(p)) = (state[i], points[i]) else { continue };
        for (level, cubes) in levels.iter().enumerate() {
            let s = size(level);
            let c = cube_of(p, s);
            let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
            for d in 0..27i32 {
                let at = [c[0] + d % 3 - 1, c[1] + d / 3 % 3 - 1, c[2] + d / 9 - 1];
                let Some(q) = cubes.get(&key(at)) else { continue };
                let far = crate::math::len(crate::math::sub(scale(q.at, 1.0 / q.n), p)) / s;
                let wt = q.n.sqrt() * (-far * far).exp();
                for k in 0..3 {
                    sum[k] += q.rgb[k] / q.n * wt;
                }
                weight += wt;
            }
            if weight > 0.0 {
                rgba[i * 4..i * 4 + 3].copy_from_slice(&sum.map(|v| (v / weight).round().clamp(0.0, 255.0) as u8));
                rgba[i * 4 + 3] = 255;
                filled.push(i);
                break;
            }
        }
    }
    for &i in &filled {
        state[i] = 2;
    }
    for _ in 0..FILL_SMOOTH_PASSES {
        let before = rgba.to_vec();
        for &i in &filled {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let (mut sum, mut n) = ([0u32; 3], 0u32);
            for (dx, dy) in [(0isize, 0isize), (-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (qx, qy) = (x + dx, y + dy);
                if qx < 0 || qy < 0 || qx as usize >= w || qy as usize >= h {
                    continue;
                }
                let j = qy as usize * w + qx as usize;
                if state[j] == 2 && owner[j] == owner[i] {
                    for k in 0..3 {
                        sum[k] += before[j * 4 + k] as u32;
                    }
                    n += 1;
                }
            }
            for k in 0..3 {
                rgba[i * 4 + k] = (sum[k] / n) as u8;
            }
        }
    }
}

/// The finest cube of [`fill_unseen`], feet; each coarser size doubles it.
const FILL_CELL_FT: f32 = 0.04;
/// Cube sizes of [`fill_unseen`]: the coarsest is 0.04 x 2^7 = 5.12 ft,
/// wider than any patch of skin no painting sees.
const FILL_LEVELS: usize = 8;
/// Smoothing passes over the pixels [`fill_unseen`] coloured.
const FILL_SMOOTH_PASSES: usize = 6;

/// A map keyed by a packed cube coordinate. The standard hasher is built to
/// resist hostile keys and is slow for the million lookups a sheet makes;
/// this mixes the key with splitmix64's finaliser, enough for the low bits
/// hashbrown indexes by to depend on every coordinate.
type Cubes<V> = std::collections::HashMap<u64, V, std::hash::BuildHasherDefault<CubeHasher>>;

#[derive(Default)]
struct CubeHasher(u64);

impl std::hash::Hasher for CubeHasher {
    fn finish(&self) -> u64 {
        let mut z = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | b as u64;
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }
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
    let worn = DepthMap::of(&body_for(p, look), look, &mask.grid, &|s| s.kind.is_garment());
    let inside = mask.inside.iter().zip(&worn.z).map(|(&body, z)| body || z.is_finite()).collect();
    rig_bake::Mask { grid: mask.grid, inside }
}

/// The side silhouette the side view is painted over: the art's, with the
/// arms as they hang ([`Puppet::with_arms_hung`]) added where they reach
/// past it. From the side an A-pose's arm lines up with the body, so the
/// sheet shows none; the side is painted, and projected, as if it hung.
/// The mask itself when no arm hangs. The grid stays the same.
pub fn side_silhouette(p: &Puppet, mask: &rig_bake::Mask) -> rig_bake::Mask {
    let Some((hung, _)) = p.with_arms_hung() else { return mask.clone() };
    let arms: std::collections::HashSet<&str> = p.arm_hang.iter().flat_map(|(a, _)| p.subtree(*a)).map(|i| p.segments[i].name.as_str()).collect();
    let arm = DepthMap::of(&hung, Look::Right, &mask.grid, &|s| arms.contains(s.name.as_str()));
    let inside = mask.inside.iter().zip(&arm.z).map(|(&body, z)| body || z.is_finite()).collect();
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
/// body seen from `view` (from the side, with any raised arm hung: see the
/// module notes), in exactly the frame of that view's paint-over template
/// -- `mask` is the silhouette it is cut from (the front one for the back
/// view; for the side, [`side_silhouette`]) -- at `scale` times the
/// template's size.
///
/// Nearest surface white, farthest [`GUIDE_FAR`] grey, outside the
/// silhouette black: the convention MiDaS-style depth models are trained on.
/// Silhouette the puppet does not reach (a fingertip past the end of its
/// limb) takes the nearest body depth, so the guide covers everything the
/// painting must; hairline seams of the outline are closed
/// ([`GUIDE_SEAM_PX`]); and a light blur softens where tubes meet.
/// Opaque RGBA8, `mask.grid.w * scale` x `mask.grid.h * scale`.
pub fn depth_guide(p: &Puppet, mask: &rig_bake::Mask, view: PaintView, scale: usize) -> Vec<u8> {
    let (g, _, z) = guide_depth(p, mask, view, scale);
    let (w, h) = (g.w, g.h);
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

/// The view's body's depth in feet behind every pixel of a guide `scale`
/// times `view`'s template (infinite outside the silhouette), grown to the
/// silhouette's edge and lightly blurred, as [`depth_guide`] draws it: the
/// guide's grid, which pixels are inside, and the depths.
fn guide_depth(p: &Puppet, mask: &rig_bake::Mask, view: PaintView, scale: usize) -> (Grid, Vec<bool>, Vec<f32>) {
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
    let mut z = DepthMap::new(&body_for(p, view.look()), view.look(), &g).z;
    for (d, &ins) in z.iter_mut().zip(&inside) {
        if !ins {
            *d = f32::INFINITY;
        }
    }
    grow_depth(&mut z, &inside, w, h);
    blur_inside(&mut z, &inside, w, h, GUIDE_BLUR_PX * s);
    (g, inside, z)
}

/// A normal guide, [`depth_guide`]'s companion: which way the view's body
/// faces at every pixel of `view`'s template (`scale` times its size),
/// from the depth in feet, in the view's frame -- x right, y up, z toward
/// the viewer -- packed as a normal map (`(n + 1) / 2` in RGB), alpha 255
/// on the body and 0 outside. What a painting's own light can be read
/// against: the paint's brightness over the body's facing is the light the
/// painter put in.
pub fn normal_guide(p: &Puppet, mask: &rig_bake::Mask, view: PaintView, scale: usize) -> Vec<u8> {
    let (g, inside, z) = guide_depth(p, mask, view, scale);
    let (w, h) = (g.w, g.h);
    // a neighbour's depth, or this pixel's own past the body's edge
    let at = |x: usize, y: usize, dx: isize, dy: isize| -> f32 {
        let (nx, ny) = (x as isize + dx, y as isize + dy);
        if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h || !inside[ny as usize * w + nx as usize] {
            return z[y * w + x];
        }
        z[ny as usize * w + nx as usize]
    };
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !inside[i] || !z[i].is_finite() {
                continue;
            }
            // depth grows away from the viewer: a surface turning away to
            // the right deepens to the right, so it faces right; the image's
            // rows run down, the frame's y up
            let ddx = (at(x, y, 1, 0) - at(x, y, -1, 0)) * 0.5 * g.px_per_ft;
            let ddy = (at(x, y, 0, 1) - at(x, y, 0, -1)) * 0.5 * g.px_per_ft;
            let n = norm_or([ddx, -ddy, 1.0], [0.0, 0.0, 1.0]);
            let o = i * 4;
            for k in 0..3 {
                rgba[o + k] = ((n[k] + 1.0) * 127.5).round().clamp(0.0, 255.0) as u8;
            }
            rgba[o + 3] = 255;
        }
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

/// The body `look` sees ([`Bodies::of`]) from that side: per pixel, the
/// nearest surface's depth, and that surface point with its outward normal
/// both on the figure as drawn (`rest`, where the front and back paintings
/// see it) and on the side's body (`side`, arms hung, where the side
/// painting sees it).
struct SurfaceMap {
    z: Vec<f32>,
    rest: Vec<(V3, V3)>,
    side: Vec<(V3, V3)>,
}

impl SurfaceMap {
    fn new(bodies: &Bodies, look: Look, g: &Grid) -> SurfaceMap {
        let (w, h) = (g.w, g.h);
        let mut map = SurfaceMap { z: vec![f32::INFINITY; w * h], rest: vec![([0.0; 3], [0.0; 3]); w * h], side: vec![([0.0; 3], [0.0; 3]); w * h] };
        let (seen, rest, side) = (bodies.of(look), bodies.rest, bodies.of(Look::Right));
        for (i, s) in seen.segments.iter().enumerate().filter(|(_, s)| !s.hidden) {
            // a ring's point and normal on one body's segment `i`, station `k`
            let on = |b: &Puppet, k: usize, sn: f32, cs: f32| -> (V3, V3) {
                let (s, st) = (&b.segments[i], &b.segments[i].stations[k]);
                let q = add(st.c, add(scale(s.axis_w, st.w * cs), scale(s.axis_d, st.d * sn)));
                (q, norm_or(add(scale(s.axis_w, cs / st.w.max(1e-3)), scale(s.axis_d, sn / st.d.max(1e-3))), [0.0, 0.0, 1.0]))
            };
            let ring = |k: usize| -> Vec<Corner> {
                (0..AROUND)
                    .map(|j| {
                        let (sn, cs) = (j as f32 / AROUND as f32 * std::f32::consts::TAU).sin_cos();
                        let (at, d) = look.project(g, on(seen, k, sn, cs).0);
                        Corner { at, d, rest: on(rest, k, sn, cs), side: on(side, k, sn, cs) }
                    })
                    .collect()
            };
            let rings: Vec<_> = (0..s.stations.len()).map(ring).collect();
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
                    self.rest[i] = (mix(&|c| c.rest.0), norm_or(mix(&|c| c.rest.1), [0.0, 0.0, 1.0]));
                    self.side[i] = (mix(&|c| c.side.0), norm_or(mix(&|c| c.side.1), [0.0, 0.0, 1.0]));
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Corner {
    at: [f32; 2],
    d: f32,
    /// The point and normal on the figure as drawn, and on the side's body.
    rest: (V3, V3),
    side: (V3, V3),
}

/// Below this facing a view's paint counts as a smear, not paint (see
/// [`project_view`]).
pub const GRAZING: f32 = 0.35;

/// Projection-first (`docs/projection-first-export-spec.md`): what the
/// paintings made so far show of the body seen from `view` (as drawn; from
/// the side, arms hung), in exactly
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
    let bodies = Bodies::new(p);
    let sources: Vec<(Look, Grid, Painting, DepthMap)> = Look::ALL
        .iter()
        .filter_map(|look| {
            let (grid, painting) = match look {
                Look::Front => (paintings.front_grid?, paintings.front?),
                Look::Back => (paintings.front_grid?, paintings.back?),
                Look::Right | Look::Left => (paintings.side_grid?, paintings.side?),
            };
            Some((*look, grid, painting, DepthMap::new(bodies.of(*look), *look, &grid)))
        })
        .collect();
    let surface = SurfaceMap::new(&bodies, view.look(), &g);
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
            let (mut sum, mut weight, mut best) = ([0.0f32; 3], 0.0f32, 0.0f32);
            for (look, grid, painting, depth) in &sources {
                let (point, normal) = if look.is_side() { surface.side[i] } else { surface.rest[i] };
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
    use crate::uv::bump_sheet;
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

    /// The normal guide covers the depth guide's body and faces the way the
    /// body does: across the chest, the middle faces the viewer and the
    /// flanks turn away to their own sides.
    #[test]
    fn a_normal_guide_faces_the_way_the_body_does() {
        let (p, front, _) = humanoid();
        let s = 2;
        let (w, h) = (front.grid.w * s, front.grid.h * s);
        let rgba = normal_guide(&p, &front, PaintView::Front, s);
        let depth = depth_guide(&p, &front, PaintView::Front, s);
        assert_eq!(rgba.len(), w * h * 4);
        for (n, d) in rgba.chunks_exact(4).zip(depth.chunks_exact(4)) {
            assert_eq!(n[3] == 255, d[0] > 0, "the normal guide's body is the depth guide's");
        }
        let n = |x: usize, y: usize| -> [f32; 3] { [0, 1, 2].map(|k| rgba[(y * w + x) * 4 + k] as f32 / 127.5 - 1.0) };
        let row = (h as f32 * 0.3) as usize;
        let inside = |x: usize| rgba[(row * w + x) * 4 + 3] == 255;
        let mid = w / 2;
        assert!(inside(mid), "the chest is at the middle of row {row}");
        let a = (0..mid).rev().find(|&x| !inside(x)).map_or(0, |x| x + 1);
        let b = (mid..w).find(|&x| !inside(x)).unwrap_or(w) - 1;
        assert!(n(mid, row)[2] > 0.8, "the chest faces the viewer: {:?}", n(mid, row));
        assert!(n(a + 2 * s, row)[0] < -0.2, "the screen-left flank faces left: {:?}", n(a + 2 * s, row));
        assert!(n(b - 2 * s, row)[0] > 0.2, "the screen-right flank faces right: {:?}", n(b - 2 * s, row));
    }

    /// The humanoid's side outline draws the arm a sliver off the body: the
    /// guide shows no edge there, and still keeps real gaps (between the
    /// legs from the front) black.
    #[test]
    fn a_depth_guide_closes_the_outlines_seams() {
        let (p, front, side) = humanoid();
        // A hairline seam cut down the side view's arm, as the old outline
        // had (the redrawn one is a single mass): two columns of outside
        // pixels through the body, well inside the guide's seam width.
        let mut side = side;
        let (w, h) = (side.grid.w, side.grid.h);
        let col = (0..w).max_by_key(|&x| (0..h).filter(|&y| side.inside[y * w + x]).count()).unwrap();
        for y in (h / 3)..(2 * h / 3) {
            for x in [col, col + 1] {
                side.inside[y * w + x] = false;
            }
        }
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

    /// A sheet of one flat colour has no relief; stripes across it lean the
    /// surface along the body, never off it; off the patches stays empty.
    #[test]
    fn a_bump_comes_from_the_paints_detail() {
        let (p, _, _) = humanoid();
        let layout = UvLayout::fit(&p, crate::SHEET_SIDE);
        let (w, h) = (layout.w, layout.h);
        let on = |x: usize, y: usize| layout.address([x as f32 + 0.5, y as f32 + 0.5]).is_some();
        let sheet = |stripes: bool| -> Vec<u8> {
            (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let v = if stripes && (x / 4) % 2 == 0 { 200 } else { 120 };
                    if on(x, y) { [v, v, v, 255] } else { [0; 4] }
                })
                .collect()
        };
        let lean = |s: &[u8], i: usize| [0, 1, 2].map(|k| crate::uv::decode_normal(s[i * 4 + k]));
        let flat = bump_sheet(&p, &layout, &sheet(false));
        let painted: Vec<usize> = (0..w * h).filter(|&i| flat[i * 4 + 3] == 255).collect();
        assert!(painted.len() > w * h / 10, "the patches are covered: {}", painted.len());
        assert!(painted.iter().all(|&i| lean(&flat, i).iter().all(|v| v.abs() < 0.01)), "flat paint, no relief");
        assert!((0..w * h).filter(|&i| !on(i % w, i / w)).all(|i| flat[i * 4 + 3] == 0), "off the patches: empty");
        let striped = bump_sheet(&p, &layout, &sheet(true));
        let (mut leaning, mut off_surface) = (0, 0.0f32);
        for &i in &painted {
            let d = lean(&striped, i);
            if d.iter().map(|v| v * v).sum::<f32>().sqrt() > 0.05 {
                leaning += 1;
                let (x, y) = (i % w, i / w);
                let (seg, station, theta) = layout.address([x as f32 + 0.5, y as f32 + 0.5]).unwrap();
                let (_, normal) = rest_point(&p, seg, station, theta).unwrap();
                off_surface = off_surface.max(dot(d, normal).abs());
            }
        }
        assert!(leaning > painted.len() / 4, "stripes lean the surface: {leaning} of {}", painted.len());
        assert!(off_surface < 0.03, "along the surface only: {off_surface}");
    }

    /// The A-pose humanoid's side is painted over its arm HANGING: a side
    /// painting's arm (red, where the hung arm shows) lands on the arm, and
    /// the thigh and hip the side sees stay the painting's blue. Through the
    /// raised arm, the red landed on the thigh -- a ghost hand on the side.
    #[test]
    fn the_sides_arm_paint_lands_on_the_hanging_arm() {
        let (p, _, side) = humanoid();
        let (hung, _) = p.with_arms_hung().expect("the humanoid is drawn in an A-pose");
        let template = side_silhouette(&p, &side);
        let g = template.grid;
        assert!(template.inside.iter().filter(|&&b| b).count() >= side.inside.iter().filter(|&&b| b).count());
        let arms: std::collections::HashSet<usize> = p.arm_hang.iter().flat_map(|(a, _)| p.subtree(*a)).collect();
        let arm = DepthMap::of(&hung, Look::Right, &g, &|s| p.segments.iter().position(|q| q.name == s.name).is_some_and(|i| arms.contains(&i)));
        let body = DepthMap::new(&hung, Look::Right, &g);
        let paint: Vec<u8> = (0..g.w * g.h)
            .flat_map(|i| match (template.inside[i], arm.z[i].is_finite() && arm.z[i] <= body.z[i] + 1e-4) {
                (false, _) => [0; 4],
                (true, true) => [220, 20, 20, 255],
                (true, false) => [20, 20, 220, 255],
            })
            .collect();
        let paintings = Paintings { side_grid: Some(g), side: Some(Painting { rgba: &paint, w: g.w, h: g.h }), ..Default::default() };
        let layout = UvLayout::fit(&p, crate::SHEET_SIDE);
        // the texels of `keep`'s segments the side sees squarely on `on`,
        // and how many of them are red
        let red_among = |sheet: &[u8], on: &Puppet, keep: &dyn Fn(usize, V3) -> bool| -> (usize, usize) {
            let depth = DepthMap::new(on, Look::Right, &g);
            let (mut seen, mut red) = (0, 0);
            for y in 0..layout.h {
                for x in 0..layout.w {
                    let Some((seg, station, theta)) = layout.address([x as f32 + 0.5, y as f32 + 0.5]) else { continue };
                    let Some((point, normal)) = rest_point(on, seg, station, theta).filter(|(q, _)| keep(seg, *q)) else { continue };
                    let (at, d) = Look::Right.project(&g, point);
                    if dot(normal, Look::Right.toward()) < 0.5 || d > depth.nearest(at) + DEPTH_TOL_FT {
                        continue;
                    }
                    seen += 1;
                    let px = &sheet[(y * layout.w + x) * 4..][..3];
                    red += usize::from(px[0] > 120 && px[2] < 120);
                }
            }
            (seen, red)
        };
        let legs = |i: usize, _: V3| p.segments[i].part.as_deref().is_some_and(|part| part.ends_with("Leg"));
        let sheet = prefill(&p, &layout, &paintings);
        let (seen, red) = red_among(&sheet, &hung, &legs);
        assert!(seen > 500 && red * 50 < seen, "the legs the side sees are the painting's blue: {red} red of {seen}");
        let (seen, red) = red_among(&sheet, &hung, &|i, _| arms.contains(&i));
        assert!(seen > 200 && red * 10 > seen * 8, "the arms the side sees are its red: {red} of {seen}");
        // Beside the hanging hand -- the hip and thigh, from the hand's tip
        // up a foot -- the body is blue; through the arm as drawn, raised,
        // the side's arm fell there: the ghost hand this guards against.
        let tip = arms.iter().flat_map(|&i| hung.segments[i].stations.iter().map(|s| s.c[1])).fold(f32::INFINITY, f32::min);
        let beside = |i: usize, q: V3| !arms.contains(&i) && p.segments[i].kind.is_body() && (tip..tip + 1.0).contains(&q[1]);
        let (seen, red) = red_among(&sheet, &hung, &beside);
        assert!(seen > 500 && red * 25 < seen, "beside the hanging hand, blue: {red} red of {seen}");
        let mut raised = p.clone();
        raised.arm_hang.clear();
        let (seen, red) = red_among(&prefill(&raised, &layout, &paintings), &p, &beside);
        assert!(red * 10 > seen, "the ghost hand, through the raised arm: {red} red of {seen}");
    }

    /// A painting's outermost pixels (green here: where a painter's
    /// background shows) are what its view sees of a surface it only grazes;
    /// that surface fills from the paint seen squarely nearby (blue) instead
    /// of being covered by a few edge pixels stretched across it. (A halo
    /// wider than that is the cut's to trim: its paint is seen squarely.)
    #[test]
    fn a_surface_seen_only_at_a_grazing_angle_takes_the_paint_beside_it() {
        let (p, front, _) = humanoid();
        let g = front.grid;
        let edge = |i: usize| {
            let (x, y) = ((i % g.w) as isize, (i / g.w) as isize);
            (-1..=1).any(|dy: isize| (-1..=1).any(|dx: isize| {
                let (qx, qy) = (x + dx, y + dy);
                qx < 0 || qy < 0 || qx as usize >= g.w || qy as usize >= g.h || !front.inside[qy as usize * g.w + qx as usize]
            }))
        };
        let paint: Vec<u8> = (0..g.w * g.h)
            .flat_map(|i| match (front.inside[i], edge(i)) {
                (false, _) => [0; 4],
                (true, true) => [20, 220, 20, 255],
                (true, false) => [20, 20, 220, 255],
            })
            .collect();
        let paintings = Paintings { front_grid: Some(g), front: Some(Painting { rgba: &paint, w: g.w, h: g.h }), ..Default::default() };
        let layout = UvLayout::fit(&p, crate::SHEET_SIDE);
        let sheet = prefill(&p, &layout, &paintings);
        let depth = DepthMap::new(&p, Look::Front, &g);
        let (mut grazed, mut green) = (0, 0);
        for y in 0..layout.h {
            for x in 0..layout.w {
                let Some((seg, station, theta)) = layout.address([x as f32 + 0.5, y as f32 + 0.5]) else { continue };
                let Some((point, normal)) = rest_point(&p, seg, station, theta) else { continue };
                let facing = dot(normal, Look::Front.toward());
                let (at, d) = Look::Front.project(&g, point);
                if !(MIN_FACING..GRAZING * 0.8).contains(&facing) || d > depth.nearest(at) + DEPTH_TOL_FT {
                    continue;
                }
                grazed += 1;
                let px = &sheet[(y * layout.w + x) * 4..][..3];
                green += usize::from(px[1] > 120 && px[2] < 120);
            }
        }
        // without the rule over half of it is the edge's green (56%); with
        // it the rest comes from the edge paint seen squarely nearby (20%)
        assert!(grazed > 1000 && green * 3 < grazed, "grazed surface takes the edge's green: {green} of {grazed}");
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
