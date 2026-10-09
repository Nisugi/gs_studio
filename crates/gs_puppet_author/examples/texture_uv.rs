//! texture_uv: the disciple on the puppet, with a real surface map.
//!
//!   cargo run -p gs_puppet --example texture_uv -- <art.png> <out_dir>
//!
//! The difference from `texture_test`: colour is fixed to a point ON THE
//! SURFACE, not to a primitive. Every quad between two stations carries its
//! four corners' `(segment, t, theta)`; that triple is sampled ONCE against
//! the artwork in the rest pose and then travels with the surface through
//! any pose. So posing cannot break the correspondence -- which is exactly
//! what defeated the per-primitive probe.
//!
//! Sampling is per-corner and the quad is Gouraud-filled, so the banding of
//! one-flat-colour-per-quad is gone too.
//!
//! Still a probe: the map is a planar front projection, so it is honest near
//! the front and smears round the sides. A back view would need a second
//! sample (a back silhouette, or mirroring the front).

use gs_puppet_author::prelude::*;
use std::path::{Path, PathBuf};

use gs_puppet::draw::Look;
use gs_puppet_author::fit::ortho;
use gs_puppet::math::{add, cross, dot, mul_mv, norm_or, rot_y, scale, sub, V3};
use gs_puppet::pose::Posed;
use gs_puppet::{Pose, PoseSet, Puppet};
use gs_puppet_author::{MASK_PX};
use rig::{Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
/// Samples around each station. More = smoother, and costs only sampling.
const AROUND: usize = 24;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

struct Art {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Art {
    fn load(path: &Path) -> Result<Art, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let (w, h) = (info.width as usize, info.height as usize);
        let px = match (info.color_type, info.bit_depth) {
            (png::ColorType::Rgba, png::BitDepth::Eight) => buf[..w * h * 4].to_vec(),
            (png::ColorType::Rgb, png::BitDepth::Eight) => {
                buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect()
            }
            other => return Err(format!("unsupported png format {other:?}")),
        };
        Ok(Art { w, h, px })
    }

    fn opaque(&self, x: usize, y: usize) -> bool {
        self.px[(y * self.w + x) * 4 + 3] > 128
    }

    /// Nearest opaque pixel to `p`, searched outward in rings. The artwork's
    /// silhouette is not the puppet's, so a surface point can land just off
    /// the art (a lean forearm, a raised hand); taking the nearest real pixel
    /// beats both a hole and a stretched edge colour.
    fn sample_near(&self, p: [f32; 2], max_r: i32) -> Option<[u8; 3]> {
        let (cx, cy) = (p[0].round() as i32, p[1].round() as i32);
        for r in 0..=max_r {
            let mut best: Option<(i32, [u8; 3])> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    // Only the ring's perimeter, except on the first pass.
                    if r > 0 && dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
                        continue;
                    }
                    let (x, y) = (x as usize, y as usize);
                    if !self.opaque(x, y) {
                        continue;
                    }
                    let d2 = dx * dx + dy * dy;
                    let i = (y * self.w + x) * 4;
                    let rgb = [self.px[i], self.px[i + 1], self.px[i + 2]];
                    if best.is_none_or(|(bd, _)| d2 < bd) {
                        best = Some((d2, rgb));
                    }
                }
            }
            if let Some((_, rgb)) = best {
                return Some(rgb);
            }
        }
        None
    }

    fn bounds(&self) -> (usize, usize, usize, usize) {
        let (mut x0, mut y0, mut x1, mut y1) = (self.w, self.h, 0usize, 0usize);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.opaque(x, y) {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x0, y0, x1, y1)
    }
}

/// One corner of a surface quad: where it is now, and what colour it wears.
#[derive(Clone, Copy)]
struct Vtx {
    p: [f32; 2],
    depth: f32,
    rgb: [u8; 3],
    /// Lambert term from the rest-pose normal, so form still reads.
    lum: f32,
}

/// A Gouraud-shaded triangle: the only thing the canvas below can draw.
struct Tri {
    v: [Vtx; 3],
    depth: f32,
}

/// The surface of one posed puppet, as world-space points and normals.
/// `[segment][station][around]`.
fn surface(posed: &Posed, yaw: f32) -> Vec<Vec<Vec<(V3, V3)>>> {
    let turn = rot_y(yaw);
    let mut out = Vec::with_capacity(posed.puppet.segments.len());
    for (s, x) in posed.puppet.segments.iter().zip(&posed.xf) {
        let (aw, ad) = (mul_mv(&turn, x.dir(s.axis_w)), mul_mv(&turn, x.dir(s.axis_d)));
        let mut rings = Vec::with_capacity(s.stations.len());
        for st in &s.stations {
            let c = mul_mv(&turn, x.point(st.c));
            let (a, b) = (scale(aw, st.w), scale(ad, st.d));
            let ring: Vec<(V3, V3)> = (0..AROUND)
                .map(|j| {
                    let (sn, cs) = (j as f32 / AROUND as f32 * std::f32::consts::TAU).sin_cos();
                    let p = add(c, add(scale(a, cs), scale(b, sn)));
                    // Surface normal of an ellipse: radial, scaled by the
                    // reciprocal semi-axes, so a flat cross-section is right.
                    let n = norm_or(
                        add(scale(aw, cs / st.w.max(1e-3)), scale(ad, sn / st.d.max(1e-3))),
                        sub(p, c),
                    );
                    (p, n)
                })
                .collect();
            rings.push(ring);
        }
        out.push(rings);
    }
    out
}

/// Sample the artwork once, in the rest pose, per surface point. Returns
/// `[segment][station][around]` colours plus the rest-pose luminance.
fn bake_uv(
    puppet: &Puppet,
    art: &Art,
    project: &dyn Fn(V3) -> ([f32; 2], f32),
    lift: f32,
    light: V3,
    to_cam: V3,
) -> (Vec<Vec<Vec<[u8; 3]>>>, Vec<Vec<Vec<f32>>>, usize, usize) {
    let rest = puppet.pose(&Pose::default());
    let surf = surface(&rest, 0.0);
    let (mut hit, mut miss) = (0usize, 0usize);
    let mut colors = Vec::with_capacity(surf.len());
    let mut lums = Vec::with_capacity(surf.len());
    for seg in &surf {
        let mut cs = Vec::with_capacity(seg.len());
        let mut ls = Vec::with_capacity(seg.len());
        for ring in seg {
            let mut cr = Vec::with_capacity(ring.len());
            let mut lr = Vec::with_capacity(ring.len());
            for (p, n) in ring {
                // Only front-facing points see the front artwork. A point on
                // the far side is sampled at the same screen position -- a
                // planar projection wrapping round -- which is the documented
                // limitation, not a bug.
                let (sp, _) = project([p[0], p[1] - lift, p[2]]);
                match art.sample_near(sp, 24) {
                    Some(rgb) => {
                        cr.push(rgb);
                        hit += 1;
                    }
                    None => {
                        cr.push([0x9a, 0x7a, 0x5c]);
                        miss += 1;
                    }
                }
                // Lambert + a little ambient, clamped so nothing goes black.
                let lam = dot(*n, light).max(0.0);
                let facing = dot(*n, to_cam).max(0.0);
                lr.push((0.55 + 0.45 * lam) * (0.75 + 0.25 * facing));
            }
            cs.push(cr);
            ls.push(lr);
        }
        colors.push(cs);
        lums.push(ls);
    }
    (colors, lums, hit, miss)
}

/// Build the triangle list for a posed figure using the baked colours.
fn triangles(
    posed: &Posed,
    colors: &[Vec<Vec<[u8; 3]>>],
    lums: &[Vec<Vec<f32>>],
    project: &dyn Fn(V3) -> ([f32; 2], f32),
    lift: f32,
    yaw: f32,
    to_cam: V3,
) -> Vec<Tri> {
    let surf = surface(posed, yaw);
    let mut tris = Vec::new();
    for (si, seg) in surf.iter().enumerate() {
        for k in 0..seg.len().saturating_sub(1) {
            for j in 0..AROUND {
                let j1 = (j + 1) % AROUND;
                let corners = [(k, j), (k, j1), (k + 1, j1), (k + 1, j)];
                let mut v = [Vtx { p: [0.0; 2], depth: 0.0, rgb: [0; 3], lum: 1.0 }; 4];
                let mut ok = true;
                let mut facing = 0.0f32;
                for (n, (kk, jj)) in corners.into_iter().enumerate() {
                    let (wp, nrm) = seg[kk][jj];
                    let (p, d) = project([wp[0], wp[1] - lift, wp[2]]);
                    if !(p[0].is_finite() && p[1].is_finite() && d.is_finite()) {
                        ok = false;
                        break;
                    }
                    facing += dot(nrm, to_cam);
                    v[n] = Vtx {
                        p,
                        depth: d,
                        rgb: colors[si][kk][jj],
                        lum: lums[si][kk][jj],
                    };
                }
                // Back-facing quads are hidden by the near surface anyway,
                // and dropping them halves the work.
                if !ok || facing <= 0.0 {
                    continue;
                }
                let depth = v.iter().map(|c| c.depth).sum::<f32>() / 4.0;
                tris.push(Tri { v: [v[0], v[1], v[2]], depth });
                tris.push(Tri { v: [v[0], v[2], v[3]], depth });
            }
        }
    }
    // Painter's algorithm, per triangle: far to near.
    tris.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    tris
}

/// A z-buffered Gouraud canvas. The crate's own `Canvas` fills flat convex
/// polygons; interpolating colour and depth per pixel needs this.
struct Shaded {
    w: usize,
    h: usize,
    ss: usize,
    color: Vec<[f32; 3]>,
    z: Vec<f32>,
}

impl Shaded {
    fn new(w: usize, h: usize, ss: usize, bg: [u8; 3]) -> Shaded {
        let n = w * ss * h * ss;
        Shaded { w, h, ss, color: vec![bg.map(|c| c as f32); n], z: vec![f32::MAX; n] }
    }

    fn tri(&mut self, t: &Tri) {
        let s = self.ss as f32;
        let p: Vec<[f32; 2]> = t.v.iter().map(|v| [v.p[0] * s, v.p[1] * s]).collect();
        let (sw, sh) = (self.w * self.ss, self.h * self.ss);
        let min = p.iter().fold([f32::MAX; 2], |m, q| [m[0].min(q[0]), m[1].min(q[1])]);
        let max = p.iter().fold([f32::MIN; 2], |m, q| [m[0].max(q[0]), m[1].max(q[1])]);
        if !min.iter().chain(&max).all(|v| v.is_finite()) {
            return;
        }
        let x0 = min[0].floor().max(0.0) as usize;
        let y0 = min[1].floor().max(0.0) as usize;
        let x1 = (max[0].ceil() as usize).min(sw.saturating_sub(1));
        let y1 = (max[1].ceil() as usize).min(sh.saturating_sub(1));
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if area.abs() < 1e-6 {
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let q = [x as f32 + 0.5, y as f32 + 0.5];
                // Barycentric weights.
                let w0 = ((p[1][0] - q[0]) * (p[2][1] - q[1]) - (p[1][1] - q[1]) * (p[2][0] - q[0])) / area;
                let w1 = ((p[2][0] - q[0]) * (p[0][1] - q[1]) - (p[2][1] - q[1]) * (p[0][0] - q[0])) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let i = y * sw + x;
                let z = w0 * t.v[0].depth + w1 * t.v[1].depth + w2 * t.v[2].depth;
                if z >= self.z[i] {
                    continue;
                }
                self.z[i] = z;
                let lum = w0 * t.v[0].lum + w1 * t.v[1].lum + w2 * t.v[2].lum;
                for k in 0..3 {
                    let c = w0 * t.v[0].rgb[k] as f32 + w1 * t.v[1].rgb[k] as f32 + w2 * t.v[2].rgb[k] as f32;
                    self.color[i][k] = (c * lum).clamp(0.0, 255.0);
                }
            }
        }
    }

    fn to_rgba8(&self) -> Vec<u8> {
        let (ss, sw) = (self.ss, self.w * self.ss);
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for y in 0..self.h {
            for x in 0..self.w {
                let mut acc = [0.0f32; 3];
                for sy in 0..ss {
                    for sx in 0..ss {
                        let c = self.color[(y * ss + sy) * sw + x * ss + sx];
                        for k in 0..3 {
                            acc[k] += c[k];
                        }
                    }
                }
                let n = (ss * ss) as f32;
                out.extend(acc.map(|v| (v / n).round().clamp(0.0, 255.0) as u8));
                out.push(255);
            }
        }
        out
    }
}

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).map_err(|e| e.to_string())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let art_path = PathBuf::from(args.next().ok_or("usage: texture_uv <art.png> <out_dir>")?);
    let out_dir = PathBuf::from(args.next().ok_or("usage: texture_uv <art.png> <out_dir>")?);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let art = Art::load(&art_path)?;
    let (x0, y0, x1, y1) = art.bounds();
    println!("art {}x{}, bbox x {x0}..{x1} y {y0}..{y1}", art.w, art.h);

    let dir = data_dir();
    let rig = Rig::load(&dir.join("rigs").join(Rig::file_name("humanoid"))).map_err(|e| e.to_string())?;
    let outlines = rig::Outlines::load(&dir.join("outlines.json")).map_err(|e| format!("outlines: {e}"))?;
    let view = |v: View| outlines.get("humanoid", v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let (front, side) = (view(View::Front), view(View::Side));
    let puppet = Puppet::build_masks(&rig, front.as_ref(), side.as_ref()).map_err(|e| e.to_string())?;

    // Render at the art's own scale, figure filling the frame.
    let height_ft = rig.ref_ft.max(1.0);
    let px_per_ft = (y1 - y0 + 1) as f32 / height_ft;
    let min_x = -((x0 + x1) as f32 * 0.5) / px_per_ft;
    let max_y = (y1 as f32 + 1.0) / px_per_ft;
    let project = ortho(View::Front, min_x, max_y, px_per_ft);
    println!("scaled to {height_ft} ft => {px_per_ft:.1} px/ft");

    // Light fixed to the view: upper-left, toward the camera.
    let to_cam: V3 = [0.0, 0.0, 1.0];
    let light = norm_or([-0.55, 0.65, 0.52], to_cam);

    let rest = puppet.pose(&Pose::default());
    let (colors, lums, hit, miss) = bake_uv(&puppet, &art, &project, rest.lift, light, to_cam);
    println!("surface: {hit} points sampled, {miss} found no art nearby");

    let poses = PoseSet::load(&dir.join("poses"), &rig.body_type)?;
    let mut shots: Vec<(String, Posed, f32)> = vec![("rest".into(), puppet.pose(&Pose::default()), 0.0)];
    for name in ["kneel", "prone", "sit"] {
        if let Some(p) = poses.get(name) {
            shots.push((name.into(), puppet.pose(p), 0.0));
        }
    }
    // A turned figure, to show how far the planar map holds up.
    shots.push(("rest_yaw40".into(), puppet.pose(&Pose::default()), 40.0));

    for (name, posed, yaw) in &shots {
        let tris = triangles(posed, &colors, &lums, &project, posed.lift, *yaw, to_cam);
        let mut canvas = Shaded::new(art.w, art.h, 1, BACKGROUND);
        for t in &tris {
            canvas.tri(t);
        }
        write_png(&out_dir.join(format!("uv_{name}.png")), art.w, art.h, &canvas.to_rgba8())?;
        println!("uv_{name}.png: {} triangles", tris.len());
    }
    let _ = Look::default();
    Ok(())
}
