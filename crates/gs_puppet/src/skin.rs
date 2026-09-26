//! The smooth skin: the same stations as [`Posed::draw_list`], rendered as
//! ONE surface into an image instead of a list of flat shapes.
//!
//! The shape list sorts whole groups (trunk, head, each limb) by their
//! average depth and paints each station pair as its own convex hull. Two
//! things follow, and both show while a figure turns: a limb and the trunk
//! swap order in one jump when their averages cross, and every hull brings
//! its own corners and contour, so joints read as bumps and notches.
//!
//! The skin fixes both at the root:
//!
//! * every station ring becomes a band of triangles, rasterised with a depth
//!   test per pixel, so what is in front is decided where it is in front;
//! * normals and colours are then averaged over a small window, but only
//!   between pixels whose depths agree, so a shoulder or a hip blends into
//!   the trunk while an arm held in front of the chest stays separate;
//! * contours come from the depth buffer: round the whole figure, and inside
//!   it only where a DIFFERENT segment sits clearly nearer -- and not one it
//!   is JOINED to ([`joined`]): the tubes a trunk is built from (spine,
//!   brisket, underline) are one body, so the valley where two of them meet
//!   is neither outlined nor shaded as a crease, and neither is the knee
//!   between a thigh and a shank.
//!
//! Shading is the shape list's cel look (a light fixed to the view, a lit
//! and a shaded band), softened at the band edges. Everything is egui-free;
//! the result is straight RGBA for any host to upload as a texture.
//!
//! Lights standing round the figure -- a campfire, a torch -- are not in
//! that look: [`Posed::skin_surface`] also keeps what every pixel shows
//! (where its surface point is, which way it faces, its colour), and
//! [`SkinSurface::lamp_light`] lights that from any [`Lamp`]s, as often as
//! they move, without drawing the figure again.

use crate::draw::{surfaces, view_basis, Look, BOSS_OUTLINE, BODY_DEAD, DARK_BELOW, DEPTH_SHADE, EYE, LIGHT_ABOVE};
use crate::math::{add, cross, dot, len, mul_mv, norm_or, rot_y, scale, sub, V3};
use crate::pose::Posed;
use crate::uv::{Texture, UvLayout};

/// Samples per pixel along each axis while rasterising.
const SS: usize = 2;
/// No skin is rendered larger than this on a side.
const MAX_SIDE: usize = 4096;
/// Normals and colours are averaged over a window this fraction of the
/// figure's height across.
const SMOOTH_OF_HEIGHT: f32 = 0.035;
/// How far a normal sheet's deviation leans the surface normal (1 = as the
/// map was painted).
const NORMAL_MAP_STRENGTH: f32 = 1.0;
/// How strongly the relief shades: tone per unit change in the light's
/// incidence the normal sheet makes over the geometry's own.
const RELIEF_GAIN: f32 = 0.7;
/// Neighbours further apart in depth than this (fraction of height) are
/// different surfaces and never blend.
const BLEND_DEPTH_OF_HEIGHT: f32 = 0.04;
/// A different segment nearer by more than this (fraction of height) draws
/// a contour on the far one.
const CONTOUR_DEPTH_OF_HEIGHT: f32 = 0.035;
/// Joined segments blend across a depth step this many times the ordinary
/// tolerance: the valley between two trunk tubes shades as one surface.
const JOINED_BLEND: f32 = 3.0;

/// Which segments are parts of one continuous surface, as a `n * n` table:
/// a segment and the next along its chain (parent and child); and, inside
/// the trunk and the head, segments of the same bone, of a bone and its
/// parent, or of two sibling bones -- except that a tail joins only its
/// parent, so a tail laid across the flank keeps its line.
pub(crate) fn joined(puppet: &crate::Puppet) -> Vec<bool> {
    let segs = &puppet.segments;
    let n = segs.len();
    let bone_parent = |i: usize| -> Option<&str> {
        // The bone this segment's CHAIN hangs from: walk to its first
        // segment, then that segment's parent's bone.
        let mut at = i;
        while segs[at].chain_index > 0 {
            at = segs[at].parent?;
        }
        segs[at].parent.map(|p| segs[p].bone.as_str())
    };
    let body = |g: usize| puppet.groups.get(g).is_some_and(|name| name == "trunk" || name == "head");
    let tail = |i: usize| segs[i].bone.contains("tail");
    let mut out = vec![false; n * n];
    for i in 0..n {
        for j in 0..n {
            let (a, b) = (&segs[i], &segs[j]);
            // A prop or a garment is not flesh: it keeps its line against
            // the body it hangs from.
            let layer = |k: crate::Kind| (k.is_prop(), k.is_garment());
            // ...except a hem where it leaves the waist: no seam line there.
            let waist = |g: &crate::Segment, body: usize| {
                g.chain_index == 0 && g.parent == Some(body) && matches!(g.kind, crate::Kind::Garment { hang: crate::Hang::Toward { .. } })
            };
            let chain = i == j
                || (a.parent == Some(j) || b.parent == Some(i)) && layer(a.kind) == layer(b.kind)
                || waist(a, j)
                || waist(b, i);
            let related = a.group == b.group && body(a.group) && {
                let (pa, pb) = (bone_parent(i), bone_parent(j));
                a.bone == b.bone
                    || pa == Some(b.bone.as_str())
                    || pb == Some(a.bone.as_str())
                    || (pa.is_some() && pa == pb && !tail(i) && !tail(j))
            };
            out[i * n + j] = chain || related;
        }
    }
    out
}
/// Half-width of the softened step between tone bands, in `dot(n, light)`.
const BAND_SOFT: f32 = 0.09;
const LIT_TONE: f32 = 1.13;
const DARK_TONE: f32 = 0.8;
/// Strength of the continuous light term under the bands.
const LAMBERT: f32 = 0.08;
/// Extra width of the boss contour past the ordinary one, pixels.
const BOSS_EXTRA_PX: f32 = 2.5;

/// Things a status does to the body itself, drawn onto the skin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Effects {
    /// Wrapped in web: strands round every limb, following the surface
    /// through any pose, and threads bridging limb to body.
    pub webbed: bool,
    /// Iced over: a cold tint with glints.
    pub frozen: bool,
}

impl Effects {
    pub fn any(&self) -> bool {
        self.webbed || self.frozen
    }
}

const WEB_RGB: [f32; 3] = [236.0, 236.0, 226.0];
const FROST_RGB: [f32; 3] = [138.0, 190.0, 240.0];
/// Strand spacing on the surface, feet.
const WEB_SPACING_FT: f32 = 0.42;
/// Threads bridging limb to body on a webbed figure.
const BIND_THREADS: u32 = 7;

fn hash(a: i32, b: u32) -> f32 {
    let mut x = (a as u32).wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// How much web covers a surface point (surface feet, from a 1 px-per-foot
/// layout): three families of wavering strands, some missing, crossing.
fn web_alpha(at: [f32; 2], half_width: f32) -> f32 {
    let mut a = 0.0f32;
    for (k, (dx, dy, sp)) in [(0.8f32, 0.6f32, 1.0f32), (-0.6, 0.8, 1.13), (0.0, 1.0, 1.7)].into_iter().enumerate() {
        let sp = sp * WEB_SPACING_FT;
        let across = (at[0] * dx + at[1] * dy) / sp;
        let along = (-at[0] * dy + at[1] * dx) / sp;
        let cell = across.floor();
        if hash(cell as i32, k as u32 + 11) < 0.22 {
            continue;
        }
        let wobble = 0.18 * (along * 1.9 + hash(cell as i32, k as u32) * std::f32::consts::TAU).sin();
        let d = (across - cell - 0.5 - wobble).abs() * sp;
        a = a.max(1.0 - smoothstep(half_width * 0.5, half_width, d));
    }
    a
}

/// A glint on frost: sparse bright cells.
fn frost_glint(at: [f32; 2]) -> bool {
    let (cx, cy) = ((at[0] / 0.07).floor() as i32, (at[1] / 0.07).floor() as i32);
    hash(cx.wrapping_mul(7919) ^ cy, 3) > 0.965
}

/// What the rasteriser paints a sample with, beyond the body colour.
struct Paint<'a> {
    tex: Option<&'a Texture>,
    /// A rest-space direction on segment `seg`, as posed and turned: what
    /// the normal sheet's deviations go through.
    rot: &'a dyn Fn(usize, V3) -> V3,
    /// A 1 px-per-foot layout of the surface, for effects measured in feet.
    surface: Option<&'a UvLayout>,
    effects: Effects,
    /// Half a web strand's width, feet.
    strand_ft: f32,
}

/// A rendered figure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skin {
    /// Where the image's top-left corner sits in the pixel space of the
    /// projection it was rendered through.
    pub origin: [f32; 2],
    pub w: usize,
    pub h: usize,
    /// Straight (not premultiplied) RGBA8, row-major, transparent outside
    /// the figure and its contour.
    pub rgba: Vec<u8>,
}

impl Skin {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}

#[derive(Clone, Copy)]
struct Vert {
    /// Supersampled pixels, relative to the image.
    p: [f32; 2],
    z: f32,
    n: V3,
    /// The surface point, world feet.
    w: V3,
    col: [f32; 3],
    seg: u32,
    /// Surface address for a texture: (station, ring angle); NaN = none
    /// (a fan centre, a stump's raw end).
    addr: [f32; 2],
    /// A wound's colour and how strongly it shows over a texture here.
    wound: [f32; 4],
}

/// Samples round a ring for a figure whose largest ring is `px` across.
fn around(px: f32) -> usize {
    ((px * 0.9) as usize).clamp(10, 36)
}

/// The supersampled buffers the triangles are rasterised into.
struct Samples {
    w: usize,
    h: usize,
    z: Vec<f32>,
    n: Vec<V3>,
    col: Vec<[f32; 3]>,
    seg: Vec<u32>,
    /// Surface points, world feet; empty unless a [`SkinSurface`] is kept.
    pos: Vec<V3>,
    /// The normal sheet's deviation here, posed (zero without one).
    dev: Vec<V3>,
}

impl Samples {
    fn new(w: usize, h: usize, keep_pos: bool) -> Samples {
        let n = w * h;
        Samples {
            w,
            h,
            z: vec![f32::INFINITY; n],
            n: vec![[0.0; 3]; n],
            col: vec![[0.0; 3]; n],
            seg: vec![u32::MAX; n],
            pos: if keep_pos { vec![[0.0; 3]; n] } else { Vec::new() },
            dev: vec![[0.0; 3]; n],
        }
    }

    fn tri(&mut self, v: [&Vert; 3], paint: &Paint) {
        // A textured triangle interpolates its corners' ADDRESSES and looks
        // each sample up on the sheet (effects do the same on a feet-scaled
        // layout). Angles are unwrapped first, so a triangle straddling
        // angle 0 does not sweep the whole ring.
        let addressed = v.iter().all(|c| c.addr[0].is_finite() && c.addr[1].is_finite());
        let tex = paint.tex.filter(|_| addressed);
        let surface = paint.surface.filter(|_| addressed);
        let mut theta = [v[0].addr[1], v[1].addr[1], v[2].addr[1]];
        if tex.is_some() || surface.is_some() {
            let lo = theta.iter().copied().fold(f32::MAX, f32::min);
            for t in &mut theta {
                if *t - lo > std::f32::consts::PI {
                    *t -= std::f32::consts::TAU;
                }
            }
        }
        let p = [v[0].p, v[1].p, v[2].p];
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if area.abs() < 1e-6 || !area.is_finite() {
            return;
        }
        let min = [p[0][0].min(p[1][0]).min(p[2][0]), p[0][1].min(p[1][1]).min(p[2][1])];
        let max = [p[0][0].max(p[1][0]).max(p[2][0]), p[0][1].max(p[1][1]).max(p[2][1])];
        let x0 = (min[0] - 0.5).ceil().max(0.0) as usize;
        let y0 = (min[1] - 0.5).ceil().max(0.0) as usize;
        let x1 = ((max[0] - 0.5).floor() as isize).min(self.w as isize - 1);
        let y1 = ((max[1] - 0.5).floor() as isize).min(self.h as isize - 1);
        if x1 < x0 as isize || y1 < y0 as isize {
            return;
        }
        let inv = 1.0 / area;
        let edge = |a: [f32; 2], b: [f32; 2], q: [f32; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
        for y in y0..=y1 as usize {
            let row = y * self.w;
            for x in x0..=x1 as usize {
                let q = [x as f32 + 0.5, y as f32 + 0.5];
                let w0 = edge(p[1], p[2], q) * inv;
                let w1 = edge(p[2], p[0], q) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                    continue;
                }
                let z = w0 * v[0].z + w1 * v[1].z + w2 * v[2].z;
                let i = row + x;
                if z >= self.z[i] {
                    continue;
                }
                self.z[i] = z;
                self.n[i] = add(add(scale(v[0].n, w0), scale(v[1].n, w1)), scale(v[2].n, w2));
                if !self.pos.is_empty() {
                    self.pos[i] = add(add(scale(v[0].w, w0), scale(v[1].w, w1)), scale(v[2].w, w2));
                }
                let lerp = |f: fn(&Vert) -> f32| w0 * f(v[0]) + w1 * f(v[1]) + w2 * f(v[2]);
                let station = lerp(|c| c.addr[0]);
                let angle = w0 * theta[0] + w1 * theta[1] + w2 * theta[2];
                let seg = v[0].seg as usize;
                let painted = tex.and_then(|t| t.sample(t.layout.locate(seg, station, angle)?));
                self.dev[i] = tex
                    .filter(|t| t.normal.is_some())
                    .and_then(|t| t.sample_normal(t.layout.locate(seg, station, angle)?))
                    .map_or([0.0; 3], |d| (paint.rot)(seg, d));
                let mut col = match painted {
                    Some(rgb) => {
                        let wound = [lerp(|c| c.wound[0]), lerp(|c| c.wound[1]), lerp(|c| c.wound[2])];
                        let a = lerp(|c| c.wound[3]);
                        [0, 1, 2].map(|k| rgb[k] + (wound[k] - rgb[k]) * a)
                    }
                    None => [0, 1, 2].map(|k| w0 * v[0].col[k] + w1 * v[1].col[k] + w2 * v[2].col[k]),
                };
                let feet = surface.and_then(|l| l.locate(seg, station, angle));
                if paint.effects.frozen {
                    col = [0, 1, 2].map(|k| col[k] + (FROST_RGB[k] - col[k]) * 0.62);
                    if feet.is_some_and(frost_glint) {
                        col = [0, 1, 2].map(|k| col[k] + (255.0 - col[k]) * 0.85);
                    }
                }
                if paint.effects.webbed {
                    if let Some(at) = feet {
                        let a = web_alpha(at, paint.strand_ft) * 0.92;
                        col = [0, 1, 2].map(|k| col[k] + (WEB_RGB[k] - col[k]) * a);
                    }
                }
                self.col[i] = col;
                // The segment of the corner this sample is closest to.
                self.seg[i] = if w0 >= w1 && w0 >= w2 { v[0].seg } else if w1 >= w2 { v[1].seg } else { v[2].seg };
            }
        }
    }
}

/// The samples averaged down to pixels.
struct Pixels {
    w: usize,
    h: usize,
    /// Fraction of the pixel's samples the figure covers.
    cov: Vec<f32>,
    /// Nearest sample's depth and segment (INFINITY / MAX when uncovered).
    z: Vec<f32>,
    seg: Vec<u32>,
    n: Vec<V3>,
    col: Vec<[f32; 3]>,
    /// The nearest sample's surface point; empty unless the samples kept them.
    pos: Vec<V3>,
    /// The normal sheet's deviation, averaged over the samples.
    dev: Vec<V3>,
}

impl Pixels {
    fn resolve(s: &Samples, w: usize, h: usize) -> Pixels {
        let len = w * h;
        let mut out = Pixels {
            w,
            h,
            cov: vec![0.0; len],
            z: vec![f32::INFINITY; len],
            seg: vec![u32::MAX; len],
            n: vec![[0.0; 3]; len],
            col: vec![[0.0; 3]; len],
            pos: if s.pos.is_empty() { Vec::new() } else { vec![[0.0; 3]; len] },
            dev: vec![[0.0; 3]; len],
        };
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let mut hits = 0;
                let (mut n, mut col, mut dev) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let j = (y * SS + sy) * s.w + x * SS + sx;
                        if !s.z[j].is_finite() {
                            continue;
                        }
                        hits += 1;
                        n = add(n, norm_or(s.n[j], [0.0, 0.0, 1.0]));
                        dev = add(dev, s.dev[j]);
                        for k in 0..3 {
                            col[k] += s.col[j][k];
                        }
                        if s.z[j] < out.z[i] {
                            out.z[i] = s.z[j];
                            out.seg[i] = s.seg[j];
                            if !s.pos.is_empty() {
                                out.pos[i] = s.pos[j];
                            }
                        }
                    }
                }
                if hits > 0 {
                    out.cov[i] = hits as f32 / (SS * SS) as f32;
                    out.n[i] = norm_or(n, [0.0, 0.0, 1.0]);
                    out.col[i] = col.map(|c| c / hits as f32);
                    out.dev[i] = scale(dev, 1.0 / hits as f32);
                }
            }
        }
        out
    }

    /// Average normals (and, with `colours`, colours) over a `2r + 1`
    /// window (one axis per pass), only between pixels whose depths are
    /// within `tol`. A texture's colours are left sharp: blending is for
    /// the shape, and for softening flat wound tints.
    fn smooth(&mut self, r: usize, tol: f32, colours: bool, joined: &dyn Fn(u32, u32) -> bool) {
        if r == 0 {
            return;
        }
        for (dx, dy) in [(1isize, 0isize), (0, 1)] {
            let (mut n2, mut c2) = (self.n.clone(), self.col.clone());
            for y in 0..self.h {
                for x in 0..self.w {
                    let i = y * self.w + x;
                    let (z0, s0) = (self.z[i], self.seg[i]);
                    if !z0.is_finite() {
                        continue;
                    }
                    let (mut n, mut col, mut k) = ([0.0f32; 3], [0.0f32; 3], 0.0f32);
                    for t in -(r as isize)..=r as isize {
                        let (qx, qy) = (x as isize + dx * t, y as isize + dy * t);
                        if qx < 0 || qy < 0 || qx >= self.w as isize || qy >= self.h as isize {
                            continue;
                        }
                        let j = qy as usize * self.w + qx as usize;
                        let dz = (self.z[j] - z0).abs();
                        if dz > tol && !(dz <= tol * JOINED_BLEND && joined(s0, self.seg[j])) {
                            continue;
                        }
                        n = add(n, self.n[j]);
                        for c in 0..3 {
                            col[c] += self.col[j][c];
                        }
                        k += 1.0;
                    }
                    n2[i] = norm_or(n, self.n[i]);
                    if colours {
                        c2[i] = col.map(|c| c / k.max(1.0));
                    }
                }
            }
            self.n = n2;
            self.col = c2;
        }
    }

    /// Distance (pixels, chamfer 1 / sqrt 2) from every pixel to the nearest
    /// pixel the figure mostly covers; 0 on those.
    fn outside_distance(&self) -> Vec<f32> {
        let (w, h) = (self.w, self.h);
        let mut d: Vec<f32> = self.cov.iter().map(|c| if *c >= 0.5 { 0.0 } else { f32::INFINITY }).collect();
        let diag = std::f32::consts::SQRT_2;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let mut v = d[i];
                if x > 0 {
                    v = v.min(d[i - 1] + 1.0);
                }
                if y > 0 {
                    v = v.min(d[i - w] + 1.0);
                    if x > 0 {
                        v = v.min(d[i - w - 1] + diag);
                    }
                    if x + 1 < w {
                        v = v.min(d[i - w + 1] + diag);
                    }
                }
                d[i] = v;
            }
        }
        for y in (0..h).rev() {
            for x in (0..w).rev() {
                let i = y * w + x;
                let mut v = d[i];
                if x + 1 < w {
                    v = v.min(d[i + 1] + 1.0);
                }
                if y + 1 < h {
                    v = v.min(d[i + w] + 1.0);
                    if x + 1 < w {
                        v = v.min(d[i + w + 1] + diag);
                    }
                    if x > 0 {
                        v = v.min(d[i + w - 1] + diag);
                    }
                }
                d[i] = v;
            }
        }
        d
    }

    /// Strength (0..=1) of an interior contour at every pixel: how close a
    /// pixel of ANOTHER segment is that sits nearer by more than `tol`.
    fn inner_contour(&self, width: f32, tol: f32, joined: &dyn Fn(u32, u32) -> bool) -> Vec<f32> {
        let mut out = vec![0.0; self.w * self.h];
        if width <= 0.0 {
            return out;
        }
        let r = (width + 1.0).ceil() as isize;
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let (z0, s0) = (self.z[i], self.seg[i]);
                if !z0.is_finite() {
                    continue;
                }
                let mut best = 0.0f32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (qx, qy) = (x as isize + dx, y as isize + dy);
                        if qx < 0 || qy < 0 || qx >= self.w as isize || qy >= self.h as isize {
                            continue;
                        }
                        let j = qy as usize * self.w + qx as usize;
                        if self.seg[j] == s0 || self.z[j] >= z0 - tol || joined(s0, self.seg[j]) {
                            continue;
                        }
                        // Fade in with the step, so a limb growing out of
                        // the body starts its contour where it parts from it.
                        let step = smoothstep(tol, 2.0 * tol, z0 - self.z[j]);
                        let near = (width - ((dx * dx + dy * dy) as f32).sqrt() + 1.0).clamp(0.0, 1.0);
                        best = best.max(step * near);
                    }
                }
                out[i] = best;
            }
        }
        out
    }
}

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `top` over `under`, both straight RGB with an alpha.
fn over(top: ([f32; 3], f32), under: ([f32; 3], f32)) -> ([f32; 3], f32) {
    let a = top.1 + under.1 * (1.0 - top.1);
    if a <= 1e-6 {
        return ([0.0; 3], 0.0);
    }
    let c = [0, 1, 2].map(|k| (top.0[k] * top.1 + under.0[k] * under.1 * (1.0 - top.1)) / a);
    (c, a)
}

impl Posed<'_> {
    /// Render the puppet as one smooth surface through `project` (world
    /// feet -> screen pixels and a depth in feet, larger = farther). Uses
    /// `look` as [`Posed::draw_list`] does, except `min_station_px`: the skin
    /// keeps every station.
    pub fn skin(&self, project: &dyn Fn(V3) -> ([f32; 2], f32), look: &Look) -> Skin {
        self.skin_textured(project, look, None)
    }

    /// [`Posed::skin`] wearing a painted sheet (see [`crate::uv`]): each
    /// pixel takes its colour from where its surface point lies on the
    /// sheet; unpainted parts of the sheet fall back to the body colour.
    /// Wounds tint over the paint; a dead body's paint is greyed.
    pub fn skin_textured(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
    ) -> Skin {
        self.skin_with(project, look, texture, Effects::default())
    }

    /// [`Posed::skin_textured`] with status [`Effects`] drawn onto the body.
    pub fn skin_with(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
        effects: Effects,
    ) -> Skin {
        self.render(project, look, texture, effects, false).0
    }

    /// [`Posed::skin_with`], and what each of its pixels shows, for lamps
    /// to light ([`SkinSurface::lamp_light`]).
    pub fn skin_surface(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
        effects: Effects,
    ) -> (Skin, SkinSurface) {
        let (skin, surface) = self.render(project, look, texture, effects, true);
        (skin, surface.unwrap_or_default())
    }

    fn render(
        &self,
        project: &dyn Fn(V3) -> ([f32; 2], f32),
        look: &Look,
        texture: Option<&Texture>,
        effects: Effects,
        keep_surface: bool,
    ) -> (Skin, Option<SkinSurface>) {
        // A sheet laid out for another puppet would put paint anywhere.
        // Held props come after the body and are never on the sheet.
        let body_segments = self.puppet.segments.iter().take_while(|s| !s.kind.is_prop()).count();
        let texture = texture.filter(|t| t.layout.patches.len() == body_segments);
        let puppet = self.puppet;
        let body = if look.dead { BODY_DEAD } else { look.body };
        let line = body.map(|c| c as f32 * 0.3);
        let view = view_basis(project, [0.0, puppet.height * 0.5, 0.0]);
        let height = puppet.height.max(0.1);

        // ---- the mesh, in unshifted supersampled pixels
        let ss = SS as f32;
        let mut verts: Vec<Vert> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        let surfs = surfaces(self, look);
        for surf in &surfs {
            let n = surf.rings.len();
            if n < 2 {
                continue;
            }
            let widest = surf
                .rings
                .iter()
                .map(|r| {
                    let (p, _) = project(r.c);
                    let e = |v: V3| {
                        let (q, _) = project(add(r.c, v));
                        (q[0] - p[0]).hypot(q[1] - p[1])
                    };
                    e(r.a).max(e(r.b))
                })
                .filter(|v| v.is_finite())
                .fold(0.0f32, f32::max);
            let m = around(widest * 2.0);
            let angles: Vec<(f32, f32)> =
                (0..m).map(|j| (j as f32 / m as f32 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| (c, s)).collect();
            let pts: Vec<Vec<V3>> = surf
                .rings
                .iter()
                .map(|r| angles.iter().map(|(c, s)| add(r.c, add(scale(r.a, *c), scale(r.b, *s)))).collect())
                .collect();
            let middle = scale(surf.rings.iter().fold([0.0; 3], |acc, r| add(acc, r.c)), 1.0 / n as f32);
            let base = verts.len() as u32;
            let measured = puppet.segments[surf.index].stations.len();
            for (k, ring) in surf.rings.iter().enumerate() {
                let col = ring.color(body, surf.base).map(|c| c as f32);
                let wound = if ring.red {
                    [col[0], col[1], col[2], 1.0]
                } else if look.dead {
                    [BODY_DEAD[0] as f32, BODY_DEAD[1] as f32, BODY_DEAD[2] as f32, 0.7]
                } else if surf.base != body {
                    [surf.base[0] as f32, surf.base[1] as f32, surf.base[2] as f32, ring.tint * 0.75]
                } else {
                    [0.0; 4]
                };
                let addressed = !ring.red && k < measured && surf.index < body_segments;
                let (prev, next) = (k.saturating_sub(2), (k + 2).min(n - 1));
                for j in 0..m {
                    let p = pts[k][j];
                    let around_t = sub(pts[k][(j + 1) % m], pts[k][(j + m - 1) % m]);
                    let along = sub(pts[next][j], pts[prev][j]);
                    let radial = sub(p, ring.c);
                    let outward = if len(radial) > 1e-4 { radial } else { sub(p, middle) };
                    let mut nrm = norm_or(cross(around_t, along), norm_or(outward, view.to_cam));
                    if dot(nrm, outward) < 0.0 {
                        nrm = scale(nrm, -1.0);
                    }
                    let (sp, z) = project(p);
                    let theta = j as f32 / m as f32 * std::f32::consts::TAU;
                    let addr = if addressed { [k as f32, theta] } else { [f32::NAN; 2] };
                    verts.push(Vert { p: [sp[0] * ss, sp[1] * ss], z, n: nrm, w: p, col, seg: surf.index as u32, addr, wound });
                }
            }
            let at = |k: usize, j: usize| base + (k * m + j % m) as u32;
            for k in 0..n - 1 {
                for j in 0..m {
                    tris.push([at(k, j), at(k, j + 1), at(k + 1, j + 1)]);
                    tris.push([at(k, j), at(k + 1, j + 1), at(k + 1, j)]);
                }
            }
            // Close both ends with a fan, so a cap's last ring leaves no hole.
            for (k, toward) in [(0, 1), (n - 1, n - 2)] {
                let ring = &surf.rings[k];
                let (sp, z) = project(ring.c);
                let out = norm_or(sub(ring.c, surf.rings[toward].c), view.to_cam);
                let centre = verts.len() as u32;
                verts.push(Vert {
                    p: [sp[0] * ss, sp[1] * ss],
                    z,
                    n: out,
                    w: ring.c,
                    col: ring.color(body, surf.base).map(|c| c as f32),
                    seg: surf.index as u32,
                    addr: [f32::NAN; 2],
                    wound: [0.0; 4],
                });
                for j in 0..m {
                    tris.push([centre, at(k, j), at(k, j + 1)]);
                }
            }
        }

        // ---- frame: the mesh's box plus room for the contours
        let margin = (look.outline_px + if look.boss_outline { BOSS_EXTRA_PX } else { 0.0 }).ceil() + 2.0;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in verts.iter().filter(|v| v.p[0].is_finite() && v.p[1].is_finite() && v.z.is_finite()) {
            for k in 0..2 {
                lo[k] = lo[k].min(v.p[k] / ss);
                hi[k] = hi[k].max(v.p[k] / ss);
            }
        }
        if lo[0] > hi[0] {
            return (Skin::default(), None);
        }
        let origin = [(lo[0] - margin).floor(), (lo[1] - margin).floor()];
        let w = ((hi[0] + margin).ceil() - origin[0]) as usize;
        let h = ((hi[1] + margin).ceil() - origin[1]) as usize;
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return (Skin::default(), None);
        }
        for v in &mut verts {
            v.p = [v.p[0] - origin[0] * ss, v.p[1] - origin[1] * ss];
        }

        // ---- rasterise, resolve, smooth
        let px_per_ft = {
            let mid = [0.0, height * 0.5, 0.0];
            let (a, _) = project(mid);
            let (b, _) = project(add(mid, [0.0, 1.0, 0.0]));
            (a[0] - b[0]).hypot(a[1] - b[1])
        };
        let surface = effects.any().then(|| UvLayout::new(puppet, 1.0));
        let turn = rot_y(look.yaw);
        let rot = |seg: usize, v: V3| mul_mv(&turn, self.xf[seg].dir(v));
        let paint = Paint {
            tex: texture,
            rot: &rot,
            surface: surface.as_ref(),
            effects,
            // At least about a pixel wide, however small the figure.
            strand_ft: (0.9 / px_per_ft.max(1e-3)).max(0.018),
        };
        let mut samples = Samples::new(w * SS, h * SS, keep_surface);
        for t in &tris {
            let v = t.map(|i| &verts[i as usize]);
            if v.iter().all(|c| c.p[0].is_finite() && c.p[1].is_finite() && c.z.is_finite()) {
                samples.tri(v, &paint);
            }
        }
        let mut px = Pixels::resolve(&samples, w, h);
        drop(samples);
        let radius = (SMOOTH_OF_HEIGHT * height * px_per_ft).round().clamp(0.0, 16.0) as usize;
        let table = joined(puppet);
        let count = puppet.segments.len();
        let is_joined = |a: u32, b: u32| {
            let (a, b) = (a as usize, b as usize);
            a < count && b < count && table[a * count + b]
        };
        px.smooth(radius, BLEND_DEPTH_OF_HEIGHT * height, texture.is_none() && !effects.any(), &is_joined);
        // The normal sheet's relief goes on AFTER the smoothing (which is
        // for the geometry's seams, and would blur a rib away), leaning the
        // surface's own normal by the painted deviation: the cel bands and
        // every lamp then see the ribs, folds and cracks the paint has.
        // The geometry's own normals are kept: the cel bands are theirs, and
        // the relief shows as the CHANGE in the light's incidence the map
        // makes on top (the bands have a tiny continuous term, and a rib
        // vanished into them).
        let relief = texture.is_some_and(|t| t.normal.is_some()).then(|| {
            let geo = px.n.clone();
            for i in 0..w * h {
                if px.dev[i] != [0.0; 3] {
                    px.n[i] = norm_or(add(px.n[i], scale(px.dev[i], NORMAL_MAP_STRENGTH)), px.n[i]);
                }
            }
            geo
        });

        // ---- shade and composite
        let (near, far) =
            px.z.iter().filter(|z| z.is_finite()).fold((f32::MAX, f32::MIN), |(n, f), z| (n.min(*z), f.max(*z)));
        let range = (far - near).max(2.0);
        let outside = px.outside_distance();
        let inner = px.inner_contour(look.outline_px, CONTOUR_DEPTH_OF_HEIGHT * height, &is_joined);
        let gold = BOSS_OUTLINE.map(|c| c as f32);
        let mut rgba = vec![0u8; w * h * 4];
        let mut kept = keep_surface.then(|| SkinSurface::blank(w, h, view.to_cam));
        for i in 0..w * h {
            let mut under = ([0.0; 3], 0.0);
            if look.boss_outline {
                let a = (look.outline_px + BOSS_EXTRA_PX - outside[i] + 1.0).clamp(0.0, 1.0);
                under = over((gold, a), under);
            }
            if look.outline_px > 0.0 {
                let a = (look.outline_px - outside[i] + 1.0).clamp(0.0, 1.0);
                under = over((line, a), under);
            }
            let mut top = ([0.0; 3], 0.0);
            if px.cov[i] > 0.0 {
                let n = px.n[i];
                let g = dot(n, view.light);
                let (gb, dg) = match &relief {
                    Some(geo) => {
                        let gb = dot(geo[i], view.light);
                        (gb, g - gb)
                    }
                    None => (g, 0.0),
                };
                let lit = smoothstep(LIGHT_ABOVE - BAND_SOFT, LIGHT_ABOVE + BAND_SOFT, gb);
                let dark = 1.0 - smoothstep(DARK_BELOW - BAND_SOFT, DARK_BELOW + BAND_SOFT, gb);
                // The cel bands, plus a gentle continuous term so a form still
                // turns between them, and the normal sheet's relief on top.
                let tone = (1.0 + (LIT_TONE - 1.0) * lit - (1.0 - DARK_TONE) * dark) * (1.0 + LAMBERT * gb) * (1.0 + RELIEF_GAIN * dg);
                let dim = 1.0 - DEPTH_SHADE * ((px.z[i] - near) / range).clamp(0.0, 1.0);
                let mut c = px.col[i].map(|v| (v * tone * dim).clamp(0.0, 255.0));
                if inner[i] > 0.0 {
                    c = [0, 1, 2].map(|k| c[k] + (line[k] - c[k]) * inner[i]);
                }
                top = (c, px.cov[i]);
                if let Some(s) = kept.as_mut() {
                    // Its own colour, unshaded: what a lamp's light shows.
                    let albedo = [0, 1, 2].map(|k| px.col[i][k] + (line[k] - px.col[i][k]) * inner[i]);
                    s.set(i, px.pos[i], px.n[i], albedo, px.cov[i]);
                }
            }
            let (c, a) = over(top, under);
            let o = i * 4;
            rgba[o..o + 3].copy_from_slice(&c.map(|v| v.round().clamp(0.0, 255.0) as u8));
            rgba[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }

        if effects.webbed {
            bind_threads(&mut rgba, &px, &puppet.family, (px_per_ft * 0.012).max(0.7));
        }

        // ---- eyes, where the surface they sit on is the one that shows --
        // unless a painted skin supplies the face (its own eyes, and dots
        // on top of them would give it four)
        if !look.dead && texture.is_none() {
            let turn = rot_y(look.yaw);
            for eye in &puppet.eyes {
                if !surfs.iter().any(|s| s.index == eye.segment) {
                    continue;
                }
                let x = &self.xf[eye.segment];
                let pos = mul_mv(&turn, x.point(eye.pos));
                if dot(mul_mv(&turn, x.dir(eye.normal)), view.to_cam) <= 0.25 {
                    continue;
                }
                let (at, z) = project(pos);
                let (edge, _) = project(add(pos, scale(mul_mv(&turn, x.dir([0.0, 1.0, 0.0])), eye.radius)));
                let r = (edge[0] - at[0]).hypot(edge[1] - at[1]).max(0.9);
                let c = [at[0] - origin[0], at[1] - origin[1]];
                if !(c[0].is_finite() && c[1].is_finite() && r.is_finite()) {
                    continue;
                }
                let (cx, cy) = (c[0].floor() as isize, c[1].floor() as isize);
                if cx < 0 || cy < 0 || cx >= w as isize || cy >= h as isize {
                    continue;
                }
                let at_px = cy as usize * w + cx as usize;
                if px.z[at_px] < z - CONTOUR_DEPTH_OF_HEIGHT * height {
                    continue;
                }
                // Under a garment (a hood's rim) it is covered, however near.
                if puppet.segments.get(px.seg[at_px] as usize).is_some_and(|s| s.kind.is_garment()) {
                    continue;
                }
                let eye_rgb = [EYE[0], EYE[1], EYE[2]].map(|v| v as f32);
                let reach = (r + 1.0).ceil() as isize;
                for y in (cy - reach).max(0)..=(cy + reach).min(h as isize - 1) {
                    for x in (cx - reach).max(0)..=(cx + reach).min(w as isize - 1) {
                        let d = (x as f32 + 0.5 - c[0]).hypot(y as f32 + 0.5 - c[1]);
                        let a = (r - d + 0.5).clamp(0.0, 1.0);
                        if a <= 0.0 {
                            continue;
                        }
                        let o = (y as usize * w + x as usize) * 4;
                        let under = ([0, 1, 2].map(|k| rgba[o + k] as f32), rgba[o + 3] as f32 / 255.0);
                        let (col, alpha) = over((eye_rgb, a), under);
                        rgba[o..o + 3].copy_from_slice(&col.map(|v| v.round().clamp(0.0, 255.0) as u8));
                        rgba[o + 3] = (alpha * 255.0).round() as u8;
                    }
                }
            }
        }
        (Skin { origin, w, h, rgba }, kept)
    }
}

/// A light standing near a figure -- a campfire, a torch -- in the frame
/// its skin was drawn in ([`SkinSurface::lamp_light`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lamp {
    /// Where its flame is, world feet.
    pub pos: V3,
    /// Its colour, sRGB.
    pub rgb: [u8; 3],
    /// How far it reaches, feet: it fades smoothly to nothing there.
    pub reach: f32,
    /// 1 = an ordinary fire.
    pub strength: f32,
    /// Shines one way only; None = all round.
    pub beam: Option<Beam>,
}

/// A lamp's beam, for one that shines one way ([`Lamp::beam`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beam {
    /// Which way it points across the floor (world; y is ignored).
    pub dir: V3,
    /// Its full width, and the width of its fading edge, degrees.
    pub spread: f32,
    pub soft: f32,
}

impl Beam {
    /// How much of the beam reaches a point `off` from the lamp, 0..=1, by
    /// its bearing across the floor: 1 inside, fading out over `soft` at
    /// the edge, and 1 at the lamp itself.
    pub fn at(&self, off: V3) -> f32 {
        let (l, dl) = (off[0].hypot(off[2]), self.dir[0].hypot(self.dir[2]));
        if l < 1e-4 || dl < 1e-6 {
            return 1.0;
        }
        let cos = ((off[0] * self.dir[0] + off[2] * self.dir[2]) / (l * dl)).clamp(-1.0, 1.0);
        let t = ((self.spread * 0.5 + self.soft.max(1e-3) * 0.5 - cos.acos().to_degrees()) / self.soft.max(1e-3)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// How far round a surface a lamp's light wraps past the line where it
/// meets the surface edge-on (0 = a hard terminator).
const LAMP_WRAP: f32 = 0.3;
/// A lamp's light on a surface facing it squarely, at the flame, as a
/// multiple of the surface's own colour.
const LAMP_GAIN: f32 = 2.4;
/// The sheen a lamp leaves where the surface mirrors it toward the viewer.
const LAMP_SHEEN: f32 = 0.16;
const LAMP_SHINE: f32 = 18.0;

/// What each pixel of a [`Skin`] shows, from [`Posed::skin_surface`]: the
/// surface point (world feet, the frame the skin was projected from), which
/// way it faces, its own colour before shading, and how much of the pixel
/// the body covers (0 = none of it).
#[derive(Debug, Clone, Default)]
pub struct SkinSurface {
    pub w: usize,
    pub h: usize,
    /// Toward the viewer, world.
    to_cam: V3,
    pos: Vec<V3>,
    /// Unit normals, packed to a byte per axis.
    normal: Vec<[i8; 3]>,
    albedo: Vec<[u8; 3]>,
    cov: Vec<u8>,
}

impl SkinSurface {
    fn blank(w: usize, h: usize, to_cam: V3) -> SkinSurface {
        let n = w * h;
        SkinSurface { w, h, to_cam, pos: vec![[0.0; 3]; n], normal: vec![[0; 3]; n], albedo: vec![[0; 3]; n], cov: vec![0; n] }
    }

    /// The unit normal at pixel `i` (row-major), or None where the body
    /// does not cover it.
    pub fn normal_at(&self, i: usize) -> Option<V3> {
        (self.cov.get(i).copied().unwrap_or(0) > 0).then(|| self.normal[i].map(|v| v as f32 / 127.0))
    }

    fn set(&mut self, i: usize, pos: V3, n: V3, albedo: [f32; 3], cov: f32) {
        self.pos[i] = pos;
        self.normal[i] = n.map(|v| (v.clamp(-1.0, 1.0) * 127.0).round() as i8);
        self.albedo[i] = albedo.map(|v| v.round().clamp(0.0, 255.0) as u8);
        self.cov[i] = (cov * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    /// The light `lamps` add to `skin` (the skin this surface came with, as
    /// it is drawn -- lit by the scene or not): brightest where a surface
    /// faces a flame and is near it, fading to nothing at a lamp's reach,
    /// with a sheen where the surface mirrors a flame toward the viewer.
    ///
    /// RGBA8, PREMULTIPLIED, with alpha 0 everywhere: each pixel's colour is
    /// how far the lamps lift the skin's there (sRGB steps, so that laid
    /// over it by addition it shows `skin + light` in linear light), scaled
    /// by the body's coverage. A premultiplied-alpha compositor draws alpha 0
    /// as pure addition; scaling it (a flicker) dims the lamps' light alone.
    pub fn lamp_light(&self, lamps: &[Lamp], skin: &Skin) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 4];
        if skin.w != self.w || skin.h != self.h || lamps.is_empty() {
            return out;
        }
        let decode = srgb_lut();
        let lamps: Vec<(Lamp, [f32; 3])> =
            lamps.iter().filter(|l| l.reach > 0.0 && l.strength > 0.0).map(|l| (*l, l.rgb.map(|c| decode[c as usize]))).collect();
        for i in 0..self.w * self.h {
            if self.cov[i] == 0 {
                continue;
            }
            let (p, n) = (self.pos[i], self.normal[i].map(|v| v as f32 / 127.0));
            let n = norm_or(n, self.to_cam);
            let albedo = self.albedo[i].map(|c| decode[c as usize]);
            let mut light = [0.0f32; 3];
            for (lamp, rgb) in &lamps {
                let to = sub(lamp.pos, p);
                let d = len(to);
                if d >= lamp.reach {
                    continue;
                }
                let u = d / lamp.reach;
                let reach = (1.0 - u * u).powi(2) * lamp.strength * lamp.beam.map_or(1.0, |b| b.at(sub(p, lamp.pos)));
                let l = if d > 1e-4 { scale(to, 1.0 / d) } else { n };
                let facing = dot(n, l);
                let diffuse = ((facing + LAMP_WRAP) / (1.0 + LAMP_WRAP)).max(0.0);
                let sheen = if facing > 0.0 {
                    dot(n, norm_or(add(l, self.to_cam), n)).max(0.0).powf(LAMP_SHINE) * LAMP_SHEEN
                } else {
                    0.0
                };
                for k in 0..3 {
                    light[k] += rgb[k] * reach * (albedo[k] * diffuse * LAMP_GAIN + sheen);
                }
            }
            if light.iter().all(|v| *v <= 1e-5) {
                continue;
            }
            let o = i * 4;
            let cov = self.cov[i] as f32 / 255.0;
            for k in 0..3 {
                let under = skin.rgba[o + k];
                let lifted = linear_to_srgb8(decode[under as usize] + light[k]);
                out[o + k] = ((lifted.saturating_sub(under)) as f32 * cov).round() as u8;
            }
        }
        out
    }
}

/// sRGB8 -> linear, per level.
fn srgb_lut() -> [f32; 256] {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn linear_to_srgb8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Threads across a webbed figure, each spanning from where it first meets
/// the body to where it last leaves it: over the gaps between an arm and
/// the chest, or between the legs, they bind limb to body. Placement is
/// fixed per family, so the web does not crawl between frames.
fn bind_threads(rgba: &mut [u8], px: &Pixels, family: &str, half_width: f32) {
    let (w, h) = (px.w, px.h);
    let seed = family.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for y in 0..h {
        for x in 0..w {
            if px.cov[y * w + x] >= 0.5 {
                lo = [lo[0].min(x as f32), lo[1].min(y as f32)];
                hi = [hi[0].max(x as f32), hi[1].max(y as f32)];
            }
        }
    }
    if lo[0] > hi[0] {
        return;
    }
    let (cx, cy) = ((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
    let reach = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    for k in 0..BIND_THREADS {
        let angle = hash(k as i32, seed) * std::f32::consts::PI;
        let offset = (hash(k as i32, seed ^ 0xA5A5) - 0.5) * 0.7;
        let (dx, dy) = angle.sin_cos();
        // A point on the line: the box centre, shifted across the line.
        let (ox, oy) = (cx - dy * offset * (hi[0] - lo[0]), cy + dx * offset * (hi[1] - lo[1]));
        // Walk the line; keep the first and last covered pixel.
        let steps = (reach * 2.0) as i32;
        let (mut first, mut last, mut covered) = (None, None, 0u32);
        for s in -steps..=steps {
            let t = s as f32 * 0.5;
            let (x, y) = (ox + dx * t, oy + dy * t);
            if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                continue;
            }
            if px.cov[y as usize * w + x as usize] >= 0.5 {
                first.get_or_insert(t);
                last = Some(t);
                covered += 1;
            }
        }
        let (Some(t0), Some(t1)) = (first, last) else { continue };
        // A thread that is mostly air (tail tip to snout over the back) reads
        // as a tripwire, not a binding: only short bridges over gaps.
        if t1 - t0 < 4.0 || (covered as f32 * 0.5) < (t1 - t0) * 0.6 {
            continue;
        }
        let (ax, ay, bx, by) = (ox + dx * t0, oy + dy * t0, ox + dx * t1, oy + dy * t1);
        let x0 = (ax.min(bx) - half_width - 1.0).max(0.0) as usize;
        let x1 = ((ax.max(bx) + half_width + 1.0) as usize).min(w - 1);
        let y0 = (ay.min(by) - half_width - 1.0).max(0.0) as usize;
        let y1 = ((ay.max(by) + half_width + 1.0) as usize).min(h - 1);
        let len2 = ((bx - ax).powi(2) + (by - ay).powi(2)).max(1e-6);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((qx - ax) * (bx - ax) + (qy - ay) * (by - ay)) / len2).clamp(0.0, 1.0);
                let d = (qx - (ax + (bx - ax) * t)).hypot(qy - (ay + (by - ay) * t));
                let a = (half_width + 0.5 - d).clamp(0.0, 1.0) * 0.8;
                if a <= 0.0 {
                    continue;
                }
                let o = (y * w + x) * 4;
                let under = ([0, 1, 2].map(|c| rgba[o + c] as f32), rgba[o + 3] as f32 / 255.0);
                let (col, alpha) = over((WEB_RGB, a), under);
                rgba[o..o + 3].copy_from_slice(&col.map(|v| v.round().clamp(0.0, 255.0) as u8));
                rgba[o + 3] = (alpha * 255.0).round() as u8;
            }
        }
    }
}
