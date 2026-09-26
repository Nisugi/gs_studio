//! A posed puppet as a Gaussian splat cloud, and a CPU renderer for it.
//!
//! Every ring of every body segment is sampled round its circumference
//! and each sample becomes one anisotropic Gaussian: long along the ring
//! and along the bone (`overlap` times the spacing each way; at the default
//! 0.6 neighbours overlap enough to close and not enough to blur), thin
//! through the surface. Its
//! colour comes from the painted sheet where one fits, else the body
//! colour. [`render`] projects the cloud, sorts it back to front and
//! alpha-blends the 2D Gaussians into a [`Skin`]-shaped image, so a host
//! can show the splat look in place of the smooth skin with nothing else
//! changed. Buried rings (a thigh in the pelvis, a socket stub) are left
//! out: a surface hides them, a cloud would show them. Wounds, status
//! effects and the contour are not drawn -- this is the look test, not the
//! stage's renderer.

use crate::math::{add, cross, dot, len, mul_mv, norm_or, rot_y, scale, sub, V3};
use crate::pose::Posed;
use crate::skin::Skin;
use crate::uv::Texture;

/// Default sample spacing along a ring and between rings, feet.
pub const SPACING_FT: f32 = 0.03;

/// What shapes the cloud. `spacing` is the sample step along a ring, feet
/// (finer = more, smaller splats); `overlap` how far each splat's
/// half-extent reaches past its step (0.5 = neighbours just touch, more =
/// softer and denser); `alpha` each splat's peak opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplatParams {
    pub spacing: f32,
    pub overlap: f32,
    pub alpha: f32,
}

impl Default for SplatParams {
    fn default() -> Self {
        SplatParams { spacing: SPACING_FT, overlap: 0.6, alpha: 0.9 }
    }
}

#[derive(Debug, Clone)]
pub struct Gaussian {
    pub c: V3,
    /// Half-extents as world vectors: along the ring, along the bone,
    /// through the surface.
    pub axes: [V3; 3],
    pub n: V3,
    pub rgb: [f32; 3],
    pub alpha: f32,
}

/// The cloud for a posed puppet, sampled every `spacing` feet, wearing
/// `texture` where it fits (its layout must be the puppet's own), else
/// `body`.
pub fn cloud(posed: &Posed<'_>, texture: Option<&Texture>, body: [u8; 3], params: &SplatParams) -> Vec<Gaussian> {
    let spacing = params.spacing.max(0.005);
    let overlap = params.overlap.clamp(0.2, 2.0);
    let alpha = params.alpha.clamp(0.05, 1.0);
    let body = body.map(|c| c as f32);
    let puppet = posed.puppet;
    let mut out = Vec::new();
    for (i, s) in puppet.segments.iter().enumerate() {
        if s.hidden || !s.kind.is_body() {
            continue;
        }
        let xf = &posed.xf[i];
        let n = s.stations.len();
        for (k, st) in s.stations.iter().enumerate() {
            if k < s.bare {
                continue;
            }
            let c = xf.point(st.c);
            let a = scale(xf.dir(s.axis_w), st.w);
            let b = scale(xf.dir(s.axis_d), st.d);
            // Along the bone: half the gap to the neighbouring rings.
            let along = {
                let prev = if k > 0 { xf.point(s.stations[k - 1].c) } else { c };
                let next = if k + 1 < n { xf.point(s.stations[k + 1].c) } else { c };
                scale(sub(next, prev), 0.5)
            };
            let along = if len(along) < 1e-4 { scale(norm_or(xf.dir(sub(s.tail, s.head)), [0.0, 1.0, 0.0]), spacing * 0.5) } else { along };
            // Ramanujan's ellipse perimeter.
            let circ = std::f32::consts::PI * (3.0 * (st.w + st.d) - ((3.0 * st.w + st.d) * (st.w + 3.0 * st.d)).sqrt());
            let m = ((circ / spacing).ceil() as usize).clamp(6, 400);
            for j in 0..m {
                let theta = j as f32 / m as f32 * std::f32::consts::TAU;
                let (sin, cos) = theta.sin_cos();
                let radial = add(scale(a, cos), scale(b, sin));
                let p = add(c, radial);
                let tangent = sub(scale(b, cos), scale(a, sin));
                let nrm = norm_or(cross(tangent, along), norm_or(radial, [0.0, 0.0, 1.0]));
                let nrm = if dot(nrm, radial) < 0.0 { scale(nrm, -1.0) } else { nrm };
                let rgb = texture.and_then(|t| t.layout.locate(i, k as f32, theta).and_then(|at| t.sample(at))).unwrap_or(body);
                let half_ring = scale(norm_or(tangent, [1.0, 0.0, 0.0]), circ / m as f32 * overlap);
                out.push(Gaussian { c: p, axes: [half_ring, scale(along, overlap), scale(nrm, spacing * 0.15)], n: nrm, rgb, alpha });
            }
        }
    }
    out
}

/// Draw `cloud` through `project` (world -> pixels and depth), the figure
/// turned by `yaw` degrees as [`crate::Look::yaw`] turns it, lit from
/// above and in front. The image covers the cloud's projected extent (plus
/// its blur) and is placed at `origin` in the projection's pixel space,
/// straight RGBA, transparent where nothing landed.
pub fn render(cloud: &[Gaussian], project: &dyn Fn(V3) -> ([f32; 2], f32), yaw: f32) -> Skin {
    if cloud.is_empty() {
        return Skin { origin: [0.0, 0.0], w: 0, h: 0, rgba: Vec::new() };
    }
    let rot = rot_y(yaw);
    let turned = |v: V3| mul_mv(&rot, v);
    let proj = |p: V3| project(turned(p));
    // Each splat's screen centre, 2D covariance and depth.
    struct Flat {
        c: [f32; 2],
        inv: [[f32; 2]; 2],
        r: [f32; 2],
        z: f32,
        shade: f32,
        i: usize,
    }
    let light = norm_or([0.3, 0.8, 0.6], [0.0, 1.0, 0.0]);
    let mut flats: Vec<Flat> = Vec::with_capacity(cloud.len());
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for (i, g) in cloud.iter().enumerate() {
        let (centre, z) = proj(g.c);
        if !(centre[0].is_finite() && centre[1].is_finite()) {
            continue;
        }
        let mut cov = [[0.0f32; 2]; 2];
        for ax in g.axes {
            let (q, _) = proj(add(g.c, ax));
            let v = [q[0] - centre[0], q[1] - centre[1]];
            for r in 0..2 {
                for c in 0..2 {
                    cov[r][c] += v[r] * v[c];
                }
            }
        }
        // A splat covers at least a pixel.
        cov[0][0] += 0.3;
        cov[1][1] += 0.3;
        let det = cov[0][0] * cov[1][1] - cov[0][1] * cov[1][0];
        if det <= 1e-6 {
            continue;
        }
        let inv = [[cov[1][1] / det, -cov[0][1] / det], [-cov[1][0] / det, cov[0][0] / det]];
        let r = [3.0 * cov[0][0].sqrt(), 3.0 * cov[1][1].sqrt()];
        lo = [lo[0].min(centre[0] - r[0]), lo[1].min(centre[1] - r[1])];
        hi = [hi[0].max(centre[0] + r[0]), hi[1].max(centre[1] + r[1])];
        let shade = 0.55 + 0.45 * dot(turned(g.n), light).max(0.0);
        flats.push(Flat { c: centre, inv, r, z, shade, i });
    }
    if flats.is_empty() || !(lo[0].is_finite() && hi[0].is_finite()) {
        return Skin { origin: [0.0, 0.0], w: 0, h: 0, rgba: Vec::new() };
    }
    let origin = [lo[0].floor(), lo[1].floor()];
    let w = ((hi[0] - origin[0]).ceil() as usize + 1).min(4096);
    let h = ((hi[1] - origin[1]).ceil() as usize + 1).min(4096);
    // Back to front, premultiplied over transparent.
    flats.sort_by(|a, b| b.z.total_cmp(&a.z));
    let mut fb = vec![[0.0f32; 4]; w * h];
    for f in &flats {
        let g = &cloud[f.i];
        let (cx, cy) = (f.c[0] - origin[0], f.c[1] - origin[1]);
        let xa = ((cx - f.r[0]).floor() as i64).max(0);
        let xb = ((cx + f.r[0]).ceil() as i64).min(w as i64 - 1);
        let ya = ((cy - f.r[1]).floor() as i64).max(0);
        let yb = ((cy + f.r[1]).ceil() as i64).min(h as i64 - 1);
        for py in ya..=yb {
            for px in xa..=xb {
                let d = [px as f32 + 0.5 - cx, py as f32 + 0.5 - cy];
                let m = d[0] * (f.inv[0][0] * d[0] + f.inv[0][1] * d[1]) + d[1] * (f.inv[1][0] * d[0] + f.inv[1][1] * d[1]);
                if m > 9.0 {
                    continue;
                }
                let a = (g.alpha * (-0.5 * m).exp()).min(0.99);
                let dst = &mut fb[py as usize * w + px as usize];
                for c in 0..3 {
                    dst[c] = g.rgb[c] * f.shade * a + dst[c] * (1.0 - a);
                }
                dst[3] = a + dst[3] * (1.0 - a);
            }
        }
    }
    let mut rgba = vec![0u8; w * h * 4];
    for (p, px) in fb.iter().zip(rgba.chunks_exact_mut(4)) {
        if p[3] > 1e-3 {
            for c in 0..3 {
                px[c] = (p[c] / p[3]).round().clamp(0.0, 255.0) as u8;
            }
            px[3] = (p[3] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Skin { origin, w, h, rgba }
}
