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
//! out: a surface hides them, a cloud would show them. Garments and props
//! are sampled like the body (a prop in its own colour). Wounds, status
//! effects and the contour are not drawn -- this is the look test, not the
//! stage's renderer.

use crate::math::{add, cross, dot, len, lerp as lerp3, mul_mv, norm_or, rot_y, scale, sub, V3};
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
    // Every body ring in posed space, for the inside test below: a sample
    // that lands inside another segment's tube (a neck ring the side view
    // read as wide as the hunch, a thigh's top in the pelvis) is interior
    // geometry a surface hides and a cloud would show.
    struct Tube {
        c: V3,
        axis: V3,
        w: V3,
        d: V3,
        hw: f32,
        hd: f32,
        seg: usize,
    }
    let mut tubes: Vec<Tube> = Vec::new();
    for (i, s) in puppet.segments.iter().enumerate() {
        if s.hidden || !s.kind.is_body() {
            continue;
        }
        let xf = &posed.xf[i];
        let axis = norm_or(xf.dir(sub(s.tail, s.head)), [0.0, 1.0, 0.0]);
        for st in &s.stations {
            tubes.push(Tube { c: xf.point(st.c), axis, w: xf.dir(s.axis_w), d: xf.dir(s.axis_d), hw: st.w, hd: st.d, seg: i });
        }
    }
    let inside_other = |p: V3, seg: usize| {
        tubes.iter().any(|t| {
            if t.seg == seg {
                return false;
            }
            let e = sub(p, t.c);
            if dot(e, t.axis).abs() > crate::puppet::STATION_STEP_FT {
                return false;
            }
            let (u, v) = (dot(e, t.w) / t.hw.max(1e-3), dot(e, t.d) / t.hd.max(1e-3));
            u * u + v * v < 0.45
        })
    };
    let mut out = Vec::new();
    for (i, s) in puppet.segments.iter().enumerate() {
        // Garments and props are drawn too: a robe hides the legs under
        // it, so a cloud without it has holes where the skin has cloth.
        if s.hidden {
            continue;
        }
        let own = match s.kind {
            crate::Kind::Prop { color } => Some(color.map(|c| c as f32)),
            _ => None,
        };
        let xf = &posed.xf[i];
        let n = s.stations.len();
        let aw = xf.dir(s.axis_w);
        let ad = xf.dir(s.axis_d);
        let bone = norm_or(xf.dir(sub(s.tail, s.head)), [0.0, 1.0, 0.0]);
        // Rings sit STATION_STEP_FT apart, coarser than the samples round
        // them; sub-rings are interpolated between neighbours so the cloud
        // is as even along the bone as round it (no streaks, no bands).
        for k in s.bare.max(0)..n {
            let st = &s.stations[k];
            let next = s.stations.get(k + 1);
            let c0 = xf.point(st.c);
            let gap = next.map_or(0.0, |nx| len(sub(xf.point(nx.c), c0)));
            let subs = if next.is_some() { ((gap / spacing).round() as usize).max(1) } else { 1 };
            for sub_k in 0..subs {
                let t = sub_k as f32 / subs as f32;
                let (c, hw, hd) = match next {
                    Some(nx) if t > 0.0 => (lerp3(c0, xf.point(nx.c), t), st.w + (nx.w - st.w) * t, st.d + (nx.d - st.d) * t),
                    _ => (c0, st.w, st.d),
                };
                let a = scale(aw, hw);
                let b = scale(ad, hd);
                let along_len = if gap > 1e-4 { gap / subs as f32 } else { spacing };
                let along = scale(bone, along_len * 0.5);
                // Ramanujan's ellipse perimeter.
                let circ = std::f32::consts::PI * (3.0 * (hw + hd) - ((3.0 * hw + hd) * (hw + 3.0 * hd)).sqrt());
                let m = ((circ / spacing).ceil() as usize).clamp(6, 400);
                for j in 0..m {
                    let theta = j as f32 / m as f32 * std::f32::consts::TAU;
                    let (sin, cos) = theta.sin_cos();
                    let radial = add(scale(a, cos), scale(b, sin));
                    let p = add(c, radial);
                    // Deep inside another segment only, and never at a
                    // segment's ends: the junction rings are what closes
                    // a shoulder or a crotch, and culling them left holes.
                    let junction = k < s.bare + 2 || k + 3 >= n;
                    if s.kind.is_body() && !junction && inside_other(p, i) {
                        continue;
                    }
                    let tangent = sub(scale(b, cos), scale(a, sin));
                    let nrm = norm_or(cross(tangent, along), norm_or(radial, [0.0, 0.0, 1.0]));
                    let nrm = if dot(nrm, radial) < 0.0 { scale(nrm, -1.0) } else { nrm };
                    let rgb = own
                        .or_else(|| texture.and_then(|tx| tx.layout.locate(i, k as f32 + t, theta).and_then(|at| tx.sample(at))))
                        .unwrap_or(body);
                    let half_ring = scale(norm_or(tangent, [1.0, 0.0, 0.0]), circ / m as f32 * overlap);
                    out.push(Gaussian { c: p, axes: [half_ring, scale(along, 2.0 * overlap), scale(nrm, spacing * 0.15)], n: nrm, rgb, alpha });
                }
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
    // The skin's own light and cel bands (`skin.rs`), so the cloud reads
    // as bright as the skin beside it instead of a muted copy.
    let centre = {
        let n = cloud.len() as f32;
        cloud.iter().fold([0.0f32; 3], |a, g| add(a, scale(g.c, 1.0 / n)))
    };
    let view = crate::draw::view_basis(&proj, centre);
    let light = view.light;
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
        let gb = dot(g.n, light);
        let lit = crate::skin::smoothstep(crate::draw::LIGHT_ABOVE - 0.09, crate::draw::LIGHT_ABOVE + 0.09, gb);
        let dark = 1.0 - crate::skin::smoothstep(crate::draw::DARK_BELOW - 0.09, crate::draw::DARK_BELOW + 0.09, gb);
        let shade = (1.0 + 0.13 * lit - 0.2 * dark) * (1.0 + 0.08 * gb);
        flats.push(Flat { c: centre, inv, r, z, shade, i });
    }
    if flats.is_empty() || !(lo[0].is_finite() && hi[0].is_finite()) {
        return Skin { origin: [0.0, 0.0], w: 0, h: 0, rgba: Vec::new() };
    }
    let (near, far) = flats.iter().fold((f32::MAX, f32::MIN), |(n, f), x| (n.min(x.z), f.max(x.z)));
    let range = (far - near).max(2.0);
    for f in &mut flats {
        f.shade *= 1.0 - crate::draw::DEPTH_SHADE * ((f.z - near) / range).clamp(0.0, 1.0);
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
