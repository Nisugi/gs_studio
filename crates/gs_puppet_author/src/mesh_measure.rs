//! A modelled body measured (authoring): its skeleton written out for a
//! tool that works out its weights ([`bones_text`], `tools/mesh_weights.py`),
//! and its puppet's rings measured from the mesh ([`BoundMesh::measured`],
//! `mesh_pose`'s `MESH_MEASURE`). A client binds and poses a body
//! (`mesh`); it never measures one.

use gs_puppet::math::*;
use gs_puppet::mesh::BoundMesh;
use gs_puppet::model::Puppet;

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

/// A ring needs this many of the mesh's vertices in its slab to be measured
/// from them.
const RING_MIN_VERTICES: usize = 8;
/// The share of a ring's vertices left out at each end of its width and
/// depth: a stray (a hair's tip, a seam's fold) does not set a ring's size.
const RING_TRIM: f32 = 0.02;

/// A bound mesh's puppet measured: `bound.measured(&puppet)` with this in
/// scope (`gs_puppet_author::prelude`).
pub trait MeasureMesh {
    fn measured(&self, puppet: &Puppet) -> Puppet;
}

impl MeasureMesh for BoundMesh {
    /// `puppet` (the one this is bound to) with its rings measured from the
    /// mesh: each ring's centre, half-width and half-depth are those of the
    /// vertices its segment holds, in the slab of the bone that ring stands
    /// for. A puppet measured from two flat outlines guesses each ring's
    /// depth from one row of the side view; the mesh has it.
    ///
    /// The skeleton, the segments and their rings' count and places along
    /// the bones stay: a pose, a wound's address and the baked format mean
    /// what they did. A ring with too few vertices keeps its measure.
    ///
    /// A trunk ring between an arm's elbow and its shoulder ends sideways
    /// at that shoulder joint: what lies past it is the arm, whatever the
    /// weights say. A raised arm's inside, where the sleeve meets the side,
    /// is heat-weighted to the chest on some meshes (half_elf_male's, out
    /// to 0.9 ft), and a chest measured round it was so wide (0.73 ft a
    /// side, against the half-elf female's 0.47) that the arm could not
    /// hang beside it (`hang`) and stood out as modelled. And an arm's ring
    /// keeps to its side of its shoulder joint: the side of the chest the
    /// weights give an arm (in to 0.23 ft of the middle) pulled its rings
    /// 0.13 ft in toward the body, and on a broad chest a hanging arm met
    /// the chest while still well out.
    fn measured(&self, puppet: &Puppet) -> Puppet {
        let mut out = puppet.clone();
        let mut of_seg: Vec<Vec<u32>> = vec![Vec::new(); puppet.segments.len()];
        for (v, s) in self.seg.iter().enumerate() {
            of_seg[*s].push(v as u32);
        }
        let slab = gs_puppet::model::STATION_STEP_FT * 0.6;
        // each arm's shoulder joint, and its elbow's height
        let shoulders: Vec<(V3, f32)> = puppet
            .segments
            .iter()
            .filter(|s| s.kind == gs_puppet::Kind::Limb && s.chain_index == 0 && s.part.as_deref().is_some_and(|p| p.ends_with("Arm")))
            .map(|s| (s.head, s.tail[1]))
            .collect();
        for (i, seg) in out.segments.iter_mut().enumerate() {
            if !seg.kind.is_body() || seg.hidden || of_seg[i].len() < RING_MIN_VERTICES {
                continue;
            }
            let axis = norm_or(sub(seg.tail, seg.head), [0.0, 1.0, 0.0]);
            let (aw, ad) = (seg.axis_w, seg.axis_d);
            let mut found: Vec<Option<(V3, f32, f32)>> = Vec::with_capacity(seg.stations.len());
            let trunk = seg.kind == gs_puppet::Kind::Trunk;
            // an arm: the shoulder joint on its side, nothing nearer the
            // body's middle than it
            let arm_side = (seg.kind == gs_puppet::Kind::Limb && seg.part.as_deref().is_some_and(|p| p.ends_with("Arm")))
                .then(|| shoulders.iter().find(|(at, _)| at[0] * seg.head[0] > 0.0).map(|(at, _)| at[0]))
                .flatten();
            for st in &seg.stations {
                // how far sideways a trunk ring here reaches at most: to the
                // shoulder joint of an arm beside it
                let walls: Vec<f32> = if trunk {
                    shoulders.iter().filter(|(at, elbow)| st.c[1] > *elbow && st.c[1] < at[1]).map(|(at, _)| dot(sub(*at, st.c), aw)).collect()
                } else {
                    Vec::new()
                };
                let (mut us, mut vs): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
                for &v in &of_seg[i] {
                    let e = sub(self.mesh.pos[v as usize], st.c);
                    if dot(e, axis).abs() <= slab {
                        let u = dot(e, aw);
                        if walls.iter().any(|&w| (w > 0.0 && u > w) || (w < 0.0 && u < w)) {
                            continue;
                        }
                        if arm_side.is_some_and(|x| self.mesh.pos[v as usize][0] * x.signum() < x.abs()) {
                            continue;
                        }
                        us.push(u);
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
                    seg.stations[k] = gs_puppet::model::Station { c, w: (was[k].w * w / was[j].w.max(0.02)).max(0.02), d: (was[k].d * d / was[j].d.max(0.02)).max(0.02) };
                    continue;
                }
                let near: Vec<(f32, f32, f32, f32)> = (k.saturating_sub(1)..=(k + 1).min(n - 1)).filter_map(|j| calm[j]).collect();
                let m = near.len() as f32;
                let (w, d, du, dv) = near.iter().fold((0.0, 0.0, 0.0, 0.0), |a, x| (a.0 + x.0 / m, a.1 + x.1 / m, a.2 + x.2 / m, a.3 + x.3 / m));
                let c = add(was[k].c, add(scale(aw, du), scale(ad, dv)));
                seg.stations[k] = gs_puppet::model::Station { c, w: w.max(0.02), d: d.max(0.02) };
            }
        }
        // what a puppet works out from its rings (how its arms hang) again
        Puppet::from_baked(&out.to_baked()).unwrap_or(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gs_puppet::mesh::Mesh;

    #[test]
    fn the_bones_name_every_parent() {
        let puppet = Puppet::from_baked_toml(gs_puppet::fixtures::HUMANOID).expect("shipped humanoid");
        let text = bones_text(&puppet);
        let names: Vec<&str> = text.lines().filter(|l| l.starts_with("bone ")).map(|l| l.split_whitespace().nth(1).unwrap()).collect();
        assert!(names.len() > 5, "{text}");
        for line in text.lines().filter(|l| l.starts_with("bone ")) {
            let parent = line.split_whitespace().nth(2).unwrap();
            assert!(parent == "-" || names.contains(&parent), "{line}");
        }
    }

    #[test]
    fn rings_are_measured_from_the_mesh() {
        let puppet = Puppet::from_baked_toml(gs_puppet::fixtures::HUMANOID).expect("shipped humanoid");
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
}
