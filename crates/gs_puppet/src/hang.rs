//! Arms drawn raised hang before a pose is applied.
//!
//! A sheet drawn in an A-pose shows the armpit and the inner arm, which a
//! sheet with the arms hanging hides against the body -- the builder then
//! measures them instead of guessing. But the pose files are written for a
//! hanging arm (`data/poses/*.toml`: "for a limb hanging down"), so a
//! raised arm is turned down first: about its shoulder, across the body,
//! to where it hangs just clear of the body beside it ([`Puppet::arm_hangs`]).
//! The rest stays as drawn -- the fit against the silhouettes, the paintings
//! projected onto the skin and its UV sheet all read the drawn figure
//! ([`Puppet::pose_as_drawn`]); only posing starts from the hanging arm.
//!
//! Only a rig that says so (`arms_raised`) hangs its arms: a sheet drawn
//! with them hanging often holds them out from a broad body, and that is
//! where they hang.

use std::collections::HashSet;

use crate::math::{add, axis_angle, len, mul_mv, sub, M3, V3};
use crate::puppet::{Kind, Puppet};

/// How far out a raised arm's upper arm is taken to hang when the builder
/// works out where the torso ends beside it, degrees from straight down:
/// the old arms-down humanoid sheet's (13), so the chest keeps the width it
/// had there, and the flesh up to the shoulder goes with the arm.
pub const TRUNK_CLIP_HANG_DEG: f32 = 12.0;

/// A drawn arm this much further out than it can hang is turned down.
pub const HANG_MIN_DEG: f32 = 6.0;
/// How far a hanging arm keeps clear of the body beside it, feet.
const CLEAR_FT: f32 = 0.02;
/// The shoulder end of the upper arm meets the body however the arm
/// hangs: its stations nearer the shoulder than this share of the segment
/// are not tested.
const SHOULDER_SHARE: f32 = 0.6;
/// Degrees a step while turning an arm down.
const STEP_DEG: f32 = 0.5;

impl Puppet {
    /// Every arm drawn raised, turned down to hang, when the rig says the
    /// sheet was drawn so ([`Puppet::arms_raised`]; none otherwise): (its
    /// first segment, the turn about that segment's head, in rest space). An arm is a limb
    /// whose part is an arm (`rightArm`, `leftArm`), first in its chain;
    /// what hangs from it (forearm, hand) turns with it. It turns about the
    /// front axis, down toward the body, until one step more would bring
    /// any of it within [`CLEAR_FT`] of the body beside it (the trunk and
    /// the legs, at the same height and depth); an arm that would turn less
    /// than [`HANG_MIN_DEG`] is drawn hanging and gets none.
    pub fn arm_hangs(&self) -> Vec<(usize, M3)> {
        if !self.arms_raised {
            return Vec::new();
        }
        let segs = &self.segments;
        let is_arm = |i: usize| {
            let s = &segs[i];
            s.chain_index == 0 && matches!(s.kind, Kind::Limb) && s.part.as_deref().is_some_and(|p| p.ends_with("Arm"))
        };
        let arms: Vec<usize> = (0..segs.len()).filter(|&i| is_arm(i)).collect();
        let in_arms: HashSet<usize> = arms.iter().flat_map(|&a| self.subtree(a)).collect();
        // what an arm must clear: body that is not an arm or what hangs from one
        let body: Vec<usize> = (0..segs.len()).filter(|&i| segs[i].kind.is_body() && !in_arms.contains(&i)).collect();
        let mut out = Vec::new();
        for &a in &arms {
            let chain = self.subtree(a);
            let pivot = segs[a].head;
            let end = chain
                .iter()
                .flat_map(|&k| segs[k].stations.iter().map(|st| st.c))
                .max_by(|p, q| len(sub(*p, pivot)).total_cmp(&len(sub(*q, pivot))));
            let Some(end) = end else { continue };
            let v = sub(end, pivot);
            if v[1] >= 0.0 {
                continue;
            }
            let side = if v[0] < 0.0 { -1.0 } else { 1.0 };
            // how far out the drawn arm is: 0 = straight down, across the body
            let drawn = v[0].abs().atan2(-v[1]).to_degrees();
            let turned = |deg: f32| axis_angle([0.0, 0.0, 1.0], side * (deg - drawn));
            let clear = |r: &M3| self.arm_clear(&chain, a, pivot, r, side, &body);
            if !clear(&turned(drawn)) {
                continue;
            }
            let mut hang = drawn;
            while hang - STEP_DEG > 0.0 && clear(&turned(hang - STEP_DEG)) {
                hang -= STEP_DEG;
            }
            if drawn - hang >= HANG_MIN_DEG {
                out.push((a, turned(hang)));
            }
        }
        out
    }

    /// Does the arm `chain` (first segment `root`), turned by `r` about
    /// `pivot`, keep [`CLEAR_FT`] from every `body` station beside it?
    fn arm_clear(&self, chain: &[usize], root: usize, pivot: V3, r: &M3, side: f32, body: &[usize]) -> bool {
        let segs = &self.segments;
        let along = |k: usize, c: V3| {
            let (h, t) = (segs[k].head, segs[k].tail);
            let d = sub(t, h);
            let l2 = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).max(1e-9);
            ((c[0] - h[0]) * d[0] + (c[1] - h[1]) * d[1] + (c[2] - h[2]) * d[2]) / l2
        };
        // an ellipse's reach along x and z, its axes turned by `m`
        let reach = |w: f32, d: f32, aw: V3, ad: V3, k: usize| ((w * aw[k]).powi(2) + (d * ad[k]).powi(2)).sqrt();
        for &k in chain {
            let s = &segs[k];
            let (aw, ad) = (mul_mv(r, s.axis_w), mul_mv(r, s.axis_d));
            for (j, st) in s.stations.iter().enumerate() {
                if k == root && (j < s.bare || along(k, st.c) < SHOULDER_SHARE) {
                    continue;
                }
                let c = add(pivot, mul_mv(r, sub(st.c, pivot)));
                let inner = side * c[0] - reach(st.w, st.d, aw, ad, 0);
                let (z, zr) = (c[2], reach(st.w, st.d, aw, ad, 2));
                for &b in body {
                    let o = &segs[b];
                    for os in &o.stations {
                        if (os.c[1] - c[1]).abs() > 0.1 || side * os.c[0] < -0.05 {
                            continue;
                        }
                        let (ox, oz) = (reach(os.w, os.d, o.axis_w, o.axis_d, 0), reach(os.w, os.d, o.axis_w, o.axis_d, 2));
                        if (os.c[2] - z).abs() > oz + zr {
                            continue;
                        }
                        if inner - (side * os.c[0] + ox) < CLEAR_FT {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rest_iou, Pose};

    fn humanoid(raised: bool) -> Puppet {
        let mut rig = rig::Rig::from_toml(include_str!("../../../data/rigs/humanoid.rig.toml")).expect("humanoid rig");
        rig.arms_raised = raised;
        let outlines = rig::Outlines::from_json(include_str!("../../../data/outlines.json")).expect("outlines");
        Puppet::build(&rig, &outlines).expect("builds")
    }

    /// How far out an arm's upper segment points, degrees from straight down.
    fn out_deg(p: &crate::Posed, bone: &str) -> f32 {
        let i = p.puppet.segments.iter().position(|s| s.bone == bone && s.chain_index == 0).unwrap();
        let s = &p.puppet.segments[i];
        let (h, t) = (p.point(i, s.head), p.point(i, s.tail));
        (t[0] - h[0]).abs().atan2(h[1] - t[1]).to_degrees()
    }

    /// The humanoid's A-pose sheet: its arms, drawn some 30 degrees out,
    /// hang within a few degrees of the old arms-down sheet's 13 -- clear
    /// of the body -- for every pose; the fit against the sheet still sees
    /// them raised.
    #[test]
    fn raised_arms_hang_for_posing() {
        let p = humanoid(true);
        assert_eq!(p.arm_hang.len(), 2, "both arms");
        let drawn = p.pose_as_drawn(&Pose::default());
        let posed = p.pose(&Pose::default());
        for bone in ["arm_r", "arm_l"] {
            let (d, h) = (out_deg(&drawn, bone), out_deg(&posed, bone));
            assert!(d > 25.0, "{bone} drawn {d}");
            assert!((4.0..16.0).contains(&h), "{bone} hangs at {h}");
        }
        let unhung = humanoid(false);
        assert!(unhung.arm_hang.is_empty(), "only when the rig says so");
        let outlines = rig::Outlines::from_json(include_str!("../../../data/outlines.json")).unwrap();
        let mask = |view| rig_bake::rasterize(&outlines.get(&p.family, view).unwrap().outline(), crate::MASK_PX);
        let mut still = p.clone();
        still.arm_hang.clear();
        for view in [rig::View::Front, rig::View::Side] {
            assert_eq!(rest_iou(&p, view, &mask(view)), rest_iou(&still, view, &mask(view)), "the fit sees the sheet");
        }
        // a hanging arm keeps clear of the body
        let hung: Vec<(usize, M3)> = p.arm_hang.clone();
        let body: Vec<usize> = (0..p.segments.len()).filter(|&i| p.segments[i].kind.is_body() && !hung.iter().any(|(a, _)| p.subtree(*a).contains(&i))).collect();
        for (a, r) in &hung {
            let side = if p.segments[*a].tail[0] < 0.0 { -1.0 } else { 1.0 };
            assert!(p.arm_clear(&p.subtree(*a), *a, p.segments[*a].head, r, side, &body));
        }
        // the baked puppet keeps the flag, and its arms hang the same
        let back = Puppet::from_baked(&p.to_baked()).unwrap();
        assert!(back.arms_raised && back.arm_hang.len() == 2);
    }
}
