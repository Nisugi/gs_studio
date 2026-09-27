//! Flesh shared at the joints.
//!
//! Each station ring is measured along one bone, and posed rigidly with it
//! the rings of two bones met at a joint in a hard fold: a knee or an ankle
//! bent kinked instead of rounding, flesh pushed through the inside of the
//! bend, the thigh's top rings pulled out of the pelvis when the legs spread
//! (the lump under the crotch), a shoulder's cap stood proud of the
//! collarbone. Here the rings near a joint turn part of the way with the
//! bone across it, about the joint itself: half and half right at the joint,
//! fading to their own bone's turn a joint's thickness away. The bend is
//! spread over a stretch of flesh as a real joint's is (Niffy's "joint
//! string": one hinge's bend shared out over the flesh round it).
//!
//! Along a limb (hip, knee, ankle; shoulder, elbow, wrist, hand) both sides
//! share. Where a limb leaves the trunk (a thigh at the pelvis, an arm at
//! its collarbone, the neck), only the limb's rings do: the trunk has several
//! limbs to follow and follows none. At rest, or wherever the two bones
//! turn alike, a shared ring is exactly where its own bone puts it.

use crate::math::{dot, len, mul_mv, norm_or, quat_from_m3, quat_to_m3, slerp, sub, Xf};
use crate::pose::Posed;
use crate::puppet::Puppet;

/// How far from a joint its flesh shares the bend, in the joint's own
/// thickness (the radius of the ring nearest it), at least
/// [`SHARE_MIN_FT`] and at most [`SHARE_MAX_OF_LENGTH`] of the bone.
const SHARE_REACH: f32 = 1.2;
const SHARE_MIN_FT: f32 = 0.06;
const SHARE_MAX_OF_LENGTH: f32 = 0.45;
/// A segment's end this close to another's counts as the joint between them.
const JOINT_FT: f32 = 0.05;

/// Who a station ring shares its bend with: the segment across the joint
/// (or, with `pair`, the halfway turn between two -- a crotch between the
/// thighs), how much of that turn it takes (0..1), and the joint it turns
/// about, rest space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Share {
    pub with: usize,
    pub pair: Option<usize>,
    pub weight: f32,
    pub joint: [f32; 3],
}

/// Every body segment's rings' [`Share`]s, station by station (None: the
/// ring turns with its own bone alone).
pub fn shares(p: &Puppet) -> Vec<Vec<Option<Share>>> {
    let segs = &p.segments;
    let body = |i: usize| segs[i].kind.is_body();
    (0..segs.len())
        .map(|i| {
            let s = &segs[i];
            if !body(i) {
                return vec![None; s.stations.len()];
            }
            let axis = norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
            let length = len(sub(s.tail, s.head));
            // the parent's side: every body segment hanging off another
            let head = s.parent.filter(|&o| body(o));
            // the child's side: the one body segment going on from this one's
            // end along the same limb (or the only one there)
            let at_tail: Vec<usize> = (0..segs.len()).filter(|&c| segs[c].parent == Some(i) && body(c) && len(sub(segs[c].head, s.tail)) < JOINT_FT).collect();
            let tail = match at_tail.as_slice() {
                [c] => Some(*c),
                many => many.iter().copied().find(|&c| segs[c].bone == s.bone),
            }
            // a trunk follows none of the limbs leaving it
            .filter(|&c| !(matches!(s.kind, crate::Kind::Trunk) && !matches!(segs[c].kind, crate::Kind::Trunk)));
            let reach = |joint: [f32; 3]| {
                let near = s.stations.iter().min_by(|a, b| len(sub(a.c, joint)).total_cmp(&len(sub(b.c, joint))));
                let r = near.map_or(0.0, |st| st.w.max(st.d));
                (SHARE_REACH * r).clamp(SHARE_MIN_FT, (SHARE_MAX_OF_LENGTH * length).max(SHARE_MIN_FT))
            };
            let (rh, rt) = (reach(s.head), reach(s.tail));
            // a trunk with two legs: its rings below the hips (the crotch,
            // the seat) follow the thighs half way as they swing -- left
            // behind, they hung down between the legs of a seated figure
            let legs: Vec<usize> = (0..segs.len())
                .filter(|&c| segs[c].parent == Some(i) && segs[c].chain_index == 0 && segs[c].part.as_deref().is_some_and(|p| p.ends_with("Leg")))
                .collect();
            let crotch = (matches!(s.kind, crate::Kind::Trunk) && legs.len() == 2).then(|| {
                let (a, b) = (segs[legs[0]].head, segs[legs[1]].head);
                let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0];
                let thick = segs[legs[0]].stations.iter().map(|st| st.w.max(st.d)).fold(0.0, f32::max).max(SHARE_MIN_FT);
                (legs[0], legs[1], mid, thick)
            });
            s.stations
                .iter()
                .map(|st| {
                    let from_head = dot(sub(st.c, s.head), axis);
                    let from_tail = dot(sub(s.tail, st.c), axis);
                    // half and half at the joint, fading to its own bone
                    // ahead of it; a ring BEHIND the joint (a shoulder's cap
                    // over the pivot, a thigh's top in the pelvis, a shin's
                    // end in the knee) goes over to the bone it is tucked
                    // into -- turned with its own, it swung out as a lump
                    let smooth = |t: f32| {
                        let t = t.clamp(0.0, 1.0);
                        t * t * (3.0 - 2.0 * t)
                    };
                    let fade = |d: f32, r: f32| if d >= 0.0 { 0.5 * (1.0 - smooth(d / r)) } else { 0.5 + 0.5 * smooth(-d / r) };
                    let h = head.map(|o| Share { with: o, pair: None, weight: fade(from_head, rh), joint: s.head });
                    let t = tail.map(|o| Share { with: o, pair: None, weight: fade(from_tail, rt), joint: s.tail });
                    let c = crotch.and_then(|(a, b, mid, thick)| {
                        let below = mid[1] - st.c[1];
                        (below > 0.0).then(|| Share { with: a, pair: Some(b), weight: smooth(below / thick), joint: mid })
                    });
                    [h, t, c].into_iter().flatten().filter(|x| x.weight > 0.005).max_by(|a, b| a.weight.total_cmp(&b.weight))
                })
                .collect()
        })
        .collect()
}

impl Posed<'_> {
    /// Where station ring `share` of segment `seg` is posed: its own bone's
    /// transform, or with a [`Share`] one turned about the joint part of the
    /// way toward the bone across it.
    pub fn ring_xf(&self, seg: usize, share: Option<Share>) -> Xf {
        let a = self.xf[seg];
        let Some(sh) = share else { return a };
        let mut qb = quat_from_m3(&self.xf[sh.with].r);
        if let Some(p) = sh.pair {
            qb = slerp(qb, quat_from_m3(&self.xf[p].r), 0.5);
        }
        let at = a.point(sh.joint);
        let r = quat_to_m3(slerp(quat_from_m3(&a.r), qb, sh.weight));
        Xf { r, t: sub(at, mul_mv(&r, sh.joint)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pose, PoseSet, SHIPPED_POSES};

    fn humanoid() -> Puppet {
        let rig = rig::Rig::from_toml(include_str!("../../../data/rigs/humanoid.rig.toml")).expect("humanoid rig");
        let outlines = rig::Outlines::from_json(include_str!("../../../data/outlines.json")).expect("outlines");
        Puppet::build(&rig, &outlines).expect("builds")
    }

    /// A knee shares its bend: the rings either side of it turn about half
    /// way, those a joint's thickness off turn with their own bone; the
    /// trunk shares with no limb; at rest nothing moves.
    #[test]
    fn a_bent_knee_rounds() {
        let p = humanoid();
        let sh = shares(&p);
        let shin = p.segments.iter().position(|s| s.bone == "leg_l" && s.chain_index == 1).unwrap();
        let thigh = p.segments[shin].parent.unwrap();
        let first = sh[shin].iter().flatten().next().expect("the shin shares");
        assert!(first.with == thigh && first.joint == p.segments[shin].head && first.weight > 0.3);
        assert!(sh[shin].iter().any(|x| x.is_none()), "the middle of the shin is its own");
        assert!(sh[shin].last().unwrap().is_some_and(|x| x.joint == p.segments[shin].tail), "and it shares the ankle");
        assert!(sh[thigh].iter().flatten().any(|x| x.with == shin && x.joint == p.segments[thigh].tail), "the thigh shares toward the knee");
        // the trunk follows no single limb; below the hips it follows both thighs
        let torso = p.segments.iter().position(|s| s.bone == "torso_lower").unwrap();
        for x in sh[torso].iter().flatten() {
            assert!(matches!(p.segments[x.with].kind, crate::Kind::Trunk) || x.pair.is_some(), "{x:?}");
        }
        assert!(sh[torso].iter().flatten().any(|x| x.pair.is_some()), "the crotch follows the thighs");
        // at rest a shared ring sits where its own bone puts it
        let rest = p.pose(&Pose::default());
        for (k, s) in sh[shin].iter().enumerate() {
            let (a, b) = (rest.xf[shin].point(p.segments[shin].stations[k].c), rest.ring_xf(shin, *s).point(p.segments[shin].stations[k].c));
            assert!(len(sub(a, b)) < 1e-4, "{a:?} vs {b:?}");
        }
        // kneeling, the ring at the knee turns half way between thigh and shin
        let set = PoseSet::from_toml(SHIPPED_POSES.iter().find(|(n, _)| *n == "biped").unwrap().1).unwrap();
        let kneel = p.pose(set.get("kneel").unwrap());
        let k0 = sh[shin].iter().position(|s| s.is_some()).unwrap();
        let r = kneel.ring_xf(shin, sh[shin][k0]);
        let up = |x: &Xf| mul_mv(&x.r, [0.0, 1.0, 0.0]);
        let (u_thigh, u_shin, u_ring) = (up(&kneel.xf[thigh]), up(&kneel.xf[shin]), up(&r));
        let angle = |a: [f32; 3], b: [f32; 3]| dot(a, b).clamp(-1.0, 1.0).acos();
        assert!(angle(u_ring, u_thigh) > 0.1 && angle(u_ring, u_shin) > 0.1, "between the two");
    }
}
