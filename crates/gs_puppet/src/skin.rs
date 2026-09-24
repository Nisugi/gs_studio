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
//!   it only where a DIFFERENT segment sits clearly nearer.
//!
//! Shading is the shape list's cel look (a light fixed to the view, a lit
//! and a shaded band), softened at the band edges. Everything is egui-free;
//! the result is straight RGBA for any host to upload as a texture.

use crate::draw::{surfaces, view_basis, Look, BOSS_OUTLINE, BODY_DEAD, DARK_BELOW, DEPTH_SHADE, EYE, LIGHT_ABOVE};
use crate::math::{add, cross, dot, len, mul_mv, norm_or, rot_y, scale, sub, V3};
use crate::pose::Posed;
use crate::uv::Texture;

/// Samples per pixel along each axis while rasterising.
const SS: usize = 2;
/// No skin is rendered larger than this on a side.
const MAX_SIDE: usize = 4096;
/// Normals and colours are averaged over a window this fraction of the
/// figure's height across.
const SMOOTH_OF_HEIGHT: f32 = 0.035;
/// Neighbours further apart in depth than this (fraction of height) are
/// different surfaces and never blend.
const BLEND_DEPTH_OF_HEIGHT: f32 = 0.04;
/// A different segment nearer by more than this (fraction of height) draws
/// a contour on the far one.
const CONTOUR_DEPTH_OF_HEIGHT: f32 = 0.035;
/// Half-width of the softened step between tone bands, in `dot(n, light)`.
const BAND_SOFT: f32 = 0.09;
const LIT_TONE: f32 = 1.13;
const DARK_TONE: f32 = 0.8;
/// Strength of the continuous light term under the bands.
const LAMBERT: f32 = 0.08;
/// Extra width of the boss contour past the ordinary one, pixels.
const BOSS_EXTRA_PX: f32 = 2.5;

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
}

impl Samples {
    fn new(w: usize, h: usize) -> Samples {
        let n = w * h;
        Samples { w, h, z: vec![f32::INFINITY; n], n: vec![[0.0; 3]; n], col: vec![[0.0; 3]; n], seg: vec![u32::MAX; n] }
    }

    fn tri(&mut self, v: [&Vert; 3], tex: Option<&Texture>) {
        // A textured triangle interpolates its corners' ADDRESSES and looks
        // each sample up on the sheet. Angles are unwrapped first, so a
        // triangle straddling angle 0 does not sweep the whole ring.
        let tex = tex.filter(|_| v.iter().all(|c| c.addr[0].is_finite() && c.addr[1].is_finite()));
        let mut theta = [v[0].addr[1], v[1].addr[1], v[2].addr[1]];
        if tex.is_some() {
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
                let lerp = |f: fn(&Vert) -> f32| w0 * f(v[0]) + w1 * f(v[1]) + w2 * f(v[2]);
                let painted = tex.and_then(|t| {
                    let station = lerp(|c| c.addr[0]);
                    let angle = w0 * theta[0] + w1 * theta[1] + w2 * theta[2];
                    t.sample(t.layout.locate(v[0].seg as usize, station, angle)?)
                });
                self.col[i] = match painted {
                    Some(rgb) => {
                        let wound = [lerp(|c| c.wound[0]), lerp(|c| c.wound[1]), lerp(|c| c.wound[2])];
                        let a = lerp(|c| c.wound[3]);
                        [0, 1, 2].map(|k| rgb[k] + (wound[k] - rgb[k]) * a)
                    }
                    None => [0, 1, 2].map(|k| w0 * v[0].col[k] + w1 * v[1].col[k] + w2 * v[2].col[k]),
                };
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
        };
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let mut hits = 0;
                let (mut n, mut col) = ([0.0f32; 3], [0.0f32; 3]);
                for sy in 0..SS {
                    for sx in 0..SS {
                        let j = (y * SS + sy) * s.w + x * SS + sx;
                        if !s.z[j].is_finite() {
                            continue;
                        }
                        hits += 1;
                        n = add(n, norm_or(s.n[j], [0.0, 0.0, 1.0]));
                        for k in 0..3 {
                            col[k] += s.col[j][k];
                        }
                        if s.z[j] < out.z[i] {
                            out.z[i] = s.z[j];
                            out.seg[i] = s.seg[j];
                        }
                    }
                }
                if hits > 0 {
                    out.cov[i] = hits as f32 / (SS * SS) as f32;
                    out.n[i] = norm_or(n, [0.0, 0.0, 1.0]);
                    out.col[i] = col.map(|c| c / hits as f32);
                }
            }
        }
        out
    }

    /// Average normals (and, with `colours`, colours) over a `2r + 1`
    /// window (one axis per pass), only between pixels whose depths are
    /// within `tol`. A texture's colours are left sharp: blending is for
    /// the shape, and for softening flat wound tints.
    fn smooth(&mut self, r: usize, tol: f32, colours: bool) {
        if r == 0 {
            return;
        }
        for (dx, dy) in [(1isize, 0isize), (0, 1)] {
            let (mut n2, mut c2) = (self.n.clone(), self.col.clone());
            for y in 0..self.h {
                for x in 0..self.w {
                    let i = y * self.w + x;
                    let z0 = self.z[i];
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
                        if (self.z[j] - z0).abs() > tol {
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
    fn inner_contour(&self, width: f32, tol: f32) -> Vec<f32> {
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
                        if self.seg[j] == s0 || self.z[j] >= z0 - tol {
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

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
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
        // A sheet laid out for another puppet would put paint anywhere.
        let texture = texture.filter(|t| t.layout.patches.len() == self.puppet.segments.len());
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
                let addressed = !ring.red && k < measured;
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
                    verts.push(Vert { p: [sp[0] * ss, sp[1] * ss], z, n: nrm, col, seg: surf.index as u32, addr, wound });
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
            return Skin::default();
        }
        let origin = [(lo[0] - margin).floor(), (lo[1] - margin).floor()];
        let w = ((hi[0] + margin).ceil() - origin[0]) as usize;
        let h = ((hi[1] + margin).ceil() - origin[1]) as usize;
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Skin::default();
        }
        for v in &mut verts {
            v.p = [v.p[0] - origin[0] * ss, v.p[1] - origin[1] * ss];
        }

        // ---- rasterise, resolve, smooth
        let mut samples = Samples::new(w * SS, h * SS);
        for t in &tris {
            let v = t.map(|i| &verts[i as usize]);
            if v.iter().all(|c| c.p[0].is_finite() && c.p[1].is_finite() && c.z.is_finite()) {
                samples.tri(v, texture);
            }
        }
        let mut px = Pixels::resolve(&samples, w, h);
        drop(samples);
        let px_per_ft = {
            let mid = [0.0, height * 0.5, 0.0];
            let (a, _) = project(mid);
            let (b, _) = project(add(mid, [0.0, 1.0, 0.0]));
            (a[0] - b[0]).hypot(a[1] - b[1])
        };
        let radius = (SMOOTH_OF_HEIGHT * height * px_per_ft).round().clamp(0.0, 16.0) as usize;
        px.smooth(radius, BLEND_DEPTH_OF_HEIGHT * height, texture.is_none());

        // ---- shade and composite
        let (near, far) =
            px.z.iter().filter(|z| z.is_finite()).fold((f32::MAX, f32::MIN), |(n, f), z| (n.min(*z), f.max(*z)));
        let range = (far - near).max(2.0);
        let outside = px.outside_distance();
        let inner = px.inner_contour(look.outline_px, CONTOUR_DEPTH_OF_HEIGHT * height);
        let gold = BOSS_OUTLINE.map(|c| c as f32);
        let mut rgba = vec![0u8; w * h * 4];
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
                let lit = smoothstep(LIGHT_ABOVE - BAND_SOFT, LIGHT_ABOVE + BAND_SOFT, g);
                let dark = 1.0 - smoothstep(DARK_BELOW - BAND_SOFT, DARK_BELOW + BAND_SOFT, g);
                // The cel bands, plus a gentle continuous term so a form still
                // turns between them.
                let tone = (1.0 + (LIT_TONE - 1.0) * lit - (1.0 - DARK_TONE) * dark) * (1.0 + LAMBERT * g);
                let dim = 1.0 - DEPTH_SHADE * ((px.z[i] - near) / range).clamp(0.0, 1.0);
                let mut c = px.col[i].map(|v| (v * tone * dim).clamp(0.0, 255.0));
                if inner[i] > 0.0 {
                    c = [0, 1, 2].map(|k| c[k] + (line[k] - c[k]) * inner[i]);
                }
                top = (c, px.cov[i]);
            }
            let (c, a) = over(top, under);
            let o = i * 4;
            rgba[o..o + 3].copy_from_slice(&c.map(|v| v.round().clamp(0.0, 255.0) as u8));
            rgba[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }

        // ---- eyes, where the surface they sit on is the one that shows
        if !look.dead {
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
                if px.z[cy as usize * w + cx as usize] < z - CONTOUR_DEPTH_OF_HEIGHT * height {
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
        Skin { origin, w, h, rgba }
    }
}
