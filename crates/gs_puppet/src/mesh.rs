//! A modelled mesh worn on a puppet's skeleton: a triangle mesh with one
//! painted sheet (made elsewhere: a multi-view image-to-3D model, reduced
//! to a count a frame can pose), bound to the puppet's body segments as a
//! fitted cloud is (`splat::skin_weights`: each vertex moves with the
//! segment whose surface it sits nearest, sharing with the adjoining ones
//! near a joint) and posed by them. Better shares come from a tool made
//! for it: [`bones_text`] writes the skeleton out, Blender's automatic
//! weights spread each bone's pull through the body (`tools/mesh_weights.py`,
//! so an armpit goes with the arm, not the ribs it lies beside), and
//! [`load_weights`] reads them back for [`BoundMesh::with_weights`].
//!
//! The mesh stands as the puppet was drawn (the paintings it was made from
//! were painted on the puppet's own templates), y up and facing +z;
//! [`Mesh::fit_to_height`] brings it to the puppet's size.
//!
//! [`render`] draws it on the CPU, for a look without the app.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::math::*;
use crate::pose::Posed;
use crate::puppet::Puppet;
use crate::skin::Skin;

/// Triangles over vertices that each have a place, a facing and a place
/// on the painted sheet (`uv`: 0..1, v down the image).
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub pos: Vec<V3>,
    pub nrm: Vec<V3>,
    pub uv: Vec<[f32; 2]>,
    pub tris: Vec<[u32; 3]>,
    /// Per vertex, which of its file's vertices (`v` lines) it is: a file's
    /// vertex with two sheet places is two of ours. What a tool says of the
    /// file's vertices ([`load_weights`]) is looked up by it. Empty: its own
    /// index.
    pub source: Vec<u32>,
}

/// A mesh's painted sheet: straight RGBA8, row-major.
#[derive(Debug, Clone)]
pub struct Paint {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Paint {
    /// The colour at `uv`, between its four pixels.
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 3] {
        let x = (uv[0].clamp(0.0, 1.0) * self.w as f32 - 0.5).max(0.0);
        let y = (uv[1].clamp(0.0, 1.0) * self.h as f32 - 0.5).max(0.0);
        let (x0, y0) = ((x as usize).min(self.w - 1), (y as usize).min(self.h - 1));
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize, c: usize| self.rgba[(y * self.w + x) * 4 + c] as f32;
        std::array::from_fn(|c| {
            let top = at(x0, y0, c) + (at(x1, y0, c) - at(x0, y0, c)) * fx;
            let low = at(x0, y1, c) + (at(x1, y1, c) - at(x0, y1, c)) * fx;
            top + (low - top) * fy
        })
    }
}

impl Mesh {
    /// A Wavefront OBJ of triangles (`v`, `vt`, `vn`, `f a/b/c ...`; its
    /// `vt` has v up the image, as the format does).
    pub fn load_obj(path: &Path) -> io::Result<Mesh> {
        let text = std::fs::read_to_string(path)?;
        let (mut v, mut vt, mut vn): (Vec<V3>, Vec<[f32; 2]>, Vec<V3>) = (Vec::new(), Vec::new(), Vec::new());
        let mut mesh = Mesh::default();
        // one vertex per (place, sheet place, facing) a corner names
        let mut seen: HashMap<[i64; 3], u32> = HashMap::new();
        let num = |t: Option<&str>| t.and_then(|t| t.parse::<f32>().ok()).unwrap_or(0.0);
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("v") => v.push([num(parts.next()), num(parts.next()), num(parts.next())]),
                Some("vt") => vt.push([num(parts.next()), 1.0 - num(parts.next())]),
                Some("vn") => vn.push([num(parts.next()), num(parts.next()), num(parts.next())]),
                Some("f") => {
                    let mut corners = Vec::with_capacity(4);
                    for corner in parts {
                        let mut ids = corner.split('/').map(|t| t.parse::<i64>().unwrap_or(0));
                        let key = [ids.next().unwrap_or(0), ids.next().unwrap_or(0), ids.next().unwrap_or(0)];
                        // 1-based; negative counts back from the end
                        let pick = |id: i64, n: usize| if id > 0 { Some(id as usize - 1) } else if id < 0 { n.checked_sub(id.unsigned_abs() as usize) } else { None };
                        let Some(p) = pick(key[0], v.len()).and_then(|k| v.get(k)) else {
                            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}: a face names no vertex", path.display())));
                        };
                        let index = *seen.entry(key).or_insert_with(|| {
                            mesh.pos.push(*p);
                            mesh.source.push(pick(key[0], v.len()).unwrap_or(0) as u32);
                            mesh.uv.push(pick(key[1], vt.len()).and_then(|k| vt.get(k)).copied().unwrap_or([0.0; 2]));
                            mesh.nrm.push(pick(key[2], vn.len()).and_then(|k| vn.get(k)).copied().unwrap_or([0.0; 3]));
                            mesh.pos.len() as u32 - 1
                        });
                        corners.push(index);
                    }
                    for k in 1..corners.len().saturating_sub(1) {
                        mesh.tris.push([corners[0], corners[k], corners[k + 1]]);
                    }
                }
                _ => {}
            }
        }
        if mesh.nrm.iter().any(|n| *n == [0.0; 3]) {
            mesh.smooth_normals();
        }
        Ok(mesh)
    }

    /// Each vertex facing as the triangles round it do (weighted by area).
    pub fn smooth_normals(&mut self) {
        let mut sum = vec![[0.0f32; 3]; self.pos.len()];
        for t in &self.tris {
            let [a, b, c] = t.map(|i| self.pos[i as usize]);
            let n = cross(sub(b, a), sub(c, a));
            for i in t {
                sum[*i as usize] = add(sum[*i as usize], n);
            }
        }
        self.nrm = sum.into_iter().map(|n| norm_or(n, [0.0, 0.0, 1.0])).collect();
    }

    /// Scaled to stand `height_ft` tall, its feet on y = 0 and centred on
    /// x and z (as `splat::fit_to_height` places a cloud). Returns what it
    /// did -- the point that went to the origin and the scale -- for what
    /// was made with the mesh and must stay on it (its splat).
    pub fn fit_to_height(&mut self, height_ft: f32) -> Option<(V3, f32)> {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &self.pos {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let span = hi[1] - lo[1];
        if !(span > 1e-6) {
            return None;
        }
        let s = height_ft / span;
        let centre = [(lo[0] + hi[0]) * 0.5, lo[1], (lo[2] + hi[2]) * 0.5];
        for p in &mut self.pos {
            *p = scale(sub(*p, centre), s);
        }
        Some((centre, s))
    }
}

/// A mesh bound to a puppet, in its rest space. Built once; posed per
/// frame.
#[derive(Debug, Clone)]
pub struct BoundMesh {
    pub mesh: Mesh,
    /// Per vertex, the segment it sits nearest.
    pub seg: Vec<usize>,
    /// Per vertex, up to four (segment, weight) shares summing to one;
    /// unused slots have weight 0.
    pub weights: Vec<Shares>,
}

/// One vertex's (segment, weight) shares.
pub type Shares = [(usize, f32); 4];

/// How far inward (cosine) a vertex's facing must turn before
/// `BoundMesh::face_out` turns it out.
const FACES_IN: f32 = 0.25;

/// How far round a splat the mesh's facings are blended into its own
/// (`VertexGrid::facing_at`), feet. A thinned mesh is folded on itself
/// where its merge ran (`tools/decimate_mesh.py`), its facings tilting
/// every way vertex to vertex: blended over less than this, a forehead's
/// splats came out as a strip tilted up to the sky between strips tilted
/// down, and an evening sky lit the strip pale (half_elf_male, 2026-10-08).
const FACING_REACH_FT: f32 = 0.15;

/// How much of the facings round a splat must agree (the length of their
/// weighted sum over the sum of the weights) to be the skin's way there;
/// less, and they cancel -- the two sides of an ear, a finger -- and the
/// nearest vertex's facing says which side the splat is on.
const FACING_AGREE: f32 = 0.3;

/// The puppet's body segments as bones, for a tool that works the shares
/// out: `height <feet>`, then a `bone <name> <parent's name or -> <head
/// xyz> <tail xyz>` line each, in the puppet's rest space (y up, facing
/// +z).
pub fn bones_text(puppet: &Puppet) -> String {
    let body = |i: usize| puppet.segments[i].kind.is_body() && !puppet.segments[i].hidden;
    let mut out = format!("height {}\n", puppet.height);
    for (_, s) in puppet.segments.iter().enumerate().filter(|(i, _)| body(*i)) {
        // the nearest ancestor that is a bone too
        let mut parent = s.parent;
        while let Some(p) = parent.filter(|p| !body(*p)) {
            parent = puppet.segments[p].parent;
        }
        out.push_str(&format!(
            "bone {} {} {} {} {} {} {} {}\n",
            s.name,
            parent.map_or("-", |p| puppet.segments[p].name.as_str()),
            s.head[0],
            s.head[1],
            s.head[2],
            s.tail[0],
            s.tail[1],
            s.tail[2]
        ));
    }
    out
}

/// Shares a tool worked out: a `bone <name>` line per bone it used, in its
/// order, then a `w <bone>:<weight> ...` line per vertex of the mesh's
/// file, in its order (none after `w`: a vertex it found no bone for). Each vertex's
/// four largest are kept; None for a vertex with none.
pub fn load_weights(path: &Path, puppet: &Puppet) -> io::Result<Vec<Option<Shares>>> {
    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {what}", path.display()));
    let mut bones: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("bone") => {
                let name = parts.next().unwrap_or("");
                bones.push(puppet.segments.iter().position(|s| s.name == name).ok_or_else(|| bad(format!("no segment {name}")))?);
            }
            Some("w") => {
                let mut shares: Vec<(usize, f32)> = Vec::new();
                for part in parts {
                    let (bone, weight) = part.split_once(':').ok_or_else(|| bad(format!("not bone:weight: {part}")))?;
                    let seg = bone.parse::<usize>().ok().and_then(|k| bones.get(k)).ok_or_else(|| bad(format!("no bone {bone}")))?;
                    shares.push((*seg, weight.parse().map_err(|_| bad(format!("not a weight: {weight}")))?));
                }
                shares.retain(|s| s.1 > 0.0);
                shares.sort_by(|a, b| b.1.total_cmp(&a.1));
                shares.truncate(4);
                let total: f32 = shares.iter().map(|s| s.1).sum();
                out.push((total > 0.0).then(|| std::array::from_fn(|k| shares.get(k).map_or((shares[0].0, 0.0), |s| (s.0, s.1 / total)))));
            }
            _ => {}
        }
    }
    Ok(out)
}

impl BoundMesh {
    /// Each vertex to the segment whose surface it sits nearest
    /// (`splat::skin_weights`).
    pub fn bind(puppet: &Puppet, mesh: Mesh) -> BoundMesh {
        BoundMesh::with_weights(puppet, mesh, &[])
    }

    /// Each vertex by its shares in `given` ([`load_weights`]: per vertex
    /// of the mesh's file, `Mesh::source`), and as [`BoundMesh::bind`] does
    /// where it has none.
    pub fn with_weights(puppet: &Puppet, mesh: Mesh, given: &[Option<Shares>]) -> BoundMesh {
        let (near, nearest) = crate::splat::skin_weights(puppet, mesh.pos.iter().copied());
        let mut seg = Vec::with_capacity(mesh.pos.len());
        let mut weights = Vec::with_capacity(mesh.pos.len());
        for (k, (near, w)) in near.into_iter().zip(nearest).enumerate() {
            match given.get(mesh.source.get(k).map_or(k, |f| *f as usize)).copied().flatten() {
                Some(shares) => {
                    seg.push(shares[0].0);
                    weights.push(shares);
                }
                None => {
                    seg.push(near);
                    weights.push([w[0], w[1], w[2], (near, 0.0)]);
                }
            }
        }
        let mut bound = BoundMesh { mesh, seg, weights };
        bound.face_out(puppet);
        bound
    }

    /// Each vertex's facing turned to face OUT of the part it rides, where
    /// it plainly faces in (ADDITION): out is away from the line through
    /// its segment's station centres. A mesh's own facings can face in --
    /// a garment's inner layer, or a thinned mesh (`tools/decimate_mesh.py`)
    /// where merging folded a garment's inner and outer layers into one
    /// and left patches of it facing in -- and a trained splat lit by the
    /// facing of the vertex it rides ([`SplatBody::bind`]) then shows as a
    /// dark fleck. Only a facing more than a little inward is turned, so a
    /// fold or a finger that faces across its own bone is left as it is.
    fn face_out(&mut self, puppet: &Puppet) {
        for (k, n) in self.mesh.nrm.iter_mut().enumerate() {
            let Some(s) = self.seg.get(k).and_then(|&s| puppet.segments.get(s)) else { continue };
            let p = self.mesh.pos[k];
            let centres: Vec<V3> = if s.stations.len() >= 2 { s.stations.iter().map(|st| st.c).collect() } else { vec![s.head, s.tail] };
            // the nearest point of that line
            let mut best: Option<(f32, V3)> = None;
            for w in centres.windows(2) {
                let (a, b) = (w[0], w[1]);
                let ab = sub(b, a);
                let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-9)).clamp(0.0, 1.0);
                let c = add(a, scale(ab, t));
                let d = len(sub(p, c));
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, c));
                }
            }
            let Some((_, c)) = best else { continue };
            let out = norm_or(sub(p, c), [0.0; 3]);
            if dot(*n, out) < -FACES_IN {
                *n = scale(*n, -1.0);
            }
        }
    }

    /// Every vertex's place and facing in `posed`'s pose (the puppet this
    /// was bound to).
    pub fn posed(&self, posed: &Posed<'_>) -> (Vec<V3>, Vec<V3>) {
        self.posed_by(posed, Blend::default())
    }

    /// [`BoundMesh::posed`], shared vertices moved as `blend` says.
    pub fn posed_by(&self, posed: &Posed<'_>, blend: Blend) -> (Vec<V3>, Vec<V3>) {
        let xf = BoundMesh::bone_xfs(posed);
        let turns: Vec<Dual> = xf.iter().map(dual_of).collect();
        self.mesh
            .pos
            .iter()
            .zip(&self.mesh.nrm)
            .zip(&self.weights)
            .map(|((p, n), w)| blend_point(*p, *n, w.iter().filter(|s| s.1 > 0.0).map(|&(s, share)| (&xf[s], &turns[s], share)), blend))
            .unzip()
    }
}

/// A ring needs this many of the mesh's vertices in its slab to be measured
/// from them.
const RING_MIN_VERTICES: usize = 8;
/// The share of a ring's vertices left out at each end of its width and
/// depth: a stray (a hair's tip, a seam's fold) does not set a ring's size.
const RING_TRIM: f32 = 0.02;

impl BoundMesh {
    /// `puppet` (the one this is bound to) with its rings measured from the
    /// mesh: each ring's centre, half-width and half-depth are those of the
    /// vertices its segment holds, in the slab of the bone that ring stands
    /// for. A puppet measured from two flat outlines guesses each ring's
    /// depth from one row of the side view; the mesh has it.
    ///
    /// The skeleton, the segments and their rings' count and places along
    /// the bones stay: a pose, a wound's address and the baked format mean
    /// what they did. A ring with too few vertices keeps its measure.
    pub fn measured(&self, puppet: &Puppet) -> Puppet {
        let mut out = puppet.clone();
        let mut of_seg: Vec<Vec<u32>> = vec![Vec::new(); puppet.segments.len()];
        for (v, s) in self.seg.iter().enumerate() {
            of_seg[*s].push(v as u32);
        }
        let slab = crate::puppet::STATION_STEP_FT * 0.6;
        for (i, seg) in out.segments.iter_mut().enumerate() {
            if !seg.kind.is_body() || seg.hidden || of_seg[i].len() < RING_MIN_VERTICES {
                continue;
            }
            let axis = norm_or(sub(seg.tail, seg.head), [0.0, 1.0, 0.0]);
            let (aw, ad) = (seg.axis_w, seg.axis_d);
            let mut found: Vec<Option<(V3, f32, f32)>> = Vec::with_capacity(seg.stations.len());
            for st in &seg.stations {
                let (mut us, mut vs): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
                for &v in &of_seg[i] {
                    let e = sub(self.mesh.pos[v as usize], st.c);
                    if dot(e, axis).abs() <= slab {
                        us.push(dot(e, aw));
                        vs.push(dot(e, ad));
                    }
                }
                if us.len() < RING_MIN_VERTICES {
                    found.push(None);
                    continue;
                }
                let span = |xs: &mut Vec<f32>| {
                    xs.sort_by(f32::total_cmp);
                    let cut = ((xs.len() as f32 * RING_TRIM) as usize).min(xs.len() / 4);
                    (xs[cut], xs[xs.len() - 1 - cut])
                };
                let ((u0, u1), (v0, v1)) = (span(&mut us), span(&mut vs));
                let c = add(st.c, add(scale(aw, (u0 + u1) * 0.5), scale(ad, (v0 + v1) * 0.5)));
                found.push(Some((c, (u1 - u0) * 0.5, (v1 - v0) * 0.5)));
            }
            // A segment the mesh mostly passes by (a collarbone, which holds
            // few vertices of its own) keeps its rings: a few measured among
            // the rest would stand out as spikes.
            if found.iter().flatten().count() * 2 < found.len() {
                continue;
            }
            // Evened along the bone. A ring near a joint takes in what the
            // next part holds there (a shoulder's ring, the sleeve's top) and
            // stands out from its neighbours as a spike: the middle of each
            // five first, then the mean of each three.
            let n = found.len();
            let middle = |k: usize, of: &dyn Fn(usize) -> Option<f32>| {
                let mut near: Vec<f32> = (k.saturating_sub(2)..=(k + 2).min(n - 1)).filter_map(of).collect();
                near.sort_by(f32::total_cmp);
                near[near.len() / 2]
            };
            let calm: Vec<Option<(f32, f32, f32, f32)>> = (0..n)
                .map(|k| {
                    let (c, _, _) = found[k]?;
                    let off = |j: usize, axis: V3| found[j].map(|(cj, _, _)| dot(sub(cj, seg.stations[j].c), axis));
                    let _ = c;
                    Some((middle(k, &|j| found[j].map(|f| f.1)), middle(k, &|j| found[j].map(|f| f.2)), middle(k, &|j| off(j, aw)), middle(k, &|j| off(j, ad))))
                })
                .collect();
            let was = seg.stations.clone();
            for k in 0..n {
                if calm[k].is_none() {
                    // unmeasured (a ring buried in the next part, a cap):
                    // changed as the nearest measured ring was
                    let Some(j) = (0..n).filter(|j| calm[*j].is_some()).min_by_key(|j| j.abs_diff(k)) else { continue };
                    let (w, d, du, dv) = calm[j].expect("just filtered");
                    let c = add(was[k].c, add(scale(aw, du), scale(ad, dv)));
                    seg.stations[k] = crate::puppet::Station { c, w: (was[k].w * w / was[j].w.max(0.02)).max(0.02), d: (was[k].d * d / was[j].d.max(0.02)).max(0.02) };
                    continue;
                }
                let near: Vec<(f32, f32, f32, f32)> = (k.saturating_sub(1)..=(k + 1).min(n - 1)).filter_map(|j| calm[j]).collect();
                let m = near.len() as f32;
                let (w, d, du, dv) = near.iter().fold((0.0, 0.0, 0.0, 0.0), |a, x| (a.0 + x.0 / m, a.1 + x.1 / m, a.2 + x.2 / m, a.3 + x.3 / m));
                let c = add(was[k].c, add(scale(aw, du), scale(ad, dv)));
                seg.stations[k] = crate::puppet::Station { c, w: w.max(0.02), d: d.max(0.02) };
            }
        }
        // what a puppet works out from its rings (how its arms hang) again
        Puppet::from_baked(&out.to_baked()).unwrap_or(out)
    }
}

/// How a vertex shared between segments is moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    /// The weighted mean of where each segment would put it. Cheap, and
    /// wrong at a joint turned far: the mean of two places on a circle is
    /// inside the circle, so a raised shoulder caves or balloons.
    #[default]
    Linear,
    /// The segments' rigid motions blended as dual quaternions: the vertex
    /// keeps its distance from the joint, so a turned joint keeps its girth.
    /// It needs shares that stay near their joints: a vertex shared by two
    /// segments turned opposite ways (both arms) is thrown far out.
    DualQuat,
}

/// A rigid motion as a dual quaternion: its turn, and half its shift times
/// its turn.
type Dual = (Quat, Quat);

fn quat_mul(a: Quat, b: Quat) -> Quat {
    [
        a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
        a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
        a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
        a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0],
    ]
}

fn dual_of(xf: &Xf) -> Dual {
    let turn = quat_from_m3(&xf.r);
    (turn, quat_mul([0.0, xf.t[0], xf.t[1], xf.t[2]], turn).map(|v| v * 0.5))
}

/// `p` facing `n`, moved by `shares` (each segment's transform, the same as
/// a dual quaternion, and its weight; the weights sum to one).
fn blend_point<'a>(p: V3, n: V3, shares: impl Iterator<Item = (&'a Xf, &'a Dual, f32)>, blend: Blend) -> (V3, V3) {
    match blend {
        Blend::Linear => {
            let (mut at, mut facing) = ([0.0f32; 3], [0.0f32; 3]);
            for (xf, _, share) in shares {
                at = add(at, scale(xf.point(p), share));
                facing = add(facing, scale(xf.dir(n), share));
            }
            (at, norm_or(facing, n))
        }
        Blend::DualQuat => {
            let (mut real, mut dual) = ([0.0f32; 4], [0.0f32; 4]);
            let mut first: Option<Quat> = None;
            for (_, (turn, shift), share) in shares {
                // q and -q are one turn: take each on the first's side
                let lead = *first.get_or_insert(*turn);
                let side = if (0..4).map(|k| lead[k] * turn[k]).sum::<f32>() < 0.0 { -share } else { share };
                for k in 0..4 {
                    real[k] += turn[k] * side;
                    dual[k] += shift[k] * side;
                }
            }
            let l = real.iter().map(|v| v * v).sum::<f32>().sqrt();
            if !(l > 1e-6) {
                return (p, n);
            }
            let (real, dual) = (real.map(|v| v / l), dual.map(|v| v / l));
            let r = quat_to_m3(real);
            let t = quat_mul(dual, [real[0], -real[1], -real[2], -real[3]]);
            (add(mul_mv(&r, p), [2.0 * t[1], 2.0 * t[2], 2.0 * t[3]]), mul_mv(&r, n))
        }
    }
}

/// A body to wear: a mesh bound to a puppet, and its sheets. Made once and
/// shared; a GPU host uploads it once.
#[derive(Debug)]
pub struct BodyMesh {
    pub bound: BoundMesh,
    pub paint: Paint,
    /// Its tangent-space normal sheet, when it has one.
    pub normal: Option<Paint>,
    /// The same body as a splat cloud riding the mesh, when it has one.
    pub splats: Option<std::sync::Arc<SplatBody>>,
    /// The same body as splats MADE from the mesh ([`BodyMesh::made_splats`]),
    /// once asked for.
    pub made: std::sync::OnceLock<std::sync::Arc<SplatBody>>,
    /// `splats` thinned for a small figure ([`BodyMesh::splats_at`]), once
    /// asked for: [`TIERS`]' shares, in order.
    pub thinned: [std::sync::OnceLock<std::sync::Arc<SplatBody>>; TIERS.len()],
    /// How many splats its made cloud aims at ([`MADE_SPLATS`] unless its
    /// figure says: a made cloud is the cost of every wound on it).
    pub made_count: usize,
    /// Its paint lands on its mesh: what splats made from them need. Not so
    /// on a thin mesh (`tools/decimate_mesh.py` merges the sheet's places
    /// away) nor without a sheet (a client's body ships only its cloud).
    pub paint_lands: bool,
}

/// A trained cloud's levels of detail: at a drawn height of at least the
/// first number (pixels), this share of its splats, the most important
/// (opaque and wide) first. The whole cloud from 240 px up; a third at 110
/// and 160; an eighth at 76 and under. Picked by the key's size step
/// (`creature_puppet::SIZE_STEPS`), so a crowd of small figures costs a
/// fraction of the doll.
pub const TIERS: [(u16, f32); 2] = [(110, 1.0 / 3.0), (0, 1.0 / 8.0)];

/// The drawn height a cloud is still shown whole at.
pub const WHOLE_CLOUD_PX: u16 = 240;

/// How many splats a body's made cloud aims at: a dot every few thousandths
/// of a foot on a figure, fine enough to carry a 4096 sheet's detail at doll
/// size, and within what one storage buffer holds.
pub const MADE_SPLATS: usize = 800_000;

impl BodyMesh {
    /// The body as splats made straight from its mesh and sheet -- no
    /// fitting: a flat Gaussian at each of about [`MADE_SPLATS`] points
    /// spread evenly over the surface, coloured from the sheet there, facing
    /// as the surface does and moved by the shares of the corner it is
    /// nearest. As sharp as the sheet (a cloud fitted to pictures of the mesh
    /// smooths it), and made in a moment. Built on first use.
    pub fn made_splats(&self) -> std::sync::Arc<SplatBody> {
        self.made.get_or_init(|| std::sync::Arc::new(SplatBody::from_mesh(&self.bound, &self.paint, self.made_count))).clone()
    }

    /// The trained cloud for a figure drawn `height_px` tall: whole from
    /// [`WHOLE_CLOUD_PX`] up, else the [`TIERS`] share for that height,
    /// thinned once and kept. None where the body has no trained cloud.
    pub fn splats_at(&self, height_px: u16) -> Option<std::sync::Arc<SplatBody>> {
        let whole = self.splats.as_ref()?;
        if height_px >= WHOLE_CLOUD_PX {
            return Some(whole.clone());
        }
        let tier = TIERS.iter().position(|(from, _)| height_px >= *from).unwrap_or(TIERS.len() - 1);
        Some(self.thinned[tier].get_or_init(|| std::sync::Arc::new(whole.thinned(TIERS[tier].1))).clone())
    }
}

/// How one Gaussian is bound to a body's mesh ([`SplatBody::bind_as`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindAs {
    /// As the Gaussian at this index of a cloud already bound to it.
    Like(u32),
    /// To this vertex.
    Vertex(u32),
    /// To the vertex nearest it, searched for.
    Near,
    /// Rigidly to this segment of the puppet: what is built round a part
    /// (a rope, a vine) holds to that part, not to whatever skin is nearest
    /// it (near an armpit, an arm's).
    Rigid(u32),
}

/// A mesh's vertices filed by place, for finding the one nearest a point.
struct VertexGrid {
    grid: HashMap<[i32; 3], Vec<u32>>,
}

impl VertexGrid {
    const CELL: f32 = 0.06;

    fn cell(p: V3) -> [i32; 3] {
        [(p[0] / Self::CELL).floor() as i32, (p[1] / Self::CELL).floor() as i32, (p[2] / Self::CELL).floor() as i32]
    }

    fn new(bound: &BoundMesh) -> VertexGrid {
        let mut grid: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        for (k, p) in bound.mesh.pos.iter().enumerate() {
            grid.entry(Self::cell(*p)).or_default().push(k as u32);
        }
        VertexGrid { grid }
    }

    /// The vertex nearest `q`, looking wider and wider until what was
    /// found is nearer than the cells not yet looked in; None past six
    /// cells (a stray far from the body).
    fn nearest(&self, bound: &BoundMesh, q: V3) -> Option<usize> {
        let c = Self::cell(q);
        for reach in 1..=6i32 {
            let mut best: Option<(f32, u32)> = None;
            for dx in -reach..=reach {
                for dy in -reach..=reach {
                    for dz in -reach..=reach {
                        for &k in self.grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).into_iter().flatten() {
                            let d = len(sub(bound.mesh.pos[k as usize], q));
                            if best.is_none_or(|(b, _)| d < b) {
                                best = Some((d, k));
                            }
                        }
                    }
                }
            }
            if let Some((_, k)) = best.filter(|(d, _)| *d <= reach as f32 * Self::CELL) {
                return Some(k as usize);
            }
        }
        None
    }

    /// The skin's facing at `q`, smooth (ADDITION): the facings of the
    /// vertices within [`FACING_REACH_FT`] of it, nearer ones counting
    /// more, those facing as most of them do -- or, where they cancel
    /// ([`FACING_AGREE`]: an ear, a finger), as vertex `near` does. A
    /// splat lit by its one nearest vertex's facing took that vertex's
    /// facing whole, and on a thinned mesh (a vertex every few hundredths
    /// of a foot, facings varying vertex to vertex) neighbouring splats
    /// were lit as a patchwork, light and dark; and kept to those facing
    /// as the nearest does, a folded vertex nearest led the rest astray.
    fn facing_at(&self, bound: &BoundMesh, q: V3, near: usize) -> V3 {
        let n0 = bound.mesh.nrm[near];
        let c = Self::cell(q);
        let reach = (FACING_REACH_FT / Self::CELL).ceil() as i32;
        let mut round: Vec<(V3, f32)> = Vec::new();
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for dz in -reach..=reach {
                    for &k in self.grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).into_iter().flatten() {
                        let d = len(sub(bound.mesh.pos[k as usize], q));
                        if d < FACING_REACH_FT {
                            let w = 1.0 - d / FACING_REACH_FT;
                            round.push((bound.mesh.nrm[k as usize], w * w));
                        }
                    }
                }
            }
        }
        let all = round.iter().fold([0.0f32; 3], |s, (n, w)| add(s, scale(*n, *w)));
        let weight: f32 = round.iter().map(|(_, w)| w).sum();
        let way = if len(all) > FACING_AGREE * weight { all } else { n0 };
        let sum = round.iter().filter(|(n, _)| dot(*n, way) > 0.0).fold([0.0f32; 3], |s, (n, w)| add(s, scale(*n, *w)));
        norm_or(sum, n0)
    }

    /// A vertex for what is far from all of them: the nearest by a coarse look.
    fn far(bound: &BoundMesh, q: V3) -> Option<usize> {
        (0..bound.mesh.pos.len()).step_by(16).min_by(|a, b| len(sub(bound.mesh.pos[*a], q)).total_cmp(&len(sub(bound.mesh.pos[*b], q))))
    }
}

/// The vertex of `bound` nearest each of `points` (by a coarse look for one
/// far from them all; None only for an empty mesh), on every core: what a
/// host binds a cloud's anchors to once ([`BindAs::Vertex`]).
pub fn nearest_vertices(bound: &BoundMesh, points: &[V3]) -> Vec<Option<u32>> {
    let grid = VertexGrid::new(bound);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    let share = points.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let running: Vec<_> = points
            .chunks(share)
            .map(|chunk| {
                let grid = &grid;
                scope.spawn(move || chunk.iter().map(|p| grid.nearest(bound, *p).or_else(|| VertexGrid::far(bound, *p)).map(|k| k as u32)).collect::<Vec<_>>())
            })
            .collect();
        running.into_iter().flat_map(|r| r.join().expect("a chunk of nearest vertices")).collect()
    })
}

/// A splat cloud fitted to a mesh (trained from pictures of it), riding
/// it: each Gaussian moves as the mesh does where it sits, and faces as the
/// mesh does there (a fitted cloud has no facing of its own to be lit by).
#[derive(Debug, Clone, Default)]
pub struct SplatBody {
    /// The cloud in the puppet's rest space, each with its facing.
    pub rest: Vec<crate::splat::Gaussian>,
    /// Per Gaussian: the shares of the mesh vertex nearest it.
    pub weights: Vec<Shares>,
    /// A hit arriving (`layers::LayerCloud::arrival`), per Gaussian: 0
    /// always shown, k + 1 the hit part's frame at `layers::ARRIVAL_AGES[k]`,
    /// shown blended by the hit's age (the GPU's, `cloud_gpu.wgsl`). Empty:
    /// all always shown.
    pub frame: Vec<u8>,
    /// What the hit threw off: each the index of its stand-in Gaussian
    /// (in `rest`, at its launch point, bound as one) and how it moves
    /// (`layers::Particle::at`, played by the GPU).
    pub particles: Vec<(u32, crate::layers::Particle)>,
    /// Per Gaussian: lit by its own facing, its facing being its shape --
    /// what is built on the body (a vine's bark, a coil of rope, a bead of
    /// blood, a wound's inside). Else it is the body's skin, whose facing a
    /// renderer may smooth with its neighbours' (`cloud_gpu.wgsl`). Empty:
    /// all skin.
    pub own_facing: Vec<bool>,
}

impl SplatBody {
    /// `cloud` (in the mesh's own space: placed on it) bound to `bound`,
    /// each Gaussian facing as the mesh does where it sits.
    pub fn bind(bound: &BoundMesh, cloud: Vec<crate::splat::Gaussian>) -> SplatBody {
        SplatBody::bind_facing(bound, cloud, false)
    }

    /// [`SplatBody::bind`]; `keep_facing`: each Gaussian keeps the facing
    /// it came with (a made cloud's: the layered doll's dots, a wound's
    /// walls among them) -- one that came with none, on the surface (a
    /// fitted cloud's own Gaussian the layered doll wears), takes the
    /// mesh's there, as an unmarked body's does, so a body lights alike
    /// hurt and whole -- and one far from the mesh stays, moving with the
    /// vertex nearest it all the same (a drop flung off a wound).
    pub fn bind_facing(bound: &BoundMesh, mut cloud: Vec<crate::splat::Gaussian>, keep_facing: bool) -> SplatBody {
        let grid = VertexGrid::new(bound);
        let mut weights = Vec::with_capacity(cloud.len());
        // a Gaussian far from every vertex is a stray the fit left in the air
        let mut kept = Vec::with_capacity(cloud.len());
        for mut g in cloud.drain(..) {
            let close = grid.nearest(bound, g.c);
            if let Some(k) = close.or_else(|| VertexGrid::far(bound, g.c).filter(|_| keep_facing)) {
                if !keep_facing || (close.is_some() && g.n == [0.0; 3]) {
                    g.n = grid.facing_at(bound, g.c, k);
                }
                weights.push(bound.weights[k]);
                kept.push(g);
            }
        }
        SplatBody { rest: kept, weights, ..Default::default() }
    }

    pub fn len(&self) -> usize {
        self.rest.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    /// `cloud` (in the mesh's own space) bound to `bound` each Gaussian as
    /// it says ([`BindAs`]): like a Gaussian of `like`, a cloud already
    /// bound to it; to a vertex already found; or to the vertex nearest,
    /// searched for as [`SplatBody::bind_facing`] keeping facings does. A
    /// Gaussian with no facing takes its binding's. What the layered doll
    /// rebuilds as a hit arrives: its fitted Gaussians and its anchors keep
    /// the bindings they had, so only what flew off is searched for.
    pub fn bind_as(bound: &BoundMesh, like: Option<&SplatBody>, cloud: Vec<(crate::splat::Gaussian, BindAs)>) -> SplatBody {
        let mut grid: Option<VertexGrid> = None;
        let mut rest = Vec::with_capacity(cloud.len());
        let mut weights = Vec::with_capacity(cloud.len());
        for (mut g, how) in cloud {
            let bare = g.n == [0.0; 3];
            match how {
                BindAs::Like(k) if like.is_some_and(|l| (k as usize) < l.len()) => {
                    let l = like.expect("checked");
                    if bare {
                        g.n = l.rest[k as usize].n;
                    }
                    weights.push(l.weights[k as usize]);
                    rest.push(g);
                    continue;
                }
                BindAs::Rigid(seg) => {
                    weights.push([(seg as usize, 1.0), (seg as usize, 0.0), (seg as usize, 0.0), (seg as usize, 0.0)]);
                    rest.push(g);
                    continue;
                }
                BindAs::Vertex(v) if (v as usize) < bound.weights.len() => {
                    if bare {
                        g.n = bound.mesh.nrm[v as usize];
                    }
                    weights.push(bound.weights[v as usize]);
                    rest.push(g);
                    continue;
                }
                _ => {}
            }
            let grid = grid.get_or_insert_with(|| VertexGrid::new(bound));
            let close = grid.nearest(bound, g.c);
            if let Some(k) = close.or_else(|| VertexGrid::far(bound, g.c)) {
                if bare && close.is_some() {
                    g.n = bound.mesh.nrm[k];
                }
                weights.push(bound.weights[k]);
                rest.push(g);
            }
        }
        SplatBody { rest, weights, ..Default::default() }
    }

    /// This cloud and `more`, one after the other (their arrival frames
    /// and particles with them).
    pub fn joined(&self, more: SplatBody) -> SplatBody {
        let n = self.len();
        let frame = if self.frame.is_empty() && more.frame.is_empty() {
            Vec::new()
        } else {
            let pad = |f: &[u8], len: usize| if f.is_empty() { vec![0; len] } else { f.to_vec() };
            let mut frame = pad(&self.frame, n);
            frame.extend(pad(&more.frame, more.len()));
            frame
        };
        let own_facing = if self.own_facing.is_empty() && more.own_facing.is_empty() {
            Vec::new()
        } else {
            let pad = |f: &[bool], len: usize| if f.is_empty() { vec![false; len] } else { f.to_vec() };
            let mut own = pad(&self.own_facing, n);
            own.extend(pad(&more.own_facing, more.len()));
            own
        };
        let mut particles = self.particles.clone();
        particles.extend(more.particles.iter().map(|(i, p)| (i + n as u32, *p)));
        let mut rest = Vec::with_capacity(n + more.len());
        rest.extend_from_slice(&self.rest);
        rest.extend(more.rest);
        let mut weights = Vec::with_capacity(rest.len());
        weights.extend_from_slice(&self.weights);
        weights.extend(more.weights);
        SplatBody { rest, weights, frame, particles, own_facing }
    }

    /// The `keep` share (0..1) of this cloud that matters most: each
    /// Gaussian weighed by its opacity times its footprint (the two longest
    /// axes), the heaviest kept, in their original order. A small figure
    /// is drawn from this, not the whole cloud ([`BodyMesh::splats_at`]).
    pub fn thinned(&self, keep: f32) -> SplatBody {
        let want = ((self.rest.len() as f32 * keep.clamp(0.0, 1.0)).round() as usize).min(self.rest.len());
        if want == self.rest.len() {
            return self.clone();
        }
        let weight = |g: &crate::splat::Gaussian| {
            let mut l = g.axes.map(len);
            l.sort_by(f32::total_cmp);
            g.alpha * l[1] * l[2]
        };
        let mut by_weight: Vec<(f32, u32)> = self.rest.iter().enumerate().map(|(i, g)| (weight(g), i as u32)).collect();
        by_weight.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut keep_idx: Vec<u32> = by_weight[..want].iter().map(|(_, i)| *i).collect();
        keep_idx.sort_unstable();
        let own_facing = if self.own_facing.is_empty() { Vec::new() } else { keep_idx.iter().map(|&i| self.own_facing[i as usize]).collect() };
        SplatBody { rest: keep_idx.iter().map(|&i| self.rest[i as usize].clone()).collect(), weights: keep_idx.iter().map(|&i| self.weights[i as usize]).collect(), own_facing, ..Default::default() }
    }

    /// A cloud made from `bound`'s mesh and its `paint`: about `count`
    /// flat Gaussians spread evenly over the surface (see
    /// [`BodyMesh::made_splats`]).
    pub fn from_mesh(bound: &BoundMesh, paint: &Paint, count: usize) -> SplatBody {
        let mesh = &bound.mesh;
        let areas: Vec<f32> = mesh
            .tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.pos[i as usize]);
                len(cross(sub(b, a), sub(c, a))) * 0.5
            })
            .collect();
        let total: f32 = areas.iter().sum();
        if !(total > 0.0) || count == 0 {
            return SplatBody { rest: Vec::new(), weights: Vec::new(), ..Default::default() };
        }
        // the surface each splat stands for, and the grid step that gives
        let each = total / count as f32;
        let mut rest = Vec::with_capacity(count + mesh.tris.len());
        let mut weights = Vec::with_capacity(count + mesh.tris.len());
        for (t, area) in mesh.tris.iter().zip(&areas) {
            if !(*area > 1e-12) {
                continue;
            }
            let [ia, ib, ic] = t.map(|i| i as usize);
            let (pa, pb, pc) = (mesh.pos[ia], mesh.pos[ib], mesh.pos[ic]);
            let face = norm_or(cross(sub(pb, pa), sub(pc, pa)), [0.0, 0.0, 1.0]);
            // n rows of points across the triangle: n * (n + 1) / 2 of them
            let want = (area / each).max(1.0);
            let n = (((8.0 * want + 1.0).sqrt() - 1.0) * 0.5).round().max(1.0) as usize;
            let points = n * (n + 1) / 2;
            // a round dot of the surface it stands for: sigma so that
            // neighbours' cores overlap, thin along the surface's facing
            let sigma = (area / points as f32).sqrt() * 0.62;
            for i in 0..n {
                for j in 0..n - i {
                    // the middle of each small triangle of the same way up
                    let (u, v) = ((i as f32 + 1.0 / 3.0) / n as f32, (j as f32 + 1.0 / 3.0) / n as f32);
                    let w = 1.0 - u - v;
                    let at = |a: V3, b: V3, c: V3| add(add(scale(a, w), scale(b, u)), scale(c, v));
                    let uv = [w * mesh.uv[ia][0] + u * mesh.uv[ib][0] + v * mesh.uv[ic][0], w * mesh.uv[ia][1] + u * mesh.uv[ib][1] + v * mesh.uv[ic][1]];
                    let facing = norm_or(at(mesh.nrm[ia], mesh.nrm[ib], mesh.nrm[ic]), face);
                    let side = norm_or(cross(facing, if facing[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }), [1.0, 0.0, 0.0]);
                    let along = cross(facing, side);
                    let corner = if w >= u && w >= v { ia } else if u >= v { ib } else { ic };
                    rest.push(crate::splat::Gaussian {
                        c: at(pa, pb, pc),
                        axes: [scale(side, sigma), scale(along, sigma), scale(facing, sigma * 0.2)],
                        n: facing,
                        rgb: paint.sample(uv),
                        alpha: 0.95,
                        glow: 0.0,
                        wet: 0.0,
                        film: false,
                        rim: 0.0,
                    });
                    weights.push(bound.weights[corner]);
                }
            }
        }
        SplatBody { rest, weights, ..Default::default() }
    }

    /// The splats' indices from the farthest to the nearest: each one's
    /// depth is `depth_row` (a projection's third row) of where `bones`
    /// (rest -> world, by rows) put its centre, sorted into 65536 steps of
    /// `span` (the depths the figure's box holds).
    pub fn far_to_near(&self, bones: &[[[f32; 4]; 3]], depth_row: [f32; 4], span: [f32; 2]) -> Vec<u32> {
        // per segment, depth as one row over the rest point
        let rows: Vec<[f32; 4]> = bones
            .iter()
            .map(|b| std::array::from_fn(|k| depth_row[0] * b[0][k] + depth_row[1] * b[1][k] + depth_row[2] * b[2][k] + if k == 3 { depth_row[3] } else { 0.0 }))
            .collect();
        const STEPS: usize = 1 << 16;
        let scale = (STEPS - 1) as f32 / (span[1] - span[0]).max(1e-6);
        let mut counts = vec![0u32; STEPS + 1];
        let steps: Vec<u16> = self
            .rest
            .iter()
            .zip(&self.weights)
            .map(|(g, w)| {
                let mut depth = 0.0;
                for &(s, share) in w {
                    if share > 0.0 {
                        let r = rows.get(s).copied().unwrap_or([0.0, 0.0, 0.0, span[0]]);
                        depth += share * (r[0] * g.c[0] + r[1] * g.c[1] + r[2] * g.c[2] + r[3]);
                    }
                }
                // the farthest first
                let step = (STEPS - 1) - (((depth - span[0]) * scale) as isize).clamp(0, STEPS as isize - 1) as usize;
                counts[step + 1] += 1;
                step as u16
            })
            .collect();
        for k in 0..STEPS {
            counts[k + 1] += counts[k];
        }
        let mut order = vec![0u32; steps.len()];
        for (i, step) in steps.iter().enumerate() {
            let at = &mut counts[*step as usize];
            order[*at as usize] = i as u32;
            *at += 1;
        }
        order
    }

    /// The cloud as the layered doll wears a fitted cloud
    /// (`layers::LayerCloud::fit_bound`): each Gaussian with its three
    /// largest shares, moved as the mesh is.
    pub fn bound_cloud(&self) -> crate::splat::BoundCloud {
        let weights = self
            .weights
            .iter()
            .map(|w| {
                let total: f32 = w[..3].iter().map(|s| s.1).sum::<f32>().max(1e-6);
                [(w[0].0, w[0].1 / total), (w[1].0, w[1].1 / total), (w[2].0, w[2].1 / total)]
            })
            .collect();
        crate::splat::BoundCloud { rest: self.rest.clone(), seg: self.weights.iter().map(|w| w[0].0).collect(), weights, modelled: true }
    }
}

/// How far past the puppet's rings a body's mesh may reach (hair, ears,
/// fingers, a pouch), feet: the frame's image is that much larger than the
/// rings' box.
const BODY_MARGIN_FT: f32 = 0.45;

/// One pose of a [`BodyMesh`], for a host that moves the vertices itself
/// (a GPU): nothing here is per vertex. The host puts vertex `p` (rest
/// space) at `sum(share * bones[segment] * p)` in the turned world, then
/// through `proj` on the screen.
#[derive(Debug, Clone)]
pub struct BodyFrame {
    pub body: std::sync::Arc<BodyMesh>,
    /// As [`Skin`]'s: the image the figure fits in (with a margin: it is
    /// found from the puppet's rings, not the vertices).
    pub origin: [f32; 2],
    pub w: usize,
    pub h: usize,
    /// Per segment of the puppet: rest space -> the turned world, a 3 x 4
    /// matrix by rows.
    pub bones: Vec<[[f32; 4]; 3]>,
    /// The turned world -> (x * depth, y * depth, depth), by rows: screen
    /// pixels are the first two over the third. Exact for a pinhole or a
    /// parallel camera.
    pub proj: [[f32; 4]; 3],
    /// The nearest and farthest depth the image's box holds.
    pub depth: [f32; 2],
    /// Toward the viewer and toward the light, in the turned world.
    pub to_cam: V3,
    pub light: V3,
    /// The box the figure is in (the turned world).
    pub corners: [V3; 8],
    /// The splat cloud it is drawn as (the body's own, or one made for it:
    /// its splats with its wounds), not its mesh.
    pub cloud: Option<std::sync::Arc<SplatBody>>,
    /// That cloud's splats from the farthest to the nearest in this pose
    /// and view: a fitted cloud is many faint Gaussians laid over each
    /// other, and only looks as it was fitted drawn in that order. (Their
    /// depths come from the segments' transforms: no splat is posed here.)
    pub order: Vec<u32>,
    /// Identifies these pixels.
    pub key: u64,
}

impl BoundMesh {
    /// Each segment's transform for the mesh in `posed`'s pose.
    ///
    /// A hanging arm's hand is turned a quarter turn about its own length
    /// (`hang.rs`: a mitten drawn broad from the front lies flat by the
    /// thigh). A modelled hand already has its palm where it belongs, and
    /// the turn wrings its wrist: taken back out.
    pub fn bone_xfs(posed: &Posed<'_>) -> Vec<Xf> {
        posed
            .xf
            .iter()
            .enumerate()
            .map(|(i, xf)| match posed.puppet.hand_turn(i).filter(|_| posed.hung) {
                Some(turn) => xf.then_inner(&Xf::about(posed.puppet.segments[i].head, transpose(turn))),
                None => *xf,
            })
            .collect()
    }
}

impl BodyFrame {
    /// Bytes this frame holds on the heap of its own (its bones and draw
    /// order; the body and cloud it points at are shared).
    pub fn heap_bytes(&self) -> usize {
        self.bones.capacity() * std::mem::size_of::<[[f32; 4]; 3]>() + self.order.capacity() * 4
    }

    /// `body` in `posed`'s pose (the puppet it was bound to), turned by
    /// `yaw` and seen through `project`; as `cloud` when there is one
    /// (bound to this body's mesh), else as its mesh. None when nothing
    /// would show.
    pub fn new(body: &std::sync::Arc<BodyMesh>, posed: &Posed<'_>, project: &dyn Fn(V3) -> ([f32; 2], f32), yaw: f32, cloud: Option<std::sync::Arc<SplatBody>>) -> Option<BodyFrame> {
        let cloud = cloud.filter(|c| !c.is_empty());
        use std::hash::{Hash, Hasher};
        let rot = rot_y(yaw);
        let bones: Vec<[[f32; 4]; 3]> = BoundMesh::bone_xfs(posed)
            .iter()
            .map(|xf| {
                let (r, t) = (mul_mm(&rot, &xf.r), mul_mv(&rot, xf.t));
                std::array::from_fn(|k| [r[k][0], r[k][1], r[k][2], t[k]])
            })
            .collect();
        let [lo, hi] = posed.bounds_where(|s| s.kind.is_body());
        if lo == hi {
            return None;
        }
        let corners: [V3; 8] = std::array::from_fn(|i| {
            let pick = |k: usize| if i >> k & 1 == 0 { lo[k] - BODY_MARGIN_FT } else { hi[k] + BODY_MARGIN_FT };
            mul_mv(&rot, [pick(0), pick(1), pick(2)])
        });
        // screen * depth and depth are affine in the point: found from the
        // projection at the box's middle and a foot along each axis
        let centre = scale(corners.iter().fold([0.0; 3], |a, c| add(a, *c)), 1.0 / 8.0);
        let homog = |p: V3| {
            let (s, d) = project(p);
            [s[0] * d, s[1] * d, d]
        };
        let h0 = homog(centre);
        let steps: [[f32; 3]; 3] = std::array::from_fn(|k| {
            let mut p = centre;
            p[k] += 1.0;
            let h = homog(p);
            [h[0] - h0[0], h[1] - h0[1], h[2] - h0[2]]
        });
        let proj: [[f32; 4]; 3] = std::array::from_fn(|row| {
            let lin = [steps[0][row], steps[1][row], steps[2][row]];
            [lin[0], lin[1], lin[2], h0[row] - dot(lin, centre)]
        });
        let (mut a, mut b, mut near, mut far) = ([f32::MAX; 2], [f32::MIN; 2], f32::MAX, f32::MIN);
        for c in &corners {
            let (s, d) = project(*c);
            if !(s[0].is_finite() && s[1].is_finite()) {
                return None;
            }
            a = [a[0].min(s[0]), a[1].min(s[1])];
            b = [b[0].max(s[0]), b[1].max(s[1])];
            near = near.min(d);
            far = far.max(d);
        }
        let origin = [a[0].floor(), a[1].floor()];
        let (w, h) = ((b[0] - origin[0]).ceil() as usize, (b[1] - origin[1]).ceil() as usize);
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return None;
        }
        let view = crate::draw::view_basis(project, centre);
        let order = cloud.as_ref().map(|c| c.far_to_near(&bones, proj[2], [near, far])).unwrap_or_default();
        let key = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (std::sync::Arc::as_ptr(body) as usize, w, h, origin.map(f32::to_bits), cloud.as_ref().map(|c| std::sync::Arc::as_ptr(c) as usize)).hash(&mut hasher);
            for row in bones.iter().flatten().chain(&proj) {
                row.map(f32::to_bits).hash(&mut hasher);
            }
            hasher.finish()
        };
        Some(BodyFrame { body: body.clone(), origin, w, h, bones, proj, depth: [near, far], to_cam: view.to_cam, light: view.light, corners, cloud, order, key })
    }

    /// Where the host puts vertex `i`: the turned world, the screen
    /// (pixels) and its depth -- what its shader works out.
    pub fn place(&self, i: usize) -> (V3, [f32; 2], f32) {
        let p = self.body.bound.mesh.pos[i];
        let mut at = [0.0f32; 3];
        for &(s, share) in &self.body.bound.weights[i] {
            if share > 0.0 {
                let m = &self.bones[s];
                at = add(at, scale(std::array::from_fn(|k| m[k][0] * p[0] + m[k][1] * p[1] + m[k][2] * p[2] + m[k][3]), share));
            }
        }
        let h: [f32; 3] = std::array::from_fn(|k| self.proj[k][0] * at[0] + self.proj[k][1] * at[1] + self.proj[k][2] * at[2] + self.proj[k][3]);
        (at, [h[0] / h[2], h[1] / h[2]], h[2])
    }
}

/// Samples a pixel side: the edges are smoothed over this many squared.
const SAMPLES: usize = 2;

/// The mesh with its vertices at `pos` facing `nrm` (its own, or
/// [`BoundMesh::posed`]'s), turned by `yaw` and drawn through `project`:
/// z-buffered, painted from `paint` (grey without) and shaded as the
/// puppet's skin is.
pub fn render(mesh: &Mesh, pos: &[V3], nrm: &[V3], paint: Option<&Paint>, project: &dyn Fn(V3) -> ([f32; 2], f32), yaw: f32) -> Skin {
    let empty = Skin { origin: [0.0, 0.0], w: 0, h: 0, rgba: Vec::new() };
    if pos.is_empty() || mesh.tris.is_empty() {
        return empty;
    }
    let rot = rot_y(yaw);
    let k = SAMPLES as f32;
    let flat: Vec<([f32; 2], f32)> = pos.iter().map(|p| project(mul_mv(&rot, *p))).collect();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for (p, _) in flat.iter().filter(|(p, _)| p[0].is_finite() && p[1].is_finite()) {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    if !(lo[0].is_finite() && hi[0] > lo[0] && hi[1] > lo[1]) {
        return empty;
    }
    let origin = [lo[0].floor() - 1.0, lo[1].floor() - 1.0];
    let (w, h) = ((hi[0] - origin[0]).ceil() as usize + 2, (hi[1] - origin[1]).ceil() as usize + 2);
    let (sw, sh) = (w * SAMPLES, h * SAMPLES);
    let n = pos.len() as f32;
    let centre = pos.iter().fold([0.0f32; 3], |a, p| add(a, scale(*p, 1.0 / n)));
    let light = crate::draw::view_basis(&|p| project(mul_mv(&rot, p)), centre).light;
    let shade: Vec<f32> = nrm
        .iter()
        .map(|n| {
            let g = dot(*n, light);
            let lit = crate::skin::smoothstep(crate::draw::LIGHT_ABOVE - 0.2, crate::draw::LIGHT_ABOVE + 0.2, g);
            let dark = 1.0 - crate::skin::smoothstep(crate::draw::DARK_BELOW - 0.2, crate::draw::DARK_BELOW + 0.2, g);
            (1.0 + 0.13 * lit - 0.2 * dark) * (1.0 + 0.08 * g)
        })
        .collect();
    let mut depth = vec![f32::MAX; sw * sh];
    let mut rgb = vec![[0.0f32; 3]; sw * sh];
    for t in &mesh.tris {
        let [a, b, c] = t.map(|i| i as usize);
        let at = |i: usize| [(flat[i].0[0] - origin[0]) * k, (flat[i].0[1] - origin[1]) * k];
        let (pa, pb, pc) = (at(a), at(b), at(c));
        let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
        if !(area.abs() > 1e-9) {
            continue;
        }
        let xa = (pa[0].min(pb[0]).min(pc[0]).floor() as i64).max(0);
        let xb = (pa[0].max(pb[0]).max(pc[0]).ceil() as i64).min(sw as i64 - 1);
        let ya = (pa[1].min(pb[1]).min(pc[1]).floor() as i64).max(0);
        let yb = (pa[1].max(pb[1]).max(pc[1]).ceil() as i64).min(sh as i64 - 1);
        for py in ya..=yb {
            for px in xa..=xb {
                let q = [px as f32 + 0.5, py as f32 + 0.5];
                let wa = ((pb[0] - q[0]) * (pc[1] - q[1]) - (pb[1] - q[1]) * (pc[0] - q[0])) / area;
                let wb = ((pc[0] - q[0]) * (pa[1] - q[1]) - (pc[1] - q[1]) * (pa[0] - q[0])) / area;
                let wc = 1.0 - wa - wb;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let z = wa * flat[a].1 + wb * flat[b].1 + wc * flat[c].1;
                let cell = py as usize * sw + px as usize;
                if z >= depth[cell] {
                    continue;
                }
                depth[cell] = z;
                let uv = [wa * mesh.uv[a][0] + wb * mesh.uv[b][0] + wc * mesh.uv[c][0], wa * mesh.uv[a][1] + wb * mesh.uv[b][1] + wc * mesh.uv[c][1]];
                let s = wa * shade[a] + wb * shade[b] + wc * shade[c];
                rgb[cell] = paint.map_or([150.0; 3], |p| p.sample(uv)).map(|v| v * s);
            }
        }
    }
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (mut sum, mut hit) = ([0.0f32; 3], 0usize);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let cell = (y * SAMPLES + sy) * sw + x * SAMPLES + sx;
                    if depth[cell] < f32::MAX {
                        sum = add(sum, rgb[cell]);
                        hit += 1;
                    }
                }
            }
            if hit > 0 {
                let px = &mut rgba[(y * w + x) * 4..][..4];
                for c in 0..3 {
                    px[c] = (sum[c] / hit as f32).round().clamp(0.0, 255.0) as u8;
                }
                px[3] = (hit * 255 / (SAMPLES * SAMPLES)) as u8;
            }
        }
    }
    Skin { origin, w, h, rgba }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bind_as_reuses_what_is_known_and_searches_the_rest() {
        let mesh = Mesh { pos: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], nrm: vec![[0.0, 0.0, 1.0], [1.0, 0.0, 0.0]], uv: vec![[0.0; 2]; 2], tris: vec![], source: vec![] };
        let bound = BoundMesh { mesh, seg: vec![0, 1], weights: vec![[(0, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)], [(1, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)]] };
        let g = |c: V3| crate::splat::Gaussian { c, axes: [[0.01, 0.0, 0.0], [0.0, 0.01, 0.0], [0.0, 0.0, 0.01]], n: [0.0; 3], rgb: [200.0; 3], alpha: 0.9, glow: 0.0, wet: 0.0, film: false, rim: 0.0 };
        let like = SplatBody { rest: vec![crate::splat::Gaussian { n: [0.0, 1.0, 0.0], ..g([0.5, 0.0, 0.0]) }], weights: vec![[(7, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)]], ..Default::default() };
        let out = SplatBody::bind_as(&bound, Some(&like), vec![(g([0.0, 0.0, 0.0]), BindAs::Like(0)), (g([0.0, 0.0, 0.0]), BindAs::Vertex(1)), (g([0.02, 0.0, 0.0]), BindAs::Near)]);
        assert_eq!(out.weights.iter().map(|w| w[0].0).collect::<Vec<_>>(), [7, 1, 0], "like the known one, the vertex given, the nearest");
        assert_eq!(out.rest[0].n, [0.0, 1.0, 0.0]);
        assert_eq!(out.rest[1].n, [1.0, 0.0, 0.0]);
        assert_eq!(out.rest[2].n, [0.0, 0.0, 1.0]);
        assert_eq!(nearest_vertices(&bound, &[[0.9, 0.0, 0.0], [0.1, 0.0, 0.0]]), [Some(1), Some(0)]);
        assert_eq!(like.joined(out).len(), 4);
    }

    /// A mesh's facings that face into the part they ride are turned out
    /// when it is bound (a thinned mesh leaves patches of them), so a splat
    /// lit by one is not dark; one facing along the bone is left alone.
    #[test]
    fn a_bound_mesh_faces_out() {
        let puppet = Puppet::from_baked_toml(include_str!("../../gs_field/data/puppets/humanoid.puppet.toml")).expect("shipped humanoid");
        let (mut pos, mut nrm, mut out) = (Vec::new(), Vec::new(), Vec::new());
        for s in puppet.segments.iter().filter(|s| s.kind.is_body()) {
            for st in &s.stations {
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::TAU / 8.0;
                    let r = add(scale(s.axis_w, st.w * a.cos()), scale(s.axis_d, st.d * a.sin()));
                    let o = norm_or(r, [0.0, 0.0, 1.0]);
                    pos.push(add(st.c, r));
                    // every other one faces in, as a thinned mesh's patches do
                    nrm.push(if k % 2 == 0 { o } else { scale(o, -1.0) });
                    out.push(o);
                }
            }
        }
        // and one facing along its bone, at a station's middle edge
        let s = &puppet.segments[0];
        let along = norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
        pos.push(add(s.stations[s.stations.len() / 2].c, scale(s.axis_w, s.stations[s.stations.len() / 2].w)));
        nrm.push(along);
        out.push(along);
        let n = pos.len();
        let bound = BoundMesh::bind(&puppet, Mesh { pos, nrm, uv: vec![[0.0; 2]; n], tris: Vec::new(), source: Vec::new() });
        let facing_out = bound.mesh.nrm.iter().zip(&out).filter(|(n, o)| dot(**n, **o) > 0.0).count();
        assert!(facing_out as f32 > n as f32 * 0.97, "{facing_out} of {n} face out");
        assert_eq!(bound.mesh.nrm[n - 1], along, "one along its bone is left");
    }

    #[test]
    fn a_thinned_cloud_keeps_the_heaviest_splats_in_order() {
        let g = |alpha: f32, size: f32| crate::splat::Gaussian { c: [0.0; 3], axes: [[size, 0.0, 0.0], [0.0, size, 0.0], [0.0, 0.0, 0.001]], n: [0.0, 0.0, 1.0], rgb: [0.0; 3], alpha, glow: 0.0, wet: 0.0, film: false, rim: 0.0 };
        // weights: 0.9*4=3.6, 0.1*4=0.4, 0.5*1=0.5, 1.0*0.01=0.01
        let body = SplatBody { rest: vec![g(0.9, 2.0), g(0.1, 2.0), g(0.5, 1.0), g(1.0, 0.1)], weights: vec![[(0, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)]; 4], ..Default::default() };
        let half = body.thinned(0.5);
        assert_eq!(half.len(), 2);
        assert_eq!(half.rest.iter().map(|g| g.alpha).collect::<Vec<_>>(), [0.9, 0.5], "the two heaviest, in their original order");
        assert_eq!(half.weights.len(), 2);
        assert_eq!(body.thinned(1.0).len(), 4);
        assert_eq!(body.thinned(0.0).len(), 0);
    }

    use super::*;

    fn square() -> Mesh {
        Mesh {
            pos: vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 2.0, 0.0], [-1.0, 2.0, 0.0]],
            nrm: vec![[0.0, 0.0, 1.0]; 4],
            uv: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            tris: vec![[0, 1, 2], [0, 2, 3]],
            source: Vec::new(),
        }
    }

    #[test]
    fn obj_corners_become_vertices() {
        let dir = std::env::temp_dir().join(format!("gs_puppet_mesh_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("quad.obj");
        // a quad whose two triangles share two corners; one corner has two sheet places
        std::fs::write(&path, "v -1 0 0\nv 1 0 0\nv 1 2 0\nv -1 2 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvt 0.5 0.5\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\nf 1/5/1 3/3/1 4/4/1\n").unwrap();
        let mesh = Mesh::load_obj(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(mesh.tris.len(), 2);
        assert_eq!(mesh.pos.len(), 5, "corner 1 is two vertices: it has two sheet places");
        assert_eq!(mesh.source, [0, 1, 2, 0, 3], "each knows its vertex in the file");
        // vt's v runs up the image; ours runs down it
        assert_eq!(mesh.uv[0], [0.0, 1.0]);
        assert_eq!(mesh.nrm[0], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn weights_from_a_tool_are_read_by_bone_name() {
        let puppet = Puppet::from_baked_toml(include_str!("../../gs_field/data/puppets/humanoid.puppet.toml")).expect("shipped humanoid");
        let text = bones_text(&puppet);
        let names: Vec<&str> = text.lines().filter(|l| l.starts_with("bone ")).map(|l| l.split_whitespace().nth(1).unwrap()).collect();
        assert!(names.len() > 5, "{text}");
        // every parent named is a bone too
        for line in text.lines().filter(|l| l.starts_with("bone ")) {
            let parent = line.split_whitespace().nth(2).unwrap();
            assert!(parent == "-" || names.contains(&parent), "{line}");
        }
        let dir = std::env::temp_dir().join(format!("gs_puppet_weights_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("weights.txt");
        // five shares (the least is dropped), one vertex with none
        std::fs::write(&path, format!("bone {}\nbone {}\nbone {}\nbone {}\nbone {}\nw 0:0.4 1:0.3 2:0.15 3:0.1 4:0.05\nw\n", names[0], names[1], names[2], names[3], names[4])).unwrap();
        let read = load_weights(&path, &puppet).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(read.len(), 2);
        let shares = read[0].unwrap();
        assert_eq!(puppet.segments[shares[0].0].name, names[0]);
        assert!((shares.iter().map(|s| s.1).sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((shares[0].1 - 0.4 / 0.95).abs() < 1e-5);
        assert!(read[1].is_none());
        // a vertex without shares takes the nearest segment's
        let bound = BoundMesh::with_weights(&puppet, square(), &[read[0], None]);
        assert_eq!(bound.weights[0], shares);
        assert!((bound.weights[1].iter().map(|s| s.1).sum::<f32>() - 1.0).abs() < 1e-5);
        // shares are looked up by the file's vertex
        let split = Mesh { source: vec![1, 0, 0, 1], ..square() };
        let bound = BoundMesh::with_weights(&puppet, split, &[read[0], None]);
        assert_eq!(bound.weights[1], shares);
        assert_eq!(bound.weights[2], shares);
        assert_ne!(bound.weights[0], shares);
    }

    #[test]
    fn a_twisted_joint_keeps_its_girth() {
        // a point one foot off the bone (the y axis), shared half and half
        // by a still segment and one turned 150 degrees about the bone
        let (still, turned) = (Xf::IDENTITY, Xf::about([0.0, 1.0, 0.0], rot_y(150.0)));
        let duals = [dual_of(&still), dual_of(&turned)];
        let shares = || [(&still, &duals[0], 0.5f32), (&turned, &duals[1], 0.5)].into_iter();
        let (p, n) = ([1.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let off_bone = |q: V3| q[0].hypot(q[2]);
        let (linear, _) = blend_point(p, n, shares(), Blend::Linear);
        let (dual, facing) = blend_point(p, n, shares(), Blend::DualQuat);
        assert!(off_bone(linear) < 0.3, "the mean of the two places caves in: {linear:?}");
        assert!((off_bone(dual) - 1.0).abs() < 1e-4, "{dual:?}");
        assert!((dual[1] - 1.0).abs() < 1e-4);
        // half way round, and facing outward still
        let half = mul_mv(&rot_y(75.0), p);
        assert!(len(sub(dual, half)) < 1e-4, "{dual:?} vs {half:?}");
        assert!(dot(facing, norm_or([dual[0], 0.0, dual[2]], n)) > 0.999);
        // one segment alone: exactly its transform, shift and all
        let moved = Xf { r: rot_z(40.0), t: [0.3, -0.2, 0.5] };
        let d = dual_of(&moved);
        let (alone, _) = blend_point(p, n, [(&moved, &d, 1.0f32)].into_iter(), Blend::DualQuat);
        assert!(len(sub(alone, moved.point(p))) < 1e-4);
    }

    #[test]
    fn a_frame_puts_vertices_where_posing_them_does() {
        let puppet = Puppet::from_baked_toml(include_str!("../../gs_field/data/puppets/humanoid.puppet.toml")).expect("shipped humanoid");
        // a few points about the body: by the chest, an elbow, a knee
        let at = |name: &str, t: f32| {
            let seg = puppet.segments.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no {name}"));
            add(lerp(seg.head, seg.tail, t), [0.1, 0.05, 0.2])
        };
        let pos = vec![at("torso_upper", 0.5), at("arm_r.0", 0.95), at("leg_l.0", 0.9), at("skull", 0.5)];
        let mesh = Mesh { nrm: vec![[0.0, 0.0, 1.0]; 4], uv: vec![[0.0; 2]; 4], tris: vec![[0, 1, 2], [0, 2, 3]], source: Vec::new(), pos };
        let body = std::sync::Arc::new(BodyMesh { bound: BoundMesh::bind(&puppet, mesh), paint: Paint { w: 1, h: 1, rgba: vec![255; 4] }, normal: None, splats: None, made: Default::default(), thinned: Default::default(), made_count: MADE_SPLATS, paint_lands: true });
        let pose = crate::Pose::default();
        let posed = puppet.pose(&pose);
        let camera = crate::Camera::framing(puppet.height * 1.3, [400.0, 400.0]);
        let project = camera.projector();
        let yaw = 35.0;
        let frame = BodyFrame::new(&body, &posed, &project, yaw, None).expect("a frame");
        let (moved, _) = body.bound.posed(&posed);
        for (i, p) in moved.iter().enumerate() {
            let turned = mul_mv(&rot_y(yaw), *p);
            let (want, depth) = project(turned);
            let (world, screen, d) = frame.place(i);
            assert!(len(sub(world, turned)) < 1e-3, "vertex {i}: {world:?} vs {turned:?}");
            assert!((screen[0] - want[0]).abs() < 0.05 && (screen[1] - want[1]).abs() < 0.05, "vertex {i}: {screen:?} vs {want:?}");
            assert!((d - depth).abs() < 1e-3);
            // inside the image and its depth span
            assert!(screen[0] >= frame.origin[0] && screen[0] <= frame.origin[0] + frame.w as f32);
            assert!(screen[1] >= frame.origin[1] && screen[1] <= frame.origin[1] + frame.h as f32);
            assert!(d >= frame.depth[0] && d <= frame.depth[1]);
        }
        // another pose is another frame
        let other = BodyFrame::new(&body, &puppet.pose_as_drawn(&pose), &project, yaw, None).expect("a frame");
        assert_ne!(frame.key, other.key);
        // a cloud rides the mesh: each Gaussian takes the vertex nearest it
        let g = |c: V3| crate::splat::Gaussian { c, axes: [[0.01, 0.0, 0.0], [0.0, 0.01, 0.0], [0.0, 0.0, 0.01]], n: [0.0; 3], rgb: [200.0; 3], alpha: 0.9, glow: 0.0, wet: 0.0, film: false, rim: 0.0 };
        let near = add(body.bound.mesh.pos[1], [0.01, 0.0, 0.0]);
        let cloud = SplatBody::bind(&body.bound, vec![g(near), g([40.0, 40.0, 40.0])]);
        assert_eq!(cloud.len(), 1, "the stray is dropped");
        assert_eq!(cloud.weights[0], body.bound.weights[1]);
        assert_eq!(cloud.rest[0].n, body.bound.mesh.nrm[1]);
        // a made cloud keeps its facing, and its strays
        let mut made = g(near);
        made.n = [0.0, 1.0, 0.0];
        let kept = SplatBody::bind_facing(&body.bound, vec![made, g([40.0, 40.0, 40.0])], true);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept.rest[0].n, [0.0, 1.0, 0.0]);
        let drawn = BodyFrame::new(&body, &posed, &project, yaw, Some(std::sync::Arc::new(kept))).expect("a frame");
        assert_ne!(drawn.key, frame.key);
        // its splats are ordered from the farthest: the stray, far behind
        // or far in front, is at one end
        assert_eq!(drawn.order.len(), 2);
        let depth = |i: usize| {
            let c = drawn.cloud.as_ref().unwrap().rest[i].c;
            project(mul_mv(&rot_y(yaw), posed.xf[0].point(c))).1
        };
        let _ = depth;
        let mut sorted = drawn.order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1]);
        // splats made from the mesh: about as many as asked, on its surface,
        // each with a corner's shares and the sheet's colour
        let made = SplatBody::from_mesh(&body.bound, &body.paint, 400);
        assert!(made.len() > 300 && made.len() < 600, "{}", made.len());
        assert_eq!(made.rest.len(), made.weights.len());
        assert!(made.rest.iter().all(|g| g.rgb == [255.0; 3] && g.alpha > 0.9));
        assert!(made.weights.iter().all(|w| body.bound.weights.contains(w)));
        assert!(std::sync::Arc::ptr_eq(&body.made_splats(), &body.made_splats()), "made once");
        // and a cloud of known depths comes out far to near
        let line = SplatBody {
            rest: (0..50).map(|k| g([0.0, 0.0, ((k * 7) % 50) as f32 * 0.1])).collect(),
            weights: vec![[(0, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)]; 50],
            ..Default::default()
        };
        let still = [[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]];
        // depth = 10 - z: the smallest z is the farthest
        let order = line.far_to_near(&still, [0.0, 0.0, -1.0, 10.0], [5.0, 10.0]);
        let zs: Vec<f32> = order.iter().map(|i| line.rest[*i as usize].c[2]).collect();
        assert!(zs.windows(2).all(|w| w[0] <= w[1]), "{zs:?}");
    }

    #[test]
    fn rings_are_measured_from_the_mesh() {
        let puppet = Puppet::from_baked_toml(include_str!("../../gs_field/data/puppets/humanoid.puppet.toml")).expect("shipped humanoid");
        // a mesh that is the puppet's own rings, a third wider and half as deep
        let mut mesh = Mesh::default();
        for s in puppet.segments.iter().filter(|s| s.kind.is_body() && !s.hidden) {
            for st in &s.stations {
                for j in 0..24 {
                    let (sin, cos) = (j as f32 / 24.0 * std::f32::consts::TAU).sin_cos();
                    mesh.pos.push(add(st.c, add(scale(s.axis_w, st.w * 1.3 * cos), scale(s.axis_d, st.d * 0.5 * sin))));
                    mesh.nrm.push([0.0, 0.0, 1.0]);
                    mesh.uv.push([0.0; 2]);
                }
            }
        }
        // each vertex held by the segment whose ring it was made from
        let seg: Vec<usize> = puppet.segments.iter().enumerate().filter(|(_, s)| s.kind.is_body() && !s.hidden).flat_map(|(i, s)| vec![i; s.stations.len() * 24]).collect();
        let weights = seg.iter().map(|s| [(*s, 1.0), (*s, 0.0), (*s, 0.0), (*s, 0.0)]).collect();
        let bound = BoundMesh { mesh, seg, weights };
        let measured = bound.measured(&puppet);
        let leg = puppet.segments.iter().position(|s| s.name == "leg_l.0").expect("a thigh");
        let (was, now) = (&puppet.segments[leg].stations, &measured.segments[leg].stations);
        assert_eq!(was.len(), now.len());
        let mid = was.len() / 2;
        assert!((now[mid].w / was[mid].w - 1.3).abs() < 0.15, "width {} -> {}", was[mid].w, now[mid].w);
        assert!((now[mid].d / was[mid].d - 0.5).abs() < 0.12, "depth {} -> {}", was[mid].d, now[mid].d);
        assert!(len(sub(now[mid].c, was[mid].c)) < 0.02, "the ring stays on its bone");
        // nothing else about the puppet changed
        assert_eq!(measured.segments.len(), puppet.segments.len());
        assert_eq!(measured.segments[leg].head, puppet.segments[leg].head);
    }

    #[test]
    fn fits_to_a_height_standing_on_the_ground() {
        let mut mesh = square();
        mesh.pos.iter_mut().for_each(|p| *p = add(*p, [3.0, 5.0, -2.0]));
        mesh.fit_to_height(6.0);
        let ys: Vec<f32> = mesh.pos.iter().map(|p| p[1]).collect();
        assert!(ys.iter().cloned().fold(f32::MAX, f32::min).abs() < 1e-5);
        assert!((ys.iter().cloned().fold(f32::MIN, f32::max) - 6.0).abs() < 1e-4);
        assert!(mesh.pos.iter().map(|p| p[0]).sum::<f32>().abs() < 1e-4, "centred on x");
    }

    #[test]
    fn renders_its_paint_inside_its_outline() {
        let mesh = square();
        // left half red, right half blue
        let paint = Paint { w: 2, h: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] };
        let project = |p: V3| ([p[0] * 20.0 + 50.0, 60.0 - p[1] * 20.0], -p[2]);
        let image = render(&mesh, &mesh.pos, &mesh.nrm, Some(&paint), &project, 0.0);
        let at = |x: f32, y: f32| {
            let (ix, iy) = ((x - image.origin[0]) as usize, (y - image.origin[1]) as usize);
            <[u8; 4]>::try_from(&image.rgba[(iy * image.w + ix) * 4..][..4]).unwrap()
        };
        let (left, right) = (at(35.0, 40.0), at(65.0, 40.0));
        assert_eq!(left[3], 255);
        assert!(left[0] > 150 && left[2] < 40, "{left:?}");
        assert!(right[2] > 150 && right[0] < 40, "{right:?}");
        assert_eq!(at(image.origin[0] + 0.5, image.origin[1] + 0.5)[3], 0, "outside the square");
    }
}
