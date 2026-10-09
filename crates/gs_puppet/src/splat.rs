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

/// How far behind a pixel's front-most splat another splat may sit and
/// still be drawn, in the projection's depth units (feet for the
/// cameras here): the thickness of the surface. Deeper is inside.
pub const SURFACE_BAND: f32 = 0.25;


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
    /// How much it gives off its own light, 0..1 (an ember, a flash): that
    /// much of it is not shaded, and a lit host keeps it bright in the dark.
    pub glow: f32,
    /// How wet it is, 0..1 (fresh blood, from the marks sheet): a lit host
    /// shines its lights off it, as off the smooth skin's wet texels.
    pub wet: f32,
    /// A see-through film over the body (ice, a web's strand): it tints
    /// what is under it but is never taken for the surface -- its core
    /// does not make a pixel opaque, so the body shows through it and,
    /// past the body's edge, the backdrop does.
    pub film: bool,
    /// How much of it shows only seen edge-on, 0..1 (ADDITION): turned to
    /// the viewer it is that much fainter ([`rim_fade`]), so a body of such
    /// splats is a bright rim round a nearly empty middle -- an invisible
    /// figure's skin. Needs a facing (`n`). [`prepare`] fades each splat by
    /// its own facing, and never takes one for the surface; a GPU host
    /// (`cloud_gpu.wgsl`) keeps them as the surface and fades that by its
    /// smoothed facing, pixel by pixel -- a trained cloud's splats are round
    /// blobs, and one edge-on still covers a limb's middle.
    pub rim: f32,
}

/// Where a splat with a [`Gaussian::rim`] shows: from this far edge-on (1
/// less how much it faces the viewer) it starts to, and from this it is
/// whole. cloud_gpu.wgsl's RIM_FROM, RIM_TO.
pub const RIM_FROM: f32 = 0.3;
pub const RIM_TO: f32 = 0.8;

/// How much of a splat with `rim` shows, facing the viewer by `facing`
/// (|its unit facing . toward the viewer|, 1 square on, 0 edge-on).
pub fn rim_fade(rim: f32, facing: f32) -> f32 {
    1.0 - rim.clamp(0.0, 1.0) * (1.0 - crate::skin::smoothstep(RIM_FROM, RIM_TO, 1.0 - facing.abs()))
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
        out.push(Gaussian { c: [g(ix), g(iy), g(iz)], axes, n: [0.0; 3], rgb, alpha, glow: 0.0, wet: 0.0, film: false, rim: 0.0 });
    }
    Ok(out)
}

/// A fitted cloud from a Niantic `.spz` file, version 4 (`NGSP`: a 32
/// byte header, six zstd streams -- positions, alphas, colours, scales,
/// rotations, harmonics -- each with its sizes in a table of contents),
/// decoded as the reference `load-spz.cc` does: positions 24-bit fixed
/// point with `fractional_bits`, scales `b / 16 - 10` (log), rotations
/// "smallest three" (2 bits name the largest component, three 10-bit
/// signed magnitudes over sqrt(1/2)), alphas straight opacity bytes,
/// colours `(b / 255 - 0.5) / 0.15` as zeroth-order harmonics. Higher
/// harmonics are skipped, as [`load_ply`] skips them. An `.spz` holds its
/// cloud right-up-back (y up, z toward the viewer: the OpenGL frame),
/// where a trainer's `.ply` is right-down-front, and the reference packer
/// converts between them; this turns it back, so a cloud comes out as
/// [`load_ply`] would hand the `.ply` it was packed from: y and z
/// negated, and the rotation's y and z with them. (Scales come back
/// clamped to e^-10 at the small end: the packer's floor, far below any
/// pixel.) The gzip versions 1..3 are not read. zstd is decoded in pure
/// Rust, so this builds for Android too.
pub fn load_spz(path: &std::path::Path) -> std::io::Result<Vec<Gaussian>> {
    let bytes = std::fs::read(path)?;
    decode_spz(&bytes)
}

/// [`load_spz`] on bytes already read.
pub fn decode_spz(bytes: &[u8]) -> std::io::Result<Vec<Gaussian>> {
    let bad = |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("spz: {what}"));
    if bytes.len() < 32 {
        return Err(bad("too short for a header"));
    }
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
    let u64_at = |i: usize| -> std::io::Result<u64> { bytes.get(i..i + 8).map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes"))).ok_or_else(|| bad("table of contents past the end")) };
    if u32_at(0) != 0x5053_474e {
        return Err(bad(if bytes[0] == 0x1f && bytes[1] == 0x8b { "a gzip spz (version 1..3) is not read" } else { "not an spz file (no NGSP magic)" }));
    }
    let version = u32_at(4);
    if version != 4 {
        return Err(bad(&format!("version {version} is not read, only 4")));
    }
    let count = u32_at(8) as usize;
    let sh_degree = bytes[12];
    let frac_bits = bytes[13];
    let streams = bytes[15] as usize;
    let toc = u32_at(16) as usize;
    let sh_dim: usize = match sh_degree {
        0 => 0,
        1 => 3,
        2 => 8,
        3 => 15,
        4 => 24,
        d => return Err(bad(&format!("harmonics degree {d}"))),
    };
    // What each stream must hold, in the order the writer writes them;
    // an empty one (no harmonics) is left out of the file.
    let wanted = [count * 9, count, count * 3, count * 3, count * 4, count * sh_dim * 3];
    let present: Vec<usize> = (0..6).filter(|i| wanted[*i] > 0).collect();
    if present.len() != streams {
        return Err(bad(&format!("{streams} streams, expected {}", present.len())));
    }
    let mut at = toc + streams * 16;
    let mut decoded: [Vec<u8>; 6] = Default::default();
    for (n, i) in present.iter().enumerate() {
        let comp = u64_at(toc + n * 16)? as usize;
        let uncomp = u64_at(toc + n * 16 + 8)? as usize;
        if uncomp != wanted[*i] {
            return Err(bad(&format!("stream {n} holds {uncomp} bytes, expected {}", wanted[*i])));
        }
        let packed = bytes.get(at..at + comp).ok_or_else(|| bad("stream past the end"))?;
        at += comp;
        if *i == 5 {
            continue; // the harmonics: not read
        }
        let mut out = Vec::with_capacity(uncomp);
        ruzstd::decoding::FrameDecoder::new().decode_all_to_vec(packed, &mut out).map_err(|e| bad(&format!("stream {n}: {e}")))?;
        if out.len() != uncomp {
            return Err(bad(&format!("stream {n} decoded to {} bytes, expected {uncomp}", out.len())));
        }
        decoded[*i] = out;
    }
    let [pos, alphas, colors, scales, rots, _] = decoded;
    let pos_scale = 1.0 / (1u32 << frac_bits.min(31)) as f32;
    const SH_C0: f32 = 0.282_094_79;
    const COLOR_SCALE: f32 = 0.15;
    let mut out = Vec::with_capacity(count);
    for v in 0..count {
        let p = |k: usize| {
            let i = (v * 3 + k) * 3;
            let fixed = (pos[i] as u32) | ((pos[i + 1] as u32) << 8) | ((pos[i + 2] as u32) << 16);
            let fixed = if fixed & 0x80_0000 != 0 { fixed | 0xff00_0000 } else { fixed } as i32;
            fixed as f32 * pos_scale
        };
        let sc = [0, 1, 2].map(|k| (scales[v * 3 + k] as f32 / 16.0 - 10.0).exp());
        let quat = unpack_smallest_three(rots[v * 4..v * 4 + 4].try_into().expect("4 bytes"));
        // the file's quaternion is x y z w; `quat_to_m3` takes w first.
        // Right-up-back to right-down-front: y and z negated, which turns
        // the rotation's y and z too (a 180 degree turn about x).
        let rot = crate::math::quat_to_m3(crate::math::quat_norm([quat[3], quat[0], -quat[1], -quat[2]]));
        let axes = [0, 1, 2].map(|k| [rot[0][k] * sc[k], rot[1][k] * sc[k], rot[2][k] * sc[k]]);
        let rgb = [0, 1, 2].map(|k| {
            let dc = (colors[v * 3 + k] as f32 / 255.0 - 0.5) / COLOR_SCALE;
            ((0.5 + SH_C0 * dc).clamp(0.0, 1.0)) * 255.0
        });
        let alpha = alphas[v] as f32 / 255.0;
        out.push(Gaussian { c: [p(0), -p(1), -p(2)], axes, n: [0.0; 3], rgb, alpha, glow: 0.0, wet: 0.0, film: false, rim: 0.0 });
    }
    Ok(out)
}

/// The spz "smallest three" rotation: 2 bits say which component is
/// largest, the other three come as 10-bit sign-and-magnitude over
/// sqrt(1/2), from the top of the word down; the largest is what unit
/// length leaves. Returned x, y, z, w.
fn unpack_smallest_three(r: [u8; 4]) -> [f32; 4] {
    let mut comp = u32::from_le_bytes(r);
    let largest = (comp >> 30) as usize;
    let mut q = [0.0f32; 4];
    let mut sum = 0.0;
    for i in (0..4).rev() {
        if i == largest {
            continue;
        }
        let mag = (comp & 0x1ff) as f32;
        let neg = (comp >> 9) & 1 == 1;
        comp >>= 10;
        let mut v = std::f32::consts::FRAC_1_SQRT_2 * mag / 511.0;
        if neg {
            v = -v;
        }
        q[i] = v;
        sum += v * v;
    }
    q[largest] = (1.0 - sum).max(0.0).sqrt();
    q
}

/// [`load_spz`] for a `.spz` path, [`load_ply`] for anything else.
pub fn load_cloud(path: &std::path::Path) -> std::io::Result<Vec<Gaussian>> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("spz")) {
        load_spz(path)
    } else {
        load_ply(path)
    }
}

/// Where a family's fitted cloud lives: `<data>/clouds/<family>.spz`, or
/// `.ply` where there is no `.spz` (the path is the `.ply`'s when neither
/// exists).
pub fn cloud_path(data_root: &std::path::Path, family: &str) -> std::path::PathBuf {
    cloud_file(&data_root.join("clouds"), family)
}

/// `<dir>/<stem>.spz` when it exists, else `<dir>/<stem>.ply`: a cloud
/// kept beside other files, by whichever name it has.
pub fn cloud_file(dir: &std::path::Path, stem: &str) -> std::path::PathBuf {
    let spz = dir.join(format!("{stem}.spz"));
    if spz.is_file() {
        return spz;
    }
    dir.join(format!("{stem}.ply"))
}

/// How far from its nearest segment's surface a Gaussian still takes a
/// share of a NEIGHBOURING segment's motion, feet: the width of the
/// smooth band across a joint. Beyond it a Gaussian is that segment's
/// alone.
pub const BLEND_FT: f32 = 0.22;

/// A fitted cloud bound to a puppet. Each Gaussian follows the body
/// segment whose ring SURFACE it sits nearest in the rest pose, in feet
/// (inside a ring counts as on it), exactly as the puppet's own rings do
/// -- measured in radii instead, a torso ring four times an arm's width
/// claimed the arm's outer side, and a raised arm left a second pair
/// hanging. Near a joint it also takes a share of the segments adjoining
/// that one in the tree (its parent, its children), falling off with
/// distance over [`BLEND_FT`], and its position and shape are the
/// weighted blend of the segments' transforms (linear blend skinning): a
/// bent elbow stretches the skin round it instead of opening a wedge.
/// Built once per puppet; posed per frame.
#[derive(Debug, Clone)]
pub struct BoundCloud {
    /// The cloud in the puppet's rest space.
    pub rest: Vec<Gaussian>,
    /// Per Gaussian, the segment it sits nearest: what it mostly moves with.
    pub seg: Vec<usize>,
    /// Per Gaussian, up to three (segment, weight) shares summing to one;
    /// unused slots have weight 0.
    pub weights: Vec<[(usize, f32); 3]>,
    /// It rides a modelled mesh (`mesh::SplatBody`): moved by the mesh's
    /// segment transforms (`mesh::BoundMesh::bone_xfs`: a hanging hand is
    /// not turned), not the puppet's own.
    pub modelled: bool,
}

impl BoundCloud {
    pub fn bind(puppet: &crate::model::Puppet, rest: Vec<Gaussian>) -> BoundCloud {
        let (seg, weights) = skin_weights(puppet, rest.iter().map(|g| g.c));
        BoundCloud { rest, seg, weights, modelled: false }
    }

    /// The cloud moved into `posed`'s pose: each Gaussian's centre and axes
    /// are the weighted blend of its segments' transforms.
    pub fn posed(&self, posed: &Posed<'_>) -> Vec<Gaussian> {
        let xfs = if self.modelled { crate::mesh::BoundMesh::bone_xfs(posed) } else { posed.xf.clone() };
        self.rest
            .iter()
            .zip(&self.weights)
            .map(|(g, w)| {
                let mut c = [0.0f32; 3];
                let mut axes = [[0.0f32; 3]; 3];
                for &(s, share) in w {
                    if share <= 0.0 {
                        continue;
                    }
                    let xf = &xfs[s];
                    c = add(c, scale(xf.point(g.c), share));
                    for k in 0..3 {
                        axes[k] = add(axes[k], scale(xf.dir(g.axes[k]), share));
                    }
                }
                Gaussian { c, axes, n: g.n, rgb: g.rgb, alpha: g.alpha, glow: g.glow, wet: g.wet, film: g.film, rim: g.rim }
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

/// What [`BoundCloud::bind`] gives each of `points` (in the puppet's rest
/// space): the segment it sits nearest, and its shares. A mesh's vertices
/// are bound by it too (`mesh::BoundMesh`).
pub fn skin_weights(puppet: &crate::model::Puppet, points: impl Iterator<Item = V3>) -> (Vec<usize>, Vec<[(usize, f32); 3]>) {
    {
        let bodies: Vec<(usize, &crate::model::Segment, V3)> = puppet
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind.is_body() && !s.hidden)
            .map(|(i, s)| (i, s, norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0])))
            .collect();
        // Feet from the segment's surface: outside the nearest ring's
        // ellipse (0 inside) and past the ring's own slab along the bone.
        let surface = |s: &crate::model::Segment, axis: V3, p: V3| {
            let mut best = f32::MAX;
            for st in &s.stations {
                let e = sub(p, st.c);
                let (u, v) = (dot(e, s.axis_w) / st.w.max(0.02), dot(e, s.axis_d) / st.d.max(0.02));
                let radial = ((u * u + v * v).sqrt() - 1.0).max(0.0) * (st.w + st.d) * 0.5;
                let along = (dot(e, axis).abs() - crate::model::STATION_STEP_FT * 0.5).max(0.0);
                best = best.min(radial * radial + along * along);
            }
            best.sqrt()
        };
        let adjoins = |a: usize, b: usize| puppet.segments[a].parent == Some(b) || puppet.segments[b].parent == Some(a);
        let mut seg = Vec::new();
        let mut weights = Vec::new();
        for p in points {
            let dists: Vec<(usize, f32)> = bodies.iter().map(|(i, s, axis)| (*i, surface(s, *axis, p))).collect();
            let &(near, d0) = dists.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or(&(0, 0.0));
            // Shares: the nearest segment, and those adjoining it in the
            // tree within the blend band, each falling off with how much
            // further away it is.
            let mut shares: Vec<(usize, f32)> = vec![(near, 1.0)];
            for &(i, d) in &dists {
                if i != near && adjoins(i, near) && d - d0 < BLEND_FT {
                    let t = (d - d0) / BLEND_FT;
                    shares.push((i, (1.0 - t) * (1.0 - t)));
                }
            }
            shares.sort_by(|a, b| b.1.total_cmp(&a.1));
            shares.truncate(3);
            let total: f32 = shares.iter().map(|s| s.1).sum();
            let mut w = [(near, 0.0f32); 3];
            for (k, (i, share)) in shares.iter().enumerate() {
                w[k] = (*i, share / total);
            }
            seg.push(near);
            weights.push(w);
        }
        (seg, weights)
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
                // Along the STEP to the next ring, not the bone: on a curved
                // trunk (a canine's neck into its head) the two differ, and
                // splats laid along the bone left a dark seam at every ring.
                let step = if gap > 1e-4 { norm_or(sub(xf.point(next.expect("gap > 0 has a next").c), c0), bone) } else { bone };
                let along = scale(step, along_len * 0.5);
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
                    out.push(Gaussian { c: p, axes: [half_ring, scale(along, 2.0 * overlap), scale(nrm, spacing * 0.15)], n: nrm, rgb, alpha, glow: 0.0, wet: 0.0, film: false, rim: 0.0 });
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
/// One splat projected, for a host that rasterises the cloud itself (the
/// GPU splats): its screen centre and inverse 2D covariance (pixels of the
/// image's own frame), how far it reaches, its depth, its colour (0..255,
/// the cel shade in) and peak opacity, and where it is and faces (the
/// turned world, for the lights).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplatFlat {
    pub c: [f32; 2],
    pub inv: [f32; 4],
    pub r: [f32; 2],
    pub z: f32,
    pub rgb: [f32; 3],
    pub alpha: f32,
    pub pos: V3,
    pub n: V3,
    /// As [`Gaussian::glow`].
    pub glow: f32,
    /// As [`Gaussian::film`].
    pub film: bool,
}

/// A cloud projected and sorted back to front, and the image it fills: what
/// [`render`] rasterises, and what a GPU host does (ADDITION).
#[derive(Debug, Clone)]
pub struct SplatFrame {
    /// As [`Skin`]'s.
    pub origin: [f32; 2],
    pub w: usize,
    pub h: usize,
    pub flats: Vec<SplatFlat>,
    /// Toward the viewer, in the turned world.
    pub to_cam: V3,
    /// Identifies these pixels.
    pub key: u64,
}

impl SplatFrame {
    /// Bytes this frame holds on the heap (its flattened splats).
    pub fn heap_bytes(&self) -> usize {
        self.flats.capacity() * std::mem::size_of::<SplatFlat>()
    }
}

/// A splat further behind a pixel's front-most core than [`SURFACE_BAND`]
/// is inside; a pixel under a core is opaque; a core is `CORE_M` in the
/// Gaussian's own distance, its reach `REACH_M` (see [`render`]).
pub const CORE_M: f32 = 2.25;
pub const REACH_M: f32 = 9.0;

/// Project `cloud` through `project` turned by `yaw`, shade it, and sort it
/// back to front: what [`render`] draws. None when nothing shows.
pub fn prepare(cloud: &[Gaussian], project: &dyn Fn(V3) -> ([f32; 2], f32), yaw: f32) -> Option<SplatFrame> {
    if cloud.is_empty() {
        return None;
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
        return None;
    }
    let (near, far) = flats.iter().fold((f32::MAX, f32::MIN), |(n, f), x| (n.min(x.z), f.max(x.z)));
    let range = (far - near).max(2.0);
    for f in &mut flats {
        f.shade *= 1.0 - crate::draw::DEPTH_SHADE * ((f.z - near) / range).clamp(0.0, 1.0);
    }
    let origin = [lo[0].floor(), lo[1].floor()];
    let w = ((hi[0] - origin[0]).ceil() as usize + 1).min(4096);
    let h = ((hi[1] - origin[1]).ceil() as usize + 1).min(4096);
    flats.sort_by(|a, b| b.z.total_cmp(&a.z));
    let flats: Vec<SplatFlat> = flats
        .iter()
        .map(|f| {
            let g = &cloud[f.i];
            SplatFlat {
                c: [f.c[0] - origin[0], f.c[1] - origin[1]],
                inv: [f.inv[0][0], f.inv[0][1], f.inv[1][0], f.inv[1][1]],
                r: f.r,
                z: f.z,
                // what glows is not shaded
                rgb: g.rgb.map(|c| c * (f.shade + (1.0 - f.shade) * g.glow.clamp(0.0, 1.0))),
                // (a rim splat: as much as it is edge-on)
                alpha: if g.rim > 0.0 && g.n != [0.0; 3] { g.alpha * rim_fade(g.rim, dot(norm_or(g.n, [0.0; 3]), view.to_cam)) } else { g.alpha },
                pos: turned(g.c),
                n: turned(g.n),
                glow: g.glow.clamp(0.0, 1.0),
                // (a rim splat, faded by itself, is never the surface)
                film: g.film || g.rim > 0.0,
            }
        })
        .collect();
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (w, h, origin.map(f32::to_bits), flats.len()).hash(&mut hasher);
        for f in flats.iter().step_by(97) {
            (f.c.map(f32::to_bits), f.z.to_bits(), f.rgb.map(f32::to_bits)).hash(&mut hasher);
        }
        hasher.finish()
    };
    Some(SplatFrame { origin, w, h, flats, to_cam: view.to_cam, key })
}

pub fn render(cloud: &[Gaussian], project: &dyn Fn(V3) -> ([f32; 2], f32), yaw: f32) -> Skin {
    let Some(SplatFrame { origin, w, h, flats, .. }) = prepare(cloud, project, yaw) else {
        return Skin { origin: [0.0, 0.0], w: 0, h: 0, rgba: Vec::new() };
    };
    // The front surface's depth per pixel, from every splat's core: a
    // splat further behind it than SURFACE_BAND is inside or beyond the
    // body and is not drawn. Between neighbouring splats a Gaussian
    // surface never quite closes, and without this the rings inside a
    // trunk (a neck in a chest) showed through it as a moire, worst when
    // zoomed in; culling them by geometry instead tore joints open.
    let mut front = vec![f32::MAX; w * h];
    // (a film -- ice, a web -- is never the surface: the body shows through it)
    for f in flats.iter().filter(|f| !f.film) {
        let (cx, cy) = (f.c[0], f.c[1]);
        let xa = ((cx - f.r[0] * 0.5).floor() as i64).max(0);
        let xb = ((cx + f.r[0] * 0.5).ceil() as i64).min(w as i64 - 1);
        let ya = ((cy - f.r[1] * 0.5).floor() as i64).max(0);
        let yb = ((cy + f.r[1] * 0.5).ceil() as i64).min(h as i64 - 1);
        for py in ya..=yb {
            for px in xa..=xb {
                let d = [px as f32 + 0.5 - cx, py as f32 + 0.5 - cy];
                let m = d[0] * (f.inv[0] * d[0] + f.inv[1] * d[1]) + d[1] * (f.inv[2] * d[0] + f.inv[3] * d[1]);
                if m <= CORE_M {
                    let at = &mut front[py as usize * w + px as usize];
                    *at = at.min(f.z);
                }
            }
        }
    }
    // Back to front, premultiplied over transparent (sorted so already).
    let mut fb = vec![[0.0f32; 4]; w * h];
    for f in &flats {
        let (cx, cy) = (f.c[0], f.c[1]);
        let xa = ((cx - f.r[0]).floor() as i64).max(0);
        let xb = ((cx + f.r[0]).ceil() as i64).min(w as i64 - 1);
        let ya = ((cy - f.r[1]).floor() as i64).max(0);
        let yb = ((cy + f.r[1]).ceil() as i64).min(h as i64 - 1);
        for py in ya..=yb {
            for px in xa..=xb {
                let d = [px as f32 + 0.5 - cx, py as f32 + 0.5 - cy];
                let m = d[0] * (f.inv[0] * d[0] + f.inv[1] * d[1]) + d[1] * (f.inv[2] * d[0] + f.inv[3] * d[1]);
                if m > REACH_M || f.z > front[py as usize * w + px as usize] + SURFACE_BAND {
                    continue;
                }
                let a = (f.alpha * (-0.5 * m).exp()).min(0.99);
                let dst = &mut fb[py as usize * w + px as usize];
                for c in 0..3 {
                    dst[c] = f.rgb[c] * a + dst[c] * (1.0 - a);
                }
                dst[3] = a + dst[3] * (1.0 - a);
            }
        }
    }
    // A pixel under some splat's core is ON the surface: it is opaque,
    // whatever the Gaussians summed to. Between neighbouring splats the
    // sum falls short (0.86 at the default overlap), and drawn as is that
    // shortfall showed the background through the body as dark rings
    // round every station, worst when zoomed in. Outside every core the
    // soft alpha stays: that is the edge.
    let mut rgba = vec![0u8; w * h * 4];
    for ((p, px), z) in fb.iter().zip(rgba.chunks_exact_mut(4)).zip(&front) {
        if p[3] > 1e-3 {
            for c in 0..3 {
                px[c] = (p[c] / p[3]).round().clamp(0.0, 255.0) as u8;
            }
            let a = if *z < f32::MAX { 1.0 } else { p[3] };
            px[3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Skin { origin, w, h, rgba }
}

#[cfg(test)]
mod rim_tests {
    use super::*;

    /// A rim splat is gone turned to the viewer, whole edge-on; one with no
    /// rim is whole either way.
    #[test]
    fn a_rim_shows_edge_on() {
        assert!(rim_fade(1.0, 1.0) < 1e-6);
        assert!((rim_fade(1.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((rim_fade(0.95, 1.0) - 0.05).abs() < 1e-4);
        assert!((rim_fade(0.0, 0.7) - 1.0).abs() < 1e-6);
        assert!(rim_fade(1.0, 0.6) < rim_fade(1.0, 0.3), "the more edge-on, the more of it");
    }
}

#[cfg(test)]
mod spz_tests {
    use super::*;

    /// `SPZ_PAIR=<dir>` names a folder holding `export_30000.ply` and
    /// `.spz` of the same cloud (Brush's export, Niantic's packer): the
    /// two decode to the same cloud within the packing's precision.
    #[test]
    fn spz_decodes_to_the_ply_it_was_packed_from() {
        let Ok(dir) = std::env::var("SPZ_PAIR") else { return };
        let dir = std::path::Path::new(&dir);
        let ply = load_ply(&dir.join("export_30000.ply")).unwrap();
        let spz = load_spz(&dir.join("export_30000.spz")).unwrap();
        assert_eq!(ply.len(), spz.len());
        let (mut dc, mut dr, mut da, mut dax, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize);
        let (mut worst_c, mut worst_ax) = (0.0f32, 0.0f32);
        let extent = {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for g in &ply { for k in 0..3 { lo[k] = lo[k].min(g.c[k]); hi[k] = hi[k].max(g.c[k]); } }
            (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max)
        };
        for (a, b) in ply.iter().zip(&spz) {
            let c = (0..3).map(|k| (a.c[k] - b.c[k]).abs()).fold(0.0f32, f32::max);
            let r = (0..3).map(|k| (a.rgb[k] - b.rgb[k]).abs()).fold(0.0f32, f32::max);
            // axis k against axis k (the same ellipsoid, the same order), either
            // way round, as a share of the figure: the packer floors tiny
            // scales at e^-10, which no pixel sees
            let ax = (0..3).map(|k| { let d = |sgn: f32| (0..3).map(|j| (a.axes[k][j] - sgn * b.axes[k][j]).abs()).fold(0.0f32, f32::max); d(1.0).min(d(-1.0)) / extent }).fold(0.0f32, f32::max);
            dc += c as f64; dr += r as f64; da += (a.alpha - b.alpha).abs() as f64; dax += ax as f64; n += 1;
            worst_c = worst_c.max(c); worst_ax = worst_ax.max(ax);
        }
        let n = n as f64;
        println!("{} splats, extent {extent:.3}: position mean {:.5} worst {worst_c:.5}; colour mean {:.2}/255; alpha mean {:.4}; axes mean {:.5} worst {worst_ax:.5} of the extent", ply.len(), dc / n, dr / n, da / n, dax / n);
        assert!(worst_c <= 1.0 / 4096.0 * 1.01, "positions within a fixed-point step: {worst_c}");
        assert!(dr / n < 2.0, "colours within quantization: {}", dr / n);
        assert!(da / n < 0.01, "alphas within a byte: {}", da / n);
        assert!(worst_ax < 0.02, "axes within the rotation's and log scale's steps: {worst_ax}");
    }

    #[test]
    fn smallest_three_unpacks_the_identity_and_a_turn() {
        // the identity: w largest (index 3), the other three zero
        let q = unpack_smallest_three((3u32 << 30).to_le_bytes());
        assert_eq!(q, [0.0, 0.0, 0.0, 1.0]);
        // x = y = sqrt(1/2), z = w = 0: largest is x (index 0); the components
        // after it in the word are, from the top, y then z then w
        let word = 511u32 << 20;
        let q = unpack_smallest_three(word.to_le_bytes());
        assert!((q[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5, "{q:?}");
        assert!((q[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5, "{q:?}");
        assert_eq!(q[2], 0.0);
        assert_eq!(q[3], 0.0);
    }
}
