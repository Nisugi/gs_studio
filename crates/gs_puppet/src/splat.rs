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
//! changed. Every ring is sampled, buried ones too: inside a hull they
//! hide behind its front face, and what pokes out closes the joint. Garments and props
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
/// (finer = more, smaller splats -- and it IS the cloud's resolution: each
/// splat carries one colour, so detail finer than the step is lost, and
/// past the default a figure filling the view wants 0.015); `overlap` how far each splat's
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
    /// through the surface (one standard deviation each way).
    pub axes: [V3; 3],
    /// Outward normal for the cel shading; all zero = unshaded (a fitted
    /// cloud carries its own light).
    pub n: V3,
    pub rgb: [f32; 3],
    pub alpha: f32,
}

/// A fitted cloud from a 3D Gaussian Splatting `.ply` (binary little
/// endian; the reference trainer's and Brush's layout: `x y z`,
/// `f_dc_0..2` (zeroth-order spherical harmonics), `opacity` (a logit),
/// `scale_0..2` (log standard deviations), `rot_0..3` (a quaternion,
/// w first); higher-order harmonics are ignored, so colour is
/// view-independent). Positions come back as they are in the file --
/// see [`fit_to_height`] -- and every Gaussian is unshaded.
pub fn load_ply(path: &std::path::Path) -> std::io::Result<Vec<Gaussian>> {
    use std::io::{BufRead, Read};
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let bad = |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_owned());
    let mut line = String::new();
    let mut props: Vec<String> = Vec::new();
    let mut count = 0usize;
    let mut little = true;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(bad("no end_header"));
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["format", f, ..] => little = f.starts_with("binary_little"),
            ["element", "vertex", n] => count = n.parse().map_err(|_| bad("vertex count"))?,
            ["property", "float", name] | ["property", "float32", name] => props.push((*name).to_owned()),
            ["property", ..] => return Err(bad("only float properties are read")),
            ["end_header"] => break,
            _ => {}
        }
    }
    let at = |name: &str| props.iter().position(|p| p == name);
    let need = |name: &str| at(name).ok_or_else(|| bad(&format!("no property {name}")));
    let (ix, iy, iz) = (need("x")?, need("y")?, need("z")?);
    let (r0, r1, r2) = (need("f_dc_0")?, need("f_dc_1")?, need("f_dc_2")?);
    let iop = need("opacity")?;
    let (s0, s1, s2) = (need("scale_0")?, need("scale_1")?, need("scale_2")?);
    let (q0, q1, q2, q3) = (need("rot_0")?, need("rot_1")?, need("rot_2")?, need("rot_3")?);
    let stride = props.len();
    let mut bytes = vec![0u8; count * stride * 4];
    reader.read_exact(&mut bytes)?;
    let f = |i: usize| {
        let b: [u8; 4] = bytes[i * 4..i * 4 + 4].try_into().expect("4 bytes");
        if little { f32::from_le_bytes(b) } else { f32::from_be_bytes(b) }
    };
    const SH_C0: f32 = 0.282_094_79;
    let mut out = Vec::with_capacity(count);
    for v in 0..count {
        let g = |k: usize| f(v * stride + k);
        let rot = crate::math::quat_to_m3(crate::math::quat_norm([g(q0), g(q1), g(q2), g(q3)]));
        let sc = [g(s0).exp(), g(s1).exp(), g(s2).exp()];
        // Column k of the rotation, scaled: the k-th principal axis.
        let axes = [0, 1, 2].map(|k| [rot[0][k] * sc[k], rot[1][k] * sc[k], rot[2][k] * sc[k]]);
        let rgb = [g(r0), g(r1), g(r2)].map(|c| ((0.5 + SH_C0 * c).clamp(0.0, 1.0)) * 255.0);
        let alpha = 1.0 / (1.0 + (-g(iop)).exp());
        out.push(Gaussian { c: [g(ix), g(iy), g(iz)], axes, n: [0.0; 3], rgb, alpha });
    }
    Ok(out)
}

/// Where a family's fitted cloud lives: `<data>/clouds/<family>.ply`.
pub fn cloud_path(data_root: &std::path::Path, family: &str) -> std::path::PathBuf {
    data_root.join("clouds").join(format!("{family}.ply"))
}

/// A fitted cloud bound to a puppet: each Gaussian follows the body
/// segment whose ring it sits nearest in the rest pose (distance over that
/// ring's radius, plus a little along the bone), exactly as the puppet's
/// own rings do. Hard binding: one segment per Gaussian, so a joint bent
/// far opens a wedge. Built once per puppet; posed per frame.
#[derive(Debug, Clone)]
pub struct BoundCloud {
    /// The cloud in the puppet's rest space.
    pub rest: Vec<Gaussian>,
    /// Per Gaussian, the segment it moves with.
    pub seg: Vec<usize>,
}

impl BoundCloud {
    pub fn bind(puppet: &crate::puppet::Puppet, rest: Vec<Gaussian>) -> BoundCloud {
        let bodies: Vec<(usize, &crate::puppet::Segment, V3)> = puppet
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind.is_body() && !s.hidden)
            .map(|(i, s)| (i, s, norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0])))
            .collect();
        let seg = rest
            .iter()
            .map(|g| {
                let mut best = (0usize, f32::MAX);
                for (i, s, axis) in &bodies {
                    for st in &s.stations {
                        let e = sub(g.c, st.c);
                        let (u, v) = (dot(e, s.axis_w) / st.w.max(0.02), dot(e, s.axis_d) / st.d.max(0.02));
                        let along = dot(e, *axis) / crate::puppet::STATION_STEP_FT;
                        let d = u * u + v * v + 0.25 * along * along;
                        if d < best.1 {
                            best = (*i, d);
                        }
                    }
                }
                best.0
            })
            .collect();
        BoundCloud { rest, seg }
    }

    /// The cloud moved into `posed`'s pose.
    pub fn posed(&self, posed: &Posed<'_>) -> Vec<Gaussian> {
        self.rest
            .iter()
            .zip(&self.seg)
            .map(|(g, &s)| {
                let xf = &posed.xf[s];
                Gaussian { c: xf.point(g.c), axes: g.axes.map(|a| xf.dir(a)), n: g.n, rgb: g.rgb, alpha: g.alpha }
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.rest.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }
}

/// Scale a cloud so it stands `height_ft` tall (its y extent), centred
/// on x and z and with its feet at y = 0 -- what [`load_ply`] hands back
/// is in whatever units it was fitted in.
pub fn fit_to_height(cloud: &mut [Gaussian], height_ft: f32) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for g in cloud.iter() {
        for k in 0..3 {
            lo[k] = lo[k].min(g.c[k]);
            hi[k] = hi[k].max(g.c[k]);
        }
    }
    let span = hi[1] - lo[1];
    if !(span > 1e-6) {
        return;
    }
    let s = height_ft / span;
    let centre = [(lo[0] + hi[0]) * 0.5, lo[1], (lo[2] + hi[2]) * 0.5];
    for g in cloud.iter_mut() {
        g.c = scale(sub(g.c, centre), s);
        g.axes = g.axes.map(|a| scale(a, s));
    }
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
        // Every ring, buried ones too: a thigh's top in the pelvis, a
        // socket stub. Inside a closed hull they are hidden by its own
        // front face once the splats are near opaque, and the part that
        // pokes out is what closes a crotch or an armpit. Culling them by
        // geometry (tried, per ring and per sample) left notches or spikes
        // at every joint.
        for k in 0..n {
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
        let shade = if g.n == [0.0; 3] {
            // Unshaded: a fitted cloud's colours carry their own light.
            1.0
        } else {
            let gb = dot(g.n, light);
            let lit = crate::skin::smoothstep(crate::draw::LIGHT_ABOVE - 0.09, crate::draw::LIGHT_ABOVE + 0.09, gb);
            let dark = 1.0 - crate::skin::smoothstep(crate::draw::DARK_BELOW - 0.09, crate::draw::DARK_BELOW + 0.09, gb);
            (1.0 + 0.13 * lit - 0.2 * dark) * (1.0 + 0.08 * gb)
        };
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
